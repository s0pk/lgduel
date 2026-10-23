//! Import from a local KovaaK's install: find it (through Steam's library list or
//! a folder the player gives), turn its themes into LGDuel themes and (later)
//! copy its hit and kill sounds.
//!
//! Everything here only *reads* the KovaaK's folder, and what is imported is copied
//! into LGDuel's own settings folder, so nothing from KovaaK's ever lives in the repo.
//! The `.pak` (built-in textures and sounds) is never touched. All input is hostile:
//! sizes are capped, nothing panics, and problems come back as text.

use crate::offscreen::srgb8_to_linear;
use crate::players::EnemyColors;
use crate::settings::Settings;
use crate::theme::{self, Material, Theme};
use serde_json::Value;
use std::path::{Path, PathBuf};

/// Steam app id of KovaaK's.
const APP_ID: &str = "824270";
/// Marker file in a theme folder we wrote (re-importing may overwrite those only).
const MARKER: &str = ".kovaaks-import";
const MAX_VDF: u64 = 1 << 20;
const MAX_THEME_FILES: usize = 2000;
const MAX_THEME_BYTES: u64 = 256 * 1024;
const MAX_SETTINGS_BYTES: u64 = 256 * 1024;

#[derive(Debug, Clone, PartialEq)]
pub struct Install {
    /// `…/common/FPSAimTrainer`
    pub root: PathBuf,
}

impl Install {
    pub fn game_dir(&self) -> PathBuf {
        self.root.join("FPSAimTrainer")
    }
    pub fn sounds_dir(&self) -> PathBuf {
        self.game_dir().join("sounds")
    }
    pub fn save_dir(&self) -> PathBuf {
        self.game_dir().join("Saved/SaveGames")
    }
    pub fn themes_dir(&self) -> PathBuf {
        self.save_dir().join("Themes")
    }
}

/// Does `root` look like a KovaaK's install (the `.exe` isn't required)?
pub fn is_install(root: &Path) -> bool {
    let g = root.join("FPSAimTrainer");
    g.join("Saved/SaveGames").is_dir() || g.join("sounds").is_dir()
}

// ---- Steam library list (Valve KeyValues text) ----

#[derive(Debug)]
enum Kv {
    Str(String),
    Obj(Vec<(String, Kv)>),
}

fn kv_tokens(text: &str) -> Vec<Result<String, char>> {
    // Ok(string) is a quoted string; Err('{') / Err('}') are braces.
    let mut out = Vec::new();
    let mut it = text.chars().peekable();
    while let Some(c) = it.next() {
        match c {
            '{' | '}' => out.push(Err(c)),
            '"' => {
                let mut s = String::new();
                while let Some(c) = it.next() {
                    match c {
                        '"' => break,
                        '\\' => match it.next() {
                            Some('n') => s.push('\n'),
                            Some('t') => s.push('\t'),
                            Some(o) => s.push(o),
                            None => break,
                        },
                        o => s.push(o),
                    }
                }
                out.push(Ok(s));
            }
            '/' if it.peek() == Some(&'/') => {
                for c in it.by_ref() {
                    if c == '\n' {
                        break;
                    }
                }
            }
            _ => {}
        }
    }
    out
}

/// Parse the members of one object; stops at its `}` or at the end of input.
fn kv_object(toks: &[Result<String, char>], pos: &mut usize, depth: u32) -> Vec<(String, Kv)> {
    let mut members = Vec::new();
    while *pos < toks.len() {
        match &toks[*pos] {
            Err('}') => {
                *pos += 1;
                return members;
            }
            Err(_) => *pos += 1,
            Ok(key) => {
                *pos += 1;
                match toks.get(*pos) {
                    Some(Ok(v)) => {
                        members.push((key.clone(), Kv::Str(v.clone())));
                        *pos += 1;
                    }
                    Some(Err('{')) if depth < 32 => {
                        *pos += 1;
                        let sub = kv_object(toks, pos, depth + 1);
                        members.push((key.clone(), Kv::Obj(sub)));
                    }
                    _ => {}
                }
            }
        }
    }
    members
}

/// The Steam libraries listed in a `libraryfolders.vdf`, each with whether it has
/// KovaaK's installed (its `apps` block lists app 824270). Never panics.
pub fn parse_library_folders(text: &str) -> Vec<(PathBuf, bool)> {
    fn walk(members: &[(String, Kv)], out: &mut Vec<(PathBuf, bool)>) {
        let path = members.iter().find_map(|(k, v)| match v {
            Kv::Str(s) if k == "path" => Some(s),
            _ => None,
        });
        if let Some(p) = path {
            let has = members.iter().any(|(k, v)| match v {
                Kv::Obj(apps) if k == "apps" => apps.iter().any(|(a, _)| a == APP_ID),
                _ => false,
            });
            out.push((PathBuf::from(p), has));
        }
        for (_, v) in members {
            if let Kv::Obj(sub) = v {
                walk(sub, out);
            }
        }
    }
    let text = &text[..text.floor_char_boundary(MAX_VDF as usize)];
    let toks = kv_tokens(text);
    let mut pos = 0;
    let root = kv_object(&toks, &mut pos, 0);
    let mut out = Vec::new();
    walk(&root, &mut out);
    out
}

/// Steam installation folders that exist on this PC.
pub fn steam_roots() -> Vec<PathBuf> {
    let mut c: Vec<PathBuf> = Vec::new();
    let env = |k: &str| {
        std::env::var_os(k)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
    };
    if cfg!(windows) {
        for k in ["ProgramFiles(x86)", "ProgramFiles"] {
            if let Some(p) = env(k) {
                c.push(p.join("Steam"));
            }
        }
        c.push(PathBuf::from(r"C:\Program Files (x86)\Steam"));
    } else if cfg!(target_os = "linux") {
        let home = env("HOME");
        match (env("XDG_DATA_HOME"), &home) {
            (Some(d), _) => c.push(d.join("Steam")),
            (None, Some(h)) => c.push(h.join(".local/share/Steam")),
            _ => {}
        }
        if let Some(h) = &home {
            c.push(h.join(".steam/steam"));
            c.push(h.join(".steam/root"));
            c.push(h.join(".var/app/com.valvesoftware.Steam/.local/share/Steam"));
        }
    }
    let mut seen: Vec<PathBuf> = Vec::new();
    for p in c {
        // Resolve symlinks (`~/.steam/steam` usually points into `~/.local/share`) so
        // the same install isn't tried twice.
        let real = p.canonicalize().unwrap_or(p.clone());
        if p.is_dir() && !seen.contains(&real) {
            seen.push(real);
        }
    }
    seen
}

fn read_capped(path: &Path, max: u64) -> Result<String, String> {
    let len = std::fs::metadata(path).map_err(|e| e.to_string())?.len();
    if len > max {
        return Err(format!(
            "file too large ({} KB, at most {} KB)",
            len / 1024,
            max / 1024
        ));
    }
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let text = String::from_utf8_lossy(&bytes);
    Ok(text.trim_start_matches('\u{feff}').to_string())
}

/// Find KovaaK's: the folder the player gave (the install, its inner game folder or a
/// Steam library holding it), else through Steam's library list.
pub fn find(manual: Option<&Path>) -> Result<Install, String> {
    let mut tried: Vec<String> = Vec::new();
    let in_library = |lib: &Path| lib.join("steamapps/common/FPSAimTrainer");
    if let Some(p) = manual.filter(|p| !p.as_os_str().is_empty()) {
        let mut cands = vec![p.to_path_buf()];
        if p.file_name().is_some_and(|n| n == "FPSAimTrainer")
            && let Some(parent) = p.parent()
        {
            cands.push(parent.to_path_buf());
        }
        cands.push(in_library(p));
        for c in cands {
            if is_install(&c) {
                return Ok(Install { root: c });
            }
            tried.push(c.display().to_string());
        }
        return Err(format!(
            "no KovaaK's install at {} (looked for FPSAimTrainer/Saved/SaveGames or FPSAimTrainer/sounds in: {})",
            p.display(),
            tried.join(", ")
        ));
    }
    let roots = steam_roots();
    if roots.is_empty() {
        return Err("Steam not found; set kovaaks.path to your KovaaK's folder".into());
    }
    let mut libs: Vec<(PathBuf, bool)> = Vec::new();
    for r in &roots {
        let vdf = r.join("steamapps/libraryfolders.vdf");
        match read_capped(&vdf, MAX_VDF) {
            Ok(t) => libs.extend(parse_library_folders(&t)),
            Err(e) => tried.push(format!("{}: {e}", vdf.display())),
        }
    }
    let mut order: Vec<PathBuf> = libs.iter().filter(|l| l.1).map(|l| l.0.clone()).collect();
    order.extend(libs.iter().filter(|l| !l.1).map(|l| l.0.clone()));
    order.extend(roots.iter().cloned());
    for lib in order {
        let c = in_library(&lib);
        if is_install(&c) {
            return Ok(Install { root: c });
        }
        tried.push(c.display().to_string());
    }
    tried.dedup();
    Err(format!("KovaaK's not found (tried: {})", tried.join(", ")))
}

// ---- the user's own settings (read only the keys we need) ----

#[derive(Debug, Default, Clone, PartialEq)]
pub struct UserSettings {
    pub current_theme: Option<String>,
    pub kill_sound: Option<String>,
    pub hit_volume: Option<f32>,
    pub sfx_volume: Option<f32>,
}

fn non_empty(s: &str) -> Option<String> {
    let s = s.trim();
    (!s.is_empty() && !s.eq_ignore_ascii_case("none")).then(|| s.to_string())
}

/// `PrimaryUserSettings.json`: only the selected theme, kill sound and volumes.
pub fn user_settings(inst: &Install) -> Result<UserSettings, String> {
    let text = read_capped(
        &inst.save_dir().join("PrimaryUserSettings.json"),
        MAX_SETTINGS_BYTES,
    )?;
    let v: Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    let string = |k: &str| {
        v.pointer(&format!("/stringSettings/{k}"))
            .and_then(Value::as_str)
            .map(str::to_string)
    };
    let float = |k: &str| {
        v.pointer(&format!("/floatSettings/{k}"))
            .and_then(Value::as_f64)
            .map(|f| f as f32)
            .filter(|f| f.is_finite())
    };
    Ok(UserSettings {
        current_theme: string("EStringSettingId::CurrentThemeName").and_then(|s| non_empty(&s)),
        kill_sound: string("EStringSettingId::KillConfirmedSound").and_then(|s| non_empty(&s)),
        hit_volume: float("EFloatSettingId::HitVolume"),
        sfx_volume: float("EFloatSettingId::SFXVolume"),
    })
}

#[derive(Debug, Default, Clone, PartialEq)]
pub struct WeaponSettings {
    pub body_hit_sound: Option<String>,
    pub head_hit_sound: Option<String>,
    pub crosshair_file: Option<String>,
}

impl WeaponSettings {
    /// The sound for LGDuel's single hit sound.
    pub fn hit_sound(&self) -> Option<&str> {
        self.body_hit_sound
            .as_deref()
            .or(self.head_hit_sound.as_deref())
    }
}

/// `weaponsettings.ini`: flat `Key=Value` lines.
pub fn weapon_settings(inst: &Install) -> Result<WeaponSettings, String> {
    let text = read_capped(
        &inst.save_dir().join("weaponsettings.ini"),
        MAX_SETTINGS_BYTES,
    )?;
    Ok(parse_weapon_ini(&text))
}

fn parse_weapon_ini(text: &str) -> WeaponSettings {
    let mut w = WeaponSettings::default();
    for line in text.lines() {
        let Some((k, v)) = line.trim_start_matches('\u{feff}').split_once('=') else {
            continue;
        };
        let v = non_empty(v.trim_end_matches('\r'));
        match k.trim() {
            "BodyHitSound" => w.body_hit_sound = v,
            "HeadHitSound" => w.head_hit_sound = v,
            "CrosshairFile" => w.crosshair_file = v,
            _ => {}
        }
    }
    w
}

// ---- themes ----

/// Which KovaaK's themes to import.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Which {
    Current,
    All,
    Named(String),
}

/// Enemy colours from a KovaaK's theme; `None` keeps the player's current colour.
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct EnemyColorsPatch {
    pub head: Option<[f32; 3]>,
    pub upper: Option<[f32; 3]>,
    pub lower: Option<[f32; 3]>,
}

impl EnemyColorsPatch {
    pub fn apply(&self, c: &mut EnemyColors) {
        if let Some(v) = self.head {
            c.head = v;
        }
        if let Some(v) = self.upper {
            c.upper = v;
        }
        if let Some(v) = self.lower {
            c.lower = v;
        }
    }
}

#[derive(Debug, Default, Clone, Copy)]
struct Surface {
    tint: Option<[f32; 3]>,
    scale: Option<f32>,
}

/// The parts of a KovaaK's theme we use. Built by hand from the JSON so a wrongly
/// typed value only loses that one field instead of the whole file.
#[derive(Debug, Default, Clone)]
pub struct KvTheme {
    pub theme_name: Option<String>,
    floor: Surface,
    wall: Surface,
    ceiling: Surface,
    ramp: Surface,
    materials: [Option<String>; 4],
    enemy_head: Option<[f32; 3]>,
    enemy_body: Option<[f32; 3]>,
    solid_sky: bool,
    sky: Option<[u8; 3]>,
}

const SURFACES: [&str; 4] = ["floor", "wall", "ceiling", "ramp"];

impl KvTheme {
    pub fn parse(text: &str) -> Result<KvTheme, String> {
        let v: Value =
            serde_json::from_str(text.trim_start_matches('\u{feff}')).map_err(|e| e.to_string())?;
        if !v.is_object() {
            return Err("not a theme (expected a JSON object)".into());
        }
        let vec3 = |k: &str| -> Option<[f32; 3]> {
            let o = v.get(k)?;
            let f = |a: &str| o.get(a)?.as_f64().map(|x| x as f32);
            Some([f("x")?, f("y")?, f("z")?])
        };
        let flag = |k: &str| v.get(k).and_then(Value::as_bool).unwrap_or(false);
        let surface = |s: &str| Surface {
            tint: vec3(&format!("{s}Tint")),
            scale: v
                .get(format!("{s}TextureScale"))
                .and_then(Value::as_f64)
                .map(|x| x as f32),
        };
        let mut t = KvTheme {
            theme_name: v
                .get("themeName")
                .and_then(Value::as_str)
                .map(str::to_string),
            floor: surface("floor"),
            wall: surface("wall"),
            ceiling: surface("ceiling"),
            ramp: surface("ramp"),
            ..KvTheme::default()
        };
        for (i, s) in SURFACES.iter().enumerate() {
            t.materials[i] = v
                .get(format!("{s}Material"))
                .and_then(Value::as_str)
                .map(str::to_string);
        }
        if flag("overrideEnemyHeadColor") {
            t.enemy_head = vec3("enemyHeadColor");
        }
        if flag("overrideEnemyBodyColor") {
            t.enemy_body = vec3("enemyBodyColor");
        }
        t.solid_sky = flag("solidSkyColor");
        if let Some(c) = v.get("skyColor") {
            let b = |a: &str| c.get(a)?.as_u64().map(|x| x.min(255) as u8);
            if let (Some(r), Some(g), Some(bl)) = (b("r"), b("g"), b("b")) {
                t.sky = Some([r, g, bl]);
            }
        }
        Ok(t)
    }
}

/// The LGDuel texture closest to a KovaaK's material name (None = a flat colour).
pub fn texture_for(material: &str) -> Option<&'static str> {
    let m = material.to_ascii_uppercase();
    let has = |words: &[&str]| words.iter().any(|w| m.contains(w));
    if has(&["PURE COLOR"]) {
        None
    } else if has(&["GRID", "TILE", "BRICK"]) {
        Some("tiles")
    } else if has(&[
        "WOOD",
        "PLANK",
        "BOARD",
        "METAL",
        "SCIFI",
        "PANEL",
        "WALLPAPER",
        "PAINTED",
    ]) {
        Some("panel")
    } else {
        // CONCRETE, DRYWALL, PLASTER, MARBLE, STONE, ROCK, COBBLE, GROUND, GRASS and the rest.
        Some("concrete")
    }
}

/// World units per texture repeat for a KovaaK's texture scale (a higher scale = more repeats).
pub fn scale_for(kv: Option<f32>) -> f32 {
    let s = kv.filter(|s| s.is_finite()).unwrap_or(1.0);
    128.0 / s.clamp(0.25, 16.0)
}

fn clean_color(c: [f32; 3]) -> [f32; 3] {
    c.map(|v| {
        if v.is_finite() {
            v.clamp(0.0, 1.0)
        } else {
            0.5
        }
    })
}

/// A folder name for a KovaaK's theme: `kovaaks-` and a slug of its name.
pub fn theme_folder_name(name: &str) -> String {
    let mut slug = String::new();
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            slug.push(c.to_ascii_lowercase());
        } else if !slug.ends_with('-') {
            slug.push('-');
        }
    }
    let slug = slug.trim_matches('-');
    let slug = if slug.is_empty() { "theme" } else { slug };
    let slug: String = slug.chars().take(48).collect();
    format!("kovaaks-{}", slug.trim_end_matches('-'))
}

#[derive(Debug, Clone)]
pub struct Converted {
    pub theme: Theme,
    pub enemy: Option<EnemyColorsPatch>,
    pub warnings: Vec<String>,
}

fn distance(a: [f32; 3], b: [f32; 3]) -> f32 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}

pub fn convert_theme(kv: &KvTheme, fallback_name: &str) -> Converted {
    let name = theme_folder_name(kv.theme_name.as_deref().unwrap_or(fallback_name));
    let mut theme = Theme {
        name,
        ..Theme::default()
    };
    // Per surface: its own values, else the fallback's (ramp → floor, ceiling → wall).
    let own = [kv.floor, kv.wall, kv.ceiling, kv.ramp];
    let fallback = [None, None, Some(1), Some(0)];
    let mut tints: [Option<[f32; 3]>; 4] = [None; 4];
    for (i, surface) in SURFACES.iter().enumerate() {
        let use_own = own[i].tint.is_some();
        let src = if use_own { i } else { fallback[i].unwrap_or(i) };
        let Some(tint) = own[src].tint else {
            continue;
        };
        let tint = clean_color(tint);
        tints[i] = Some(tint);
        theme.materials.insert(
            surface.to_string(),
            Material {
                texture: kv.materials[src]
                    .as_deref()
                    .and_then(texture_for)
                    .map(str::to_string),
                color: Some(tint),
                tint_map_color: false,
                scale: scale_for(own[src].scale),
                shade: true,
            },
        );
    }
    if let Some(rgb) = kv.sky {
        let sky = rgb.map(srgb8_to_linear);
        theme.sky = sky;
        if !kv.solid_sky {
            // A skybox preset we can't draw: a gentle gradient in its colour instead.
            theme.sky_top = Some(sky.map(|v| v * 0.6));
        }
    }
    let mut warnings = Vec::new();
    let enemy = (kv.enemy_head.is_some() || kv.enemy_body.is_some()).then(|| {
        let head = kv.enemy_head.map(clean_color);
        let upper = kv.enemy_body.map(clean_color);
        let lower = upper.map(|u| u.map(|v| v * 0.75));
        for (part, c) in [("head", head), ("upper", upper), ("lower", lower)] {
            let Some(c) = c else { continue };
            for (surface, t) in [("floor", tints[0]), ("wall", tints[1])] {
                if t.is_some_and(|t| distance(c, t) < 0.25) {
                    warnings.push(format!(
                        "enemy {part} colour is close to the {surface} colour: enemies may be hard to see"
                    ));
                }
            }
        }
        EnemyColorsPatch { head, upper, lower }
    });
    Converted {
        theme,
        enemy,
        warnings,
    }
}

/// The KovaaK's themes: display name and file, sorted by name, plus problem lines.
pub fn list_themes(inst: &Install) -> (Vec<(String, PathBuf)>, Vec<String>) {
    let mut found = Vec::new();
    let mut problems = Vec::new();
    let Ok(rd) = std::fs::read_dir(inst.themes_dir()) else {
        return (found, problems);
    };
    let mut files: Vec<PathBuf> = rd
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("json"))
                && p.is_file()
        })
        .collect();
    files.sort();
    if files.len() > MAX_THEME_FILES {
        problems.push(format!(
            "{} theme files; only the first {MAX_THEME_FILES} are read",
            files.len()
        ));
        files.truncate(MAX_THEME_FILES);
    }
    for f in files {
        let stem = f
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        let file_name = f
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        match read_capped(&f, MAX_THEME_BYTES).and_then(|t| KvTheme::parse(&t)) {
            Ok(t) => found.push((
                t.theme_name
                    .filter(|n| !n.trim().is_empty())
                    .unwrap_or(stem),
                f,
            )),
            Err(e) => problems.push(format!("{file_name}: {e}")),
        }
    }
    found.sort_by_key(|(n, _)| n.to_lowercase());
    (found, problems)
}

/// Write the converted theme as `<themes_dir>/<name>/theme.toml` and return the
/// folder name. A folder we wrote earlier is replaced; the player's own theme with
/// the same name is never touched (we use `-2`, `-3`, … instead).
pub fn install_theme(conv: &Converted, themes_dir: &Path) -> Result<String, String> {
    let base = conv.theme.name.clone();
    if !theme::is_valid_theme_name(&base) {
        return Err(format!("bad theme name {base}"));
    }
    let mut name = base.clone();
    let mut n = 1;
    while themes_dir.join(&name).exists() && !themes_dir.join(&name).join(MARKER).is_file() {
        n += 1;
        if n > 999 {
            return Err(format!("too many themes called {base}"));
        }
        name = format!("{base}-{n}");
    }
    let mut theme = conv.theme.clone();
    theme.name = name.clone();
    let text = toml::to_string(&theme).map_err(|e| e.to_string())?;
    let fail = |e: std::io::Error| format!("{}: {e}", themes_dir.display());
    std::fs::create_dir_all(themes_dir).map_err(fail)?;
    let tmp = themes_dir.join(format!(".{name}.tmp"));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir(&tmp).map_err(fail)?;
    let write = || -> std::io::Result<()> {
        std::fs::write(tmp.join("theme.toml"), &text)?;
        std::fs::write(tmp.join(MARKER), b"")?;
        let dest = themes_dir.join(&name);
        if dest.exists() {
            std::fs::remove_dir_all(&dest)?;
        }
        std::fs::rename(&tmp, &dest)
    };
    if let Err(e) = write() {
        let _ = std::fs::remove_dir_all(&tmp);
        return Err(fail(e));
    }
    Ok(name)
}

#[derive(Debug, Default, Clone)]
pub struct ImportReport {
    /// Theme folder names written (or sound file names copied).
    pub imported: Vec<String>,
    pub problems: Vec<String>,
    pub warnings: Vec<String>,
    /// Enemy colours of the current / named theme (not of `All`).
    pub enemy: Option<EnemyColorsPatch>,
}

/// Import KovaaK's themes into `themes_dir`.
pub fn import_themes(inst: &Install, which: Which, themes_dir: &Path) -> ImportReport {
    let mut r = ImportReport::default();
    let (all, problems) = list_themes(inst);
    r.problems = problems;
    let stem_of = |p: &Path| {
        p.file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default()
    };
    let picked: Vec<&(String, PathBuf)> = match &which {
        Which::All => all.iter().collect(),
        Which::Current | Which::Named(_) => {
            let want = match &which {
                Which::Named(n) => Some(n.clone()),
                _ => match user_settings(inst) {
                    Ok(u) => u.current_theme,
                    Err(e) => {
                        r.problems.push(format!("PrimaryUserSettings.json: {e}"));
                        None
                    }
                },
            };
            let Some(want) = want else {
                r.problems.push("no KovaaK's theme is selected".into());
                return r;
            };
            let w = want.to_lowercase();
            let hit = all
                .iter()
                .find(|(n, _)| n.to_lowercase() == w)
                .or_else(|| all.iter().find(|(_, p)| stem_of(p).to_lowercase() == w));
            match hit {
                Some(h) => vec![h],
                None => {
                    r.problems
                        .push(format!("no KovaaK's theme called {want:?}"));
                    return r;
                }
            }
        }
    };
    let single = which != Which::All;
    for (name, path) in picked {
        let parsed = read_capped(path, MAX_THEME_BYTES).and_then(|t| KvTheme::parse(&t));
        let kv = match parsed {
            Ok(k) => k,
            Err(e) => {
                r.problems.push(format!("{name}: {e}"));
                continue;
            }
        };
        let conv = convert_theme(&kv, &stem_of(path));
        match install_theme(&conv, themes_dir) {
            Ok(folder) => {
                r.imported.push(folder);
                r.warnings
                    .extend(conv.warnings.iter().map(|w| format!("{name}: {w}")));
                if single {
                    r.enemy = conv.enemy;
                }
            }
            Err(e) => r.problems.push(format!("{name}: {e}")),
        }
    }
    r
}

/// What importing the sounds did: the file names now in `<settings dir>/sounds/`.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct SoundImport {
    pub hit: Option<String>,
    pub kill: Option<String>,
    /// `HitVolume × SFXVolume` from KovaaK's (0..1), if it has one.
    pub volume: Option<f32>,
    /// Why a sound was not imported (or other trouble), one line each.
    pub problems: Vec<String>,
}

/// `<sounds>/<stem>.ogg|.wav`, matching the stem case-insensitively (KovaaK's names
/// come from Windows; a Linux file system is case-sensitive). `.ogg` wins over `.wav`.
pub fn find_sound(inst: &Install, stem: &str) -> Result<PathBuf, String> {
    let want = Path::new(stem)
        .file_stem()
        .filter(|_| stem.to_lowercase().ends_with(".ogg") || stem.to_lowercase().ends_with(".wav"))
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| stem.to_string())
        .to_lowercase();
    let mut best: Option<(u8, PathBuf)> = None;
    if let Ok(rd) = std::fs::read_dir(inst.sounds_dir()) {
        for e in rd.flatten() {
            let p = e.path();
            let (Some(name), Some(ext)) = (p.file_stem(), p.extension()) else {
                continue;
            };
            let rank = match ext.to_string_lossy().to_lowercase().as_str() {
                "ogg" => 0,
                "wav" => 1,
                _ => continue,
            };
            if name.to_string_lossy().to_lowercase() == want
                && best.as_ref().is_none_or(|b| rank < b.0)
            {
                best = Some((rank, p));
            }
        }
    }
    best.map(|b| b.1)
        .ok_or_else(|| format!("{stem:?} is built into KovaaK's, can't be imported"))
}

/// Validate `src` and copy its original bytes into `out_dir`; returns the file name.
fn copy_sound(src: &Path, out_dir: &Path) -> Result<String, String> {
    crate::sound_file::load(src)?;
    let name = src
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .filter(|n| crate::settings::is_plain_file_name(n))
        .ok_or("bad file name")?;
    std::fs::create_dir_all(out_dir).map_err(|e| format!("{}: {e}", out_dir.display()))?;
    let dest = out_dir.join(&name);
    let tmp = out_dir.join(format!(".{name}.tmp"));
    std::fs::copy(src, &tmp)
        .and_then(|_| std::fs::rename(&tmp, &dest))
        .map_err(|e| format!("{}: {e}", dest.display()))?;
    Ok(name)
}

/// Copy the player's KovaaK's hit sound (`BodyHitSound`, else `HeadHitSound`) and kill
/// sound (`KillConfirmedSound`) into `out_dir` (`<settings dir>/sounds/`).
pub fn import_sounds(inst: &Install, out_dir: &Path, hit: bool, kill: bool) -> SoundImport {
    let mut r = SoundImport::default();
    let one = |label: &str, stem: Option<String>, problems: &mut Vec<String>| {
        let Some(stem) = stem else {
            problems.push(format!("no {label} sound is set in KovaaK's"));
            return None;
        };
        match find_sound(inst, &stem).and_then(|p| copy_sound(&p, out_dir)) {
            Ok(n) => Some(n),
            Err(e) => {
                problems.push(format!("{label} sound: {e}"));
                None
            }
        }
    };
    let weapon = match weapon_settings(inst) {
        Ok(w) => w,
        Err(e) => {
            r.problems.push(format!("weaponsettings.ini: {e}"));
            WeaponSettings::default()
        }
    };
    let user = match user_settings(inst) {
        Ok(u) => u,
        Err(e) => {
            r.problems.push(format!("PrimaryUserSettings.json: {e}"));
            UserSettings::default()
        }
    };
    let mut problems = std::mem::take(&mut r.problems);
    if hit {
        r.hit = one("hit", weapon.hit_sound().map(str::to_string), &mut problems);
    }
    if kill {
        r.kill = one("kill", user.kill_sound.clone(), &mut problems);
    }
    r.problems = problems;
    r.volume = user
        .hit_volume
        .map(|h| (h * user.sfx_volume.unwrap_or(1.0)).clamp(0.0, 1.0));
    r
}

/// Number of `.ogg` / `.wav` files in KovaaK's sounds folder.
pub fn count_sounds(inst: &Install) -> usize {
    std::fs::read_dir(inst.sounds_dir())
        .map(|rd| {
            rd.flatten()
                .filter(|e| {
                    e.path().extension().is_some_and(|x| {
                        x.eq_ignore_ascii_case("ogg") || x.eq_ignore_ascii_case("wav")
                    })
                })
                .count()
        })
        .unwrap_or(0)
}

/// What a look at the install found (the Share tab keeps this instead of rescanning every frame).
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Scan {
    pub root: Option<PathBuf>,
    /// Why it wasn't found.
    pub error: Option<String>,
    /// Theme display names.
    pub themes: Vec<String>,
    pub theme_problems: usize,
    pub sounds: usize,
    /// The theme selected in KovaaK's.
    pub current: Option<String>,
}

pub fn scan(manual: Option<&Path>) -> Scan {
    match find(manual) {
        Err(e) => Scan {
            error: Some(e),
            ..Scan::default()
        },
        Ok(inst) => {
            let (themes, problems) = list_themes(&inst);
            Scan {
                sounds: count_sounds(&inst),
                current: user_settings(&inst).ok().and_then(|u| u.current_theme),
                themes: themes.into_iter().map(|t| t.0).collect(),
                theme_problems: problems.len(),
                root: Some(inst.root),
                error: None,
            }
        }
    }
}

/// Something to do with KovaaK's, from the console, the Share tab or the command line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Cmd {
    Status,
    Themes,
    Import { which: Which, enemy: bool },
    Sounds { hit: bool, kill: bool, volume: bool },
}

/// Run `cmd`, changing `s` where it should (the caller saves and applies), and
/// return the lines to show. Files are copied below `settings_dir`.
pub fn run(s: &mut Settings, settings_dir: Option<&Path>, cmd: Cmd) -> Vec<String> {
    let manual = Some(s.kovaaks.path.trim())
        .filter(|p| !p.is_empty())
        .map(PathBuf::from);
    let inst = match find(manual.as_deref()) {
        Ok(i) => i,
        Err(e) => return vec![e],
    };
    let Some(dir) = settings_dir else {
        if matches!(cmd, Cmd::Import { .. } | Cmd::Sounds { .. }) {
            return vec!["no settings folder to copy into (started with --no-settings)".into()];
        }
        return run_readonly(&inst, cmd);
    };
    match cmd {
        Cmd::Status | Cmd::Themes => run_readonly(&inst, cmd),
        Cmd::Import { which, enemy } => {
            let all = which == Which::All;
            let r = import_themes(&inst, which, &dir.join("themes"));
            let mut lines: Vec<String> = r
                .imported
                .iter()
                .map(|f| format!("imported theme {f}"))
                .collect();
            lines.extend(r.warnings.iter().map(|w| format!("warning: {w}")));
            lines.extend(r.problems.iter().map(|p| format!("problem: {p}")));
            if !all && let Some(first) = r.imported.first() {
                s.video.theme = first.clone();
                lines.push(format!("theme set to {first}"));
                if enemy && let Some(patch) = r.enemy {
                    patch.apply(&mut s.enemy_colors);
                    lines.push("enemy colours set from the theme".into());
                }
            }
            lines
        }
        Cmd::Sounds { hit, kill, volume } => {
            let r = import_sounds(&inst, &dir.join("sounds"), hit, kill);
            let mut lines = Vec::new();
            if let Some(h) = r.hit {
                lines.push(format!("hit sound: {h}"));
                s.audio.hit_sound = Some(h);
            }
            if let Some(k) = r.kill {
                lines.push(format!("kill sound: {k}"));
                s.audio.kill_sound_file = Some(k);
                s.audio.kill_sound = true;
            }
            if volume && let Some(v) = r.volume {
                s.audio.hit_volume = v;
                lines.push(format!("hit volume {v:.2}"));
            }
            lines.extend(r.problems.iter().map(|p| format!("problem: {p}")));
            lines
        }
    }
}

fn run_readonly(inst: &Install, cmd: Cmd) -> Vec<String> {
    match cmd {
        Cmd::Themes => {
            let (themes, problems) = list_themes(inst);
            let mut lines: Vec<String> = themes.into_iter().map(|t| t.0).collect();
            lines.extend(problems.iter().map(|p| format!("problem: {p}")));
            lines
        }
        _ => vec![report_for(inst)],
    }
}

/// `lgduel --kovaaks-report`: one line, found or not.
pub fn report(manual: Option<&Path>) -> String {
    match find(manual) {
        Err(e) => format!("KOVAAKS not found: {e}"),
        Ok(inst) => report_for(&inst),
    }
}

fn report_for(inst: &Install) -> String {
    let (themes, problems) = list_themes(inst);
    let user = user_settings(inst).unwrap_or_default();
    let weapon = weapon_settings(inst).unwrap_or_default();
    format!(
        "KOVAAKS found {} | themes {} (problems {}) | sounds {} | current \"{}\" | hit \"{}\" | kill \"{}\"",
        inst.root.display(),
        themes.len(),
        problems.len(),
        count_sounds(inst),
        user.current_theme.unwrap_or_default(),
        weapon.hit_sound().unwrap_or_default(),
        user.kill_sound.unwrap_or_default(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::ThemeStyle;

    const FIXTURE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/testdata/kovaaks");

    fn fixture() -> Install {
        find(Some(Path::new(FIXTURE))).expect("fixture install")
    }

    fn temp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("lgduel-kv-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn vdf_two_libraries() {
        let text = r#"
"libraryfolders"
{
	// a comment
	"0"
	{
		"path"		"C:\\Program Files (x86)\\Steam"
		"apps"
		{
			"228980"		"1234"
		}
	}
	"1"
	{
		"path"		"E:\\SteamLibrary"
		"label"		"say \"hi\""
		"apps"
		{
			"824270"		"5678"
		}
	}
}
"#;
        let libs = parse_library_folders(text);
        assert_eq!(
            libs,
            vec![
                (PathBuf::from(r"C:\Program Files (x86)\Steam"), false),
                (PathBuf::from(r"E:\SteamLibrary"), true),
            ]
        );
    }

    #[test]
    fn vdf_garbage_and_truncated() {
        assert!(parse_library_folders("").is_empty());
        assert!(parse_library_folders("}}}{{{\"").is_empty());
        let libs = parse_library_folders(
            "\"libraryfolders\" { \"0\" { \"path\" \"/a\" \"apps\" { \"824270\" \"1\"",
        );
        assert_eq!(libs, vec![(PathBuf::from("/a"), true)]);
        let deep = "\"a\" {".repeat(10_000);
        let _ = parse_library_folders(&deep);
    }

    #[test]
    fn find_on_fixture() {
        let root = Path::new(FIXTURE);
        assert_eq!(find(Some(root)).unwrap().root, root);
        let inner = root.join("FPSAimTrainer");
        assert_eq!(find(Some(&inner)).unwrap().root, root);
        let err = find(Some(&root.join("nope"))).unwrap_err();
        assert!(err.contains("no KovaaK's install"), "{err}");
        assert!(!is_install(
            &root.join("FPSAimTrainer/sounds/..").join("crosshairs")
        ));
    }

    #[test]
    fn library_layout_is_found() {
        let lib = temp("lib");
        let game = lib.join("steamapps/common/FPSAimTrainer/FPSAimTrainer/sounds");
        std::fs::create_dir_all(&game).unwrap();
        let inst = find(Some(&lib)).unwrap();
        assert_eq!(inst.root, lib.join("steamapps/common/FPSAimTrainer"));
        let _ = std::fs::remove_dir_all(&lib);
    }

    #[test]
    fn texture_table() {
        for (m, t) in [
            ("PURE COLOR", None),
            ("GRID 16", Some("tiles")),
            ("BIANCA CARRARA MARBLE TILES", Some("tiles")),
            ("BRICK CLAY BEVELED", Some("tiles")),
            ("WHITE WOOD BOARD", Some("panel")),
            ("METAL STEEL", Some("panel")),
            ("SCIFI WALL", Some("panel")),
            ("painted wall a", Some("panel")),
            ("OUTLAWS WALLPAPER", Some("panel")),
            ("MARBLE POLISHED", Some("concrete")),
            ("CONCRETE BASIC", Some("concrete")),
            ("DRYWALL", Some("concrete")),
            ("GROUND GRASS", Some("concrete")),
            ("SOMETHING NEW", Some("concrete")),
        ] {
            assert_eq!(texture_for(m), t, "{m}");
        }
    }

    #[test]
    fn folder_names() {
        assert_eq!(theme_folder_name("!lobter air"), "kovaaks-lobter-air");
        assert_eq!(theme_folder_name("3 AM"), "kovaaks-3-am");
        assert_eq!(theme_folder_name("!!!"), "kovaaks-theme");
        assert_eq!(theme_folder_name("日本"), "kovaaks-theme");
        let long = theme_folder_name(&"ab ".repeat(40));
        assert!(long.len() <= "kovaaks-".len() + 48 && !long.ends_with('-'));
        for n in ["!lobter air", "3 AM", "", "a/b", "..", "x\\y"] {
            assert!(theme::is_valid_theme_name(&theme_folder_name(n)), "{n}");
        }
    }

    #[test]
    fn scale_direction() {
        assert_eq!(scale_for(None), 128.0);
        assert_eq!(scale_for(Some(2.0)), 64.0);
        assert_eq!(scale_for(Some(1000.0)), 8.0);
        assert_eq!(scale_for(Some(0.0)), 512.0);
        assert_eq!(scale_for(Some(f32::NAN)), 128.0);
    }

    #[test]
    fn lists_themes_and_reports_broken() {
        let (themes, problems) = list_themes(&fixture());
        let names: Vec<&str> = themes.iter().map(|t| t.0.as_str()).collect();
        assert_eq!(names, ["!lgduel minimal", "LGDuel Test Full"]);
        assert_eq!(problems.len(), 1);
        assert!(problems[0].starts_with("broken.json:"), "{problems:?}");
    }

    fn convert(file: &str) -> Converted {
        let path = fixture().themes_dir().join(file);
        let kv = KvTheme::parse(&read_capped(&path, MAX_THEME_BYTES).unwrap()).unwrap();
        convert_theme(&kv, "x")
    }

    #[test]
    fn converts_full_theme() {
        let c = convert("LGDuel Test Full.json");
        let m = &c.theme.materials;
        assert_eq!(c.theme.name, "kovaaks-lgduel-test-full");
        assert_eq!(m["wall"].texture.as_deref(), Some("tiles"));
        assert_eq!(m["wall"].color, Some([0.25, 0.3, 0.4]));
        assert_eq!(m["wall"].scale, 64.0);
        assert_eq!(m["floor"].texture.as_deref(), Some("concrete"));
        assert_eq!(m["floor"].scale, 32.0);
        assert_eq!(m["ceiling"].texture, None);
        assert_eq!(m["ramp"].texture.as_deref(), Some("concrete"));
        assert!(m.values().all(|x| x.shade && !x.tint_map_color));
        let want = [20u8, 30, 60].map(srgb8_to_linear);
        assert_eq!(c.theme.sky, want);
        assert_eq!(c.theme.sky_top, None);
        let e = c.enemy.unwrap();
        assert_eq!(e.head, Some([1.0, 0.5, 0.0]));
        assert_eq!(e.upper, Some([0.0, 1.0, 0.5]));
        assert_eq!(e.lower, Some([0.0, 0.75, 0.375]));
        assert!(c.warnings.is_empty(), "{:?}", c.warnings);
    }

    #[test]
    fn converts_minimal_theme_with_fallbacks() {
        let c = convert("!lgduel minimal.json");
        let m = &c.theme.materials;
        assert_eq!(c.theme.name, "kovaaks-lgduel-minimal");
        // No ceiling → wall, no ramp → floor (texture, tint and scale).
        assert_eq!(m["ceiling"], m["wall"]);
        assert_eq!(m["ramp"], m["floor"]);
        assert_eq!(m["wall"].texture.as_deref(), Some("tiles"));
        assert_eq!(m["floor"].scale, 128.0);
        assert!(c.theme.sky_top.is_some());
        let e = c.enemy.unwrap();
        assert_eq!(e.head, None);
        assert_eq!(e.upper, Some([1.0, 0.0, 1.0]));
    }

    #[test]
    fn close_enemy_colour_warns() {
        let kv = KvTheme::parse(
            r#"{"floorTint":{"x":0.5,"y":0.5,"z":0.5},"wallTint":{"x":0.1,"y":0.1,"z":0.1},
                "overrideEnemyHeadColor":true,"enemyHeadColor":{"x":0.5,"y":0.5,"z":0.5}}"#,
        )
        .unwrap();
        let c = convert_theme(&kv, "t");
        assert_eq!(c.warnings.len(), 1);
        assert!(c.warnings[0].contains("head") && c.warnings[0].contains("floor"));
    }

    #[test]
    fn hostile_themes_do_not_panic() {
        for text in [
            "",
            "[]",
            "{}",
            "null",
            r#"{"floorTint":"red","skyColor":5,"wallTextureScale":"x"}"#,
            r#"{"floorTint":{"x":1e999,"y":-1e999,"z":0},"floorTextureScale":1e30}"#,
            r#"{"skyColor":{"r":999,"g":-1,"b":2}}"#,
        ] {
            if let Ok(kv) = KvTheme::parse(text) {
                let c = convert_theme(&kv, "t");
                for m in c.theme.materials.values() {
                    assert!(m.color.unwrap().iter().all(|v| (0.0..=1.0).contains(v)));
                    assert!(m.scale.is_finite() && m.scale > 0.0);
                }
            }
        }
    }

    #[test]
    fn installed_theme_loads_cleanly_and_collides_safely() {
        let dir = temp("themes");
        let inst = fixture();
        let r = import_themes(&inst, Which::Current, &dir);
        assert_eq!(r.imported, ["kovaaks-lgduel-test-full"]);
        assert!(r.enemy.is_some());
        let style = ThemeStyle::load("kovaaks-lgduel-test-full", std::slice::from_ref(&dir));
        assert!(style.problems.is_empty(), "{:?}", style.problems);
        assert_eq!(style.name(), "kovaaks-lgduel-test-full");
        // Re-import replaces our own folder.
        let again = import_themes(&inst, Which::Current, &dir);
        assert_eq!(again.imported, ["kovaaks-lgduel-test-full"]);
        // A player's own theme of that name is left alone.
        let own = temp("own");
        std::fs::create_dir_all(own.join("kovaaks-lgduel-test-full")).unwrap();
        std::fs::write(
            own.join("kovaaks-lgduel-test-full/theme.toml"),
            "name = \"mine\"",
        )
        .unwrap();
        let r = import_themes(&inst, Which::Current, &own);
        assert_eq!(r.imported, ["kovaaks-lgduel-test-full-2"]);
        assert_eq!(
            std::fs::read_to_string(own.join("kovaaks-lgduel-test-full/theme.toml")).unwrap(),
            "name = \"mine\""
        );
        // All: both good themes, the broken one reported, no enemy patch.
        let all = import_themes(&inst, Which::All, &dir);
        assert_eq!(all.imported.len(), 2);
        assert_eq!(all.problems.len(), 1);
        assert!(all.enemy.is_none());
        let missing = import_themes(&inst, Which::Named("nope".into()), &dir);
        assert!(missing.imported.is_empty());
        assert!(
            missing
                .problems
                .iter()
                .any(|p| p.contains("no KovaaK's theme called"))
        );
        for d in [dir, own] {
            let _ = std::fs::remove_dir_all(d);
        }
    }

    #[test]
    fn reads_user_and_weapon_settings() {
        let inst = fixture();
        let u = user_settings(&inst).unwrap();
        assert_eq!(u.current_theme.as_deref(), Some("LGDuel Test Full"));
        assert_eq!(u.kill_sound.as_deref(), Some("lgduel_boop"));
        assert!((u.hit_volume.unwrap() - 0.8).abs() < 1e-6);
        let w = weapon_settings(&inst).unwrap();
        assert_eq!(w.hit_sound(), Some("lgduel_beep"));
        assert_eq!(w.crosshair_file.as_deref(), Some("lgduel_dot.png"));
        let w = parse_weapon_ini("\u{feff}BodyHitSound=\r\nHeadHitSound=ding\r\n");
        assert_eq!(w.hit_sound(), Some("ding"));
    }

    #[test]
    fn imports_hit_and_kill_sounds() {
        let out = temp("sounds").join("sounds");
        let inst = fixture();
        let r = import_sounds(&inst, &out, true, true);
        assert_eq!(r.hit.as_deref(), Some("lgduel_beep.ogg"));
        assert_eq!(r.kill.as_deref(), Some("lgduel_boop.wav"));
        assert!(r.problems.is_empty(), "{:?}", r.problems);
        assert!((r.volume.unwrap() - 0.8).abs() < 1e-6);
        // The original bytes are copied, and what was copied decodes.
        for n in ["lgduel_beep.ogg", "lgduel_boop.wav"] {
            let src = std::fs::read(inst.sounds_dir().join(n)).unwrap();
            assert_eq!(std::fs::read(out.join(n)).unwrap(), src);
            assert!(crate::sound_file::load(&out.join(n)).is_ok());
        }
        assert_eq!(
            std::fs::read_dir(&out).unwrap().count(),
            2,
            "no temp files left"
        );
        let _ = std::fs::remove_dir_all(out.parent().unwrap());
    }

    #[test]
    fn sound_lookup_is_case_insensitive_and_reports_builtins() {
        let inst = fixture();
        let want = inst.sounds_dir().join("lgduel_beep.ogg");
        assert_eq!(find_sound(&inst, "LGDuel_BEEP").unwrap(), want);
        assert_eq!(find_sound(&inst, "lgduel_beep.ogg").unwrap(), want);
        let e = find_sound(&inst, "Shot").unwrap_err();
        assert!(e.contains("built into KovaaK's, can't be imported"), "{e}");
        // A broken file is reported, not copied.
        let out = temp("badsound");
        let e = copy_sound(&inst.sounds_dir().join("broken.ogg"), &out).unwrap_err();
        assert!(e.starts_with("broken.ogg: "), "{e}");
        assert_eq!(std::fs::read_dir(&out).unwrap().count(), 0);
        let _ = std::fs::remove_dir_all(out);
    }

    #[test]
    fn run_imports_into_settings() {
        let dir = temp("run");
        let mut s = Settings::default();
        s.kovaaks.path = FIXTURE.into();
        let lines = run(
            &mut s,
            Some(&dir),
            Cmd::Import {
                which: Which::Current,
                enemy: true,
            },
        );
        assert_eq!(s.video.theme, "kovaaks-lgduel-test-full", "{lines:?}");
        assert_eq!(s.enemy_colors.head, [1.0, 0.5, 0.0]);
        // Without "use my enemy colours" they stay as they were.
        let mut t = Settings {
            kovaaks: s.kovaaks.clone(),
            ..Settings::default()
        };
        run(
            &mut t,
            Some(&dir),
            Cmd::Import {
                which: Which::Current,
                enemy: false,
            },
        );
        assert_eq!(t.enemy_colors, EnemyColors::default());
        let lines = run(
            &mut s,
            Some(&dir),
            Cmd::Sounds {
                hit: true,
                kill: false,
                volume: true,
            },
        );
        assert_eq!(
            s.audio.hit_sound.as_deref(),
            Some("lgduel_beep.ogg"),
            "{lines:?}"
        );
        assert_eq!(s.audio.kill_sound_file, None);
        assert!((s.audio.hit_volume - 0.8).abs() < 1e-6);
        run(
            &mut s,
            Some(&dir),
            Cmd::Sounds {
                hit: false,
                kill: true,
                volume: false,
            },
        );
        assert_eq!(s.audio.kill_sound_file.as_deref(), Some("lgduel_boop.wav"));
        assert!(s.audio.kill_sound);
        assert!(dir.join("sounds/lgduel_beep.ogg").is_file());
        // Read-only commands and a missing install.
        assert!(run(&mut s, None, Cmd::Status)[0].starts_with("KOVAAKS found"));
        assert_eq!(
            run(&mut s, None, Cmd::Themes),
            [
                "!lgduel minimal",
                "LGDuel Test Full",
                "problem: broken.json: EOF while parsing a value at line 2 column 0"
            ]
        );
        assert!(
            run(
                &mut s,
                None,
                Cmd::Sounds {
                    hit: true,
                    kill: true,
                    volume: false
                }
            )[0]
            .contains("--no-settings")
        );
        s.kovaaks.path = "/nonexistent".into();
        assert!(run(&mut s, Some(&dir), Cmd::Status)[0].contains("no KovaaK's install"));
        assert!(scan(Some(Path::new("/nonexistent"))).error.is_some());
        let sc = scan(Some(Path::new(FIXTURE)));
        assert_eq!((sc.themes.len(), sc.sounds, sc.theme_problems), (2, 5, 1));
        assert_eq!(sc.current.as_deref(), Some("LGDuel Test Full"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn report_line() {
        let line = report(Some(Path::new(FIXTURE)));
        assert!(
            line.contains("themes 2 (problems 1) | sounds 5 | current \"LGDuel Test Full\" | hit \"lgduel_beep\" | kill \"lgduel_boop\""),
            "{line}"
        );
        assert!(report(Some(Path::new("/nonexistent"))).starts_with("KOVAAKS not found"));
    }
}
