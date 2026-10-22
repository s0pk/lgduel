//! Swept-box traces against world brushes (also used with a zero hull for rays).
//!
//! Each brush is treated as the intersection of six half-spaces, expanded by
//! the moving hull. Like Quake's brush tracer, hits stop a small epsilon short
//! of the surface so the mover never ends up exactly on (or inside) a plane.

use crate::math::Vec3;
use crate::world::{Aabb, World};

/// Distance kept between a stopped mover and the surface it hit.
pub const SURFACE_EPSILON: f32 = 0.125;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TraceResult {
    /// Fraction of the move completed, 0..=1.
    pub fraction: f32,
    /// Final position (`start + (end - start) * fraction`).
    pub end: Vec3,
    /// Normal of the surface hit; zero if nothing was hit.
    pub normal: Vec3,
    /// Start position was inside a brush.
    pub start_solid: bool,
    /// Entire move was inside a brush.
    pub all_solid: bool,
}

impl TraceResult {
    pub fn hit(&self) -> bool {
        self.fraction < 1.0
    }
}

/// Trace a box of `mins..maxs` from `start` to `end` through a set of boxes.
pub fn trace_boxes(boxes: &[Aabb], start: Vec3, end: Vec3, mins: Vec3, maxs: Vec3) -> TraceResult {
    let mut fraction = 1.0f32;
    let mut normal = Vec3::ZERO;
    let mut start_solid = false;
    let mut all_solid = false;

    for b in boxes {
        let e = b.expanded(mins, maxs);
        // Planes as (normal, dist): inside means dot(p, n) - dist <= 0.
        let planes = [
            (Vec3::new(1.0, 0.0, 0.0), e.max.x),
            (Vec3::new(-1.0, 0.0, 0.0), -e.min.x),
            (Vec3::new(0.0, 1.0, 0.0), e.max.y),
            (Vec3::new(0.0, -1.0, 0.0), -e.min.y),
            (Vec3::new(0.0, 0.0, 1.0), e.max.z),
            (Vec3::new(0.0, 0.0, -1.0), -e.min.z),
        ];
        let mut enter = -1.0f32;
        let mut leave = 1.0f32;
        let mut enter_normal = Vec3::ZERO;
        let mut starts_out = false;
        let mut gets_out = false;
        let mut missed = false;

        for (n, dist) in planes {
            let d1 = start.dot(n) - dist;
            let d2 = end.dot(n) - dist;
            if d2 > 0.0 {
                gets_out = true;
            }
            if d1 > 0.0 {
                starts_out = true;
            }
            // Entirely in front of this plane: cannot touch the brush.
            if d1 > 0.0 && (d2 >= SURFACE_EPSILON || d2 >= d1) {
                missed = true;
                break;
            }
            // Entirely behind this plane: this plane does not clip.
            if d1 <= 0.0 && d2 <= 0.0 {
                continue;
            }
            if d1 > d2 {
                // Entering.
                let f = ((d1 - SURFACE_EPSILON) / (d1 - d2)).max(0.0);
                if f > enter {
                    enter = f;
                    enter_normal = n;
                }
            } else {
                // Leaving.
                let f = ((d1 + SURFACE_EPSILON) / (d1 - d2)).min(1.0);
                if f < leave {
                    leave = f;
                }
            }
        }
        if missed {
            continue;
        }
        if !starts_out {
            start_solid = true;
            if !gets_out {
                all_solid = true;
                fraction = 0.0;
            }
            continue;
        }
        if enter < leave && enter > -1.0 && enter < fraction {
            fraction = enter.max(0.0);
            normal = enter_normal;
        }
    }

    let end_pos = if fraction == 1.0 {
        end
    } else {
        start + (end - start) * fraction
    };
    TraceResult {
        fraction,
        end: end_pos,
        normal,
        start_solid,
        all_solid,
    }
}

impl World {
    pub fn trace(&self, start: Vec3, end: Vec3, mins: Vec3, maxs: Vec3) -> TraceResult {
        trace_boxes(&self.brushes, start, end, mins, maxs)
    }

    /// Zero-size ray (hitscan).
    pub fn trace_ray(&self, start: Vec3, end: Vec3) -> TraceResult {
        self.trace(start, end, Vec3::ZERO, Vec3::ZERO)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINS: Vec3 = Vec3::new(-15.0, -15.0, -24.0);
    const MAXS: Vec3 = Vec3::new(15.0, 15.0, 32.0);

    fn floor_and_wall() -> Vec<Aabb> {
        vec![
            Aabb::new(
                Vec3::new(-512.0, -512.0, -32.0),
                Vec3::new(512.0, 512.0, 0.0),
            ),
            Aabb::new(
                Vec3::new(200.0, -512.0, 0.0),
                Vec3::new(232.0, 512.0, 256.0),
            ),
        ]
    }

    #[test]
    fn falls_onto_floor_epsilon_above() {
        let t = trace_boxes(
            &floor_and_wall(),
            Vec3::new(0.0, 0.0, 100.0),
            Vec3::new(0.0, 0.0, -100.0),
            MINS,
            MAXS,
        );
        assert!(t.hit() && !t.start_solid);
        assert_eq!(t.normal, Vec3::new(0.0, 0.0, 1.0));
        // Feet stop just above z = 0: origin.z ≈ 24 + epsilon (within float error).
        assert!(
            (t.end.z - (24.0 + SURFACE_EPSILON)).abs() < 1e-3,
            "{}",
            t.end.z
        );
    }

    #[test]
    fn hits_wall_with_normal() {
        let t = trace_boxes(
            &floor_and_wall(),
            Vec3::new(0.0, 0.0, 40.0),
            Vec3::new(400.0, 0.0, 40.0),
            MINS,
            MAXS,
        );
        assert_eq!(t.normal, Vec3::new(-1.0, 0.0, 0.0));
        assert!((t.end.x - (200.0 - 15.0 - SURFACE_EPSILON)).abs() < 1e-3);
    }

    #[test]
    fn slide_along_floor_is_free() {
        // Resting epsilon above the floor, moving horizontally: no hit.
        let z = 24.0 + SURFACE_EPSILON;
        let t = trace_boxes(
            &floor_and_wall(),
            Vec3::new(0.0, 0.0, z),
            Vec3::new(-300.0, 50.0, z),
            MINS,
            MAXS,
        );
        assert!(!t.hit() && !t.start_solid);
        assert_eq!(t.end, Vec3::new(-300.0, 50.0, z));
    }

    #[test]
    fn start_solid_and_all_solid() {
        let boxes = floor_and_wall();
        let t = trace_boxes(
            &boxes,
            Vec3::new(0.0, 0.0, 10.0),
            Vec3::new(0.0, 0.0, 100.0),
            MINS,
            MAXS,
        );
        assert!(t.start_solid && !t.all_solid);
        let t = trace_boxes(
            &boxes,
            Vec3::new(0.0, 0.0, -10.0),
            Vec3::new(5.0, 0.0, -10.0),
            Vec3::ZERO,
            Vec3::ZERO,
        );
        assert!(t.all_solid && t.fraction == 0.0);
    }

    #[test]
    fn ray_misses_and_hits() {
        let boxes = floor_and_wall();
        let miss = trace_boxes(
            &boxes,
            Vec3::new(0.0, 0.0, 300.0),
            Vec3::new(400.0, 0.0, 300.0),
            Vec3::ZERO,
            Vec3::ZERO,
        );
        assert_eq!(miss.fraction, 1.0);
        let hit = trace_boxes(
            &boxes,
            Vec3::new(0.0, 0.0, 100.0),
            Vec3::new(400.0, 0.0, 100.0),
            Vec3::ZERO,
            Vec3::ZERO,
        );
        assert!((hit.end.x - (200.0 - SURFACE_EPSILON)).abs() < 1e-3);
    }

    #[test]
    fn diagonal_into_corner_takes_nearest() {
        let t = trace_boxes(
            &floor_and_wall(),
            Vec3::new(0.0, 0.0, 100.0),
            Vec3::new(400.0, 0.0, -300.0),
            MINS,
            MAXS,
        );
        // Path reaches the floor (z=24 at x=76) before the wall plane (x=185).
        assert_eq!(t.normal, Vec3::new(0.0, 0.0, 1.0));
        assert!(t.end.x < 185.0);
    }
}
