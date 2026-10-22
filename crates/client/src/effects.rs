//! Lightning Gun beam and impact sparks, built as camera-facing quads in the
//! flat pipeline. Purely visual: hits are decided by the sim / server.

use crate::render::Vertex;
use lgduel_sim::math::Vec3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BeamStyle {
    /// No beam at all (some players prefer an unobstructed view).
    None,
    /// Thin straight line.
    Thin,
    /// Thick straight line.
    Thick,
    /// Jagged lightning look.
    Zigzag,
}

impl BeamStyle {
    pub fn parse(s: &str) -> Option<BeamStyle> {
        Some(match s {
            "none" => BeamStyle::None,
            "thin" => BeamStyle::Thin,
            "thick" => BeamStyle::Thick,
            "zigzag" => BeamStyle::Zigzag,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EffectSettings {
    pub beam_style: BeamStyle,
    pub beam_color: [f32; 3],
    /// Impact sparks on/off.
    pub sparks: bool,
    /// Spark half-size in world units.
    pub spark_size: f32,
    pub spark_color: [f32; 3],
}

impl Default for EffectSettings {
    fn default() -> Self {
        Self {
            beam_style: BeamStyle::Thin,
            beam_color: [0.55, 0.8, 1.0],
            sparks: true,
            spark_size: 3.0,
            spark_color: [1.0, 1.0, 1.0],
        }
    }
}

/// Muzzle offset from the eye (forward, right, down) so your own beam is visible.
pub const MUZZLE: (f32, f32, f32) = (14.0, 6.0, 8.0);

/// Quad from `a` to `b` of `width`, turned to face `eye`.
pub fn push_segment(
    out: &mut Vec<Vertex>,
    a: Vec3,
    b: Vec3,
    eye: Vec3,
    width: f32,
    color: [f32; 3],
) {
    let dir = b - a;
    let to_eye = eye - (a + b) * 0.5;
    let side = dir.cross(to_eye).normalize() * (width * 0.5);
    if side == Vec3::ZERO {
        return; // looking straight down the segment: nothing visible
    }
    let p = [a - side, a + side, b + side, b - side];
    for i in [0, 1, 2, 0, 2, 3] {
        out.push(Vertex {
            pos: p[i].to_array(),
            color,
        });
    }
}

/// Cheap deterministic hash for per-frame jitter.
fn jitter(seed: u32, i: u32) -> f32 {
    let mut x = seed.wrapping_mul(0x9E37_79B9) ^ i.wrapping_mul(0x85EB_CA6B);
    x ^= x >> 15;
    x = x.wrapping_mul(0x2C1B_3C6D);
    x ^= x >> 12;
    (x & 0xFFFF) as f32 / 32767.5 - 1.0
}

pub fn push_beam(
    out: &mut Vec<Vertex>,
    from: Vec3,
    to: Vec3,
    eye: Vec3,
    s: &EffectSettings,
    seed: u32,
) {
    match s.beam_style {
        BeamStyle::None => {}
        BeamStyle::Thin => push_segment(out, from, to, eye, 1.2, s.beam_color),
        BeamStyle::Thick => push_segment(out, from, to, eye, 3.5, s.beam_color),
        BeamStyle::Zigzag => {
            let len = (to - from).length();
            let n = ((len / 24.0) as u32).clamp(2, 40);
            let dir = (to - from).normalize();
            // Two axes perpendicular to the beam for the jag offsets.
            let up = if dir.z.abs() < 0.9 {
                Vec3::new(0.0, 0.0, 1.0)
            } else {
                Vec3::new(1.0, 0.0, 0.0)
            };
            let p1 = dir.cross(up).normalize();
            let p2 = dir.cross(p1);
            let mut prev = from;
            for i in 1..=n {
                let t = i as f32 / n as f32;
                let mut pt = from + (to - from) * t;
                if i < n {
                    pt += p1 * (jitter(seed, i * 2) * 5.0) + p2 * (jitter(seed, i * 2 + 1) * 5.0);
                }
                push_segment(out, prev, pt, eye, 1.6, s.beam_color);
                prev = pt;
            }
        }
    }
}

/// Impact sparks at `at` for a beam travelling along `beam_dir` (unit). Streaks
/// bounce back toward the shooter so they are never hidden inside the target.
pub fn push_sparks(
    out: &mut Vec<Vertex>,
    at: Vec3,
    beam_dir: Vec3,
    eye: Vec3,
    s: &EffectSettings,
    seed: u32,
) {
    if !s.sparks || s.spark_size <= 0.0 {
        return;
    }
    let origin = at - beam_dir * 1.5;
    for i in 0..6 {
        let mut d = Vec3::new(
            jitter(seed, 100 + i * 3),
            jitter(seed, 101 + i * 3),
            jitter(seed, 102 + i * 3),
        );
        // Reflect into the hemisphere facing back along the beam.
        let along = d.dot(beam_dir);
        if along > 0.0 {
            d -= beam_dir * (2.0 * along);
        }
        let d = d.normalize();
        push_segment(
            out,
            origin,
            origin + d * (s.spark_size * 2.5),
            eye,
            s.spark_size * 0.6,
            s.spark_color,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EYE: Vec3 = Vec3::new(0.0, -100.0, 0.0);

    #[test]
    fn styles_vertex_counts() {
        let a = Vec3::ZERO;
        let b = Vec3::new(500.0, 0.0, 0.0);
        let count = |style| {
            let mut v = vec![];
            push_beam(
                &mut v,
                a,
                b,
                EYE,
                &EffectSettings {
                    beam_style: style,
                    ..Default::default()
                },
                1,
            );
            v.len()
        };
        assert_eq!(count(BeamStyle::None), 0);
        assert_eq!(count(BeamStyle::Thin), 6);
        assert_eq!(count(BeamStyle::Thick), 6);
        assert_eq!(count(BeamStyle::Zigzag), 6 * 20);
    }

    #[test]
    fn segment_faces_camera_with_width() {
        let mut v = vec![];
        push_segment(
            &mut v,
            Vec3::ZERO,
            Vec3::new(100.0, 0.0, 0.0),
            EYE,
            2.0,
            [1.0; 3],
        );
        // Eye along -y: quad spans z (perpendicular to beam and view), width 2.
        let zs: Vec<f32> = v.iter().map(|v| v.pos[2]).collect();
        assert_eq!(zs.iter().cloned().fold(f32::NEG_INFINITY, f32::max), 1.0);
        assert_eq!(zs.iter().cloned().fold(f32::INFINITY, f32::min), -1.0);
        assert!(v.iter().all(|v| v.pos[1] == 0.0));
    }

    #[test]
    fn zigzag_starts_and_ends_on_target() {
        let mut v = vec![];
        let b = Vec3::new(300.0, 50.0, 20.0);
        push_beam(
            &mut v,
            Vec3::ZERO,
            b,
            EYE,
            &EffectSettings {
                beam_style: BeamStyle::Zigzag,
                ..Default::default()
            },
            7,
        );
        let mid = |i: usize| {
            let q = &v[i..i + 6];
            let s: [f32; 3] =
                [0, 1, 2].map(|k| (q[0].pos[k] + q[1].pos[k] + q[2].pos[k] + q[5].pos[k]) / 4.0);
            Vec3::from_array(s)
        };
        // Segment endpoints are the quad centres of first and last segments' ends.
        let first_start = Vec3::from_array([0, 1, 2].map(|k| (v[0].pos[k] + v[1].pos[k]) / 2.0));
        let last = v.len() - 6;
        let last_end =
            Vec3::from_array([0, 1, 2].map(|k| (v[last + 2].pos[k] + v[last + 5].pos[k]) / 2.0));
        assert!(first_start.length() < 1e-3);
        assert!((last_end - b).length() < 1e-3);
        let _ = mid(0);
    }

    #[test]
    fn sparks_toggle_and_size() {
        let dir = Vec3::new(1.0, 0.0, 0.0);
        let mut v = vec![];
        push_sparks(
            &mut v,
            Vec3::ZERO,
            dir,
            EYE,
            &EffectSettings {
                sparks: false,
                ..Default::default()
            },
            3,
        );
        assert!(v.is_empty());
        push_sparks(
            &mut v,
            Vec3::ZERO,
            dir,
            EYE,
            &EffectSettings {
                spark_size: 4.0,
                ..Default::default()
            },
            3,
        );
        assert!(!v.is_empty());
        let reach = v
            .iter()
            .map(|v| Vec3::from_array(v.pos).length())
            .fold(0.0f32, f32::max);
        assert!(
            reach <= 1.5 + 4.0 * 2.5 + 4.0 * 0.6 && reach > 4.0,
            "{reach}"
        );
        // Every spark vertex stays on the shooter's side of the impact surface.
        assert!(
            v.iter().all(|v| v.pos[0] < 0.0),
            "sparks must not enter the target"
        );
    }

    #[test]
    fn parse_styles() {
        assert_eq!(BeamStyle::parse("none"), Some(BeamStyle::None));
        assert_eq!(BeamStyle::parse("zigzag"), Some(BeamStyle::Zigzag));
        assert_eq!(BeamStyle::parse("rail"), None);
    }
}
