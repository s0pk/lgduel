//! Player movement. Behaviour follows the Quake 3 / Quake Live movement model
//! (ground friction + acceleration, air acceleration, trapezoidal gravity,
//! clip-plane slide move, step-up), implemented from its documented behaviour.
//! Constants come from `PhysicsConfig`; see DECISIONS D3.

use crate::config::PhysicsConfig;
use crate::math::{Vec3, angle_vectors};
use crate::trace::TraceResult;
use crate::usercmd::{MOVE_MAX, UserCmd, buttons};
use crate::world::World;
use serde::{Deserialize, Serialize};

/// Velocity clip overbounce: pushes slightly off planes so we don't re-hit them.
const OVERCLIP: f32 = 1.001;
/// Ground planes steeper than this (normal.z below) are not walkable.
const MIN_WALK_NORMAL: f32 = 0.7;
const MAX_CLIP_PLANES: usize = 5;
const NUM_BUMPS: usize = 4;
/// How far below the hull we look for ground.
const GROUND_PROBE: f32 = 0.25;

/// Movement-relevant player state. Everything here is predicted by the client.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct PlayerMove {
    pub origin: Vec3,
    pub velocity: Vec3,
    /// Normal of the ground under the player, if any (may be too steep to walk).
    pub ground_normal: Option<Vec3>,
    pub ducked: bool,
    /// Jump button was held last tick (for non-auto-hop).
    pub jump_held: bool,
}

impl PlayerMove {
    pub fn at(origin: Vec3) -> Self {
        Self {
            origin,
            ..Self::default()
        }
    }

    /// On ground that is flat enough to walk on.
    pub fn walking(&self) -> bool {
        self.ground_normal.is_some_and(|n| n.z >= MIN_WALK_NORMAL)
    }

    pub fn on_ground(&self) -> bool {
        self.ground_normal.is_some()
    }

    /// Current hull (mins, maxs).
    pub fn hull(&self, cfg: &PhysicsConfig) -> (Vec3, Vec3) {
        let mins = Vec3::from_array(cfg.player_mins);
        let mut maxs = Vec3::from_array(cfg.player_maxs);
        if self.ducked {
            maxs.z = cfg.crouch_maxs_z;
        }
        (mins, maxs)
    }

    /// Eye position (hitscan origin).
    pub fn eye(&self, cfg: &PhysicsConfig) -> Vec3 {
        let h = if self.ducked {
            cfg.crouch_view_height
        } else {
            cfg.view_height
        };
        self.origin + Vec3::new(0.0, 0.0, h)
    }
}

/// `v` with its component into `normal` removed (and a hair more).
pub fn clip_velocity(v: Vec3, normal: Vec3, overbounce: f32) -> Vec3 {
    let mut backoff = v.dot(normal);
    if backoff < 0.0 {
        backoff *= overbounce;
    } else {
        backoff /= overbounce;
    }
    v - normal * backoff
}

struct Ctx<'a> {
    pm: &'a mut PlayerMove,
    cmd: &'a UserCmd,
    world: &'a World,
    cfg: &'a PhysicsConfig,
    dt: f32,
}

/// Advance one tick of `dt` seconds.
pub fn pmove(pm: &mut PlayerMove, cmd: &UserCmd, world: &World, cfg: &PhysicsConfig, dt: f32) {
    let mut c = Ctx {
        pm,
        cmd,
        world,
        cfg,
        dt,
    };
    if !c.cmd.pressed(buttons::JUMP) {
        c.pm.jump_held = false;
    }
    c.check_duck();
    c.ground_trace();
    if c.pm.walking() {
        c.walk_move();
    } else {
        c.air_move();
    }
    c.ground_trace();
    if c.cfg.snap_velocity {
        let v = &mut c.pm.velocity;
        *v = Vec3::new(v.x.round(), v.y.round(), v.z.round());
    }
}

impl Ctx<'_> {
    fn trace(&self, start: Vec3, end: Vec3) -> TraceResult {
        let (mins, maxs) = self.pm.hull(self.cfg);
        self.world.trace(start, end, mins, maxs)
    }

    fn check_duck(&mut self) {
        if self.cmd.pressed(buttons::CROUCH) {
            self.pm.ducked = true;
        } else if self.pm.ducked {
            // Stand up only if there is room.
            let mins = Vec3::from_array(self.cfg.player_mins);
            let maxs = Vec3::from_array(self.cfg.player_maxs);
            let o = self.pm.origin;
            if !self.world.trace(o, o, mins, maxs).all_solid {
                self.pm.ducked = false;
            }
        }
    }

    fn ground_trace(&mut self) {
        let o = self.pm.origin;
        let t = self.trace(o, o - Vec3::new(0.0, 0.0, GROUND_PROBE));
        if t.all_solid || t.fraction == 1.0 {
            self.pm.ground_normal = None;
            return;
        }
        // Moving up and away from the plane: just jumped / knocked off.
        let v = self.pm.velocity;
        if v.z > 0.0 && v.dot(t.normal) > 10.0 {
            self.pm.ground_normal = None;
            return;
        }
        self.pm.ground_normal = Some(t.normal);
    }

    /// Scale so that diagonal input is no faster than straight input.
    fn cmd_scale(&self) -> f32 {
        let f = self.cmd.forward as f32;
        let r = self.cmd.right as f32;
        let max = f.abs().max(r.abs());
        if max == 0.0 {
            return 0.0;
        }
        let total = (f * f + r * r).sqrt();
        self.cfg.max_speed * max / (MOVE_MAX as f32 * total)
    }

    /// Horizontal forward/right from view yaw.
    fn flat_axes(&self) -> (Vec3, Vec3) {
        let (f, r, _) = angle_vectors(self.cmd.pitch, self.cmd.yaw);
        (
            Vec3::new(f.x, f.y, 0.0).normalize(),
            Vec3::new(r.x, r.y, 0.0).normalize(),
        )
    }

    fn check_jump(&mut self) -> bool {
        if !self.cmd.pressed(buttons::JUMP) {
            return false;
        }
        if self.pm.jump_held && !self.cfg.auto_hop {
            return false;
        }
        self.pm.ground_normal = None;
        self.pm.velocity.z = self.cfg.jump_velocity;
        self.pm.jump_held = true;
        true
    }

    fn friction(&mut self) {
        let v = self.pm.velocity;
        // Walking: ignore vertical speed for friction.
        let speed = Vec3::new(v.x, v.y, 0.0).length();
        if speed < 1.0 {
            self.pm.velocity.x = 0.0;
            self.pm.velocity.y = 0.0;
            return;
        }
        let control = speed.max(self.cfg.stop_speed);
        let drop = control * self.cfg.friction * self.dt;
        let new_speed = (speed - drop).max(0.0) / speed;
        self.pm.velocity = v * new_speed;
    }

    fn accelerate(&mut self, wish_dir: Vec3, wish_speed: f32, accel: f32) {
        let current = self.pm.velocity.dot(wish_dir);
        let add = wish_speed - current;
        if add <= 0.0 {
            return;
        }
        let accel_speed = (accel * self.dt * wish_speed).min(add);
        self.pm.velocity = self.pm.velocity.mad(accel_speed, wish_dir);
    }

    fn walk_move(&mut self) {
        if self.check_jump() {
            self.air_move();
            return;
        }
        self.friction();
        let scale = self.cmd_scale();
        let ground = self.pm.ground_normal.expect("walking implies ground");
        let (f, r) = self.flat_axes();
        // Project movement axes onto the ground plane.
        let f = clip_velocity(f, ground, OVERCLIP).normalize();
        let r = clip_velocity(r, ground, OVERCLIP).normalize();
        let wish = f * self.cmd.forward as f32 + r * self.cmd.right as f32;
        let (wish_dir, len) = wish.normalize_len();
        let mut wish_speed = len * scale;
        if self.pm.ducked {
            wish_speed = wish_speed.min(self.cfg.max_speed * self.cfg.crouch_speed_scale);
        }
        self.accelerate(wish_dir, wish_speed, self.cfg.accelerate);
        // Slide along the ground without losing speed on slopes. Exact projection
        // (overbounce 1.0, unlike Quake's OVERCLIP here) so no residual velocity
        // into/out of the ground survives the renormalisation (DECISIONS D7).
        let speed = self.pm.velocity.length();
        self.pm.velocity = clip_velocity(self.pm.velocity, ground, 1.0).normalize() * speed;
        if self.pm.velocity.x == 0.0 && self.pm.velocity.y == 0.0 {
            return;
        }
        self.step_slide_move(false);
    }

    fn air_move(&mut self) {
        let scale = self.cmd_scale();
        let (f, r) = self.flat_axes();
        let wish = f * self.cmd.forward as f32 + r * self.cmd.right as f32;
        let (wish_dir, len) = wish.normalize_len();
        self.accelerate(wish_dir, len * scale, self.cfg.air_accelerate);
        // Steep ground: slide along it.
        if let Some(n) = self.pm.ground_normal {
            self.pm.velocity = clip_velocity(self.pm.velocity, n, OVERCLIP);
        }
        self.step_slide_move(true);
    }

    /// Move through the world, clipping velocity against hit planes.
    /// Returns true if anything was hit.
    fn slide_move(&mut self, gravity: bool) -> bool {
        let mut time_left = self.dt;
        let mut end_velocity = self.pm.velocity;
        if gravity {
            end_velocity.z -= self.cfg.gravity * self.dt;
            // Trapezoidal integration: average of start and end vertical speed.
            self.pm.velocity.z = (self.pm.velocity.z + end_velocity.z) * 0.5;
            if let Some(n) = self.pm.ground_normal {
                self.pm.velocity = clip_velocity(self.pm.velocity, n, OVERCLIP);
            }
        }

        let mut planes = [Vec3::ZERO; MAX_CLIP_PLANES];
        let mut num_planes = 0;
        if let Some(n) = self.pm.ground_normal {
            planes[0] = n;
            num_planes = 1;
        }
        // Never turn back against the original direction.
        planes[num_planes] = self.pm.velocity.normalize();
        num_planes += 1;

        let mut bump = 0;
        while bump < NUM_BUMPS {
            let start = self.pm.origin;
            let t = self.trace(start, start.mad(time_left, self.pm.velocity));
            if t.all_solid {
                // Stuck: don't build up falling speed.
                self.pm.velocity.z = 0.0;
                return true;
            }
            if t.fraction > 0.0 {
                self.pm.origin = t.end;
            }
            if t.fraction == 1.0 {
                break;
            }
            time_left -= time_left * t.fraction;
            if num_planes >= MAX_CLIP_PLANES {
                self.pm.velocity = Vec3::ZERO;
                return true;
            }
            // Hit the same plane twice: nudge off it instead of clipping again.
            if planes[..num_planes].iter().any(|p| t.normal.dot(*p) > 0.99) {
                self.pm.velocity += t.normal;
                bump += 1;
                continue;
            }
            planes[num_planes] = t.normal;
            num_planes += 1;

            for i in 0..num_planes {
                if self.pm.velocity.dot(planes[i]) >= 0.1 {
                    continue;
                }
                let mut clip = clip_velocity(self.pm.velocity, planes[i], OVERCLIP);
                let mut end_clip = clip_velocity(end_velocity, planes[i], OVERCLIP);
                for j in 0..num_planes {
                    if j == i || clip.dot(planes[j]) >= 0.1 {
                        continue;
                    }
                    clip = clip_velocity(clip, planes[j], OVERCLIP);
                    end_clip = clip_velocity(end_clip, planes[j], OVERCLIP);
                    if clip.dot(planes[i]) >= 0.0 {
                        continue;
                    }
                    // Two planes: slide along their crease.
                    let dir = planes[i].cross(planes[j]).normalize();
                    clip = dir * dir.dot(self.pm.velocity);
                    end_clip = dir * dir.dot(end_velocity);
                    for (k, pk) in planes[..num_planes].iter().enumerate() {
                        if k == i || k == j || clip.dot(*pk) >= 0.1 {
                            continue;
                        }
                        // Three planes: wedged, stop dead.
                        self.pm.velocity = Vec3::ZERO;
                        return true;
                    }
                }
                self.pm.velocity = clip;
                end_velocity = end_clip;
                break;
            }
            bump += 1;
        }
        if gravity {
            self.pm.velocity = end_velocity;
        }
        bump != 0
    }

    /// Slide move that also tries stepping up onto ledges up to `step_height`.
    fn step_slide_move(&mut self, gravity: bool) {
        let start_o = self.pm.origin;
        let start_v = self.pm.velocity;
        if !self.slide_move(gravity) {
            return; // moved the whole way
        }
        let step = self.cfg.step_height;
        let below = self.trace(start_o, start_o - Vec3::new(0.0, 0.0, step));
        // Never step up while still moving up through open air.
        if self.pm.velocity.z > 0.0 && (below.fraction == 1.0 || below.normal.z < MIN_WALK_NORMAL) {
            return;
        }
        let up = self.trace(start_o, start_o + Vec3::new(0.0, 0.0, step));
        if up.all_solid {
            return;
        }
        let step_size = up.end.z - start_o.z;
        self.pm.origin = up.end;
        self.pm.velocity = start_v;
        self.slide_move(gravity);
        // Push back down onto the step.
        let o = self.pm.origin;
        let down = self.trace(o, o - Vec3::new(0.0, 0.0, step_size));
        if !down.all_solid {
            self.pm.origin = down.end;
        }
        if down.fraction < 1.0 {
            self.pm.velocity = clip_velocity(self.pm.velocity, down.normal, OVERCLIP);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::Angle;
    use crate::tick::Tick;
    use crate::world::{Aabb, Spawn};

    const DT: f32 = 0.008;
    const REST_Z: f32 = 24.0 + crate::trace::SURFACE_EPSILON;

    fn world(extra: &[Aabb]) -> World {
        let mut brushes = vec![Aabb::new(
            Vec3::new(-4096.0, -4096.0, -32.0),
            Vec3::new(4096.0, 4096.0, 0.0),
        )];
        brushes.extend_from_slice(extra);
        World {
            convex: Vec::new(),
            name: "test".into(),
            colors: vec![[0.5; 3]; brushes.len()],
            brushes,
            spawns: vec![Spawn {
                origin: Vec3::ZERO,
                yaw: Angle::ZERO,
            }],
        }
    }

    fn cmd(forward: i8, right: i8, buttons: u8) -> UserCmd {
        UserCmd {
            tick: Tick(0),
            pitch: Angle::ZERO,
            yaw: Angle::ZERO,
            forward,
            right,
            buttons,
        }
    }

    fn run(pm: &mut PlayerMove, w: &World, c: UserCmd, ticks: usize) {
        let cfg = PhysicsConfig::default();
        for _ in 0..ticks {
            pmove(pm, &c, w, &cfg, DT);
        }
    }

    fn standing() -> PlayerMove {
        PlayerMove::at(Vec3::new(0.0, 0.0, REST_Z))
    }

    #[test]
    fn stands_still_on_floor() {
        let w = world(&[]);
        let mut pm = standing();
        run(&mut pm, &w, cmd(0, 0, 0), 125);
        assert!(pm.walking());
        assert_eq!(pm.origin, Vec3::new(0.0, 0.0, REST_Z));
        assert_eq!(pm.velocity, Vec3::ZERO);
    }

    #[test]
    fn falls_and_lands() {
        let w = world(&[]);
        let mut pm = PlayerMove::at(Vec3::new(0.0, 0.0, 200.0));
        run(&mut pm, &w, cmd(0, 0, 0), 1);
        assert!(!pm.on_ground());
        run(&mut pm, &w, cmd(0, 0, 0), 125);
        assert!(pm.walking());
        assert!((pm.origin.z - REST_Z).abs() < 0.01, "{}", pm.origin.z);
        assert_eq!(pm.velocity.z, 0.0);
    }

    #[test]
    fn ground_accel_reaches_max_speed() {
        let w = world(&[]);
        let mut pm = standing();
        run(&mut pm, &w, cmd(127, 0, 0), 250);
        assert_eq!(pm.velocity.length(), 320.0);
        assert!(pm.velocity.x > 0.0);
    }

    #[test]
    fn diagonal_is_not_faster() {
        let w = world(&[]);
        let mut pm = standing();
        run(&mut pm, &w, cmd(127, 127, 0), 250);
        assert!(
            (pm.velocity.length() - 320.0).abs() <= 1.0,
            "{}",
            pm.velocity.length()
        );
    }

    #[test]
    fn friction_stops_player() {
        let w = world(&[]);
        let mut pm = standing();
        pm.velocity = Vec3::new(320.0, 0.0, 0.0);
        run(&mut pm, &w, cmd(0, 0, 0), 125);
        assert_eq!(pm.velocity, Vec3::ZERO);
    }

    #[test]
    fn crouch_walk_is_slow_and_stand_needs_room() {
        let w = world(&[]);
        let mut pm = standing();
        run(&mut pm, &w, cmd(127, 0, buttons::CROUCH), 250);
        assert!(pm.ducked);
        assert_eq!(pm.velocity.length(), 80.0);
        // Under a 50-unit-high ceiling (crouch hull fits, standing doesn't).
        let low = world(&[Aabb::new(
            Vec3::new(-100.0, -100.0, 50.0),
            Vec3::new(100.0, 100.0, 60.0),
        )]);
        let mut pm = standing();
        run(&mut pm, &low, cmd(0, 0, buttons::CROUCH), 2);
        pm.origin = Vec3::new(0.0, 0.0, REST_Z);
        run(&mut pm, &low, cmd(0, 0, 0), 10);
        assert!(pm.ducked, "must stay crouched under low ceiling");
    }

    #[test]
    fn wall_blocks_and_slides() {
        // Wall across +x at x=200.
        let w = world(&[Aabb::new(
            Vec3::new(200.0, -1000.0, 0.0),
            Vec3::new(232.0, 1000.0, 300.0),
        )]);
        let mut pm = standing();
        let mut c = cmd(127, 0, 0);
        c.yaw = Angle::from_degrees(30.0);
        run(&mut pm, &w, c, 250);
        assert!(pm.origin.x <= 200.0 - 15.0);
        assert!(
            pm.origin.y > 200.0,
            "should slide along the wall, y={}",
            pm.origin.y
        );
        assert!(pm.velocity.x.abs() <= 1.0);
    }

    #[test]
    fn steps_up_small_ledge_not_tall_one() {
        let small = world(&[Aabb::new(
            Vec3::new(100.0, -500.0, 0.0),
            Vec3::new(600.0, 500.0, 16.0),
        )]);
        let mut pm = standing();
        run(&mut pm, &small, cmd(127, 0, 0), 125);
        assert!(
            pm.origin.x > 150.0,
            "should step onto 16u ledge, x={}",
            pm.origin.x
        );
        assert!(
            (pm.origin.z - (16.0 + REST_Z)).abs() < 0.5,
            "z={}",
            pm.origin.z
        );

        let tall = world(&[Aabb::new(
            Vec3::new(100.0, -500.0, 0.0),
            Vec3::new(600.0, 500.0, 24.0),
        )]);
        let mut pm = standing();
        run(&mut pm, &tall, cmd(127, 0, 0), 125);
        assert!(
            pm.origin.x < 100.0 - 15.0 + 0.01,
            "24u ledge must block, x={}",
            pm.origin.x
        );
    }

    #[test]
    fn jump_requires_repress_without_auto_hop() {
        let w = world(&[]);
        let cfg = PhysicsConfig {
            auto_hop: false,
            ..PhysicsConfig::default()
        };
        let mut pm = standing();
        let held = cmd(0, 0, buttons::JUMP);
        pmove(&mut pm, &held, &w, &cfg, DT);
        assert!(!pm.on_ground() && pm.velocity.z > 0.0);
        for _ in 0..250 {
            pmove(&mut pm, &held, &w, &cfg, DT);
        }
        assert!(pm.walking(), "holding jump must not re-jump");
        // Default (auto hop): holding jump keeps hopping.
        let mut pm = standing();
        run(&mut pm, &w, held, 250);
        let mut airborne = 0;
        for _ in 0..250 {
            run(&mut pm, &w, held, 1);
            airborne += usize::from(!pm.on_ground());
        }
        assert!(airborne > 200, "auto hop airborne ticks {airborne}");
    }

    #[test]
    fn crouch_eye_lower() {
        let cfg = PhysicsConfig::default();
        let mut pm = standing();
        let stand = pm.eye(&cfg).z;
        pm.ducked = true;
        assert_eq!(stand - pm.eye(&cfg).z, 14.0);
        assert_eq!(pm.hull(&cfg).1.z, 16.0);
    }
}
