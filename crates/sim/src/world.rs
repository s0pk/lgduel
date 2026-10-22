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
    spawn: Vec<SpawnFile>,
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
        let mut spawns = Vec::with_capacity(f.spawn.len());
        for (i, s) in f.spawn.iter().enumerate() {
            let origin = Vec3::from_array(s.origin);
            let body = Aabb::at(origin, hull.0, hull.1);
            if let Some(j) = brushes.iter().position(|b| b.overlaps(&body)) {
                return bad(format!("spawn {i} overlaps brush {j}"));
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
            convex: Vec::new(),
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
