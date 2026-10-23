//! Share codes: your look (and optionally controls) as one pasteable line.
//!
//! A code is `LGP1-` + base64url( zlib( "key value" lines ) + CRC32 ). Only a
//! whitelist of visual keys is ever read from a code (controls only on request),
//! every value goes through the same checked path as the console (`console::set`),
//! and sizes are capped before anything is allocated: codes come from strangers.

use crate::console;
use crate::settings::Settings;
use crate::theme::{self, Theme};
use flate2::Compression;
use flate2::read::ZlibDecoder;
use flate2::write::ZlibEncoder;
use std::io::{Read, Write};
use std::path::PathBuf;

pub const PREFIX: &str = "LGP1-";
/// Longest code accepted (characters, after removing whitespace).
pub const MAX_CODE: usize = 16_000;
/// Most bytes a code may inflate to.
pub const MAX_INFLATED: usize = 65_536;
const MAX_LINES: usize = 300;
const MAX_KEY: usize = 64;
const MAX_VALUE: usize = 2_000;
/// Longest embedded theme.toml.
const MAX_THEME_TOML: usize = 16_000;
const THEME_KEY: &str = "@theme.toml";

/// Settings that make up "your look".
pub fn is_look_key(key: &str) -> bool {
    key == "video.theme"
        || key == "video.msaa"
        || key == "video.ui_scale"
        || key.starts_with("effects.")
        || key.starts_with("enemy_colors.")
        || key.starts_with("models.")
        || key.starts_with("ui.")
        || (key.starts_with("hud.") && key != "hud.net_graph")
        || (key.starts_with("audio.")
            && key != "audio.volume"
            // File names mean nothing on another PC.
            && key != "audio.hit_sound"
            && key != "audio.kill_sound_file")
}

/// Settings of the optional controls group.
pub fn is_controls_key(key: &str) -> bool {
    key.starts_with("mouse.") || key == "video.fov" || key == "audio.volume"
}

// ---- base64url (no padding) ---------------------------------------------

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

fn b64_encode(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for c in data.chunks(3) {
        let n = (c[0] as u32) << 16
            | (*c.get(1).unwrap_or(&0) as u32) << 8
            | *c.get(2).unwrap_or(&0) as u32;
        for i in 0..=c.len() {
            out.push(B64[(n >> (18 - 6 * i) & 63) as usize] as char);
        }
    }
    out
}

fn b64_decode(s: &str) -> Result<Vec<u8>, String> {
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let (mut acc, mut bits) = (0u32, 0);
    for ch in s.bytes() {
        let v = B64
            .iter()
            .position(|b| *b == ch)
            .ok_or("the code has characters that aren't allowed")? as u32;
        acc = acc << 6 | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    if s.len() % 4 == 1 {
        return Err("the code is cut off".into());
    }
    Ok(out)
}

// ---- profile ---------------------------------------------------------------

/// A decoded code: `key value` entries and an optional embedded theme.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Profile {
    pub entries: Vec<(String, String)>,
    /// `theme.toml` of the active theme, embedded when it uses no PNG files.
    pub theme_toml: Option<String>,
}

/// One setting a code would change.
#[derive(Debug, Clone, PartialEq)]
pub struct Change {
    pub key: String,
    pub old: String,
    pub new: String,
}

/// What importing a code does (computed without touching anything).
#[derive(Debug, Clone, Default)]
pub struct Report {
    /// The settings after applying; None when nothing applied.
    pub result: Option<Settings>,
    pub changes: Vec<Change>,
    /// Keys the code carried that aren't shareable (ignored).
    pub skipped: Vec<String>,
    pub problems: Vec<String>,
    /// A theme to write to the themes folder: (final name, theme.toml).
    pub theme_install: Option<(String, String)>,
    /// The code names a theme that isn't installed (and has no embedded copy).
    pub missing_theme: Option<String>,
}

/// Make a code from `s`. `theme_dirs` are searched for the active theme's folder.
pub fn export(s: &Settings, theme_dirs: &[PathBuf], with_controls: bool) -> String {
    let mut text = String::from("v 1\n");
    for (k, v) in console::cvars(s) {
        if is_look_key(&k) || (with_controls && is_controls_key(&k)) {
            text.push_str(&format!("{k} {v}\n"));
        }
    }
    if let Some(dir) = theme::find_theme_dir(&s.video.theme, theme_dirs)
        && let Ok(toml_text) = std::fs::read_to_string(dir.join("theme.toml"))
        && !toml_text.to_ascii_lowercase().contains(".png")
        && toml_text.len() <= MAX_THEME_TOML
    {
        text.push_str(&format!(
            "{THEME_KEY} {}\n",
            b64_encode(toml_text.as_bytes())
        ));
    }
    let mut enc = ZlibEncoder::new(Vec::new(), Compression::best());
    enc.write_all(text.as_bytes()).expect("write to memory");
    let mut bytes = enc.finish().expect("compress to memory");
    let mut crc = flate2::Crc::new();
    crc.update(&bytes);
    bytes.extend_from_slice(&crc.sum().to_be_bytes());
    format!("{PREFIX}{}", b64_encode(&bytes))
}

/// Parse a code. Validates size, checksum and line format; no side effects.
pub fn decode(code: &str) -> Result<Profile, String> {
    let code: String = code.chars().filter(|c| !c.is_whitespace()).collect();
    if code.len() > MAX_CODE {
        return Err(format!(
            "the code is too long (at most {MAX_CODE} characters)"
        ));
    }
    let body = code
        .strip_prefix(PREFIX)
        .ok_or("this isn't a share code (it should start with LGP1-)")?;
    let bytes = b64_decode(body)?;
    if bytes.len() < 5 {
        return Err("the code is too short".into());
    }
    let (data, crc_bytes) = bytes.split_at(bytes.len() - 4);
    let mut crc = flate2::Crc::new();
    crc.update(data);
    if crc.sum().to_be_bytes() != crc_bytes {
        return Err("the code is damaged (checksum mismatch): copy it again".into());
    }
    let mut text = Vec::new();
    ZlibDecoder::new(data)
        .take(MAX_INFLATED as u64 + 1)
        .read_to_end(&mut text)
        .map_err(|_| "the code's data is damaged".to_string())?;
    if text.len() > MAX_INFLATED {
        return Err("the code expands to too much data".into());
    }
    let text = String::from_utf8(text).map_err(|_| "the code isn't text".to_string())?;
    let mut p = Profile::default();
    for (i, line) in text.lines().enumerate() {
        if i >= MAX_LINES {
            return Err("the code has too many settings".into());
        }
        let (key, value) = line.split_once(' ').unwrap_or((line, ""));
        if key.len() > MAX_KEY || value.len() > MAX_VALUE.max(MAX_THEME_TOML * 2) {
            return Err("a setting in the code is too long".into());
        }
        match key {
            "v" => {
                if value.trim() != "1" {
                    return Err("this code is from a newer LGDuel: update the game".into());
                }
            }
            THEME_KEY => {
                let t = b64_decode(value.trim())
                    .ok()
                    .and_then(|b| String::from_utf8(b).ok())
                    .ok_or("the embedded theme is damaged")?;
                if t.len() > MAX_THEME_TOML {
                    return Err("the embedded theme is too large".into());
                }
                p.theme_toml = Some(t);
            }
            "" => {}
            _ => p.entries.push((key.to_string(), value.to_string())),
        }
    }
    Ok(p)
}

/// Work out what applying `p` to `current` does. Pure: reads theme folders, writes nothing.
pub fn evaluate(
    p: &Profile,
    current: &Settings,
    theme_dirs: &[PathBuf],
    with_controls: bool,
) -> Report {
    let mut r = Report::default();
    let mut s = current.clone();
    let before = console::cvars(current);
    let old_of = |key: &str| {
        before
            .iter()
            .find(|(k, _)| k == key)
            .map_or(String::new(), |(_, v)| v.to_string())
    };
    let mut theme_name = None;
    for (key, value) in &p.entries {
        if !(is_look_key(key) || (with_controls && is_controls_key(key))) {
            r.skipped.push(key.clone());
            continue;
        }
        if key == "video.theme" {
            theme_name = Some(value.trim().trim_matches('"').to_string());
            continue;
        }
        match console::set(&mut s, key, value) {
            Ok(_) => {}
            Err(e) => r.problems.push(format!("{key}: {e}")),
        }
    }
    // The theme: an embedded copy gets installed, a missing one is reported.
    if let Some(name) = theme_name {
        let installed = theme::available(theme_dirs).contains(&name);
        match (&p.theme_toml, installed) {
            (Some(text), _)
                if theme::is_valid_theme_name(&name)
                    && !theme::BUILTIN.contains(&name.as_str()) =>
            {
                if let Err(e) = Theme::from_toml(text) {
                    r.problems.push(format!("embedded theme: {e}"));
                } else if text.to_ascii_lowercase().contains(".png") {
                    r.problems
                        .push("embedded theme uses image files: not accepted".into());
                } else {
                    // Same text already installed → reuse; different → a free "-2", "-3"… name.
                    let mut final_name = name.clone();
                    let mut n = 2;
                    loop {
                        match theme::find_theme_dir(&final_name, theme_dirs) {
                            None => {
                                r.theme_install = Some((final_name.clone(), text.clone()));
                                break;
                            }
                            Some(d)
                                if std::fs::read_to_string(d.join("theme.toml"))
                                    .ok()
                                    .as_deref()
                                    == Some(text) =>
                            {
                                break;
                            }
                            Some(_) => {
                                final_name = format!("{name}-{n}");
                                n += 1;
                            }
                        }
                    }
                    s.video.theme = final_name;
                }
            }
            (_, true) => s.video.theme = name,
            (_, false) => r.missing_theme = Some(name),
        }
    }
    s.sanitize();
    let after = console::cvars(&s);
    for (k, v) in after {
        let new = v.to_string();
        let old = old_of(&k);
        if new != old {
            r.changes.push(Change { key: k, old, new });
        }
    }
    if !r.changes.is_empty() || r.theme_install.is_some() {
        r.result = Some(s);
    }
    r
}

/// Write the theme `report` wants installed into `themes_dir/<name>/theme.toml`.
pub fn install_theme(report: &Report, themes_dir: &std::path::Path) -> Result<(), String> {
    let Some((name, text)) = &report.theme_install else {
        return Ok(());
    };
    if !theme::is_valid_theme_name(name) {
        return Err("unsafe theme name".into());
    }
    let dir = themes_dir.join(name);
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    std::fs::write(dir.join("theme.toml"), text).map_err(|e| e.to_string())
}

/// A built-in look.
pub struct Preset {
    pub name: &'static str,
    pub description: &'static str,
    pub entries: Vec<(String, String)>,
}

fn entries(list: &[(&str, &str)]) -> Vec<(String, String)> {
    list.iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

/// The built-in presets: `classic` resets every look setting to its default.
pub fn presets() -> Vec<Preset> {
    let defaults: Vec<(String, String)> = console::cvars(&Settings::default())
        .into_iter()
        .filter(|(k, _)| is_look_key(k))
        .map(|(k, v)| (k, v.to_string()))
        .collect();
    vec![
        Preset {
            name: "classic",
            description: "the original look: every look setting back to its default",
            entries: defaults,
        },
        Preset {
            name: "high-visibility",
            description: "thick white crosshair, thick beam, bigger text and sparks",
            entries: entries(&[
                ("hud.crosshair.size", "12.0"),
                ("hud.crosshair.thickness", "3.0"),
                ("hud.crosshair.color", "[1.0, 1.0, 1.0]"),
                ("effects.beam_style", "\"thick\""),
                ("effects.spark_size", "5.0"),
                ("hud.text_scale", "4.0"),
            ]),
        },
        Preset {
            name: "colour-blind-safe",
            description: "enemy colours that differ strongly in brightness (yellow / blue / orange)",
            entries: entries(&[
                ("enemy_colors.head", "[1.0, 0.95, 0.2]"),
                ("enemy_colors.upper", "[0.1, 0.35, 1.0]"),
                ("enemy_colors.lower", "[1.0, 0.6, 0.0]"),
                ("models.forced_colors", "true"),
            ]),
        },
        Preset {
            name: "minimal",
            description: "no impact sparks and no damage numbers",
            entries: entries(&[("effects.sparks", "false"), ("hud.damage_numbers", "false")]),
        },
        Preset {
            name: "showcase",
            description: "textured grid theme, humanoid enemies in their own skins, gun model",
            entries: entries(&[
                ("video.theme", "\"grid\""),
                ("models.enemy", "\"humanoid\""),
                ("models.forced_colors", "false"),
                ("effects.gun", "\"normal\""),
            ]),
        },
    ]
}

/// The preset called `name` as a profile (apply it with `evaluate`).
pub fn preset_profile(name: &str) -> Option<Profile> {
    presets()
        .into_iter()
        .find(|p| p.name == name)
        .map(|p| Profile {
            entries: p.entries,
            theme_toml: None,
        })
}

/// Human-readable lines for a report (preview and result).
pub fn describe(r: &Report) -> Vec<String> {
    let mut v: Vec<String> = r
        .changes
        .iter()
        .map(|c| format!("{}: {} -> {}", c.key, c.old, c.new))
        .collect();
    if let Some((n, _)) = &r.theme_install {
        v.push(format!("installs theme {n}"));
    }
    if v.is_empty() {
        v.push("nothing would change".into());
    }
    if let Some(t) = &r.missing_theme {
        v.push(format!(
            "theme {t} is not installed: ask for its .lgtheme file (theme not applied)"
        ));
    }
    if !r.skipped.is_empty() {
        v.push(format!("ignored (not shareable): {}", r.skipped.join(", ")));
    }
    v.extend(r.problems.iter().map(|p| format!("problem: {p}")));
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    fn customised() -> Settings {
        let mut s = Settings::default();
        s.hud.crosshair.size = 14.0;
        s.hud.crosshair.color = [0.1, 0.9, 0.3];
        s.effects.beam_style = crate::effects::BeamStyle::Zigzag;
        s.enemy_colors.head = [1.0, 0.0, 1.0];
        s.models.enemy = crate::models::ModelKind::Humanoid;
        s.models.forced_colors = false;
        s.video.theme = "grid".into();
        s.mouse.cm_per_360 = 41.5;
        s.name = "Alice".into();
        s
    }

    /// Hand-build a code from text (for hostile inputs).
    fn code_of(text: &str) -> String {
        let mut enc = ZlibEncoder::new(Vec::new(), Compression::best());
        enc.write_all(text.as_bytes()).unwrap();
        let mut bytes = enc.finish().unwrap();
        let mut crc = flate2::Crc::new();
        crc.update(&bytes);
        bytes.extend_from_slice(&crc.sum().to_be_bytes());
        format!("{PREFIX}{}", b64_encode(&bytes))
    }

    #[test]
    fn sound_files_are_not_shared() {
        let mut s = customised();
        s.audio.hit_sound = Some("ding.ogg".into());
        s.audio.kill_sound_file = Some("frag.wav".into());
        s.audio.kill_sound = true;
        assert!(!is_look_key("audio.hit_sound") && !is_look_key("audio.kill_sound_file"));
        assert!(is_look_key("audio.kill_sound"));
        let p = decode(&export(&s, &[], true)).unwrap();
        assert!(
            p.entries
                .iter()
                .all(|(k, _)| k != "audio.hit_sound" && k != "audio.kill_sound_file"),
            "{:?}",
            p.entries
        );
    }

    #[test]
    fn base64_round_trips_every_length() {
        for n in 0..40usize {
            let data: Vec<u8> = (0..n).map(|i| (i * 37 + 11) as u8).collect();
            assert_eq!(b64_decode(&b64_encode(&data)).unwrap(), data, "{n}");
        }
        assert!(b64_decode("ab+/").is_err());
        assert!(
            b64_decode("abcde").is_err(),
            "length 5 mod 4 = 1 is impossible"
        );
    }

    #[test]
    fn round_trip_applies_the_look_and_nothing_personal() {
        let s = customised();
        let code = export(&s, &[], false);
        assert!(
            code.starts_with(PREFIX) && code.len() < 2_000,
            "{}",
            code.len()
        );
        let p = decode(&code).unwrap();
        let mut target = Settings {
            name: "Bob".into(),
            ..Settings::default()
        };
        target.mouse.cm_per_360 = 20.0;
        let r = evaluate(&p, &target, &[], false);
        assert!(r.problems.is_empty(), "{:?}", r.problems);
        assert!(r.skipped.is_empty());
        let out = r.result.unwrap();
        assert_eq!(out.hud.crosshair.size, 14.0);
        assert_eq!(out.hud.crosshair.color, [0.1, 0.9, 0.3]);
        assert_eq!(out.effects.beam_style, crate::effects::BeamStyle::Zigzag);
        assert_eq!(out.enemy_colors.head, [1.0, 0.0, 1.0]);
        assert_eq!(out.models.enemy, crate::models::ModelKind::Humanoid);
        assert!(!out.models.forced_colors);
        assert_eq!(out.video.theme, "grid");
        // Not part of the look: name and controls stay as they were.
        assert_eq!((out.name.as_str(), out.mouse.cm_per_360), ("Bob", 20.0));
        // The report lists exactly what changed.
        assert!(
            r.changes
                .iter()
                .any(|c| c.key == "hud.crosshair.size" && c.new == "14.0")
        );
        assert!(!r.changes.iter().any(|c| c.key.starts_with("mouse.")));
    }

    #[test]
    fn controls_only_with_the_flag() {
        let s = customised();
        assert!(
            decode(&export(&s, &[], false))
                .unwrap()
                .entries
                .iter()
                .all(|(k, _)| !k.starts_with("mouse."))
        );
        let p = decode(&export(&s, &[], true)).unwrap();
        assert!(
            p.entries
                .iter()
                .any(|(k, v)| k == "mouse.cm_per_360" && v == "41.5")
        );
        // A code that carries controls does not apply them unless asked.
        let t = Settings::default();
        assert!(
            evaluate(&p, &t, &[], false)
                .skipped
                .iter()
                .any(|k| k == "mouse.cm_per_360")
        );
        let r = evaluate(&p, &t, &[], true);
        assert_eq!(r.result.unwrap().mouse.cm_per_360, 41.5);
    }

    #[test]
    fn personal_and_dangerous_keys_are_ignored() {
        let code = code_of(
            "v 1\nname \"Mallory\"\nrendezvous \"evil.example\"\nhost.port 80\nvideo.size [99999, 99999]\n\
             video.fullscreen true\nvideo.vsync true\nservers []\nhud.net_graph false\nhud.crosshair.size 9.0\n",
        );
        let r = evaluate(&decode(&code).unwrap(), &Settings::default(), &[], false);
        for k in [
            "name",
            "rendezvous",
            "host.port",
            "video.size",
            "video.fullscreen",
            "video.vsync",
            "servers",
            "hud.net_graph",
        ] {
            assert!(
                r.skipped.iter().any(|s| s == k),
                "{k} not skipped: {:?}",
                r.skipped
            );
        }
        let out = r.result.unwrap();
        assert_eq!(out.hud.crosshair.size, 9.0);
        assert_eq!(out.name, Settings::default().name);
        assert_eq!(out.video.size, Settings::default().video.size);
    }

    #[test]
    fn damaged_and_hostile_codes_are_rejected() {
        let good = export(&customised(), &[], false);
        // Wrong prefix, empty, newer version.
        assert!(decode("hello").unwrap_err().contains("LGP1-"));
        assert!(decode("").is_err());
        assert!(
            decode(&code_of("v 2\nhud.crosshair.size 9.0\n"))
                .unwrap_err()
                .contains("newer")
        );
        // A changed character fails the checksum (or the alphabet).
        let mut bad = good.clone().into_bytes();
        let i = PREFIX.len() + 10;
        bad[i] = if bad[i] == b'A' { b'B' } else { b'A' };
        assert!(decode(&String::from_utf8(bad).unwrap()).is_err());
        // Cut off.
        assert!(decode(&good[..good.len() - 7]).is_err());
        // Huge input is refused by length before decoding.
        let huge = format!("{PREFIX}{}", "A".repeat(200_000));
        assert!(decode(&huge).unwrap_err().contains("too long"));
        // A small valid zlib stream that inflates to 10 MB.
        let bomb = code_of(&"a".repeat(10_000_000));
        assert!(
            bomb.len() < MAX_CODE,
            "bomb must be small to be a bomb: {}",
            bomb.len()
        );
        assert!(decode(&bomb).unwrap_err().contains("too much"));
        // Too many lines, over-long key / value.
        assert!(
            decode(&code_of(&"a 1\n".repeat(400)))
                .unwrap_err()
                .contains("too many")
        );
        assert!(
            decode(&code_of(&format!("{} 1\n", "k".repeat(100))))
                .unwrap_err()
                .contains("too long")
        );
        assert!(
            decode(&code_of(&format!("hud.x {}\n", "v".repeat(40_000))))
                .unwrap_err()
                .contains("too long")
        );
        // Garbage bytes under a correct checksum are not valid zlib.
        let mut raw = vec![1, 2, 3, 4, 5, 6];
        let mut crc = flate2::Crc::new();
        crc.update(&raw);
        raw.extend_from_slice(&crc.sum().to_be_bytes());
        assert!(
            decode(&format!("{PREFIX}{}", b64_encode(&raw)))
                .unwrap_err()
                .contains("damaged")
        );
    }

    #[test]
    fn bad_values_are_skipped_while_good_ones_apply() {
        let code = code_of(
            "hud.crosshair.size banana\nhud.crosshair.thickness 3.0\nmodels.enemy robot\nnot.a.key 1\n",
        );
        let r = evaluate(&decode(&code).unwrap(), &Settings::default(), &[], false);
        assert_eq!(r.problems.len(), 2, "{:?}", r.problems);
        assert_eq!(r.skipped, vec!["not.a.key".to_string()]);
        let out = r.result.unwrap();
        assert_eq!(out.hud.crosshair.thickness, 3.0);
        assert_eq!(
            out.hud.crosshair.size,
            Settings::default().hud.crosshair.size
        );
        // Nothing valid at all → nothing to apply.
        let r = evaluate(
            &decode(&code_of("hud.crosshair.size banana\n")).unwrap(),
            &Settings::default(),
            &[],
            false,
        );
        assert!(r.result.is_none());
    }

    #[test]
    fn wrapped_and_padded_codes_decode() {
        let code = export(&customised(), &[], false);
        let wrapped: String = code
            .as_bytes()
            .chunks(40)
            .map(|c| String::from_utf8_lossy(c).to_string())
            .collect::<Vec<_>>()
            .join("\r\n  ");
        assert_eq!(
            decode(&format!("  \n{wrapped}\n ")).unwrap(),
            decode(&code).unwrap()
        );
    }

    #[test]
    fn presets_apply_cleanly_and_classic_resets_the_look() {
        let all = presets();
        assert!(all.len() >= 5);
        for pr in &all {
            let p = preset_profile(pr.name).unwrap();
            let r = evaluate(&p, &Settings::default(), &[], false);
            assert!(r.problems.is_empty(), "{}: {:?}", pr.name, r.problems);
            assert!(r.skipped.is_empty(), "{}: {:?}", pr.name, r.skipped);
            assert!(r.missing_theme.is_none(), "{}", pr.name);
            if pr.name != "classic" {
                assert!(r.result.is_some(), "{} changes nothing", pr.name);
            }
        }
        // Showcase, then classic: the look is back to the defaults (the rest is untouched).
        let mut s = Settings {
            name: "Keep".into(),
            ..Settings::default()
        };
        s.mouse.cm_per_360 = 25.0;
        let showcase = evaluate(&preset_profile("showcase").unwrap(), &s, &[], false)
            .result
            .unwrap();
        assert_eq!(showcase.models.enemy, crate::models::ModelKind::Humanoid);
        let back = evaluate(&preset_profile("classic").unwrap(), &showcase, &[], false)
            .result
            .unwrap();
        let look = |s: &Settings| {
            console::cvars(s)
                .into_iter()
                .filter(|(k, _)| is_look_key(k))
                .collect::<Vec<_>>()
        };
        assert_eq!(look(&back), look(&Settings::default()));
        assert_eq!((back.name.as_str(), back.mouse.cm_per_360), ("Keep", 25.0));
        assert!(preset_profile("nope").is_none());
    }

    #[test]
    fn colour_blind_preset_differs_in_brightness() {
        let lin = |c: f32| {
            if c <= 0.04045 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        };
        let lum = |c: [f32; 3]| 0.2126 * lin(c[0]) + 0.7152 * lin(c[1]) + 0.0722 * lin(c[2]);
        let r = evaluate(
            &preset_profile("colour-blind-safe").unwrap(),
            &Settings::default(),
            &[],
            false,
        )
        .result
        .unwrap();
        let c = r.enemy_colors;
        let l = [lum(c.head), lum(c.upper), lum(c.lower)];
        for (i, j) in [(0, 1), (0, 2), (1, 2)] {
            assert!((l[i] - l[j]).abs() >= 0.25, "{l:?}");
        }
    }

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("lgduel-profile-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn embedded_theme_installs_and_collisions_get_a_suffix() {
        let src = temp("src");
        std::fs::create_dir_all(src.join("mine")).unwrap();
        let toml_text = "sky = [0.3, 0.1, 0.1]\n[materials.floor]\ncolor = [0.7, 0.2, 0.2]\n";
        std::fs::write(src.join("mine/theme.toml"), toml_text).unwrap();
        let mut s = Settings::default();
        s.video.theme = "mine".into();
        let code = export(&s, std::slice::from_ref(&src), false);
        let p = decode(&code).unwrap();
        assert_eq!(p.theme_toml.as_deref(), Some(toml_text));
        // A receiver without the theme gets it installed.
        let dst = temp("dst");
        let r = evaluate(&p, &Settings::default(), std::slice::from_ref(&dst), false);
        assert_eq!(r.theme_install.as_ref().map(|t| t.0.as_str()), Some("mine"));
        assert_eq!(r.result.as_ref().unwrap().video.theme, "mine");
        install_theme(&r, &dst).unwrap();
        assert_eq!(
            std::fs::read_to_string(dst.join("mine/theme.toml")).unwrap(),
            toml_text
        );
        // Same theme again: reused, nothing to install.
        let r = evaluate(&p, &Settings::default(), std::slice::from_ref(&dst), false);
        assert!(r.theme_install.is_none());
        assert_eq!(r.result.as_ref().unwrap().video.theme, "mine");
        // A different theme with the same name: installed as mine-2.
        std::fs::write(dst.join("mine/theme.toml"), "sky = [0.0, 0.0, 0.0]\n").unwrap();
        let r = evaluate(&p, &Settings::default(), std::slice::from_ref(&dst), false);
        assert_eq!(
            r.theme_install.as_ref().map(|t| t.0.as_str()),
            Some("mine-2")
        );
        assert_eq!(r.result.as_ref().unwrap().video.theme, "mine-2");
        install_theme(&r, &dst).unwrap();
        assert!(dst.join("mine-2/theme.toml").is_file());
        let _ = std::fs::remove_dir_all(&src);
        let _ = std::fs::remove_dir_all(&dst);
    }

    #[test]
    fn hostile_themes_in_codes_are_refused() {
        let dst = temp("hostile");
        let embed = |name: &str, body: &str| {
            let v = b64_encode(body.as_bytes());
            code_of(&format!("video.theme \"{name}\"\n{THEME_KEY} {v}\n"))
        };
        // Path traversal in the name: nothing installed.
        for name in ["../evil", "a/b", "..", "C:x"] {
            let r = evaluate(
                &decode(&embed(name, "sky = [0.0, 0.0, 0.0]\n")).unwrap(),
                &Settings::default(),
                std::slice::from_ref(&dst),
                false,
            );
            assert!(r.theme_install.is_none(), "{name}");
            assert!(
                install_theme(
                    &Report {
                        theme_install: Some((name.into(), "x".into())),
                        ..Default::default()
                    },
                    &dst
                )
                .is_err()
            );
        }
        // PNG references, invalid TOML, unknown keys: refused with a problem.
        for body in [
            "[materials.floor]\ntexture = \"x.png\"\n",
            "sky = [",
            "bogus = 1\n",
        ] {
            let r = evaluate(
                &decode(&embed("ok", body)).unwrap(),
                &Settings::default(),
                std::slice::from_ref(&dst),
                false,
            );
            assert!(
                r.theme_install.is_none() && !r.problems.is_empty(),
                "{body}: {:?}",
                r.problems
            );
        }
        // A theme that isn't installed and isn't embedded is reported, not applied.
        let r = evaluate(
            &decode(&code_of("video.theme \"fancy\"\nhud.crosshair.size 9.0\n")).unwrap(),
            &Settings::default(),
            std::slice::from_ref(&dst),
            false,
        );
        assert_eq!(r.missing_theme.as_deref(), Some("fancy"));
        assert_eq!(r.result.unwrap().video.theme, "classic");
        let _ = std::fs::remove_dir_all(&dst);
    }
}
