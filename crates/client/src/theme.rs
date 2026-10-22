//! Map themes: what the world looks like. A theme gives the sky colour and a
//! set of materials (texture, tint, scale); map entries pick a material with
//! their `material` tag, untagged surfaces get one by orientation (floor / wall /
//! ceiling / ramp). Anything a theme doesn't define falls back to the classic
//! flat look, so a theme can be as small as one material.
//!
//! Themes are client-side only: they never reach the server, the simulation or
//! the map's content hash.

use crate::render::{Look, Surface, WorldStyle};
use crate::textures::{Image, TEX, TextureSet};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};

pub const CLASSIC: &str = "classic";

/// Clear colour of the original look.
pub const CLASSIC_SKY: [f32; 3] = [0.02, 0.02, 0.03];

/// One material.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Material {
    /// Built-in texture name or (user themes) a PNG file next to `theme.toml`; none = untextured.
    pub texture: Option<String>,
    /// Surface colour; none = the map's colour.
    pub color: Option<[f32; 3]>,
    /// With `color`: also multiply by the map's colour.
    pub tint_map_color: bool,
    /// World units per texture repeat.
    pub scale: f32,
    /// Per-face brightness (top brighter than sides) on or off.
    pub shade: bool,
}

impl Default for Material {
    fn default() -> Self {
        Self {
            texture: None,
            color: None,
            tint_map_color: false,
            scale: 128.0,
            shade: true,
        }
    }
}

/// Material names for untagged surfaces.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Defaults {
    pub floor: String,
    pub wall: String,
    pub ceiling: String,
    pub ramp: String,
}

impl Default for Defaults {
    fn default() -> Self {
        Self {
            floor: "floor".into(),
            wall: "wall".into(),
            ceiling: "ceiling".into(),
            ramp: "ramp".into(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct MapOverride {
    pub materials: BTreeMap<String, Material>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Theme {
    pub name: String,
    /// Clear colour (linear).
    pub sky: [f32; 3],
    pub defaults: Defaults,
    pub materials: BTreeMap<String, Material>,
    /// Per-map overrides, keyed by map name.
    pub maps: BTreeMap<String, MapOverride>,
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            name: CLASSIC.into(),
            sky: CLASSIC_SKY,
            defaults: Defaults::default(),
            materials: BTreeMap::new(),
            maps: BTreeMap::new(),
        }
    }
}

const GRID_TOML: &str = r#"
name = "grid"
sky = [0.05, 0.06, 0.08]

[materials.floor]
texture = "tiles"
tint_map_color = true
color = [1.0, 1.0, 1.0]
scale = 128.0

[materials.ramp]
texture = "tiles"
tint_map_color = true
color = [1.0, 1.0, 1.0]
scale = 128.0

[materials.wall]
texture = "concrete"
tint_map_color = true
color = [1.0, 1.0, 1.0]
scale = 256.0

[materials.ceiling]
texture = "panel"
tint_map_color = true
color = [1.0, 1.0, 1.0]
scale = 256.0
"#;

/// Names of the themes built into the game.
pub const BUILTIN: [&str; 2] = [CLASSIC, "grid"];

impl Theme {
    pub fn classic() -> Theme {
        Theme::default()
    }

    /// A built-in theme by name.
    pub fn builtin(name: &str) -> Option<Theme> {
        match name {
            CLASSIC => Some(Theme::classic()),
            "grid" => Some(toml::from_str(GRID_TOML).expect("built-in grid theme parses")),
            _ => None,
        }
    }

    // Used by the tests now and by user themes (loaded from folders) next.
    #[allow(dead_code)]
    pub fn from_toml(text: &str) -> Result<Theme, String> {
        toml::from_str(text).map_err(|e| e.to_string())
    }

    /// Every texture name any material uses.
    fn texture_names(&self) -> Vec<String> {
        let mut v: Vec<String> = self
            .materials
            .values()
            .chain(self.maps.values().flat_map(|m| m.materials.values()))
            .filter_map(|m| m.texture.clone())
            .collect();
        v.sort();
        v.dedup();
        v
    }

    fn lookup(&self, map: &str, tag: &str) -> Option<&Material> {
        self.maps
            .get(map)
            .and_then(|m| m.materials.get(tag))
            .or_else(|| self.materials.get(tag))
    }
}

/// Which default material an untagged surface gets.
fn orientation_tag<'a>(d: &'a Defaults, s: &Surface) -> &'a str {
    let nz = s.normal.z;
    if s.is_convex && nz > 0.2 && nz <= 0.99 {
        &d.ramp
    } else if nz > 0.7 {
        &d.floor
    } else if nz < -0.7 {
        &d.ceiling
    } else {
        &d.wall
    }
}

/// A theme ready to draw with: textures loaded into layers.
#[derive(Debug)]
pub struct ThemeStyle {
    pub theme: Theme,
    textures: TextureSet,
    /// texture name → layer
    layers: HashMap<String, u32>,
    key: u64,
    /// Problems found while loading (shown once to the player).
    pub problems: Vec<String>,
}

fn classic_look(s: &Surface) -> Look {
    Look {
        layer: 0,
        color: s.map_color,
        scale: 128.0,
        shade: true,
    }
}

impl ThemeStyle {
    /// Build with `load` supplying each texture by name (None = missing → untextured).
    pub fn build(theme: Theme, load: &dyn Fn(&str) -> Result<Image, String>) -> ThemeStyle {
        let mut textures = TextureSet::new();
        let mut layers = HashMap::new();
        let mut problems = Vec::new();
        for name in theme.texture_names() {
            match load(&name) {
                Ok(img) => {
                    layers.insert(name.clone(), textures.add(&name, img));
                }
                Err(e) => problems.push(format!("theme {}: texture {name}: {e}", theme.name)),
            }
        }
        let key = {
            // A cheap stable fingerprint of the theme and its texture pixels.
            let mut h = lgduel_sim::math::StateHasher::new();
            h.bytes(theme.name.as_bytes());
            let mut names: Vec<_> = layers.iter().collect();
            names.sort();
            for (n, l) in names {
                h.bytes(n.as_bytes());
                h.u32(*l);
            }
            for img in &textures.layers {
                h.bytes(&img.rgba[..64]);
                h.bytes(&img.rgba[img.rgba.len() / 2..img.rgba.len() / 2 + 64]);
            }
            let toml_text = toml::to_string(&theme).unwrap_or_default();
            h.bytes(toml_text.as_bytes());
            h.finish()
        };
        ThemeStyle {
            theme,
            textures,
            layers,
            key,
            problems,
        }
    }

    /// A built-in theme; unknown names give classic.
    pub fn builtin(name: &str) -> ThemeStyle {
        let theme = Theme::builtin(name).unwrap_or_else(Theme::classic);
        ThemeStyle::build(theme, &|n| {
            procedural(n).ok_or_else(|| "no such built-in texture".to_string())
        })
    }

    pub fn name(&self) -> &str {
        &self.theme.name
    }

    pub fn is_classic(&self) -> bool {
        self.theme.name == CLASSIC
    }

    /// Clear colour for this theme.
    pub fn sky(&self) -> wgpu::Color {
        let [r, g, b] = self.theme.sky;
        wgpu::Color {
            r: r as f64,
            g: g as f64,
            b: b as f64,
            a: 1.0,
        }
    }
}

impl WorldStyle for ThemeStyle {
    fn key(&self) -> u64 {
        self.key
    }

    fn textures(&self) -> &TextureSet {
        &self.textures
    }

    fn look(&self, map: &str, s: Surface) -> Look {
        let tag = s.material.filter(|t| self.theme.lookup(map, t).is_some());
        let tag = tag.unwrap_or_else(|| orientation_tag(&self.theme.defaults, &s));
        let Some(m) = self.theme.lookup(map, tag) else {
            return classic_look(&s);
        };
        let color = match (m.color, m.tint_map_color) {
            (Some(c), true) => [
                c[0] * s.map_color[0],
                c[1] * s.map_color[1],
                c[2] * s.map_color[2],
            ],
            (Some(c), false) => c,
            (None, _) => s.map_color,
        };
        Look {
            layer: m
                .texture
                .as_ref()
                .and_then(|t| self.layers.get(t))
                .copied()
                .unwrap_or(0),
            color,
            scale: m.scale,
            shade: m.shade,
        }
    }
}

// ---- procedural textures ------------------------------------------------

/// Deterministic integer hash (no floats, no platform dependence).
fn hash2(x: u32, y: u32, seed: u32) -> u32 {
    let mut h =
        x.wrapping_mul(0x9E37_79B1) ^ y.wrapping_mul(0x85EB_CA77) ^ seed.wrapping_mul(0xC2B2_AE3D);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2C1B_3C6D);
    h ^= h >> 12;
    h = h.wrapping_mul(0x297A_2D39);
    h ^ (h >> 15)
}

fn from_fn(f: impl Fn(u32, u32) -> u8) -> Image {
    let mut rgba = Vec::with_capacity((TEX * TEX * 4) as usize);
    for y in 0..TEX {
        for x in 0..TEX {
            let g = f(x, y);
            rgba.extend_from_slice(&[g, g, g, 255]);
        }
    }
    Image { rgba }
}

/// Built-in textures by name. They are light, low-contrast greys (they are
/// multiplied by a colour), so players in bright forced colours stay the most
/// saturated thing on screen.
pub fn procedural(name: &str) -> Option<Image> {
    Some(match name {
        // 4x4 tiles of 64 px, dark 2 px grout lines.
        "tiles" => from_fn(|x, y| {
            let line = (x % 64) < 2 || (y % 64) < 2;
            let n = (hash2(x / 64, y / 64, 1) % 12) as u8; // slight per-tile variation
            if line { 120 } else { 205 - n }
        }),
        // Soft value noise.
        "concrete" => from_fn(|x, y| {
            let cell = |s: u32, sz: u32| hash2(x / sz, y / sz, s) % 32;
            (190 + cell(2, 32) / 2 + cell(3, 8) / 4 + (hash2(x, y, 4) % 8)) as u8
        }),
        // Horizontal panel stripes with seams.
        "panel" => from_fn(|x, y| {
            let seam = (y % 32) < 2 || (x % 128) < 2;
            let band = if (y / 32) % 2 == 0 { 215 } else { 195 };
            if seam { 135 } else { band }
        }),
        _ => return None,
    })
}

/// Themes the player can pick (built-ins; folders are added in the user-theme milestone).
pub fn available() -> Vec<String> {
    BUILTIN.iter().map(|s| s.to_string()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use lgduel_sim::math::Vec3;

    fn surface(tag: Option<&str>, n: Vec3, convex: bool) -> Surface<'_> {
        Surface {
            material: tag,
            normal: n,
            map_color: [0.4, 0.5, 0.6],
            is_convex: convex,
        }
    }

    const UP: Vec3 = Vec3 {
        x: 0.0,
        y: 0.0,
        z: 1.0,
    };
    const DOWN: Vec3 = Vec3 {
        x: 0.0,
        y: 0.0,
        z: -1.0,
    };
    const SIDE: Vec3 = Vec3 {
        x: 1.0,
        y: 0.0,
        z: 0.0,
    };

    #[test]
    fn classic_theme_looks_classic_everywhere() {
        let s = ThemeStyle::builtin("classic");
        assert!(s.is_classic());
        for (tag, n, convex) in [
            (None, UP, false),
            (Some("trim"), SIDE, false),
            (None, DOWN, false),
            (None, UP, true),
        ] {
            let l = s.look("arena_flat", surface(tag, n, convex));
            assert_eq!(
                l,
                Look {
                    layer: 0,
                    color: [0.4, 0.5, 0.6],
                    scale: 128.0,
                    shade: true
                }
            );
        }
        let w = s.sky();
        assert!(
            (w.r - 0.02).abs() < 1e-6 && (w.g - 0.02).abs() < 1e-6 && (w.b - 0.03).abs() < 1e-6
        );
    }

    #[test]
    fn grid_theme_textures_by_orientation() {
        let s = ThemeStyle::builtin("grid");
        assert!(s.problems.is_empty(), "{:?}", s.problems);
        let layer = |n, convex| s.look("arena_flat", surface(None, n, convex)).layer;
        let (floor, wall, ceiling) = (layer(UP, false), layer(SIDE, false), layer(DOWN, false));
        assert!(floor > 0 && wall > 0 && ceiling > 0);
        assert_ne!(wall, ceiling);
        // A gentle slope is a ramp: same texture as the floor in this theme.
        assert_eq!(layer(Vec3::new(-0.4, 0.0, 0.9), true), floor);
        // tint_map_color multiplies by the map colour.
        let l = s.look("arena_flat", surface(None, UP, false));
        assert_eq!(l.color, [0.4, 0.5, 0.6]);
    }

    #[test]
    fn resolution_order_tag_override_orientation_fallback() {
        let theme = Theme::from_toml(
            r#"
name = "t"
[defaults]
floor = "ground"
[materials.ground]
color = [0.1, 0.1, 0.1]
[materials.trim]
color = [0.9, 0.0, 0.0]
[maps.special.materials.trim]
color = [0.0, 0.9, 0.0]
"#,
        )
        .unwrap();
        let s = ThemeStyle::build(theme, &|_| Err("none".into()));
        let col = |map, tag, n| s.look(map, surface(tag, n, false)).color;
        assert_eq!(col("m", Some("trim"), SIDE), [0.9, 0.0, 0.0], "tag");
        assert_eq!(
            col("special", Some("trim"), SIDE),
            [0.0, 0.9, 0.0],
            "per-map override"
        );
        assert_eq!(col("m", None, UP), [0.1, 0.1, 0.1], "orientation default");
        assert_eq!(
            col("m", Some("unknown"), UP),
            [0.1, 0.1, 0.1],
            "unknown tag → orientation"
        );
        assert_eq!(
            col("m", None, SIDE),
            [0.4, 0.5, 0.6],
            "no wall material → classic"
        );
    }

    #[test]
    fn theme_toml_rejects_typos() {
        assert!(Theme::from_toml("sky = [0.0, 0.0, 0.0]\nbogus = 1\n").is_err());
        assert!(Theme::from_toml("[materials.floor]\ntextur = \"tiles\"\n").is_err());
        assert!(Theme::from_toml("[materials.floor]\ntexture = \"tiles\"\n").is_ok());
    }

    #[test]
    fn missing_textures_are_reported_and_drawn_untextured() {
        let theme =
            Theme::from_toml("name = \"t\"\n[materials.floor]\ntexture = \"nope.png\"\n").unwrap();
        let s = ThemeStyle::build(theme, &|_| Err("not found".into()));
        assert_eq!(
            s.problems,
            vec!["theme t: texture nope.png: not found".to_string()]
        );
        assert_eq!(s.look("m", surface(None, UP, false)).layer, 0);
    }

    #[test]
    fn procedural_textures_are_deterministic_and_light() {
        let mut sum = 0u64;
        for name in ["tiles", "concrete", "panel"] {
            let a = procedural(name).unwrap();
            assert_eq!(a, procedural(name).unwrap());
            assert_eq!(a.rgba.len(), (TEX * TEX * 4) as usize);
            let mean = a.rgba.chunks(4).map(|p| p[0] as u64).sum::<u64>() / (TEX * TEX) as u64;
            assert!((150..235).contains(&mean), "{name}: mean {mean}");
            sum += mean;
        }
        // Pinned fingerprint of the three textures (change deliberately).
        assert_eq!(sum, PINNED_MEANS);
        assert!(procedural("nope").is_none());
    }

    #[test]
    fn style_key_changes_with_the_theme() {
        let a = ThemeStyle::builtin("classic");
        let b = ThemeStyle::builtin("grid");
        assert_ne!(a.key(), b.key());
        assert_eq!(b.key(), ThemeStyle::builtin("grid").key());
    }

    const PINNED_MEANS: u64 = 598;
}
