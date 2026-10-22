//! Static world geometry: a list of solid axis-aligned boxes plus spawn points,
//! loaded from a TOML map file so the community can add maps.

use crate::math::{Angle, Vec3};
use serde::{Deserialize, Serialize};
use std::fmt;

/// Axis-aligned box.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Aabb {
    pub min: Vec3,
    pub max: Vec3,
}

impl Aabb {
    pub fn new(min: Vec3, max: Vec3) -> Self {
        Self { min, max }
    }

    /// Box of `mins..maxs` placed at `origin`.
    pub fn at(origin: Vec3, mins: Vec3, maxs: Vec3) -> Self {
        Self::new(origin + mins, origin + maxs)
    }

    /// Strict overlap (touching faces do not count).
    pub fn overlaps(&self, o: &Aabb) -> bool {
        self.min.x < o.max.x
            && self.max.x > o.min.x
            && self.min.y < o.max.y
            && self.max.y > o.min.y
            && self.min.z < o.max.z
            && self.max.z > o.min.z
    }

    /// Minkowski expansion by a hull (for point/ray tests against boxes).
    pub fn expanded(&self, mins: Vec3, maxs: Vec3) -> Aabb {
        Aabb::new(self.min - maxs, self.max - mins)
    }
}

/// Player spawn point; `origin` is the player origin (hull centre-ish, feet at origin.z + mins.z).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Spawn {
    pub origin: Vec3,
    pub yaw: Angle,
}

#[derive(Debug, Clone, PartialEq)]
pub struct World {
    pub name: String,
    pub brushes: Vec<Aabb>,
    /// RGB per brush for flat-shaded rendering (sim ignores it).
    pub colors: Vec<[f32; 3]>,
    /// Non-box solids (ramps, wedges); each carries its own colour.
    pub convex: Vec<crate::brush::ConvexBrush>,
    pub spawns: Vec<Spawn>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum MapError {
    Parse(String),
    Invalid(String),
}

impl fmt::Display for MapError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MapError::Parse(m) => write!(f, "map parse error: {m}"),
            MapError::Invalid(m) => write!(f, "invalid map: {m}"),
        }
    }
}

impl std::error::Error for MapError {}

// On-disk format.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MapFile {
    name: String,
    #[serde(default = "default_color")]
    default_color: [f32; 3],
    brush: Vec<BrushFile>,
    #[serde(default)]
    ramp: Vec<RampFile>,
    #[serde(default)]
    stairs: Vec<StairsFile>,
    #[serde(default)]
    convex: Vec<ConvexFile>,
    spawn: Vec<SpawnFile>,
}

/// Wedge in the box min..max rising toward `rise` ("+x", "-x", "+y", "-y").
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RampFile {
    min: [f32; 3],
    max: [f32; 3],
    rise: String,
    color: Option<[f32; 3]>,
}

/// `steps` boxes in min..max climbing toward `rise`, the last reaching max.z.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StairsFile {
    min: [f32; 3],
    max: [f32; 3],
    rise: String,
    steps: u32,
    color: Option<[f32; 3]>,
}

/// Any convex solid: planes as [nx, ny, nz, d] (inside: n·p <= d).
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ConvexFile {
    planes: Vec<[f32; 4]>,
    color: Option<[f32; 3]>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BrushFile {
    min: [f32; 3],
    max: [f32; 3],
    color: Option<[f32; 3]>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SpawnFile {
    origin: [f32; 3],
    #[serde(default)]
    yaw: f64,
}

fn default_color() -> [f32; 3] {
    [0.5, 0.5, 0.5]
}

impl World {
    /// Parse and validate a map. `hull` = standing player (mins, maxs), used to
    /// check that every spawn is in free space.
    pub fn from_toml_str(s: &str, hull: (Vec3, Vec3)) -> Result<World, MapError> {
        let f: MapFile = toml::from_str(s).map_err(|e| MapError::Parse(e.to_string()))?;
        let bad = |m: String| Err(MapError::Invalid(m));
        if f.brush.is_empty() {
            return bad("map has no brushes".into());
        }
        if f.spawn.len() < 2 {
            return bad("map needs at least 2 spawns".into());
        }
        let mut brushes = Vec::with_capacity(f.brush.len());
        let mut colors = Vec::with_capacity(f.brush.len());
        for (i, b) in f.brush.iter().enumerate() {
            let all = b.min.iter().chain(&b.max);
            if all.clone().any(|v| !v.is_finite()) {
                return bad(format!("brush {i}: non-finite coordinate"));
            }
            if (0..3).any(|a| b.min[a] >= b.max[a]) {
                return bad(format!("brush {i}: min must be < max on every axis"));
            }
            brushes.push(Aabb::new(Vec3::from_array(b.min), Vec3::from_array(b.max)));
            colors.push(b.color.unwrap_or(f.default_color));
        }
        let rise = |s: &str, what: String| {
            crate::brush::Rise::parse(s).ok_or(MapError::Invalid(format!(
                "{what}: rise must be +x, -x, +y or -y"
            )))
        };
        let check_box = |min: [f32; 3], max: [f32; 3], what: &str| -> Result<(), MapError> {
            if min.iter().chain(&max).any(|v| !v.is_finite()) || (0..3).any(|a| min[a] >= max[a]) {
                return Err(MapError::Invalid(format!(
                    "{what}: min must be < max (finite) on every axis"
                )));
            }
            Ok(())
        };
        for (i, s) in f.stairs.iter().enumerate() {
            let what = format!("stairs {i}");
            check_box(s.min, s.max, &what)?;
            let r = rise(&s.rise, what.clone())?;
            if s.steps == 0 || s.steps > 64 {
                return bad(format!("{what}: steps must be 1..=64"));
            }
            let (min, max) = (Vec3::from_array(s.min), Vec3::from_array(s.max));
            let n = s.steps as f32;
            for k in 0..s.steps {
                let kf = k as f32;
                let top = min.z + (max.z - min.z) * (kf + 1.0) / n;
                // Each step reaches from its front edge to the high end.
                let (mut lo, mut hi) = (min, Vec3::new(max.x, max.y, top));
                match r {
                    crate::brush::Rise::PosX => lo.x = min.x + (max.x - min.x) * kf / n,
                    crate::brush::Rise::NegX => hi.x = max.x - (max.x - min.x) * kf / n,
                    crate::brush::Rise::PosY => lo.y = min.y + (max.y - min.y) * kf / n,
                    crate::brush::Rise::NegY => hi.y = max.y - (max.y - min.y) * kf / n,
                }
                brushes.push(Aabb::new(lo, hi));
                colors.push(s.color.unwrap_or(f.default_color));
            }
        }
        let mut convex = Vec::new();
        for (i, r) in f.ramp.iter().enumerate() {
            let what = format!("ramp {i}");
            check_box(r.min, r.max, &what)?;
            let dir = rise(&r.rise, what.clone())?;
            let c = r.color.unwrap_or(f.default_color);
            convex.push(
                crate::brush::ConvexBrush::ramp(
                    Vec3::from_array(r.min),
                    Vec3::from_array(r.max),
                    dir,
                    c,
                )
                .map_err(|e| MapError::Invalid(format!("{what}: {e}")))?,
            );
        }
        for (i, c) in f.convex.iter().enumerate() {
            let faces: Vec<(Vec3, f32)> = c
                .planes
                .iter()
                .map(|p| (Vec3::new(p[0], p[1], p[2]), p[3]))
                .collect();
            convex.push(
                crate::brush::ConvexBrush::from_planes(&faces, c.color.unwrap_or(f.default_color))
                    .map_err(|e| MapError::Invalid(format!("convex {i}: {e}")))?,
            );
        }
        let mut spawns = Vec::with_capacity(f.spawn.len());
        for (i, s) in f.spawn.iter().enumerate() {
            let origin = Vec3::from_array(s.origin);
            let body = Aabb::at(origin, hull.0, hull.1);
            if let Some(j) = brushes.iter().position(|b| b.overlaps(&body)) {
                return bad(format!("spawn {i} overlaps brush {j}"));
            }
            if crate::trace::trace_convex(&convex, origin, origin, hull.0, hull.1).start_solid {
                return bad(format!("spawn {i} is inside a ramp/convex solid"));
            }
            spawns.push(Spawn {
                origin,
                yaw: Angle::from_degrees(s.yaw),
            });
        }
        Ok(World {
            name: f.name,
            brushes,
            colors,
            convex,
            spawns,
        })
    }
}

/// Maps shipped in the repo (`maps/`), embedded so tests and the server always have them.
pub mod builtin {
    pub const ARENA_FLAT: &str = include_str!("../../../maps/arena_flat.toml");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::PhysicsConfig;

    fn hull() -> (Vec3, Vec3) {
        let p = PhysicsConfig::default();
        (
            Vec3::from_array(p.player_mins),
            Vec3::from_array(p.player_maxs),
        )
    }

    #[test]
    fn builtin_maps_load() {
        let w = World::from_toml_str(builtin::ARENA_FLAT, hull()).unwrap();
        assert!(w.brushes.len() >= 6);
        assert_eq!(w.colors.len(), w.brushes.len());
        assert!(w.spawns.len() >= 2);
    }

    #[test]
    fn rejects_bad_maps() {
        let base =
            "name = \"t\"\n[[brush]]\nmin = [-100.0, -100.0, -16.0]\nmax = [100.0, 100.0, 0.0]\n";
        let spawns =
            "[[spawn]]\norigin = [0.0, 0.0, 24.0]\n[[spawn]]\norigin = [50.0, 0.0, 24.0]\n";
        World::from_toml_str(&format!("{base}{spawns}"), hull()).unwrap();
        for doc in [
            // spawn in floor
            format!(
                "{base}[[spawn]]\norigin = [0.0, 0.0, 10.0]\n[[spawn]]\norigin = [50.0, 0.0, 24.0]\n"
            ),
            // one spawn
            format!("{base}[[spawn]]\norigin = [0.0, 0.0, 24.0]\n"),
            // inverted brush
            format!(
                "name = \"t\"\n[[brush]]\nmin = [1.0, 0.0, 0.0]\nmax = [0.0, 1.0, 1.0]\n{spawns}"
            ),
        ] {
            assert!(
                matches!(
                    World::from_toml_str(&doc, hull()),
                    Err(MapError::Invalid(_))
                ),
                "{doc}"
            );
        }
        assert!(matches!(
            World::from_toml_str("name = 1", hull()),
            Err(MapError::Parse(_))
        ));
    }

    #[test]
    fn ramps_stairs_and_convex_load() {
        let doc = r#"
name = "t"
[[brush]]
min = [-500.0, -500.0, -16.0]
max = [500.0, 500.0, 0.0]
[[ramp]]
min = [0.0, -50.0, 0.0]
max = [100.0, 50.0, 40.0]
rise = "+x"
color = [1.0, 0.0, 0.0]
[[stairs]]
min = [-200.0, -50.0, 0.0]
max = [-100.0, 50.0, 64.0]
rise = "-x"
steps = 4
[[convex]]
planes = [[0.0, 0.0, -1.0, 0.0], [1.0, 0.0, 1.0, 300.0], [-1.0, 0.0, 0.0, -200.0], [0.0, 1.0, 0.0, 50.0], [0.0, -1.0, 0.0, 50.0]]
[[spawn]]
origin = [-400.0, 0.0, 24.0]
[[spawn]]
origin = [400.0, 0.0, 24.0]
"#;
        let w = World::from_toml_str(doc, hull()).unwrap();
        assert_eq!(w.brushes.len(), 1 + 4, "floor + 4 steps");
        assert_eq!(w.convex.len(), 2);
        assert_eq!(w.convex[0].color, [1.0, 0.0, 0.0]);
        // Highest step is at the -x end (rise -x), reaching max.z.
        let tallest = w.brushes[1..]
            .iter()
            .max_by(|a, b| a.max.z.total_cmp(&b.max.z))
            .unwrap();
        assert_eq!(tallest.max.z, 64.0);
        assert_eq!(tallest.min.x, -200.0);
        // Bad entries are rejected with a reason.
        for (bad, why) in [
            (doc.replace("rise = \"+x\"", "rise = \"up\""), "rise"),
            (doc.replace("steps = 4", "steps = 0"), "steps"),
            (
                doc.replace("origin = [400.0, 0.0, 24.0]", "origin = [80.0, 0.0, 24.0]"),
                "inside a ramp",
            ),
        ] {
            match World::from_toml_str(&bad, hull()) {
                Err(MapError::Invalid(m)) => assert!(m.contains(why), "{m}"),
                other => panic!("{why}: {other:?}"),
            }
        }
    }

    #[test]
    fn aabb_overlap_and_expand() {
        let a = Aabb::new(Vec3::ZERO, Vec3::new(1.0, 1.0, 1.0));
        let touching = Aabb::new(Vec3::new(1.0, 0.0, 0.0), Vec3::new(2.0, 1.0, 1.0));
        assert!(!a.overlaps(&touching));
        let inside = Aabb::new(Vec3::new(0.5, 0.5, 0.5), Vec3::new(3.0, 3.0, 3.0));
        assert!(a.overlaps(&inside));
        let e = a.expanded(Vec3::new(-1.0, -1.0, -2.0), Vec3::new(1.0, 1.0, 3.0));
        assert_eq!(e.min, Vec3::new(-1.0, -1.0, -3.0));
        assert_eq!(e.max, Vec3::new(2.0, 2.0, 3.0));
    }
}
