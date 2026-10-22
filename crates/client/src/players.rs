//! Player models: the hitbox split into lower / upper / head bands, each drawn
//! in a solid, unshaded forced colour for maximum visibility. What you see is
//! exactly what you can hit. A small nub at head height shows facing.

use crate::render::Vertex;
use lgduel_sim::config::PhysicsConfig;
use lgduel_sim::math::{Angle, Vec3, angle_vectors};
use lgduel_sim::world::Aabb;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EnemyColors {
    pub head: [f32; 3],
    pub upper: [f32; 3],
    pub lower: [f32; 3],
}

impl Default for EnemyColors {
    fn default() -> Self {
        // Bright, saturated, unlike anything in the grey arena.
        Self {
            head: [1.0, 1.0, 0.0],
            upper: [0.0, 1.0, 0.0],
            lower: [0.0, 0.6, 1.0],
        }
    }
}

/// Same faces as `render::push_box` but one solid colour (no face shading).
pub fn push_box_solid(out: &mut Vec<Vertex>, b: &Aabb, color: [f32; 3]) {
    let start = out.len();
    crate::render::push_box(out, b, color);
    for v in &mut out[start..] {
        v.color = color;
    }
}

/// Fraction of hull height where lower body ends / head starts.
const LOWER_TOP: f32 = 0.45;
const HEAD_BOTTOM: f32 = 0.85;

/// Append a player model at `origin` (interpolated), facing `yaw`.
pub fn push_player(
    out: &mut Vec<Vertex>,
    origin: Vec3,
    yaw: Angle,
    ducked: bool,
    phys: &PhysicsConfig,
    c: &EnemyColors,
) {
    let mins = Vec3::from_array(phys.player_mins);
    let mut maxs = Vec3::from_array(phys.player_maxs);
    if ducked {
        maxs.z = phys.crouch_maxs_z;
    }
    let h = maxs.z - mins.z;
    let z0 = origin.z + mins.z;
    let z_lower = z0 + h * LOWER_TOP;
    let z_head = z0 + h * HEAD_BOTTOM;
    let z1 = origin.z + maxs.z;
    let xy = |z_lo: f32, z_hi: f32, inset: f32| {
        Aabb::new(
            Vec3::new(origin.x + mins.x + inset, origin.y + mins.y + inset, z_lo),
            Vec3::new(origin.x + maxs.x - inset, origin.y + maxs.y - inset, z_hi),
        )
    };
    push_box_solid(out, &xy(z0, z_lower, 0.0), c.lower);
    push_box_solid(out, &xy(z_lower, z_head, 0.0), c.upper);
    push_box_solid(out, &xy(z_head, z1, 0.0), c.head);
    // Facing nub: small cube poking out of the head in the look direction.
    let (f, _, _) = angle_vectors(Angle::ZERO, yaw);
    let centre = Vec3::new(origin.x, origin.y, (z_head + z1) * 0.5) + f * (maxs.x + 3.0);
    let s = 3.0;
    push_box_solid(
        out,
        &Aabb::new(centre - Vec3::new(s, s, s), centre + Vec3::new(s, s, s)),
        c.head,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_bands_fill_hitbox() {
        let phys = PhysicsConfig::default();
        let mut v = vec![];
        push_player(
            &mut v,
            Vec3::new(0.0, 0.0, 24.0),
            Angle::ZERO,
            false,
            &phys,
            &EnemyColors::default(),
        );
        assert_eq!(v.len(), 36 * 4);
        let zs: Vec<f32> = v[..36 * 3].iter().map(|v| v.pos[2]).collect();
        let min = zs.iter().cloned().fold(f32::INFINITY, f32::min);
        let max = zs.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
        assert_eq!((min, max), (0.0, 56.0));
        let c = EnemyColors::default();
        assert!(v[..36].iter().all(|v| v.color == c.lower));
        assert!(v[36..72].iter().all(|v| v.color == c.upper));
        assert!(v[72..108].iter().all(|v| v.color == c.head));
        // Nub sits in front of the hitbox (+x for yaw 0), touching its face.
        assert!(v[108..].iter().all(|v| v.pos[0] >= 15.0));
    }

    #[test]
    fn crouched_model_is_shorter() {
        let phys = PhysicsConfig::default();
        let mut v = vec![];
        push_player(
            &mut v,
            Vec3::new(0.0, 0.0, 24.0),
            Angle::ZERO,
            true,
            &phys,
            &EnemyColors::default(),
        );
        let max = v[..108]
            .iter()
            .map(|v| v.pos[2])
            .fold(f32::NEG_INFINITY, f32::max);
        assert_eq!(max, 24.0 + 16.0);
    }
}
