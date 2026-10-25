//! Leg animation state of the players you see. A player's legs follow how they
//! actually move, not where they look: they turn toward the direction of travel
//! (strafing, diagonals), run backwards when moving backwards, stride in step
//! with the distance covered, and tuck while airborne. Torso, head and gun keep
//! following the view angles, so the hips twist the way they do in Quake.
//!
//! Purely visual and client-side: the velocity comes from the snapshots the
//! client already receives; nothing here reaches the sim or the network.

use crate::scene::OtherView;
use lgduel_sim::game::PlayerId;
use lgduel_sim::math::{Angle, Vec3, angle_vectors};
use std::collections::HashMap;
use std::f32::consts::PI;

/// Distance travelled per full stride cycle (two steps).
pub const CYCLE: f32 = 105.0;
/// Ground speed that gives the biggest stride (the sim's top running speed).
pub const RUN_SPEED: f32 = 320.0;
/// Below this ground speed the legs stand still.
const MIN_SPEED: f32 = 30.0;
/// The legs never turn further than this from the view direction (radians).
const LEG_TURN_LIMIT: f32 = 70.0 * PI / 180.0;
/// How fast the legs turn toward the direction of travel (radians/s).
const TURN_RATE: f32 = 12.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Gait {
    /// Direction the legs face, as a yaw added to the view yaw (radians, + = left).
    pub leg_yaw: f32,
    /// Stride phase in cycles (0..1).
    pub phase: f32,
    /// Stride size 0..1 (0 = standing).
    pub amp: f32,
    /// 0 on the ground .. 1 in the air.
    pub air: f32,
    /// +1 running forwards, -1 running backwards.
    pub dir: f32,
    /// Smoothed ground speed that drives the cadence.
    pub(crate) speed: f32,
}

impl Default for Gait {
    /// Standing still, legs straight.
    fn default() -> Self {
        Self {
            leg_yaw: 0.0,
            phase: 0.0,
            amp: 0.0,
            air: 0.0,
            dir: 1.0,
            speed: 0.0,
        }
    }
}

/// Move `v` toward `target` by at most `max`.
fn approach(v: f32, target: f32, max: f32) -> f32 {
    v + (target - v).clamp(-max, max)
}

/// Exponential smoothing factor for a rate (per second) over `dt`.
fn smoothing(rate: f32, dt: f32) -> f32 {
    1.0 - (-rate * dt).exp()
}

impl Gait {
    /// Advance by `dt` seconds for a player with view yaw `yaw` and velocity `vel`.
    pub fn step(&mut self, yaw: Angle, vel: Vec3, on_ground: bool, dt: f32) {
        let dt = dt.clamp(0.0, 0.1);
        let (f, r, _) = angle_vectors(Angle::ZERO, yaw);
        let speed = vel.x.hypot(vel.y);
        let air_target = if on_ground { 0.0 } else { 1.0 };
        self.air += (air_target - self.air) * smoothing(14.0, dt);
        if on_ground && speed > MIN_SPEED {
            // Angle of travel to the right of the view direction.
            let psi = vel.dot(r).atan2(vel.dot(f));
            // Hysteresis: switch to running backwards past 100 degrees, back at 80.
            let limit = (if self.dir < 0.0 { 80.0f32 } else { 100.0 }).to_radians();
            let back = psi.abs() > limit;
            self.dir = if back { -1.0 } else { 1.0 };
            // The legs face the way the body goes; running backwards they face the opposite way.
            let facing = if back { psi - psi.signum() * PI } else { psi };
            let want = (-facing).clamp(-LEG_TURN_LIMIT, LEG_TURN_LIMIT);
            self.leg_yaw = approach(self.leg_yaw, want, TURN_RATE * dt);
            self.speed += (speed - self.speed) * smoothing(10.0, dt);
        } else {
            self.leg_yaw = approach(self.leg_yaw, 0.0, TURN_RATE * 0.3 * dt);
            self.speed += (0.0 - self.speed) * smoothing(9.0, dt);
        }
        self.amp = (self.speed / RUN_SPEED).sqrt().clamp(0.0, 1.0);
        if self.speed < 3.0 {
            self.amp = 0.0;
        }
        self.phase = (self.phase + self.dir * self.speed * dt / CYCLE).rem_euclid(1.0);
    }
}

/// Gait of every player on screen.
#[derive(Debug, Clone, Default)]
pub struct Animator {
    states: HashMap<PlayerId, Gait>,
}

impl Animator {
    /// Advance everyone by `dt` seconds; players that left are forgotten, dead ones stand.
    pub fn update(&mut self, others: &[OtherView], dt: f32) {
        self.states
            .retain(|id, _| others.iter().any(|o| o.id == *id));
        for o in others {
            let g = self.states.entry(o.id).or_default();
            if o.alive {
                g.step(o.yaw, o.velocity, o.on_ground, dt);
            } else {
                *g = Gait::default();
            }
        }
    }

    pub fn gait(&self, id: PlayerId) -> Gait {
        self.states.get(&id).copied().unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DT: f32 = 1.0 / 125.0;

    /// Velocity of `speed` toward `angle` degrees to the right of the view direction.
    fn heading(yaw: Angle, angle: f32, speed: f32) -> Vec3 {
        let (f, r, _) = angle_vectors(Angle::ZERO, yaw);
        let a = angle.to_radians();
        f * (a.cos() * speed) + r * (a.sin() * speed)
    }

    fn run(yaw: Angle, angle: f32, speed: f32, seconds: f32) -> Gait {
        let mut g = Gait::default();
        for _ in 0..(seconds / DT) as usize {
            g.step(yaw, heading(yaw, angle, speed), true, DT);
        }
        g
    }

    #[test]
    fn standing_players_stand() {
        let mut g = run(Angle::from_degrees(30.0), 0.0, 320.0, 1.0);
        assert!(g.amp > 0.9);
        for _ in 0..250 {
            g.step(Angle::from_degrees(30.0), Vec3::ZERO, true, DT);
        }
        assert_eq!(g.amp, 0.0, "legs straighten when the player stops");
        assert!(g.leg_yaw.abs() < 0.05);
    }

    #[test]
    fn legs_turn_toward_the_direction_of_travel() {
        let yaw = Angle::from_degrees(100.0);
        let fwd = run(yaw, 0.0, 320.0, 1.0);
        assert!(fwd.leg_yaw.abs() < 0.02 && fwd.dir > 0.0);
        // Forward + strafe right (45 degrees): legs turn right (negative yaw offset), still forwards.
        let diag = run(yaw, 45.0, 320.0, 1.0);
        assert!(
            (diag.leg_yaw + 45.0f32.to_radians()).abs() < 0.05,
            "{}",
            diag.leg_yaw
        );
        assert!(diag.dir > 0.0);
        // Strafing left (-90): legs turn as far as they may, to the left.
        let left = run(yaw, -90.0, 320.0, 1.0);
        assert!(
            (left.leg_yaw - LEG_TURN_LIMIT).abs() < 0.05,
            "{}",
            left.leg_yaw
        );
        // Backwards: the legs keep facing forward and run in reverse.
        let back = run(yaw, 180.0, 320.0, 1.0);
        assert!(back.leg_yaw.abs() < 0.05 && back.dir < 0.0, "{back:?}");
        // Backwards and to the right (135): legs face forward-left (+45) running in reverse.
        let back_right = run(yaw, 135.0, 320.0, 1.0);
        assert!(
            (back_right.leg_yaw - 45.0f32.to_radians()).abs() < 0.05,
            "{back_right:?}"
        );
        assert!(back_right.dir < 0.0);
    }

    #[test]
    fn stride_follows_distance_and_direction() {
        let yaw = Angle::ZERO;
        let mut g = Gait::default();
        let mut travelled = 0.0;
        let mut turns = 0.0;
        let mut last = g.phase;
        for _ in 0..500 {
            g.step(yaw, heading(yaw, 0.0, 200.0), true, DT);
            travelled += 200.0 * DT;
            let d = (g.phase - last + 1.5).rem_euclid(1.0) - 0.5;
            turns += d;
            last = g.phase;
        }
        // About one cycle per CYCLE units once the cadence has settled (it eases in).
        let expected = travelled / CYCLE;
        assert!(
            (turns - expected).abs() < 0.2 * expected,
            "{turns} vs {expected}"
        );
        // Running backwards the phase runs the other way.
        let mut b = Gait::default();
        let start = b.phase;
        // Fewer than one cycle, so the phase doesn't wrap around more than once.
        for _ in 0..25 {
            b.step(yaw, heading(yaw, 180.0, 200.0), true, DT);
        }
        let moved = (b.phase - start + 1.5).rem_euclid(1.0) - 0.5;
        assert!(moved < 0.0, "backwards runs the stride in reverse: {moved}");
    }

    #[test]
    fn jumping_tucks_and_landing_recovers() {
        let mut g = run(Angle::ZERO, 0.0, 300.0, 0.5);
        for _ in 0..60 {
            g.step(Angle::ZERO, Vec3::new(300.0, 0.0, 200.0), false, DT);
        }
        assert!(g.air > 0.95);
        for _ in 0..60 {
            g.step(Angle::ZERO, Vec3::new(300.0, 0.0, 0.0), true, DT);
        }
        assert!(g.air < 0.05);
    }

    #[test]
    fn animator_tracks_players_and_forgets_the_gone() {
        let o = |id: u8, alive: bool| OtherView {
            id: PlayerId(id),
            origin: Vec3::ZERO,
            pitch: Angle::ZERO,
            yaw: Angle::ZERO,
            ducked: false,
            alive,
            buttons: 0,
            velocity: Vec3::new(300.0, 0.0, 0.0),
            on_ground: true,
        };
        let mut a = Animator::default();
        for _ in 0..100 {
            a.update(&[o(1, true), o(2, false)], DT);
        }
        assert!(a.gait(PlayerId(1)).amp > 0.5);
        assert_eq!(a.gait(PlayerId(2)), Gait::default(), "dead players stand");
        assert_eq!(a.gait(PlayerId(9)), Gait::default(), "unknown ids stand");
        a.update(&[o(2, true)], DT);
        assert_eq!(a.gait(PlayerId(1)), Gait::default(), "gone");
    }
}
