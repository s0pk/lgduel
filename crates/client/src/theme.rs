//! Map themes: what the world looks like. A theme gives the sky colour and a
//! set of materials (texture, tint, scale); map entries pick a material with
//! their `material` tag, untagged surfaces get one by orientation (floor / wall /
//! ceiling / ramp). Anything a theme doesn't define falls back to the classic
//! flat look, so a theme can be as small as one material.
//!
//! Themes are client-side only: they never reach the server, the simulation or
//! the map's content hash.

use crate::render::{Look, Surface, WorldStyle};
use crate::textures::{self, Image, TEX, TextureSet};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

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
    /// The name asked for (settings value); `theme.name` is what was loaded (classic on failure).
    pub requested: String,
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
                h.bytes(&img.rgba);
            }
            let toml_text = toml::to_string(&theme).unwrap_or_default();
            h.bytes(toml_text.as_bytes());
            h.finish()
        };
        ThemeStyle {
            requested: theme.name.clone(),
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

    /// The theme `name`: built-in, else a folder `<dir>/<name>/theme.toml` in the first
    /// of `dirs` that has one. Any failure gives the classic look plus a problem message.
    pub fn load(name: &str, dirs: &[PathBuf]) -> ThemeStyle {
        let mut style = Self::load_inner(name, dirs);
        style.requested = name.to_string();
        style
    }

    fn load_inner(name: &str, dirs: &[PathBuf]) -> ThemeStyle {
        if BUILTIN.contains(&name) {
            return Self::builtin(name);
        }
        let fail = |why: String| {
            let mut s = Self::builtin(CLASSIC);
            s.problems
                .push(format!("theme {name}: {why}; using classic"));
            s
        };
        let Some(folder) = find_theme_dir(name, dirs) else {
            return fail("no such theme (built-in or folder with a theme.toml)".into());
        };
        let text = match std::fs::read_to_string(folder.join("theme.toml")) {
            Ok(t) => t,
            Err(e) => return fail(format!("theme.toml: {e}")),
        };
        let mut theme = match Theme::from_toml(&text) {
            Ok(t) => t,
            Err(e) => return fail(format!("theme.toml: {e}")),
        };
        theme.name = name.to_string();
        let used = std::cell::RefCell::new(std::collections::BTreeSet::new());
        ThemeStyle::build(theme, &|n| {
            let mut u = used.borrow_mut();
            u.insert(n.to_string());
            if u.len() > MAX_THEME_TEXTURES {
                return Err(format!(
                    "too many textures (at most {MAX_THEME_TEXTURES} per theme)"
                ));
            }
            drop(u);
            if n.to_ascii_lowercase().ends_with(".png") {
                textures::load_png_file(&folder, n)
            } else {
                procedural(n)
                    .ok_or_else(|| "no such built-in texture (files end in .png)".to_string())
            }
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

/// Most distinct textures a folder theme may use.
pub const MAX_THEME_TEXTURES: usize = 32;

/// A theme folder name is a plain name (no separators or dot tricks).
pub fn is_valid_theme_name(name: &str) -> bool {
    !name.is_empty() && name != "." && !name.contains("..") && !name.contains(['/', '\\', ':'])
}

/// `<dir>/<name>` for the first dir that has a theme.toml there.
pub fn find_theme_dir(name: &str, dirs: &[PathBuf]) -> Option<PathBuf> {
    if !is_valid_theme_name(name) {
        return None;
    }
    dirs.iter()
        .map(|d| d.join(name))
        .find(|p| p.join("theme.toml").is_file())
}

/// Themes the player can pick: the built-ins, then every folder with a theme.toml.
pub fn available(dirs: &[PathBuf]) -> Vec<String> {
    let mut v: Vec<String> = BUILTIN.iter().map(|s| s.to_string()).collect();
    let mut found = Vec::new();
    for d in dirs {
        let Ok(rd) = std::fs::read_dir(d) else {
            continue;
        };
        for e in rd.flatten() {
            let n = e.file_name().to_string_lossy().to_string();
            if e.path().join("theme.toml").is_file() && is_valid_theme_name(&n) && !v.contains(&n) {
                found.push(n);
            }
        }
    }
    found.sort();
    found.dedup();
    v.extend(found);
    v
}

/// The folders themes are searched in: an explicit one, the settings folder's
/// `themes/`, and `themes/` next to the executable.
pub fn search_dirs(extra: Option<&Path>, settings_dir: Option<&Path>) -> Vec<PathBuf> {
    let mut v = Vec::new();
    if let Some(e) = extra {
        v.push(e.to_path_buf());
    }
    if let Some(s) = settings_dir {
        v.push(s.join("themes"));
    }
    if let Some(exe) = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.join("themes")))
    {
        v.push(exe);
    }
    #[cfg(target_os = "linux")]
    {
        let var = |k: &str| {
            std::env::var_os(k)
                .filter(|v| !v.is_empty())
                .map(PathBuf::from)
        };
        let exe = std::env::current_exe().ok();
        v.extend(linux_theme_dirs(
            var("HOME").as_deref(),
            var("XDG_DATA_HOME").as_deref(),
            exe.as_deref(),
        ));
    }
    v
}

/// Where Linux installs look for themes: the user's data folder, then the
/// package's shared folder (relative to the binary, so `/usr/bin/lgduel`
/// finds `/usr/share/lgduel/themes`), then the system folders.
// Only called on Linux; kept compiled everywhere so the test runs on every OS.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn linux_theme_dirs(
    home: Option<&Path>,
    data_home: Option<&Path>,
    exe: Option<&Path>,
) -> Vec<PathBuf> {
    let mut v = Vec::new();
    match (data_home, home) {
        (Some(d), _) => v.push(d.join("lgduel/themes")),
        (None, Some(h)) => v.push(h.join(".local/share/lgduel/themes")),
        _ => {}
    }
    if let Some(dir) = exe.and_then(|e| e.parent()) {
        v.push(dir.join("../share/lgduel/themes"));
    }
    v.push(PathBuf::from("/usr/local/share/lgduel/themes"));
    v.push(PathBuf::from("/usr/share/lgduel/themes"));
    v
}

/// The files of the example theme (`theme.toml` and its PNGs), written by
/// `lgduel --make-example-theme DIR`; a copy lives in `assets/themes/example`.
pub fn example_theme_files() -> Vec<(&'static str, Vec<u8>)> {
    let rgb = |f: &dyn Fn(u32, u32) -> [u8; 3]| {
        let mut px = Vec::with_capacity((TEX * TEX * 3) as usize);
        for y in 0..TEX {
            for x in 0..TEX {
                px.extend_from_slice(&f(x, y));
            }
        }
        textures::encode_png_rgb(TEX, TEX, &px)
    };
    // Warm sandstone floor: 32 px checks with a little grain.
    let floor = rgb(&|x, y| {
        let base = if ((x / 32) + (y / 32)) % 2 == 0 {
            214u32
        } else {
            188
        };
        let v = (base + hash2(x, y, 11) % 10).min(255);
        [v as u8, (v * 92 / 100) as u8, (v * 74 / 100) as u8]
    });
    // Brick wall: 64x32 bricks, offset rows, darker mortar.
    let wall = rgb(&|x, y| {
        let row = y / 32;
        let sx = x + if row % 2 == 1 { 32 } else { 0 };
        let mortar = sx % 64 < 3 || y % 32 < 3;
        let n = hash2(sx / 64, row, 12) % 24;
        if mortar {
            [120, 112, 104]
        } else {
            [
                (190 + n).min(255) as u8,
                (120 + n / 2) as u8,
                (100 + n / 2) as u8,
            ]
        }
    });
    // Cool ceiling: vertical stripes.
    let ceiling = rgb(&|x, _y| {
        let v = if (x / 16) % 2 == 0 { 170u8 } else { 150 };
        [v, v + 8, v + 30]
    });
    let toml = r#"# Example LGDuel theme (CC0). Copy this folder to <settings folder>/themes/<your name>/ and edit.
sky = [0.10, 0.10, 0.16]

# Materials not listed here fall back to the classic look. A map's `material = "..."` tag picks one
# by name; untagged surfaces use the names under [defaults].
[defaults]
floor = "floor"
wall = "wall"
ceiling = "ceiling"
ramp = "floor"

[materials.floor]
texture = "floor.png"      # a PNG next to this file (any size up to 2048x2048; tiling looks best)
tint_map_color = true      # also multiply by the map's own colour
color = [1.0, 1.0, 1.0]
scale = 128.0              # world units per texture repeat

[materials.wall]
texture = "wall.png"
tint_map_color = true
color = [1.0, 1.0, 1.0]
scale = 256.0

[materials.ceiling]
texture = "ceiling.png"
tint_map_color = true
color = [1.0, 1.0, 1.0]
scale = 256.0

# A material maps can ask for, e.g. `material = "trim"` on a [[brush]]:
[materials.trim]
color = [0.8, 0.55, 0.2]
shade = true

# Override a material for one map only:
# [maps.arena_tiers.materials.floor]
# color = [0.6, 0.7, 0.8]
"#;
    vec![
        ("theme.toml", toml.as_bytes().to_vec()),
        ("floor.png", floor),
        ("wall.png", wall),
        ("ceiling.png", ceiling),
    ]
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
    fn linux_theme_dirs_follow_xdg_and_the_install_prefix() {
        let h = Path::new("/home/u");
        let exe = Path::new("/usr/bin/lgduel");
        let d = linux_theme_dirs(Some(h), None, Some(exe));
        assert_eq!(d[0], Path::new("/home/u/.local/share/lgduel/themes"));
        assert_eq!(d[1], Path::new("/usr/bin/../share/lgduel/themes"));
        assert_eq!(d.last().unwrap(), Path::new("/usr/share/lgduel/themes"));
        let d = linux_theme_dirs(Some(h), Some(Path::new("/data")), None);
        assert_eq!(d[0], Path::new("/data/lgduel/themes"));
        assert_eq!(d.len(), 3, "no exe: data dir + two system dirs");
        assert!(linux_theme_dirs(None, None, None).len() == 2);
    }

    #[test]
    fn style_key_changes_with_the_theme() {
        let a = ThemeStyle::builtin("classic");
        let b = ThemeStyle::builtin("grid");
        assert_ne!(a.key(), b.key());
        assert_eq!(b.key(), ThemeStyle::builtin("grid").key());
    }

    const PINNED_MEANS: u64 = 598;

    fn example_dir() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/themes")
    }

    #[test]
    fn committed_example_theme_matches_the_generator_and_loads() {
        let dir = example_dir().join("example");
        for (name, bytes) in example_theme_files() {
            assert_eq!(
                std::fs::read(dir.join(name)).unwrap(),
                bytes,
                "{name} is stale: rerun lgduel --make-example-theme"
            );
        }
        let s = ThemeStyle::load("example", &[example_dir()]);
        assert!(s.problems.is_empty(), "{:?}", s.problems);
        assert_eq!(s.requested, "example");
        assert_eq!(s.name(), "example");
        assert!(!s.is_classic());
        let floor = s.look("arena_flat", surface(None, UP, false));
        let wall = s.look("arena_flat", surface(None, SIDE, false));
        assert!(floor.layer > 0 && wall.layer > 0 && floor.layer != wall.layer);
        // The `trim` material is colour-only.
        let trim = s.look("arena_flat", surface(Some("trim"), SIDE, false));
        assert_eq!((trim.layer, trim.color), (0, [0.8, 0.55, 0.2]));
        assert!(available(&[example_dir()]).contains(&"example".to_string()));
    }

    #[test]
    fn broken_themes_fall_back_to_classic_with_a_message() {
        let tmp = std::env::temp_dir().join(format!("lgduel-themes-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let mk = |name: &str, toml: &str| {
            std::fs::create_dir_all(tmp.join(name)).unwrap();
            std::fs::write(tmp.join(name).join("theme.toml"), toml).unwrap();
        };
        let dirs = [tmp.clone()];
        // Unknown name, bad TOML, unsafe name.
        let s = ThemeStyle::load("nothere", &dirs);
        assert!(
            s.is_classic() && s.problems[0].contains("no such theme"),
            "{:?}",
            s.problems
        );
        assert_eq!(s.requested, "nothere");
        mk("badtoml", "sky = [");
        assert!(ThemeStyle::load("badtoml", &dirs).problems[0].contains("theme.toml"));
        assert!(ThemeStyle::load("../etc", &dirs).problems[0].contains("no such theme"));
        // Missing / escaping / oversized texture files: that material falls back, others still work.
        mk(
            "files",
            "[materials.floor]
texture = \"missing.png\"
[materials.wall]
texture = \"../x.png\"
[materials.ceiling]
texture = \"/abs.png\"
[materials.ramp]
texture = \"tiles\"
",
        );
        let s = ThemeStyle::load("files", &dirs);
        let msgs = s.problems.join(
            "
",
        );
        assert!(msgs.contains("missing.png: not found"), "{msgs}");
        assert_eq!(
            s.problems.iter().filter(|p| p.contains("relative")).count(),
            2,
            "{msgs}"
        );
        assert_eq!(
            s.look("m", surface(None, UP, false)).layer,
            0,
            "floor untextured"
        );
        assert!(
            s.look("m", surface(None, Vec3::new(-0.4, 0.0, 0.9), true))
                .layer
                > 0,
            "built-in texture name still works"
        );
        // Too many textures.
        let many: String = (0..40)
            .map(|i| {
                format!(
                    "[materials.m{i}]
texture = \"t{i}.png\"
"
                )
            })
            .collect();
        mk("many", &many);
        let s = ThemeStyle::load("many", &dirs);
        assert!(s.problems.iter().any(|p| p.contains("too many textures")));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn oversized_texture_files_are_refused() {
        let tmp = std::env::temp_dir().join(format!("lgduel-big-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(
            tmp.join("big.png"),
            vec![0u8; (textures::MAX_FILE_BYTES + 1) as usize],
        )
        .unwrap();
        let e = textures::load_png_file(&tmp, "big.png").unwrap_err();
        assert!(e.contains("largest allowed"), "{e}");
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
