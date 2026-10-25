//! Enemy player models, built in code. `classic` is the original three-band box
//! figure; `humanoid` has legs, torso, arms, head and a small LG. Both live inside
//! the player's hitbox (what you see is what you can hit) and are posed from
//! position and view angles only (no per-player state, nothing to sync).
//!
//! With forced colours on (the default) every part is drawn in the solid enemy
//! colours by body band: legs `lower`, torso/arms/gun `upper`, head `head`. With
//! them off the model shows its own skin colours with face shading.

use crate::players::{self, EnemyColors};
use crate::render::{Vertex, push_obox};
use lgduel_sim::config::PhysicsConfig;
use lgduel_sim::math::{Angle, Vec3, angle_vectors};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModelKind {
    Classic,
    Humanoid,
    /// Detailed, with legs that follow the direction of travel (see `fighter`).
    Fighter,
}

impl ModelKind {
    pub fn parse(s: &str) -> Option<ModelKind> {
        match s {
            "classic" => Some(ModelKind::Classic),
            "humanoid" => Some(ModelKind::Humanoid),
            "fighter" => Some(ModelKind::Fighter),
            _ => None,
        }
    }
}

/// Colours of a model that isn't drawn in forced colours.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Skin {
    pub head: [f32; 3],
    pub torso: [f32; 3],
    pub arms: [f32; 3],
    pub legs: [f32; 3],
}

impl Default for Skin {
    fn default() -> Self {
        Self {
            // Warm and dark tones: they stand out from the grey arenas and themes.
            head: [0.95, 0.75, 0.55],
            torso: [0.80, 0.25, 0.20],
            arms: [0.72, 0.22, 0.18],
            legs: [0.15, 0.20, 0.50],
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ModelSettings {
    /// Model drawn for other players.
    pub enemy: ModelKind,
    /// Draw enemies in the solid forced colours (recommended); off shows the skin.
    pub forced_colors: bool,
    pub skin: Skin,
}

impl Default for ModelSettings {
    fn default() -> Self {
        Self {
            enemy: ModelKind::Classic,
            forced_colors: true,
            skin: Skin::default(),
        }
    }
}

/// Where and how a player stands.
#[derive(Debug, Clone, Copy)]
pub struct ModelPose {
    pub origin: Vec3,
    pub yaw: Angle,
    pub pitch: Angle,
    pub ducked: bool,
    /// How the legs move (only the fighter uses it).
    pub gait: crate::gait::Gait,
}

/// Largest leg swing (radians): keeps the feet inside the hull's footprint.
const SWING: f32 = 0.28;
/// Largest arm/gun pitch (radians).
const AIM_LIMIT: f32 = 0.6;

/// Append the model for `pose`.
pub fn push_model(
    out: &mut Vec<Vertex>,
    kind: ModelKind,
    pose: ModelPose,
    phys: &PhysicsConfig,
    enemy: &EnemyColors,
    m: &ModelSettings,
) {
    match kind {
        ModelKind::Classic => {
            let colors = if m.forced_colors {
                *enemy
            } else {
                EnemyColors {
                    head: m.skin.head,
                    upper: m.skin.torso,
                    lower: m.skin.legs,
                }
            };
            players::push_player(out, pose.origin, pose.yaw, pose.ducked, phys, &colors);
        }
        ModelKind::Humanoid => push_humanoid(out, pose, phys, enemy, m),
        ModelKind::Fighter => crate::fighter::push_fighter(out, pose, phys, enemy, m),
    }
}

/// Tilt `(f, u)` about the right axis by `a` (positive = top leans forward).
pub(crate) fn tilt(f: Vec3, u: Vec3, a: f32) -> (Vec3, Vec3) {
    let (s, c) = a.sin_cos();
    (f * c - u * s, u * c + f * s)
}

fn push_humanoid(
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
    let h = maxs.z - mins.z;
    let z0 = pose.origin.z + mins.z;
    let (f, r, _) = angle_vectors(Angle::ZERO, pose.yaw);
    let up = Vec3::new(0.0, 0.0, 1.0);
    let colour = |forced: [f32; 3], skin: [f32; 3]| if m.forced_colors { forced } else { skin };
    let shade = !m.forced_colors;
    let at = |fwd: f32, right: f32, z: f32| {
        Vec3::new(pose.origin.x, pose.origin.y, z) + f * fwd + r * right
    };

    // Legs swing against each other as the body moves (phase follows position).
    let phase = (pose.origin.x + pose.origin.y) / 48.0;
    let swing = phase.sin() * SWING;
    let leg_len = h * 0.45;
    let hip_z = z0 + leg_len + 1.5;
    for (side, a) in [(-1.0f32, swing), (1.0, -swing)] {
        let (lf, lu) = tilt(f, up, a);
        let hip = at(0.0, side * 6.0, hip_z);
        push_obox(
            out,
            hip - lu * (leg_len * 0.5 + 0.75),
            (lf, r, lu),
            Vec3::new(4.0, 4.0, leg_len * 0.5 + 0.75),
            colour(enemy.lower, m.skin.legs),
            shade,
        );
    }
    // Torso leans slightly forward; the head rides on it.
    let torso_len = h * 0.35;
    let (tf, tu) = tilt(f, up, 0.12);
    let waist = at(0.0, 0.0, hip_z);
    push_obox(
        out,
        waist + tu * (torso_len * 0.5),
        (tf, r, tu),
        Vec3::new(5.5, 8.5, torso_len * 0.5),
        colour(enemy.upper, m.skin.torso),
        shade,
    );
    let neck = waist + tu * torso_len;
    let head_half = (z0 + h - (neck.z + 0.5)).clamp(2.0, 5.0) * 0.5;
    push_obox(
        out,
        neck + tu * (0.5 + head_half),
        (tf, r, tu),
        Vec3::new(head_half.max(3.5), head_half.max(3.5), head_half),
        colour(enemy.head, m.skin.torso),
        shade,
    );
    // Arms and the LG point where the player looks (limited, so they stay inside the hull).
    let (look, _, _) = angle_vectors(pose.pitch, pose.yaw);
    let aim_up = look.z.clamp(-1.0, 1.0).asin().clamp(-AIM_LIMIT, AIM_LIMIT);
    let (af, au) = tilt(f, up, -aim_up);
    let shoulder_z = hip_z + torso_len * 0.88;
    for side in [-1.0f32, 1.0] {
        let shoulder = at(1.0, side * 9.5, shoulder_z);
        push_obox(
            out,
            shoulder + af * 3.5,
            (af, r, au),
            Vec3::new(3.5, 2.0, 2.0),
            colour(enemy.upper, m.skin.arms),
            shade,
        );
    }
    let hands = at(1.0, 0.0, shoulder_z - 1.0);
    push_obox(
        out,
        hands + af * 8.0,
        (af, r, au),
        Vec3::new(4.0, 1.6, 1.6),
        colour(enemy.upper, m.skin.arms),
        shade,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pose(x: f32, y: f32, yaw: f64, pitch: f64, ducked: bool) -> ModelPose {
        ModelPose {
            origin: Vec3::new(x, y, 24.0),
            yaw: Angle::from_degrees(yaw),
            pitch: Angle::from_degrees(pitch),
            ducked,
            gait: crate::gait::Gait::default(),
        }
    }

    fn build(kind: ModelKind, p: ModelPose, m: &ModelSettings) -> Vec<Vertex> {
        let mut v = vec![];
        push_model(
            &mut v,
            kind,
            p,
            &PhysicsConfig::default(),
            &EnemyColors::default(),
            m,
        );
        v
    }

    #[test]
    fn humanoid_stays_inside_the_hitbox() {
        let phys = PhysicsConfig::default();
        let m = ModelSettings {
            enemy: ModelKind::Humanoid,
            ..ModelSettings::default()
        };
        let mut checked = 0;
        for ducked in [false, true] {
            let top = if ducked {
                phys.crouch_maxs_z
            } else {
                phys.player_maxs[2]
            };
            for yaw in [0.0, 30.0, 45.0, 90.0, 135.0, 200.0, 315.0] {
                for pitch in [-89.0, -40.0, 0.0, 40.0, 89.0] {
                    for x in [0.0, 12.0, 24.0, 36.0, 100.0, 1000.5] {
                        let p = pose(x, 3.0, yaw, pitch, ducked);
                        for v in build(ModelKind::Humanoid, p, &m) {
                            let (dx, dy) = (v.pos[0] - p.origin.x, v.pos[1] - p.origin.y);
                            // A circle of radius 15 fits in the 30x30 hull at every yaw.
                            assert!(
                                (dx * dx + dy * dy).sqrt() <= 15.5,
                                "out sideways at yaw {yaw} pitch {pitch} x {x}: {dx},{dy}"
                            );
                            let z = v.pos[2] - p.origin.z;
                            assert!(
                                z >= phys.player_mins[2] - 1.0 && z <= top + 1.0,
                                "out vertically: {z} (ducked {ducked})"
                            );
                            checked += 1;
                        }
                    }
                }
            }
        }
        assert!(checked > 10_000);
    }

    #[test]
    fn forced_colours_show_exactly_the_three_bands() {
        let c = EnemyColors::default();
        let m = ModelSettings {
            enemy: ModelKind::Humanoid,
            ..ModelSettings::default()
        };
        let v = build(ModelKind::Humanoid, pose(0.0, 0.0, 20.0, 10.0, false), &m);
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
        // Skin mode: face-shaded skin colours only (nothing forced).
        let skin = ModelSettings {
            forced_colors: false,
            ..m
        };
        let v = build(
            ModelKind::Humanoid,
            pose(0.0, 0.0, 20.0, 10.0, false),
            &skin,
        );
        assert!(
            v.iter()
                .all(|x| x.color != c.head && x.color != c.upper && x.color != c.lower)
        );
        let distinct: std::collections::BTreeSet<_> =
            v.iter().map(|x| format!("{:?}", x.color)).collect();
        assert!(
            distinct.len() > 8,
            "skin parts and face shades: {}",
            distinct.len()
        );
    }

    #[test]
    fn classic_model_is_the_original_player() {
        let phys = PhysicsConfig::default();
        let p = pose(5.0, 6.0, 77.0, 0.0, false);
        let mut original = vec![];
        players::push_player(
            &mut original,
            p.origin,
            p.yaw,
            p.ducked,
            &phys,
            &EnemyColors::default(),
        );
        assert_eq!(
            build(ModelKind::Classic, p, &ModelSettings::default()),
            original
        );
        // Without forced colours the classic bands use the skin.
        let skin = ModelSettings {
            forced_colors: false,
            ..ModelSettings::default()
        };
        let v = build(ModelKind::Classic, p, &skin);
        assert_eq!(v[0].color, skin.skin.legs);
        assert_eq!(v[36].color, skin.skin.torso);
        assert_eq!(v[72].color, skin.skin.head);
    }

    #[test]
    fn legs_swing_with_position_and_the_body_follows_yaw() {
        let m = ModelSettings {
            enemy: ModelKind::Humanoid,
            ..ModelSettings::default()
        };
        let a = build(ModelKind::Humanoid, pose(0.0, 0.0, 0.0, 0.0, false), &m);
        let b = build(ModelKind::Humanoid, pose(24.0, 0.0, 0.0, 0.0, false), &m);
        assert_eq!(a.len(), b.len());
        // Move one pose by the same offset: if nothing animated the shapes would be identical.
        let shifted: Vec<[f32; 3]> = a
            .iter()
            .map(|v| [v.pos[0] + 24.0, v.pos[1], v.pos[2]])
            .collect();
        assert!(
            shifted
                .iter()
                .zip(&b)
                .any(|(s, v)| (s[0] - v.pos[0]).abs() > 0.5),
            "legs did not move"
        );
        // Torso and head don't swing (the first two legs are the first 72 vertices).
        assert!(
            shifted[72..120]
                .iter()
                .zip(&b[72..120])
                .all(|(s, v)| (s[0] - v.pos[0]).abs() < 1e-3)
        );
        // Turning to face +y moves the front of the model onto +y.
        let c = build(ModelKind::Humanoid, pose(0.0, 0.0, 90.0, 0.0, false), &m);
        let max_y = c.iter().map(|v| v.pos[1]).fold(f32::MIN, f32::max);
        let max_x = c.iter().map(|v| v.pos[0]).fold(f32::MIN, f32::max);
        assert!(max_y > 10.0 && max_x < 12.0, "{max_x} {max_y}");
    }

    #[test]
    fn crouching_lowers_the_humanoid() {
        let m = ModelSettings {
            enemy: ModelKind::Humanoid,
            ..ModelSettings::default()
        };
        let top = |d| {
            build(ModelKind::Humanoid, pose(0.0, 0.0, 0.0, 0.0, d), &m)
                .iter()
                .map(|v| v.pos[2])
                .fold(f32::MIN, f32::max)
        };
        assert!(top(true) < top(false) - 10.0);
    }

    #[test]
    fn settings_round_trip_and_parse() {
        assert_eq!(ModelKind::parse("humanoid"), Some(ModelKind::Humanoid));
        assert_eq!(ModelKind::parse("nope"), None);
        let m = ModelSettings::default();
        assert_eq!(m.enemy, ModelKind::Classic);
        assert!(m.forced_colors);
        let t = toml::to_string(&m).unwrap();
        assert_eq!(toml::from_str::<ModelSettings>(&t).unwrap(), m);
        assert_eq!(
            toml::from_str::<ModelSettings>("enemy = \"humanoid\"")
                .unwrap()
                .enemy,
            ModelKind::Humanoid
        );
    }
}
