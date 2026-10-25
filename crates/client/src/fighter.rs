//! The "fighter": a more detailed player model than the plain humanoid. Boots,
//! two-segment legs with knees, pelvis, abdomen and chest that twist separately,
//! shoulder pads, a head with a visor, arms bent around a Lightning Gun held in
//! both hands. All boxes, built in code (no assets).
//!
//! The legs are driven by the player's `Gait` (see `gait.rs`): they turn toward
//! the direction of travel, run backwards when moving backwards, stride with the
//! distance covered and tuck in the air. The lowest foot always stands on the
//! floor of the hitbox; the body bobs with the stride. The torso, head and gun
//! follow the view angles. Everything stays inside the hitbox (what you see is
//! what you can hit) and, with forced colours on, uses exactly the three
//! band colours: legs `lower`, torso/arms/gun `upper`, head `head`.

use crate::models::{ModelPose, ModelSettings, tilt};
use crate::players::EnemyColors;
use crate::render::{Vertex, push_obox};
use lgduel_sim::config::PhysicsConfig;
use lgduel_sim::math::{Angle, Vec3, angle_vectors};
use std::f32::consts::{PI, TAU};

/// Biggest thigh swing (radians) at a full run: keeps the feet inside the hull's footprint.
const SWING: f32 = 0.42;
/// Extra knee bend while a leg swings through.
const KNEE: f32 = 1.0;
/// Largest arm/gun pitch (radians).
const AIM_LIMIT: f32 = 0.6;
/// Furthest an elbow may stand out sideways from the body's middle.
const MAX_ELBOW_OUT: f32 = 10.5;

#[derive(Clone, Copy)]
enum Band {
    Head,
    Upper,
    Lower,
}

struct Painter<'a> {
    out: &'a mut Vec<Vertex>,
    colors: &'a EnemyColors,
    forced: bool,
}

fn scale(c: [f32; 3], k: f32) -> [f32; 3] {
    [c[0] * k, c[1] * k, c[2] * k]
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

impl Painter<'_> {
    /// A box: in the band's solid colour when colours are forced, else the face-shaded `skin`.
    fn obox(
        &mut self,
        centre: Vec3,
        axes: (Vec3, Vec3, Vec3),
        half: Vec3,
        band: Band,
        skin: [f32; 3],
    ) {
        let (color, shade) = if self.forced {
            (
                match band {
                    Band::Head => self.colors.head,
                    Band::Upper => self.colors.upper,
                    Band::Lower => self.colors.lower,
                },
                false,
            )
        } else {
            (skin, true)
        };
        push_obox(self.out, centre, axes, half, color, shade);
    }

    /// A box from `a` to `b` (a limb segment); `side` says which way its width lies.
    #[allow(clippy::too_many_arguments)]
    fn limb(
        &mut self,
        a: Vec3,
        b: Vec3,
        side: Vec3,
        half_w: f32,
        half_d: f32,
        band: Band,
        skin: [f32; 3],
    ) {
        let d = b - a;
        let len = d.length();
        if len < 1e-3 {
            return;
        }
        let u = d * (1.0 / len);
        let r = (side - u * u.dot(side)).normalize();
        if r == Vec3::ZERO {
            return;
        }
        let f = u.cross(r);
        self.obox(
            (a + b) * 0.5,
            (f, r, u),
            Vec3::new(half_d, half_w, len * 0.5 + 0.4),
            band,
            skin,
        );
    }
}

/// Elbow of a two-bone arm from `a` (shoulder) toward `b` (hand): bends toward `pole`.
/// Returns (elbow, hand), the hand clamped to what the arm can reach.
fn two_bone(a: Vec3, b: Vec3, l1: f32, l2: f32, pole: Vec3) -> (Vec3, Vec3) {
    let d = b - a;
    let dist = d.length().clamp(0.01, l1 + l2 - 0.02);
    let dir = d.normalize();
    let hand = a + dir * dist;
    let along = (l1 * l1 - l2 * l2 + dist * dist) / (2.0 * dist);
    let height = (l1 * l1 - along * along).max(0.0).sqrt();
    let perp = (pole - dir * dir.dot(pole)).normalize();
    (a + dir * along + perp * height, hand)
}

/// Append the fighter for `pose`.
pub fn push_fighter(
    out: &mut Vec<Vertex>,
    pose: ModelPose,
    phys: &PhysicsConfig,
    enemy: &EnemyColors,
    m: &ModelSettings,
) {
    let mins = Vec3::from_array(phys.player_mins);
    let mut maxs = Vec3::from_array(phys.player_maxs);
    if pose.ducked {
        maxs.z = phys.crouch_maxs_z;
    }
    // Heights scale with the hull (crouching); widths stay (the footprint is the same).
    let k = (maxs.z - mins.z) / 56.0;
    let z0 = pose.origin.z + mins.z;
    let g = pose.gait;
    let sk = &m.skin;
    let up = Vec3::new(0.0, 0.0, 1.0);
    let yaw_plus = |a: f32| pose.yaw.wrapping_add(Angle::from_radians(a as f64));
    let (vf, vr, _) = angle_vectors(Angle::ZERO, pose.yaw);
    let (lf, lr, _) = angle_vectors(Angle::ZERO, yaw_plus(g.leg_yaw));
    let (mf, mr, _) = angle_vectors(Angle::ZERO, yaw_plus(g.leg_yaw * 0.5));
    let mut p = Painter {
        out,
        colors: enemy,
        forced: m.forced_colors,
    };

    // ---- legs: thigh angle and knee bend per side, from the stride phase ----
    let (boot_h, shin, thigh) = (3.4 * k, 12.3 * k, 12.3 * k);
    let full = boot_h + shin + thigh;
    let sides = [-1.0f32, 1.0];
    let angles = sides.map(|side| {
        let ph = TAU * g.phase + if side > 0.0 { 0.0 } else { PI };
        let run = g.amp * (1.0 - g.air);
        let swing = SWING * run * ph.sin();
        let bend = run * (0.1 + KNEE * ph.cos().max(0.0).powi(2));
        // In the air: one leg forward and bent, the other trailing.
        let (air_t, air_k) = if side > 0.0 {
            (0.55, 0.9)
        } else {
            (-0.15, 0.4)
        };
        (lerp(swing, air_t, g.air), lerp(bend, air_k, g.air))
    });
    // Direction from a joint to the next for a segment swung `a` radians forward of straight down.
    let seg = |a: f32| lf * a.sin() - up * a.cos();
    let reach = angles.map(|(t, kn)| thigh * t.cos() + shin * (t - kn).cos() + boot_h);
    // The lowest foot stands on the floor; in the air the body keeps its standing height.
    let hip_h = lerp(reach[0].max(reach[1]), full, g.air);
    let pelvis = Vec3::new(pose.origin.x, pose.origin.y, z0 + hip_h);
    for (i, side) in sides.iter().enumerate() {
        let (t, kn) = angles[i];
        let hip = pelvis + lr * (side * 3.9);
        let knee = hip + seg(t) * thigh;
        let ankle = knee + seg(t - kn) * shin;
        p.limb(hip, knee, lr, 2.8, 3.0, Band::Lower, sk.legs);
        p.limb(knee, ankle, lr, 2.4, 2.4, Band::Lower, scale(sk.legs, 0.85));
        p.obox(
            knee,
            (lf, lr, up),
            Vec3::new(2.2, 2.6, 2.0 * k.max(0.6)),
            Band::Lower,
            scale(sk.legs, 1.2),
        );
        p.obox(
            ankle + lf * 1.4 - up * (boot_h * 0.5),
            (lf, lr, up),
            Vec3::new(3.2, 2.4, boot_h * 0.5),
            Band::Lower,
            scale(sk.legs, 0.5),
        );
    }

    // ---- torso: pelvis follows the legs, abdomen is half way, chest follows the view ----
    let lean = 0.06 + 0.12 * g.amp;
    let (tf1, tu1) = tilt(vf, up, lean);
    // The abdomen is turned half way for the twist, but everything stacks along the view's lean.
    let (tf0, tu0) = tilt(mf, up, lean * 0.5);
    let tu0_stack = tilt(vf, up, lean * 0.5).1;
    p.obox(
        pelvis + up * (2.0 * k),
        (lf, lr, up),
        Vec3::new(4.2, 6.0, 2.0 * k),
        Band::Lower,
        scale(sk.legs, 0.7),
    );
    let abdomen = pelvis + up * (4.0 * k) + tu0_stack * (2.5 * k);
    p.obox(
        abdomen,
        (tf0, mr, tu0),
        Vec3::new(3.8, 5.4, 2.5 * k),
        Band::Upper,
        scale(sk.torso, 0.8),
    );
    let chest_base = abdomen + tu0_stack * (2.5 * k);
    let chest = chest_base + tu1 * (4.5 * k);
    p.obox(
        chest,
        (tf1, vr, tu1),
        Vec3::new(4.4, 7.2, 4.5 * k),
        Band::Upper,
        sk.torso,
    );
    let chest_top = chest_base + tu1 * (9.0 * k);

    // ---- neck, head (tilts a little with the view pitch) and visor ----
    let (look, _, _) = angle_vectors(pose.pitch, pose.yaw);
    let aim_up = look.z.clamp(-1.0, 1.0).asin().clamp(-AIM_LIMIT, AIM_LIMIT);
    p.obox(
        chest_top + tu1 * k,
        (tf1, vr, tu1),
        Vec3::new(2.2, 2.4, k),
        Band::Upper,
        scale(sk.head, 0.8),
    );
    let (hf, hu) = tilt(vf, up, -aim_up * 0.5);
    let head = chest_top + tu1 * (2.0 * k) + hu * (3.8 * k);
    p.obox(
        head,
        (hf, vr, hu),
        Vec3::new(3.8, 3.4, 3.8 * k),
        Band::Head,
        sk.head,
    );
    p.obox(
        head + hf * 3.6 + hu * (0.6 * k),
        (hf, vr, hu),
        Vec3::new(0.9, 3.0, 1.0 * k),
        Band::Upper,
        [0.12, 0.12, 0.16],
    );
    for side in sides {
        p.obox(
            chest_top - tu1 * (2.0 * k) + vr * (side * 9.0),
            (tf1, vr, tu1),
            Vec3::new(2.8, 2.4, 2.2 * k),
            Band::Upper,
            scale(sk.arms, 1.15),
        );
    }

    // ---- the gun, held in both hands, pointing where the player looks ----
    let (af, au) = tilt(vf, up, -aim_up);
    let gun = Vec3::new(pose.origin.x, pose.origin.y, chest.z - 0.5 * k) + vf * 5.2 + vr * 2.4;
    let gun_skin = [0.30, 0.31, 0.35];
    p.obox(
        gun - af * 1.4,
        (af, vr, au),
        Vec3::new(4.2, 1.6, 2.0),
        Band::Upper,
        gun_skin,
    );
    p.obox(
        gun + af * 4.4,
        (af, vr, au),
        Vec3::new(3.0, 0.9, 0.9),
        Band::Upper,
        scale(gun_skin, 0.8),
    );
    p.obox(
        gun + af * 6.8,
        (af, vr, au),
        Vec3::new(0.7, 1.5, 1.5),
        Band::Upper,
        scale(gun_skin, 1.3),
    );
    let hands = [
        (1.0f32, gun - af * 2.0 + vr * 1.0 - au * 1.6),
        (-1.0, gun + af * 3.4 - vr * 0.3 - au * 1.2),
    ];
    for (side, target) in hands {
        let shoulder = chest_top - tu1 * (2.2 * k) + vr * (side * 7.4);
        let pole = vr * side - up * 0.8 - vf * 0.2;
        let (mut elbow, hand) = two_bone(shoulder, target, 8.0, 8.0, pole);
        // Keep the elbow inside the hull's footprint however steeply the gun points.
        let out = (elbow - shoulder).dot(vr) * side + 7.4;
        if out > MAX_ELBOW_OUT {
            elbow -= vr * (side * (out - MAX_ELBOW_OUT));
        }
        p.limb(shoulder, elbow, vr, 2.0, 2.0, Band::Upper, sk.arms);
        p.limb(elbow, hand, vr, 1.7, 1.7, Band::Upper, scale(sk.arms, 0.9));
        p.obox(
            hand,
            (af, vr, au),
            Vec3::new(1.8, 1.7, 1.7),
            Band::Upper,
            sk.head,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gait::Gait;
    use crate::models::{ModelKind, push_model};

    fn pose(yaw: f64, pitch: f64, ducked: bool, gait: Gait) -> ModelPose {
        ModelPose {
            origin: Vec3::new(3.0, -7.0, 24.0),
            yaw: Angle::from_degrees(yaw),
            pitch: Angle::from_degrees(pitch),
            ducked,
            gait,
        }
    }

    fn gaits() -> Vec<Gait> {
        let mut v = vec![];
        for leg_yaw in [-70.0f32, 0.0, 70.0] {
            for phase in [0.0, 0.13, 0.25, 0.5, 0.77] {
                for (amp, air, dir) in [
                    (0.0, 0.0, 1.0),
                    (0.5, 0.0, 1.0),
                    (1.0, 0.0, 1.0),
                    (1.0, 0.0, -1.0),
                    (0.0, 1.0, 1.0),
                    (1.0, 1.0, 1.0),
                    (0.4, 0.5, 1.0),
                ] {
                    v.push(Gait {
                        leg_yaw: leg_yaw.to_radians(),
                        phase,
                        amp,
                        air,
                        dir,
                        ..Gait::default()
                    });
                }
            }
        }
        v
    }

    fn build(p: ModelPose, m: &ModelSettings) -> Vec<Vertex> {
        let mut v = vec![];
        push_model(
            &mut v,
            ModelKind::Fighter,
            p,
            &PhysicsConfig::default(),
            &EnemyColors::default(),
            m,
        );
        v
    }

    fn fighter_settings() -> ModelSettings {
        ModelSettings {
            enemy: ModelKind::Fighter,
            ..ModelSettings::default()
        }
    }

    #[test]
    fn fighter_stays_inside_the_hitbox_in_every_pose() {
        let phys = PhysicsConfig::default();
        let m = fighter_settings();
        let mut checked = 0;
        for ducked in [false, true] {
            let top = if ducked {
                phys.crouch_maxs_z
            } else {
                phys.player_maxs[2]
            };
            for g in gaits() {
                for yaw in [0.0, 37.0, 90.0, 200.0, 315.0] {
                    for pitch in [-89.0, -30.0, 0.0, 45.0, 89.0] {
                        let p = pose(yaw, pitch, ducked, g);
                        for v in build(p, &m) {
                            let (dx, dy) = (v.pos[0] - p.origin.x, v.pos[1] - p.origin.y);
                            assert!(
                                (dx * dx + dy * dy).sqrt() <= 15.5,
                                "out sideways ({dx},{dy}) at yaw {yaw} pitch {pitch} {g:?} ducked {ducked}"
                            );
                            let z = v.pos[2] - p.origin.z;
                            assert!(
                                z >= phys.player_mins[2] - 0.05 && z <= top + 1.0,
                                "out vertically: {z} (ducked {ducked}, {g:?})"
                            );
                            checked += 1;
                        }
                    }
                }
            }
        }
        assert!(checked > 100_000);
    }

    #[test]
    fn the_lowest_foot_stands_on_the_floor() {
        let phys = PhysicsConfig::default();
        let m = fighter_settings();
        for ducked in [false, true] {
            for g in gaits().into_iter().filter(|g| g.air == 0.0) {
                let p = pose(25.0, 0.0, ducked, g);
                let lowest = build(p, &m)
                    .iter()
                    .map(|v| v.pos[2])
                    .fold(f32::MAX, f32::min);
                let floor = p.origin.z + phys.player_mins[2];
                assert!(
                    (lowest - floor).abs() < 0.05,
                    "feet at {lowest}, floor {floor} ({g:?}, ducked {ducked})"
                );
            }
        }
    }

    #[test]
    fn the_stride_moves_the_legs_and_the_body_bobs() {
        let m = fighter_settings();
        let at = |phase: f32| {
            let g = Gait {
                phase,
                amp: 1.0,
                ..Gait::default()
            };
            build(pose(0.0, 0.0, false, g), &m)
        };
        let (a, b) = (at(0.0), at(0.25));
        assert_eq!(a.len(), b.len());
        assert_ne!(a, b, "a different stride phase must look different");
        let top = |v: &[Vertex]| v.iter().map(|x| x.pos[2]).fold(f32::MIN, f32::max);
        assert!(
            top(&a) != top(&b) || a.iter().zip(&b).any(|(x, y)| x.pos != y.pos),
            "legs move"
        );
        // Standing still: the model is symmetric in time (phase doesn't matter at amp 0).
        let still = |phase: f32| {
            let g = Gait {
                phase,
                ..Gait::default()
            };
            build(pose(0.0, 0.0, false, g), &m)
        };
        assert_eq!(still(0.0), still(0.4));
    }

    #[test]
    fn legs_turn_with_the_gait_but_the_torso_follows_the_view() {
        let m = fighter_settings();
        let straight = build(pose(0.0, 0.0, false, Gait::default()), &m);
        let turned = build(
            pose(
                0.0,
                0.0,
                false,
                Gait {
                    leg_yaw: 60.0f32.to_radians(),
                    ..Gait::default()
                },
            ),
            &m,
        );
        let c = EnemyColors::default();
        let part = |v: &[Vertex], color: [f32; 3]| -> Vec<[f32; 3]> {
            v.iter()
                .filter(|x| x.color == color)
                .map(|x| x.pos)
                .collect()
        };
        assert_ne!(
            part(&straight, c.lower),
            part(&turned, c.lower),
            "legs turn"
        );
        assert_eq!(
            part(&straight, c.head),
            part(&turned, c.head),
            "head stays with the view"
        );
    }

    #[test]
    fn forced_colours_show_exactly_the_three_bands() {
        let c = EnemyColors::default();
        let m = fighter_settings();
        let g = Gait {
            amp: 0.7,
            phase: 0.3,
            ..Gait::default()
        };
        let v = build(pose(20.0, 10.0, false, g), &m);
        let mut seen = std::collections::BTreeSet::new();
        for vert in &v {
            assert!(
                vert.color == c.head || vert.color == c.upper || vert.color == c.lower,
                "unexpected colour {:?}",
                vert.color
            );
            seen.insert(format!("{:?}", vert.color));
        }
        assert_eq!(seen.len(), 3);
        let skin = ModelSettings {
            forced_colors: false,
            ..m
        };
        let v = build(pose(20.0, 10.0, false, g), &skin);
        assert!(
            v.iter()
                .all(|x| x.color != c.head && x.color != c.upper && x.color != c.lower)
        );
        let distinct: std::collections::BTreeSet<_> =
            v.iter().map(|x| format!("{:?}", x.color)).collect();
        assert!(
            distinct.len() > 12,
            "skin parts and face shades: {}",
            distinct.len()
        );
    }

    #[test]
    fn arm_ik_reaches_and_clamps() {
        let a = Vec3::new(0.0, 0.0, 0.0);
        let pole = Vec3::new(0.0, 0.0, -1.0);
        // Reachable: both bones keep their length and the hand lands on the target.
        let target = Vec3::new(10.0, 3.0, -4.0);
        let (e, h) = two_bone(a, target, 8.0, 8.0, pole);
        assert!(((e - a).length() - 8.0).abs() < 1e-3);
        assert!(((h - e).length() - 8.0).abs() < 1e-3);
        assert!((h - target).length() < 1e-3);
        // Out of reach: the arm straightens toward the target.
        let (e, h) = two_bone(a, Vec3::new(50.0, 0.0, 0.0), 8.0, 8.0, pole);
        assert!((h.x - 15.98).abs() < 0.05 && e.x > 7.0);
        // The elbow bends toward the pole (down here).
        let (e, _) = two_bone(a, Vec3::new(10.0, 0.0, 0.0), 8.0, 8.0, pole);
        assert!(e.z < 0.0);
    }
}
