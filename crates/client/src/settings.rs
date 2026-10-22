//! Player settings, saved as TOML in the user's config directory
//! (`%APPDATA%\LGDuel\settings.toml`, `~/.config/lgduel/settings.toml`,
//! `~/Library/Application Support/LGDuel/settings.toml`). Unknown or missing
//! keys fall back to defaults so old files keep working.

use crate::effects::EffectSettings;
use crate::hud::HudSettings;
use crate::input::MouseSettings;
use crate::players::EnemyColors;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Video {
    /// Borderless fullscreen at the monitor's native resolution.
    pub fullscreen: bool,
    /// Window size when not fullscreen.
    pub size: (u32, u32),
    pub vsync: bool,
    /// Frame-rate cap (None = uncapped).
    pub fps_cap: Option<u32>,
    /// Horizontal FOV at 4:3, Quake style (10–130).
    pub fov: f32,
}

impl Default for Video {
    fn default() -> Self {
        Self {
            fullscreen: false,
            size: (1280, 720),
            vsync: false,
            fps_cap: None,
            fov: 110.0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Audio {
    /// 0 = muted, 1 = full.
    pub volume: f32,
}

impl Default for Audio {
    fn default() -> Self {
        Self { volume: 0.7 }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Practice {
    /// Offline opponent on/off.
    pub bot: bool,
    /// Strafe pattern: still | ad | mirror | anti | hsw | hswanti.
    pub pattern: String,
    /// Aim: easy | medium | hard | aimbot.
    pub skill: String,
}

impl Default for Practice {
    fn default() -> Self {
        Self {
            bot: true,
            pattern: "ad".into(),
            skill: "medium".into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ServerEntry {
    pub name: String,
    /// host:port
    pub address: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub name: String,
    pub mouse: MouseSettings,
    pub video: Video,
    pub effects: EffectSettings,
    pub hud: HudSettings,
    pub enemy_colors: EnemyColors,
    pub audio: Audio,
    pub practice: Practice,
    /// Servers the player added (the official list ships separately).
    pub servers: Vec<ServerEntry>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            name: "player".into(),
            mouse: MouseSettings::default(),
            video: Video::default(),
            effects: EffectSettings::default(),
            hud: HudSettings::default(),
            enemy_colors: EnemyColors::default(),
            audio: Audio::default(),
            practice: Practice::default(),
            servers: Vec::new(),
        }
    }
}

impl Settings {
    /// Clamp values into their valid ranges (hand-edited files can hold anything).
    pub fn sanitize(&mut self) {
        let finite_pos = |v: f64, d: f64| if v.is_finite() && v > 0.0 { v } else { d };
        let d = MouseSettings::default();
        self.mouse.dpi = finite_pos(self.mouse.dpi, d.dpi);
        self.mouse.cm_per_360 = finite_pos(self.mouse.cm_per_360, d.cm_per_360);
        self.video.fov = if self.video.fov.is_finite() {
            self.video.fov.clamp(10.0, 130.0)
        } else {
            110.0
        };
        self.video.size = (
            self.video.size.0.clamp(320, 16384),
            self.video.size.1.clamp(240, 16384),
        );
        self.video.fps_cap = self.video.fps_cap.filter(|c| *c >= 30);
        self.audio.volume = if self.audio.volume.is_finite() {
            self.audio.volume.clamp(0.0, 1.0)
        } else {
            0.7
        };
        self.effects.spark_size = self.effects.spark_size.clamp(0.0, 32.0);
        self.name = self
            .name
            .chars()
            .filter(|c| !c.is_control())
            .take(lgduel_net::protocol::MAX_NAME)
            .collect();
        if self.name.trim().is_empty() {
            self.name = "player".into();
        }
    }

    pub fn to_toml(&self) -> String {
        toml::to_string(self).expect("settings serialize")
    }

    pub fn from_toml(s: &str) -> Result<Settings, String> {
        let mut st: Settings = toml::from_str(s).map_err(|e| e.to_string())?;
        st.sanitize();
        Ok(st)
    }
}

/// Default settings file location for this OS (None if no home directory).
pub fn default_path() -> Option<PathBuf> {
    let env = |k: &str| {
        std::env::var_os(k)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
    };
    let dir = if cfg!(windows) {
        env("APPDATA")?.join("LGDuel")
    } else if cfg!(target_os = "macos") {
        env("HOME")?.join("Library/Application Support/LGDuel")
    } else {
        env("XDG_CONFIG_HOME")
            .unwrap_or(env("HOME")?.join(".config"))
            .join("lgduel")
    };
    Some(dir.join("settings.toml"))
}

/// Load settings; a missing file gives defaults. A broken file also gives
/// defaults plus a warning (and is left untouched).
pub fn load(path: &Path) -> (Settings, Option<String>) {
    match std::fs::read_to_string(path) {
        Ok(text) => match Settings::from_toml(&text) {
            Ok(s) => (s, None),
            Err(e) => (
                Settings::default(),
                Some(format!("{}: {e} (using defaults)", path.display())),
            ),
        },
        Err(_) => (Settings::default(), None),
    }
}

/// Write atomically (temp file + rename).
pub fn save(path: &Path, s: &Settings) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("toml.tmp");
    std::fs::write(&tmp, s.to_toml())?;
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::effects::BeamStyle;

    #[test]
    fn round_trip_is_identical() {
        let mut s = Settings {
            name: "tester".into(),
            ..Settings::default()
        };
        s.mouse.cm_per_360 = 55.0;
        s.video.fullscreen = true;
        s.video.fps_cap = Some(240);
        s.effects.beam_style = BeamStyle::Zigzag;
        s.hud.crosshair.size = 12.0;
        s.enemy_colors.head = [1.0, 0.0, 1.0];
        s.servers.push(ServerEntry {
            name: "home".into(),
            address: "1.2.3.4:27960".into(),
        });
        assert_eq!(Settings::from_toml(&s.to_toml()).unwrap(), s);
    }

    #[test]
    fn partial_and_unknown_keys_fall_back_to_defaults() {
        let s = Settings::from_toml(
            "name = \"x\"\n[mouse]\ndpi = 1600.0\n[future_section]\nwhatever = 1\n",
        )
        .unwrap();
        assert_eq!(s.name, "x");
        assert_eq!(s.mouse.dpi, 1600.0);
        assert_eq!(s.mouse.cm_per_360, MouseSettings::default().cm_per_360);
        assert_eq!(s.video, Video::default());
    }

    #[test]
    fn sanitizes_out_of_range_values() {
        let s = Settings::from_toml("name = \"\"\n[video]\nfov = 500.0\nfps_cap = 5\n[audio]\nvolume = 3.0\n[mouse]\ndpi = -1.0\n").unwrap();
        assert_eq!(s.video.fov, 130.0);
        assert_eq!(s.video.fps_cap, None);
        assert_eq!(s.audio.volume, 1.0);
        assert_eq!(s.mouse.dpi, MouseSettings::default().dpi);
        assert_eq!(s.name, "player");
    }

    #[test]
    fn save_and_load_file() {
        let dir = std::env::temp_dir().join(format!("lgduel-settings-test-{}", std::process::id()));
        let path = dir.join("settings.toml");
        let mut s = Settings::default();
        s.audio.volume = 0.25;
        save(&path, &s).unwrap();
        assert_eq!(load(&path), (s, None));
        std::fs::write(&path, "this is [not toml").unwrap();
        let (d, warn) = load(&path);
        assert_eq!(d, Settings::default());
        assert!(warn.is_some());
        let _ = std::fs::remove_dir_all(dir);
        assert_eq!(load(&path).1, None, "missing file is not an error");
    }
}
