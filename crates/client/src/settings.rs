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
    /// Map theme (`classic` = the original flat look).
    pub theme: String,
    /// Menu / console size multiplier (0.75 to 2.0).
    pub ui_scale: f32,
}

impl Default for Video {
    fn default() -> Self {
        Self {
            fullscreen: false,
            size: (1280, 720),
            vsync: false,
            fps_cap: None,
            fov: 110.0,
            theme: "classic".into(),
            ui_scale: 1.0,
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
    /// Built-in map name.
    pub map: String,
}

impl Default for Practice {
    fn default() -> Self {
        Self {
            bot: true,
            pattern: "ad".into(),
            skill: "medium".into(),
            map: "arena_flat".into(),
        }
    }
}

/// "Host a game" options.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct HostSettings {
    pub name: String,
    pub port: u16,
    /// Classic first-to-N instead of vamp.
    pub classic: bool,
    pub lifesteal_percent: u32,
    pub anti_lag: bool,
    /// Built-in map name.
    pub map: String,
    /// Register with the rendezvous service for a join code.
    pub join_code: bool,
}

impl Default for HostSettings {
    fn default() -> Self {
        Self {
            name: "LGDuel server".into(),
            port: 27960,
            classic: false,
            lifesteal_percent: 100,
            anti_lag: true,
            map: "arena_flat".into(),
            join_code: true,
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
    pub models: crate::models::ModelSettings,
    pub audio: Audio,
    pub practice: Practice,
    /// Servers the player added (the official list ships separately).
    pub servers: Vec<ServerEntry>,
    pub host: HostSettings,
    /// Rendezvous/relay service for join codes (host[:port]); empty = the
    /// one shipped in servers.toml, if any.
    pub rendezvous: String,
    /// Server browser filters.
    pub filter: BrowserFilter,
}

/// Which servers the browser shows (rows that don't answer are always shown).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct BrowserFilter {
    /// any | vamp | classic
    pub mode: String,
    /// any | on | off
    pub anti_lag: String,
    /// 0 = any.
    pub max_ping: u32,
    /// Empty = any; otherwise the server's region must contain this (any case).
    pub region: String,
    pub hide_empty: bool,
    pub hide_full: bool,
}

impl Default for BrowserFilter {
    fn default() -> Self {
        Self {
            mode: "any".into(),
            anti_lag: "any".into(),
            max_ping: 0,
            region: String::new(),
            hide_empty: false,
            hide_full: false,
        }
    }
}

impl BrowserFilter {
    /// Does a server with this info and ping pass?
    pub fn allows(&self, info: &lgduel_net::protocol::ServerInfo, rtt_ms: f64) -> bool {
        let mode_ok = match self.mode.as_str() {
            "vamp" => !info.classic,
            "classic" => info.classic,
            _ => true,
        };
        let lag_ok = match self.anti_lag.as_str() {
            "on" => info.anti_lag,
            "off" => !info.anti_lag,
            _ => true,
        };
        let region = self.region.trim().to_lowercase();
        mode_ok
            && lag_ok
            && (self.max_ping == 0 || rtt_ms <= self.max_ping as f64)
            && (region.is_empty() || info.region.to_lowercase().contains(&region))
            && !(self.hide_empty && info.players == 0)
            && !(self.hide_full && info.players >= info.max_players)
    }
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
            models: crate::models::ModelSettings::default(),
            audio: Audio::default(),
            practice: Practice::default(),
            servers: Vec::new(),
            host: HostSettings::default(),
            rendezvous: String::new(),
            filter: BrowserFilter::default(),
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
        self.video.ui_scale = if self.video.ui_scale.is_finite() {
            self.video.ui_scale.clamp(0.75, 2.0)
        } else {
            1.0
        };
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
        self.host.lifesteal_percent = self.host.lifesteal_percent.min(100);
        for map in [&mut self.practice.map, &mut self.host.map] {
            if lgduel_sim::world::builtin::source(map).is_none() {
                *map = "arena_flat".into();
            }
        }
        if self.host.port == 0 {
            self.host.port = 27960;
        }
        self.host.name = self
            .host
            .name
            .chars()
            .filter(|c| !c.is_control())
            .take(lgduel_net::protocol::MAX_INFO_TEXT)
            .collect();
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
        let mut t = Settings::default();
        t.video.ui_scale = 9.0;
        t.sanitize();
        assert_eq!(t.video.ui_scale, 2.0);
        t.video.ui_scale = f32::NAN;
        t.sanitize();
        assert_eq!(t.video.ui_scale, 1.0);
        let s =
            Settings::from_toml("[practice]\nmap = \"nowhere\"\n[host]\nmap = \"arena_tiers\"\n")
                .unwrap();
        assert_eq!(s.practice.map, "arena_flat");
        assert_eq!(s.host.map, "arena_tiers");
    }

    #[test]
    fn browser_filter() {
        let info = |classic, anti_lag, players, region: &str| lgduel_net::protocol::ServerInfo {
            classic,
            anti_lag,
            players,
            max_players: 16,
            region: region.into(),
            ..Default::default()
        };
        let any = BrowserFilter::default();
        assert!(any.allows(&info(true, false, 0, ""), 300.0));
        let f = BrowserFilter {
            mode: "vamp".into(),
            anti_lag: "on".into(),
            max_ping: 80,
            region: "eu".into(),
            hide_empty: true,
            hide_full: true,
        };
        assert!(f.allows(&info(false, true, 3, "EU-West"), 40.0));
        assert!(!f.allows(&info(true, true, 3, "EU"), 40.0), "classic");
        assert!(!f.allows(&info(false, false, 3, "EU"), 40.0), "no anti-lag");
        assert!(!f.allows(&info(false, true, 3, "EU"), 120.0), "ping");
        assert!(!f.allows(&info(false, true, 3, "NA"), 40.0), "region");
        assert!(!f.allows(&info(false, true, 0, "EU"), 40.0), "empty");
        assert!(!f.allows(&info(false, true, 16, "EU"), 40.0), "full");
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
