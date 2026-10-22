//! In-game console (` key): cvars are the settings file's keys
//! (`mouse.cm_per_360`, `hud.crosshair.size`, …), changed live with
//! `set KEY VALUE` (or just `KEY VALUE`); `writeconfig NAME` saves every cvar
//! as a text profile and `exec NAME` runs one, so setups can be swapped and shared.

use crate::settings::Settings;
use std::collections::VecDeque;
use std::path::{Path, PathBuf};

const HELP: &str = "console commands:
  cvars [text]          list settings (filtered by text)
  KEY                   show a setting, e.g. mouse.cm_per_360
  set KEY VALUE         change it now (KEY VALUE works too), e.g. set fov 110 -> video.fov 110
  writeconfig NAME      save all settings to NAME.cfg (a profile)
  exec NAME             run NAME.cfg
  reloadtheme           re-read the current theme folder (after editing it)
  exporttheme NAME      write the theme folder NAME as one .lgtheme file (themepacks/ next to settings)
  importtheme FILE [as NAME]   install a .lgtheme file (or drag it onto the window)
  presets | preset NAME  built-in looks (classic, high-visibility, colour-blind-safe, minimal, showcase)
  exportprofile [controls]   print a share code for your look (controls = also mouse/FOV/volume)
  importprofile CODE    apply a share code (see what it changes first with Settings -> Share)
  record NAME | stoprecord | playdemo NAME   demos (practice: exact replay)
  connect ADDRESS | join CODE | practice | disconnect | quit
  clear | help";

/// Things the console asks the app to do.
#[derive(Debug, Clone, PartialEq)]
pub enum ConsoleAction {
    ExportTheme(String),
    ImportTheme { file: String, name: Option<String> },
    Preset(String),
    ExportProfile { controls: bool },
    ImportProfile(String),
    ReloadTheme,
    Record(String),
    StopRecord,
    PlayDemo(String),
    Connect(String),
    Join(String),
    Practice,
    Disconnect,
    Quit,
}

/// Short names for the most used cvars.
const ALIASES: [(&str, &str); 6] = [
    ("fov", "video.fov"),
    ("sensitivity", "mouse.cm_per_360"),
    ("dpi", "mouse.dpi"),
    ("volume", "audio.volume"),
    ("name", "name"),
    ("beam", "effects.beam_style"),
];

#[derive(Debug, Clone, Default)]
pub struct Console {
    pub open: bool,
    pub input: String,
    /// Output, newest last.
    pub lines: VecDeque<String>,
}

impl Console {
    pub fn print(&mut self, text: &str) {
        for l in text.lines() {
            self.lines.push_back(l.to_string());
        }
        while self.lines.len() > 500 {
            self.lines.pop_front();
        }
    }
}

/// Every settable leaf as (dotted key, TOML value), sorted.
pub fn cvars(s: &Settings) -> Vec<(String, toml::Value)> {
    fn walk(prefix: &str, t: &toml::Table, out: &mut Vec<(String, toml::Value)>) {
        for (k, v) in t {
            let key = if prefix.is_empty() {
                k.clone()
            } else {
                format!("{prefix}.{k}")
            };
            match v {
                toml::Value::Table(sub) => walk(&key, sub, out),
                // Lists of tables (the player's server list) aren't cvars.
                toml::Value::Array(a) if a.iter().any(|x| x.is_table()) => {}
                _ => out.push((key, v.clone())),
            }
        }
    }
    let table: toml::Table = toml::from_str(&s.to_toml()).unwrap_or_default();
    let mut out = Vec::new();
    walk("", &table, &mut out);
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

fn resolve_alias(key: &str) -> &str {
    ALIASES
        .iter()
        .find(|(a, _)| *a == key)
        .map_or(key, |(_, k)| k)
}

/// Set one cvar; the result is validated and sanitised like the settings file.
pub fn set(s: &mut Settings, key: &str, value: &str) -> Result<String, String> {
    let key = resolve_alias(key);
    let old = cvars(s)
        .into_iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v)
        .ok_or(format!("no setting {key:?} (cvars lists them)"))?;
    let parsed = toml::from_str::<toml::Table>(&format!("v = {value}"))
        .ok()
        .and_then(|mut t| t.remove("v"))
        .unwrap_or_else(|| toml::Value::String(value.trim_matches('"').to_string()));
    let parsed = match (&old, parsed) {
        (toml::Value::Float(_), toml::Value::Integer(i)) => toml::Value::Float(i as f64),
        (toml::Value::Integer(_), toml::Value::Float(f)) if f.fract() == 0.0 => {
            toml::Value::Integer(f as i64)
        }
        (_, v) => v,
    };
    if std::mem::discriminant(&old) != std::mem::discriminant(&parsed) {
        return Err(format!("{key} needs a {} value", type_name(&old)));
    }
    let mut table: toml::Table = toml::from_str(&s.to_toml()).map_err(|e| e.to_string())?;
    let mut parts: Vec<&str> = key.split('.').collect();
    let leaf = parts.pop().unwrap();
    let mut t = &mut table;
    for p in parts {
        t = t
            .get_mut(p)
            .and_then(|v| v.as_table_mut())
            .ok_or(format!("no setting {key:?}"))?;
    }
    t.insert(leaf.to_string(), parsed);
    let text = toml::to_string(&table).map_err(|e| e.to_string())?;
    *s = Settings::from_toml(&text)?;
    // Show what sanitising made of it.
    let now = cvars(s)
        .into_iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.to_string())
        .unwrap_or_default();
    Ok(format!("{key} = {now}"))
}

fn type_name(v: &toml::Value) -> &'static str {
    match v {
        toml::Value::String(_) => "text",
        toml::Value::Integer(_) => "whole-number",
        toml::Value::Float(_) => "number",
        toml::Value::Boolean(_) => "true/false",
        toml::Value::Array(_) => "list ([a, b, c])",
        _ => "other",
    }
}

/// Demo file for `name` in the demos folder next to the settings.
pub fn demo_path(settings_dir: Option<&Path>, name: &str) -> Result<PathBuf, String> {
    let name = name.trim().trim_end_matches(".lgdemo");
    let p = profile(settings_dir.map(|d| d.join("demos")).as_deref(), name)?;
    Ok(p.with_extension("lgdemo"))
}

/// Profile file for `name` (letters, digits, - and _ only).
fn profile(dir: Option<&Path>, name: &str) -> Result<PathBuf, String> {
    let dir = dir.ok_or("no settings folder (started with --no-settings)")?;
    let name = name.trim().trim_end_matches(".cfg");
    if name.is_empty()
        || !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err("profile names: letters, digits, - and _".into());
    }
    Ok(dir.join(format!("{name}.cfg")))
}

/// Run one console line against the settings. Returns output lines, an
/// optional request for the app, and whether settings changed.
pub fn run(
    s: &mut Settings,
    line: &str,
    dir: Option<&Path>,
) -> (Vec<String>, Option<ConsoleAction>, bool) {
    run_depth(s, line, dir, 0)
}

fn run_depth(
    s: &mut Settings,
    line: &str,
    dir: Option<&Path>,
    depth: u32,
) -> (Vec<String>, Option<ConsoleAction>, bool) {
    let line = line.trim();
    let line = line.split("//").next().unwrap_or("").trim();
    if line.is_empty() || line.starts_with('#') {
        return (vec![], None, false);
    }
    let (cmd, rest) = line.split_once(' ').unwrap_or((line, ""));
    let rest = rest.trim();
    let out = |t: String| (vec![t], None, false);
    match cmd {
        "help" => (HELP.lines().map(String::from).collect(), None, false),
        "cvars" => (
            cvars(s)
                .into_iter()
                .filter(|(k, _)| k.contains(rest))
                .map(|(k, v)| format!("{k} {v}"))
                .collect(),
            None,
            false,
        ),
        "set" => match rest.split_once(' ') {
            Some((k, v)) => match set(s, k, v.trim()) {
                Ok(t) => (vec![t], None, true),
                Err(e) => out(e),
            },
            None => out("usage: set KEY VALUE".into()),
        },
        "writeconfig" => match profile(dir, rest) {
            Ok(path) => {
                let mut text = String::from("// LGDuel profile: exec this file to apply it\n");
                for (k, v) in cvars(s) {
                    text.push_str(&format!("set {k} {v}\n"));
                }
                match std::fs::create_dir_all(path.parent().unwrap())
                    .and_then(|_| std::fs::write(&path, text))
                {
                    Ok(()) => out(format!("wrote {}", path.display())),
                    Err(e) => out(format!("{}: {e}", path.display())),
                }
            }
            Err(e) => out(e),
        },
        "exec" => {
            if depth >= 4 {
                return out("exec: too deeply nested".into());
            }
            let path = match profile(dir, rest) {
                Ok(p) => p,
                Err(e) => return out(e),
            };
            let text = match std::fs::read_to_string(&path) {
                Ok(t) => t,
                Err(e) => return out(format!("{}: {e}", path.display())),
            };
            let (mut lines, mut action, mut changed) = (Vec::new(), None, false);
            let mut errors = 0;
            for l in text.lines() {
                let (o, a, c) = run_depth(s, l, dir, depth + 1);
                if !c && !o.is_empty() && !l.trim_start().starts_with("cvars") {
                    errors += 1;
                    lines.extend(o);
                }
                changed |= c;
                action = action.or(a);
            }
            lines.push(format!("executed {} ({errors} problems)", path.display()));
            (lines, action, changed)
        }
        "connect" if !rest.is_empty() => (vec![], Some(ConsoleAction::Connect(rest.into())), false),
        "join" if !rest.is_empty() => (vec![], Some(ConsoleAction::Join(rest.into())), false),
        "practice" => (vec![], Some(ConsoleAction::Practice), false),
        "record" if !rest.is_empty() => (vec![], Some(ConsoleAction::Record(rest.into())), false),
        "stoprecord" => (vec![], Some(ConsoleAction::StopRecord), false),
        "reloadtheme" => (vec![], Some(ConsoleAction::ReloadTheme), false),
        "exporttheme" if !rest.is_empty() => {
            (vec![], Some(ConsoleAction::ExportTheme(rest.into())), false)
        }
        "importtheme" if !rest.is_empty() => {
            let (file, name) = match rest.rsplit_once(" as ") {
                Some((f, n)) => (f.trim().to_string(), Some(n.trim().to_string())),
                None => (rest.to_string(), None),
            };
            (
                vec![],
                Some(ConsoleAction::ImportTheme { file, name }),
                false,
            )
        }
        "presets" => (
            crate::profile::presets()
                .iter()
                .map(|p| format!("{:<18} {}", p.name, p.description))
                .collect(),
            None,
            false,
        ),
        "preset" if !rest.is_empty() => (vec![], Some(ConsoleAction::Preset(rest.into())), false),
        "exportprofile" => (
            vec![],
            Some(ConsoleAction::ExportProfile {
                controls: rest == "controls",
            }),
            false,
        ),
        "importprofile" if !rest.is_empty() => (
            vec![],
            Some(ConsoleAction::ImportProfile(rest.into())),
            false,
        ),
        "playdemo" if !rest.is_empty() => {
            (vec![], Some(ConsoleAction::PlayDemo(rest.into())), false)
        }
        "disconnect" => (vec![], Some(ConsoleAction::Disconnect), false),
        "quit" | "exit" => (vec![], Some(ConsoleAction::Quit), false),
        key if rest.is_empty() => {
            let key = resolve_alias(key);
            match cvars(s).into_iter().find(|(k, _)| k == key) {
                Some((k, v)) => out(format!("{k} = {v}")),
                None => out(format!("unknown command or setting {key:?} (try help)")),
            }
        }
        key => match set(s, key, rest) {
            Ok(t) => (vec![t], None, true),
            Err(e) => out(e),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_get_aliases_and_validation() {
        let mut s = Settings::default();
        let (o, _, changed) = run(&mut s, "set mouse.cm_per_360 55", None);
        assert!(changed, "{o:?}");
        assert_eq!(s.mouse.cm_per_360, 55.0);
        // Quake style "key value", aliases, nested keys, booleans, lists.
        run(&mut s, "sensitivity 30.5", None);
        assert_eq!(s.mouse.cm_per_360, 30.5);
        run(&mut s, "fov 100", None);
        assert_eq!(s.video.fov, 100.0);
        run(&mut s, "hud.crosshair.size 12", None);
        assert_eq!(s.hud.crosshair.size, 12.0);
        run(&mut s, "set video.vsync true", None);
        assert!(s.video.vsync);
        run(&mut s, "set enemy_colors.head [1.0, 0.0, 1.0]", None);
        assert_eq!(s.enemy_colors.head, [1.0, 0.0, 1.0]);
        run(&mut s, "beam zigzag", None);
        assert_eq!(s.effects.beam_style, crate::effects::BeamStyle::Zigzag);
        run(&mut s, "video.theme grid", None);
        assert_eq!(s.video.theme, "grid");
        run(&mut s, "models.enemy humanoid", None);
        assert_eq!(s.models.enemy, crate::models::ModelKind::Humanoid);
        run(&mut s, "models.forced_colors false", None);
        assert!(!s.models.forced_colors);
        run(&mut s, "set models.skin.legs [0.1, 0.2, 0.3]", None);
        assert_eq!(s.models.skin.legs, [0.1, 0.2, 0.3]);
        let (o, _, changed) = run(&mut s, "models.enemy robot", None);
        assert!(
            !changed || s.models.enemy == crate::models::ModelKind::Humanoid,
            "{o:?}"
        );
        run(&mut s, "name Alice Smith", None);
        assert_eq!(s.name, "Alice Smith");
        assert_eq!(run(&mut s, "fov", None).0, vec!["video.fov = 100.0"]);
        // Sanitising applies; wrong types and unknown keys are refused.
        assert_eq!(run(&mut s, "fov 500", None).0, vec!["video.fov = 130.0"]);
        let (o, _, changed) = run(&mut s, "video.vsync maybe", None);
        assert!(!changed && o[0].contains("true/false"), "{o:?}");
        let (o, _, changed) = run(&mut s, "mouse.cm_per_36 5", None);
        assert!(!changed && o[0].contains("no setting"), "{o:?}");
        assert!(
            run(&mut s, "cvars crosshair", None)
                .0
                .iter()
                .all(|l| l.contains("crosshair"))
        );
        assert_eq!(
            run(&mut s, "join LG-7KQ2", None).1,
            Some(ConsoleAction::Join("LG-7KQ2".into()))
        );
        assert_eq!(run(&mut s, "// just a comment", None).0.len(), 0);
    }

    #[test]
    fn writeconfig_and_exec_round_trip() {
        let dir = std::env::temp_dir().join(format!("lgduel-console-{}", std::process::id()));
        let mut a = Settings::default();
        run(&mut a, "sensitivity 41", Some(&dir));
        run(&mut a, "set effects.sparks false", Some(&dir));
        let (o, _, _) = run(&mut a, "writeconfig lowsens", Some(&dir));
        assert!(o[0].starts_with("wrote"), "{o:?}");
        let mut b = Settings::default();
        let (o, _, changed) = run(&mut b, "exec lowsens", Some(&dir));
        assert!(
            changed && o.last().unwrap().contains("(0 problems)"),
            "{o:?}"
        );
        assert_eq!(b, a);
        assert!(run(&mut b, "writeconfig ../evil", Some(&dir)).0[0].contains("letters"));
        assert!(run(&mut b, "exec nothere", Some(&dir)).0[0].contains("nothere.cfg"));
        assert!(run(&mut b, "exec x", None).0[0].contains("no settings folder"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
