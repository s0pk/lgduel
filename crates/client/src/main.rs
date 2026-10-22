//! LGDuel client.
//!
//! `lgduel host [--port N] [--config server.toml]`                   run a server (same as lgduel-server)
//! `lgduel [--frames N] [--vsync] [--fps-cap N] [--size WxH]`        offline vs bot
//! `lgduel --connect HOST:PORT [--name NAME]`                       online
//! `lgduel --headless --screenshot out.png [--frames N] [--size WxH]`
//! `lgduel --headless --connect HOST:PORT [--secs N] [--screenshot out.png]`
//!
//! `--frames N` renders N frames, prints frame-time stats and exits (perf runs).
//! `--headless` renders offscreen (no window), saves a screenshot and checks
//! that it rendered (non-blank, expected colours present); exit code 1 if not.

mod audio;
mod browser;
mod console;
mod demo_play;
mod effects;
mod fight;
mod font;
mod frame_stats;
mod gfx;
mod hud;
mod icon;
mod input;
mod local;
mod menu;
mod models;
mod net_session;
mod offscreen;
mod players;
mod profile;
mod render;
mod scene;
mod settings;
mod textures;
mod theme;
mod themepack;
mod ttf;
mod ui;

use effects::BeamStyle;
use frame_stats::FrameStats;
use gfx::Gfx;
use hud::{DamageNumbers, HudSettings};
use input::{Keys, MouseLook};
use lgduel_bot::{AimSkill, Bot, BotConfig, StrafePattern};
use lgduel_sim::config::GameConfig;
use lgduel_sim::game::PlayerId;
use lgduel_sim::math::{Vec3, look_angles};
use lgduel_sim::usercmd::{UserCmd, buttons};
use lgduel_sim::world::{World, builtin};
use local::LocalSession;
use net_session::NetSession;
use players::EnemyColors;
use render::{Camera, Renderer};
use scene::FrameView;
use settings::Settings;
use std::sync::Arc;
use std::time::{Duration, Instant};
use winit::application::ApplicationHandler;
use winit::event::{DeviceEvent, DeviceId, ElementState, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::CursorGrabMode;
use winit::window::{Window, WindowId};

/// `lgduel --doctor`: what the game sees on this machine (for bug reports). Returns the exit code.
fn doctor(opts: &Options) -> i32 {
    let env = |k: &str| std::env::var(k).unwrap_or_else(|_| "-".into());
    println!(
        "lgduel {} on {}-{}",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS,
        std::env::consts::ARCH
    );
    println!(
        "session: XDG_SESSION_TYPE={} WAYLAND_DISPLAY={} DISPLAY={}",
        env("XDG_SESSION_TYPE"),
        env("WAYLAND_DISPLAY"),
        env("DISPLAY")
    );
    let code = match offscreen::Offscreen::new(64, 64) {
        Ok(off) => {
            println!(
                "adapter: {} ({:?}, {:?})",
                off.adapter_info.name, off.adapter_info.backend, off.adapter_info.device_type
            );
            0
        }
        Err(e) => {
            println!("adapter: none ({e}); try WGPU_BACKEND=gl or install a Vulkan driver");
            1
        }
    };
    let audio = audio::Audio::open(0.5);
    println!(
        "audio: {}",
        if audio.is_live() {
            "output device opened"
        } else {
            "no output device (the game runs muted)"
        }
    );
    match opts.settings_path.as_ref() {
        Some(p) => println!("settings: {}", p.display()),
        None => println!("settings: none (--no-settings)"),
    }
    for d in theme_dirs(opts) {
        println!(
            "themes: {}{}",
            d.display(),
            if d.is_dir() { "" } else { " (missing)" }
        );
    }
    code
}

/// Write theme `name` as `<settings>/themepacks/<name>.lgtheme`. Returns a message.
fn export_theme(opts: &Options, name: &str) -> String {
    let Some(folder) = theme::find_theme_dir(name, &theme_dirs(opts)) else {
        return format!("no theme folder called {name} (built-in themes can't be exported)");
    };
    let Some(dir) = opts
        .settings_path
        .as_ref()
        .and_then(|p| p.parent())
        .map(|p| p.join("themepacks"))
    else {
        return "no settings folder to write to (started with --no-settings)".into();
    };
    match themepack::pack(&folder) {
        Ok(bytes) => {
            let file = dir.join(format!("{name}.lgtheme"));
            match std::fs::create_dir_all(&dir).and_then(|_| std::fs::write(&file, &bytes)) {
                Ok(()) => format!("wrote {} ({} KB)", file.display(), bytes.len() / 1024 + 1),
                Err(e) => format!("{}: {e}", file.display()),
            }
        }
        Err(e) => format!("can't pack {name}: {e}"),
    }
}

/// Install the theme pack `file` (as `name`, default from the file name) and select it.
fn import_theme(opts: &mut Options, file: &std::path::Path, name: Option<&str>) -> String {
    let Some(dir) = user_themes_dir(opts) else {
        return "no settings folder to install into (started with --no-settings)".into();
    };
    let Some(name) = name
        .map(String::from)
        .or_else(|| themepack::name_from_file(file))
    else {
        return format!("can't make a theme name from {}", file.display());
    };
    let size = std::fs::metadata(file).map(|m| m.len());
    match size {
        Ok(n) if n > themepack::MAX_PACK_FILE => {
            return format!("{}: file is too large", file.display());
        }
        Err(e) => return format!("{}: {e}", file.display()),
        _ => {}
    }
    let bytes = match std::fs::read(file) {
        Ok(b) => b,
        Err(e) => return format!("{}: {e}", file.display()),
    };
    match themepack::unpack_to(&bytes, &dir, &name, false) {
        Ok(_) => {
            opts.s.video.theme = name.clone();
            format!("installed theme {name} and selected it")
        }
        Err(e) => format!("can't import {}: {e}", file.display()),
    }
}

/// Apply the built-in preset `name` to `opts.s`. Returns the lines to show.
fn apply_preset(opts: &mut Options, name: &str) -> Vec<String> {
    let Some(p) = profile::preset_profile(name) else {
        return vec![format!("no preset called {name} (try: presets)")];
    };
    let report = profile::evaluate(&p, &opts.s, &theme_dirs(opts), false);
    let mut lines = profile::describe(&report);
    match report.result {
        Some(s) => {
            opts.s = s;
            lines.insert(0, format!("applied preset {name}:"));
        }
        None => lines.insert(0, format!("preset {name}: already set")),
    }
    lines
}

/// `<settings folder>/themes`, where imported themes go.
fn user_themes_dir(opts: &Options) -> Option<std::path::PathBuf> {
    opts.settings_path
        .as_ref()?
        .parent()
        .map(|p| p.join("themes"))
}

/// Apply a share code to `opts.s` (CLI and menu). Returns the lines to show.
fn import_profile(opts: &mut Options, code: &str, controls: bool, apply: bool) -> Vec<String> {
    let profile = match profile::decode(code) {
        Ok(p) => p,
        Err(e) => return vec![format!("problem: {e}")],
    };
    let report = profile::evaluate(&profile, &opts.s, &theme_dirs(opts), controls);
    let mut lines = profile::describe(&report);
    if apply {
        match (&report.result, user_themes_dir(opts)) {
            (Some(_), dir) => {
                if report.theme_install.is_some() {
                    match dir {
                        Some(d) => {
                            if let Err(e) = profile::install_theme(&report, &d) {
                                lines.push(format!("problem: {e}"));
                                return lines;
                            }
                        }
                        None => {
                            lines
                                .push("problem: no settings folder to install the theme in".into());
                            return lines;
                        }
                    }
                }
                opts.s = report.result.clone().unwrap();
                lines.insert(0, "applied:".into());
            }
            (None, _) => lines.insert(0, "nothing applied".into()),
        }
    } else {
        lines.insert(0, "preview (nothing changed yet):".into());
    }
    lines
}

/// Folders themes are searched in.
fn theme_dirs(opts: &Options) -> Vec<std::path::PathBuf> {
    let settings_dir = opts.settings_path.as_ref().and_then(|p| p.parent());
    theme::search_dirs(opts.theme_dir.as_deref(), settings_dir)
}

/// The map theme named in the settings (built-in or a folder; failures give classic).
fn theme_style(opts: &Options) -> theme::ThemeStyle {
    let style = theme::ThemeStyle::load(&opts.s.video.theme, &theme_dirs(opts));
    for p in &style.problems {
        eprintln!("lgduel: {p}");
    }
    style
}

#[derive(Debug, Clone, PartialEq)]
struct Options {
    /// Saved player settings (with this run's command-line overrides applied).
    s: Settings,
    /// Where settings are loaded from / saved to (None = don't persist).
    settings_path: Option<std::path::PathBuf>,
    frames: Option<u64>,
    headless: bool,
    /// Headless: draw this menu screen over the scene (UI render check).
    menu: Option<menu::Screen>,
    screenshot: Option<String>,
    /// Feed scripted input (hold forward+right, turn right) for unattended runs.
    synthetic_input: bool,
    sound_test: bool,
    /// Offline opponent: None = idle target dummy (from settings, CLI can override).
    bot: Option<(StrafePattern, AimSkill)>,
    /// Play online against this server instead of offline.
    connect: Option<String>,
    /// Join by code ("LG-XXXX") through the rendezvous service.
    join: Option<String>,
    /// Joining by code: skip the direct attempt and use the relay.
    force_relay: bool,
    /// Headless picture: how far in front of you the opponent stands (default 400).
    enemy_distance: f32,
    /// Install this .lgtheme file at start.
    import_theme: Option<String>,
    /// Apply this share code to the settings at start.
    import_profile: Option<String>,
    /// Extra folder to look for themes in.
    theme_dir: Option<std::path::PathBuf>,
    /// Record a demo to this file.
    record: Option<String>,
    /// Play this demo file.
    play_demo: Option<String>,
    /// Skip the main menu and start a practice session.
    practice: bool,
    /// Skip the main menu: host a game with the saved host settings and join it.
    host_game: bool,
    /// Online: ask the server to let us watch instead of play.
    spectate: bool,
    /// Headless `--menu pause-online`: draw the online pause menu with sample players.
    menu_online: bool,
    /// Headless: draw a sample end-of-fight summary.
    summary_demo: bool,
    /// Headless `--menu console`: the console with a few commands run.
    menu_console: bool,
    /// Headless `--menu share`: the Share tab with a sample code and preview.
    menu_share: bool,
    /// Headless online run length.
    secs: f64,
}

/// Offline opponent described by the practice settings.
fn practice_bot(p: &settings::Practice) -> Option<(StrafePattern, AimSkill)> {
    p.bot.then(|| {
        (
            StrafePattern::parse(&p.pattern).unwrap_or(StrafePattern::RandomAd {
                min_ms: 150,
                max_ms: 600,
            }),
            AimSkill::preset(&p.skill).unwrap_or(AimSkill::MEDIUM),
        )
    })
}

impl Options {
    fn with_settings(s: Settings, settings_path: Option<std::path::PathBuf>) -> Self {
        Self {
            bot: practice_bot(&s.practice),
            s,
            settings_path,
            frames: None,
            headless: false,
            menu: None,
            screenshot: None,
            synthetic_input: false,
            sound_test: false,
            connect: None,
            practice: false,
            host_game: false,
            spectate: false,
            join: None,
            force_relay: false,
            enemy_distance: 400.0,
            import_theme: None,
            import_profile: None,
            theme_dir: None,
            record: None,
            play_demo: None,
            menu_online: false,
            summary_demo: false,
            menu_console: false,
            menu_share: false,
            secs: 5.0,
        }
    }
}

impl Default for Options {
    fn default() -> Self {
        Self::with_settings(Settings::default(), None)
    }
}

fn parse_args(args: impl IntoIterator<Item = String>, base: Options) -> Result<Options, String> {
    let mut o = base;
    let mut it = args.into_iter();
    while let Some(a) = it.next() {
        let mut val = |name: &str| it.next().ok_or(format!("{name} needs a value"));
        let num = |name: &str, v: String| {
            v.parse::<u64>()
                .map_err(|_| format!("{name} needs a number"))
        };
        match a.as_str() {
            "--frames" => o.frames = Some(num("--frames", val("--frames")?)?),
            "--fps-cap" => o.s.video.fps_cap = Some(num("--fps-cap", val("--fps-cap")?)? as u32),
            "--theme" => o.s.video.theme = val("--theme")?,
            "--enemy-model" => {
                let v = val("--enemy-model")?;
                o.s.models.enemy = models::ModelKind::parse(&v)
                    .ok_or(format!("--enemy-model must be classic|humanoid, got {v}"))?;
            }
            "--skins" => o.s.models.forced_colors = false,
            "--enemy-distance" => {
                o.enemy_distance = val("--enemy-distance")?
                    .parse::<f32>()
                    .ok()
                    .filter(|d| (60.0..=1500.0).contains(d))
                    .ok_or("--enemy-distance needs a number from 60 to 1500")?;
            }
            "--import-theme" => o.import_theme = Some(val("--import-theme")?),
            "--import-profile" => o.import_profile = Some(val("--import-profile")?),
            "--msaa" => {
                o.s.video.msaa = val("--msaa")?
                    .parse::<u32>()
                    .ok()
                    .filter(|n| matches!(n, 1 | 2 | 4))
                    .ok_or("--msaa needs 1, 2 or 4")?;
            }
            "--ui-scale" => {
                o.s.video.ui_scale = val("--ui-scale")?
                    .parse::<f32>()
                    .map_err(|_| "--ui-scale needs a number (0.75 to 2.0)")?;
                o.s.sanitize();
            }
            "--crosshair" => {
                let v = val("--crosshair")?;
                o.s.hud.crosshair.shape = hud::CrosshairShape::ALL
                    .into_iter()
                    .find(|s| format!("{s:?}").eq_ignore_ascii_case(&v))
                    .ok_or(format!(
                        "--crosshair must be cross|dot|circle|tshape, got {v}"
                    ))?;
            }
            "--crosshair-outline" => o.s.hud.crosshair.outline = true,
            "--accent" => {
                let v = val("--accent")?;
                let hex = v.trim_start_matches('#');
                let n = (hex.len() == 6)
                    .then(|| u32::from_str_radix(hex, 16).ok())
                    .flatten()
                    .ok_or("--accent needs a colour like #ff8800")?;
                o.s.ui.accent = [
                    ((n >> 16) & 255) as f32 / 255.0,
                    ((n >> 8) & 255) as f32 / 255.0,
                    (n & 255) as f32 / 255.0,
                ];
            }
            "--auto-hud" => o.s.hud.auto_scale = true,
            "--theme-dir" => o.theme_dir = Some(val("--theme-dir")?.into()),
            "--make-example-theme" => {
                let dir = std::path::PathBuf::from(val("--make-example-theme")?);
                std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
                for (name, bytes) in theme::example_theme_files() {
                    std::fs::write(dir.join(name), bytes)
                        .map_err(|e| format!("{}: {e}", dir.join(name).display()))?;
                }
                println!("example theme written to {}", dir.display());
                std::process::exit(0);
            }
            "--fov" => {
                o.s.video.fov = val("--fov")?
                    .parse::<f32>()
                    .map_err(|_| "--fov needs a number".to_string())?
                    .clamp(10.0, 130.0)
            }
            "--size" => {
                let v = val("--size")?;
                let (w, h) = v.split_once('x').ok_or("--size needs WxH")?;
                o.s.video.size = (
                    w.parse().map_err(|_| "--size needs WxH")?,
                    h.parse().map_err(|_| "--size needs WxH")?,
                );
            }
            "--sens-cm" => {
                o.s.mouse.cm_per_360 = val("--sens-cm")?
                    .parse::<f64>()
                    .ok()
                    .filter(|v| *v > 0.0)
                    .ok_or("--sens-cm needs a positive number")?
            }
            "--dpi" => {
                o.s.mouse.dpi = val("--dpi")?
                    .parse::<f64>()
                    .ok()
                    .filter(|v| *v > 0.0)
                    .ok_or("--dpi needs a positive number")?
            }
            "--invert" => o.s.mouse.invert_pitch = true,
            "--beam" => {
                let v = val("--beam")?;
                o.s.effects.beam_style = BeamStyle::parse(&v)
                    .ok_or(format!("--beam must be none|thin|thick|zigzag, got {v}"))?
            }
            "--gun" => {
                o.s.effects.gun = match val("--gun")?.as_str() {
                    "none" => effects::GunModel::None,
                    "small" => effects::GunModel::Small,
                    "normal" => effects::GunModel::Normal,
                    other => return Err(format!("--gun must be none|small|normal, got {other}")),
                }
            }
            "--input-display" => o.s.hud.input_display = true,
            "--no-sparks" => o.s.effects.sparks = false,
            "--no-net-graph" => o.s.hud.net_graph = false,
            "--no-sound" => o.s.audio.volume = 0.0,
            "--no-bot" => o.bot = None,
            "--bot" => {
                let v = val("--bot")?;
                let p = StrafePattern::parse(&v).ok_or(format!(
                    "--bot must be still|ad|mirror|anti|hsw|hswanti, got {v}"
                ))?;
                let skill = o.bot.map_or(AimSkill::MEDIUM, |b| b.1);
                o.bot = Some((p, skill));
            }
            "--bot-skill" => {
                let v = val("--bot-skill")?;
                let s = AimSkill::preset(&v).ok_or(format!(
                    "--bot-skill must be easy|medium|hard|aimbot, got {v}"
                ))?;
                let p = o.bot.map_or(
                    StrafePattern::RandomAd {
                        min_ms: 150,
                        max_ms: 600,
                    },
                    |b| b.0,
                );
                o.bot = Some((p, s));
            }
            "--sound-test" => o.sound_test = true,
            "--volume" => {
                o.s.audio.volume = val("--volume")?
                    .parse::<f32>()
                    .ok()
                    .filter(|v| (0.0..=1.0).contains(v))
                    .ok_or("--volume needs a number in 0..=1")?
            }
            "--spark-size" => {
                o.s.effects.spark_size = val("--spark-size")?
                    .parse::<f32>()
                    .ok()
                    .filter(|v| (0.0..=32.0).contains(v))
                    .ok_or("--spark-size needs a number in 0..=32")?
            }
            "--synthetic-input" => o.synthetic_input = true,
            "--no-settings" => o.settings_path = None,
            "--settings" => o.settings_path = Some(val("--settings")?.into()),
            "--connect" => o.connect = Some(val("--connect")?),
            "--join" => o.join = Some(val("--join")?),
            "--rendezvous" => o.s.rendezvous = val("--rendezvous")?,
            "--force-relay" => o.force_relay = true,
            "--record" => o.record = Some(val("--record")?),
            "--play-demo" => o.play_demo = Some(val("--play-demo")?),
            "--practice" => o.practice = true,
            "--map" => {
                let m = val("--map")?;
                if builtin::source(&m).is_none() {
                    let names: Vec<&str> = builtin::ALL.iter().map(|(n, _)| *n).collect();
                    return Err(format!("unknown map {m} (built-in: {})", names.join(", ")));
                }
                o.s.practice.map = m.clone();
                o.s.host.map = m;
            }
            "--host-game" => o.host_game = true,
            "--spectate" => o.spectate = true,
            "--summary-demo" => o.summary_demo = true,
            "--name" => o.s.name = val("--name")?,
            "--secs" => {
                o.secs = val("--secs")?
                    .parse::<f64>()
                    .ok()
                    .filter(|v| *v > 0.0)
                    .ok_or("--secs needs a positive number")?
            }
            "--vsync" => o.s.video.vsync = true,
            "--fullscreen" => o.s.video.fullscreen = true,
            "--windowed" => o.s.video.fullscreen = false,
            "--headless" => o.headless = true,
            "--menu" => {
                o.menu = Some(match val("--menu")?.as_str() {
                    "pause" => menu::Screen::Pause,
                    "pause-online" => {
                        o.menu_online = true;
                        menu::Screen::Pause
                    }
                    "console" => {
                        o.menu_console = true;
                        menu::Screen::Closed
                    }
                    "share" => {
                        o.menu_share = true;
                        menu::Screen::Settings
                    }
                    "settings" => menu::Screen::Settings,
                    "main" => menu::Screen::Main,
                    "practice" => menu::Screen::Practice,
                    "online" => menu::Screen::Online,
                    "host" => menu::Screen::Host,
                    other => return Err(format!("--menu must be pause|settings, got {other}")),
                })
            }
            "--screenshot" => o.screenshot = Some(val("--screenshot")?),
            "--print-settings" => {
                // Every setting with its current value (defaults with --no-settings).
                print!("{}", o.s.to_toml());
                std::process::exit(0);
            }
            "--version" => {
                println!(
                    "lgduel {} (sim tick {} Hz, protocol v{}) {}-{}",
                    env!("CARGO_PKG_VERSION"),
                    lgduel_sim::DEFAULT_TICK_RATE_HZ,
                    lgduel_net::PROTOCOL_VERSION,
                    std::env::consts::OS,
                    std::env::consts::ARCH
                );
                std::process::exit(0);
            }
            "--make-icons" => {
                let dir = std::path::PathBuf::from(val("--make-icons")?);
                std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
                for (name, bytes) in icon::icon_files() {
                    std::fs::write(dir.join(&name), bytes)
                        .map_err(|e| format!("{}: {e}", dir.join(&name).display()))?;
                }
                println!("icons written to {}", dir.display());
                std::process::exit(0);
            }
            "--doctor" => std::process::exit(doctor(&o)),
            other => return Err(format!("unknown argument: {other}")),
        }
    }
    Ok(o)
}

/// The rendezvous service to use: the player's setting, else the shipped one.
fn rendezvous_of(s: &Settings) -> Option<String> {
    Some(s.rendezvous.trim().to_string())
        .filter(|r| !r.is_empty())
        .or_else(browser::shipped_rendezvous)
}

/// Start joining `code` (a NetSession that resolves it via the rendezvous).
fn join_session(
    s: &Settings,
    code: &str,
    force_relay: bool,
    world: World,
) -> Result<NetSession, String> {
    let c = lgduel_net::rendezvous::parse_code(code)
        .ok_or(format!("{code:?} is not a join code (like LG-7KQ2)"))?;
    let rdv = rendezvous_of(s).ok_or("no rendezvous service set (Play online → \"via\")")?;
    let addr = lgduel_server::host::resolve_rendezvous(&rdv)?;
    NetSession::connect_code(addr, c, force_relay, &s.name, world).map_err(|e| e.to_string())
}

/// The online player list for the Esc menu.
fn player_rows(n: &NetSession) -> Vec<menu::PlayerRow> {
    let me = n.client.player().map(|p| p.0);
    n.roster()
        .iter()
        .map(|e| menu::PlayerRow {
            id: e.id,
            name: e.name.clone(),
            place: match (e.arena, e.spectating) {
                (Some(a), _) => format!("arena {}", a as u32 + 1),
                (None, true) => "spectating".into(),
                (None, false) => "queued".into(),
            },
            me: Some(e.id) == me,
        })
        .collect()
}

/// Headless `--record FILE`: a scripted practice duel against the practice bot
/// (the player aims at it, strafes and fires), recorded as a demo.
fn run_headless_record(opts: &Options) -> Result<(), String> {
    let path = opts.record.clone().unwrap_or_default();
    let world = load_world(&opts.s.practice.map);
    let (session, _) = new_practice(opts, &world);
    let Session::Local { mut s, bot } = session else {
        return Err("practice session expected".into());
    };
    let (mut bot, bot_id) = bot.ok_or("--record needs the practice bot")?;
    s.start_recording();
    let ticks = (opts.secs * s.game.cfg.tick_rate_hz as f64) as u32;
    for t in 0..ticks {
        let me = s.game.player(s.me).ok_or("no player")?;
        let target = s.game.player(bot_id).ok_or("no bot")?.pm.origin;
        let (pitch, yaw) = look_angles(target - me.pm.origin);
        let mine = UserCmd {
            tick: s.game.tick,
            pitch,
            yaw,
            forward: 0,
            right: if (t / 60) % 2 == 0 { 127 } else { -127 },
            buttons: lgduel_sim::usercmd::buttons::FIRE,
        };
        let theirs = bot.think(&s.game, bot_id);
        s.step_with_cmds(&[(bot_id, theirs), (s.me, mine)]);
    }
    let demo = s.stop_recording().ok_or("not recording")?;
    let (tick, hash) = demo.end().unwrap_or_default();
    std::fs::write(&path, demo.encode()).map_err(|e| format!("{path}: {e}"))?;
    let st = s.game.player(s.me).map(|p| p.stats).unwrap_or_default();
    println!(
        "DEMO RECORDED {path}: {ticks} ticks to tick {tick}, hash {hash:016x}, you dealt {} took {}",
        st.damage_dealt, st.damage_taken
    );
    Ok(())
}

/// Headless `--play-demo FILE`: play to the end (local demos are re-simulated
/// and must reproduce the recorded state hash), then render the last frame.
fn run_headless_demo(opts: &Options) -> Result<(), String> {
    let path = opts.play_demo.clone().unwrap_or_default();
    let bytes = std::fs::read(&path).map_err(|e| format!("{path}: {e}"))?;
    let mut d = demo_play::DemoPlayer::open(&bytes)?;
    match &mut d {
        demo_play::DemoPlayer::Local(r) => {
            let v = r.run_to_end();
            println!("DEMO REPLAY {v:?}");
            if !matches!(v, demo_play::Verdict::Match(_)) {
                return Err(format!("replay does not reproduce the recording: {v:?}"));
            }
        }
        demo_play::DemoPlayer::Net(_) => {
            while !d.finished() {
                d.advance(8_000);
            }
            println!("DEMO REPLAY online demo played to the end");
        }
    }
    let fv = d.frame_view().ok_or("nothing to show")?;
    let (world, cfg) = d.world_cfg();
    let (w, h) = opts.s.video.size;
    let off = offscreen::Offscreen::with_samples(w, h, opts.s.video.msaa)?;
    let style = theme_style(opts);
    let mut r = Renderer::with_samples(
        &off.device,
        offscreen::FORMAT,
        render::sane_samples(opts.s.video.msaa),
    );
    r.set_world(&off.device, &off.queue, world, &style);
    let cam = scene::view_camera(
        &fv,
        Default::default(),
        Default::default(),
        opts.s.video.fov,
    );
    let firing = fv.spectate.is_some_and(|s| s.firing);
    let hud_s = HudSettings {
        input_display: true,
        ..HudSettings::default()
    };
    let input = d.input_view();
    off.render(
        &mut r,
        3,
        |i| {
            let scene = scene::scene_mesh(
                &fv,
                world,
                cfg,
                &cam,
                firing,
                &EnemyColors::default(),
                &opts.s.models,
                &opts.s.effects,
                i,
            );
            let hud = scene::hud_mesh(
                &fv,
                &cam,
                (w, h),
                &DamageNumbers::default(),
                &hud_s,
                None,
                Some(input),
            );
            (cam, scene, hud)
        },
        style.sky(),
    );
    if let Some(shot) = &opts.screenshot {
        offscreen::save_png(std::path::Path::new(shot), w, h, &off.read_pixels())?;
        println!("SCREENSHOT {shot}");
    }
    println!("DEMO CHECK OK");
    Ok(())
}

/// A built-in map by name (arena_flat if unknown).
fn load_world(name: &str) -> World {
    let p = GameConfig::default().physics;
    let hull = (
        Vec3::from_array(p.player_mins),
        Vec3::from_array(p.player_maxs),
    );
    builtin::load(name, hull).unwrap_or_else(|e| {
        eprintln!("{e}; using arena_flat");
        builtin::load("arena_flat", hull).expect("builtin map")
    })
}

fn run_headless(opts: &Options) -> Result<(), String> {
    let (w, h) = opts.s.video.size;
    let off = offscreen::Offscreen::with_samples(w, h, opts.s.video.msaa)?;
    println!(
        "GPU {} ({:?}) offscreen {w}x{h}",
        off.adapter_info.name, off.adapter_info.backend
    );
    let world = load_world(&opts.s.practice.map);
    println!("MAP {} ({:016x})", world.name, world.content_hash());
    let style = theme_style(opts);
    let mut r = Renderer::with_samples(
        &off.device,
        offscreen::FORMAT,
        render::sane_samples(opts.s.video.msaa),
    );
    r.set_world(&off.device, &off.queue, &world, &style);
    // Local player at spawn 1, an idle opponent at spawn 2; look at the opponent's chest.
    // Opponent placed 400u in front (mid LG range) so effects are a few pixels wide.
    // Knockback off so the opponent stays put and the picture is stable frame to frame.
    let mut cfg = GameConfig::default();
    cfg.lg.knockback = 0.0;
    let mut session = LocalSession::new(cfg, world.clone(), 1);
    let me = session.game.player(session.me).unwrap().pm.origin;
    let enemy_id = session.game.players[1].id;
    session.teleport(enemy_id, me + Vec3::new(opts.enemy_distance, 0.0, 0.0));
    let enemy = session.game.player(enemy_id).unwrap().pm.origin;
    let eye = session.eye();
    let (pitch, yaw) = look_angles(enemy + Vec3::new(0.0, 0.0, 8.0) - eye);
    let cam = Camera {
        eye,
        pitch,
        yaw,
        fov: opts.s.video.fov,
    };
    let colors = EnemyColors::default();
    // Default HUD look (the pixel checks expect it), plus the optional input display.
    let hud_settings = HudSettings {
        input_display: opts.s.hud.input_display,
        auto_scale: opts.s.hud.auto_scale,
        crosshair: opts.s.hud.crosshair,
        ..HudSettings::default()
    };
    let mut numbers = DamageNumbers::default();
    let demo = opts.summary_demo.then_some(fight::FightSummary {
        won: true,
        secs: 12.4,
        acc: 46.0,
        opp_acc: 38.0,
        dealt: 212,
        taken: 180,
    });
    let mut total_hits = 0;
    let frames = opts.frames.unwrap_or(120) as u32;
    // Each frame advances the sim by a fixed virtual 2 ms while firing at the opponent,
    // so the HUD shows real stats and damage numbers (300 frames = 0.6 s = 12 LG cells).
    const VIRTUAL_FRAME_US: u64 = 2_000;
    let fire = UserCmd {
        pitch,
        yaw,
        buttons: buttons::FIRE,
        ..UserCmd::default()
    };
    let stats = off.render(
        &mut r,
        frames,
        |i| {
            session.advance(
                VIRTUAL_FRAME_US,
                |t| UserCmd { tick: t, ..fire },
                |_, _| vec![],
            );
            let fv = session.frame_view();
            total_hits += scene::track_hits(&fv, &mut numbers);
            numbers.update(VIRTUAL_FRAME_US as f32 / 1e6);
            let g = &session.game;
            let scene = scene::scene_mesh(
                &fv,
                &g.world,
                &g.cfg,
                &cam,
                true,
                &colors,
                &opts.s.models,
                &opts.s.effects,
                i,
            );
            // Sample input for the input display: strafing right while firing.
            let input = hud::InputView {
                forward: true,
                right: true,
                fire: true,
                ..Default::default()
            };
            let hud = scene::hud_mesh(
                &fv,
                &cam,
                (w, h),
                &numbers,
                &hud_settings,
                demo.as_ref(),
                Some(input),
            );
            (cam, scene, hud)
        },
        style.sky(),
    );
    println!("SIM hits {total_hits}, HUD {:?}", session.frame_view().hud);
    if let Some(s) = stats.summary() {
        println!("FRAMESTATS {s} | offscreen (GPU wait per frame)");
    }
    if let Some(screen) = opts.menu {
        let mut ui = ui::Ui::new(&off.device, offscreen::FORMAT, None);
        ui.set_ui_scale(opts.s.video.ui_scale);
        ui.set_accent(opts.s.ui.accent);
        let mut menu = menu::Menu {
            screen,
            tab: menu::Tab::Visuals,
            ..menu::Menu::default()
        };
        let mut s = opts.s.clone();
        let entries = server_entries(&s);
        let mut b = browser::Browser::new();
        if screen == menu::Screen::Online {
            // Give listed servers half a second to answer.
            b.set_master(rendezvous_of(&s));
            b.refresh(&entries);
            let t = Instant::now();
            while t.elapsed() < Duration::from_millis(500) {
                b.update(&entries);
                std::thread::sleep(Duration::from_millis(5));
            }
        }
        let rows = b.rows(&entries);
        menu.has_rendezvous = rendezvous_of(&s).is_some();
        if opts.menu_share {
            menu.tab = menu::Tab::Share;
            let mut sample = s.clone();
            sample.hud.crosshair.size = 12.0;
            sample.models.enemy = models::ModelKind::Humanoid;
            menu.share.code = profile::export(&sample, &[], false);
            menu.share.input = menu.share.code.clone();
            let p = profile::decode(&menu.share.code).unwrap_or_default();
            let r = profile::evaluate(&p, &s, &[], false);
            menu.share.report = profile::describe(&r);
        }
        if opts.menu_console {
            menu.console.open = true;
            for line in ["help", "sensitivity 41.5", "fov 500", "cvars crosshair"] {
                menu.console.print(&format!("] {line}"));
                for l in console::run(&mut s, line, None).0 {
                    menu.console.print(&l);
                }
            }
        }
        if opts.menu_online {
            menu.spectating = Some(false);
            menu.invite = Some((3, "dave".into()));
            menu.players = [
                ("alice", "arena 1", true),
                ("bob", "arena 1", false),
                ("dave", "arena 2", false),
                ("erin", "queued", false),
            ]
            .iter()
            .enumerate()
            .map(|(i, (name, place, me))| menu::PlayerRow {
                id: i as u8,
                name: name.to_string(),
                place: place.to_string(),
                me: *me,
            })
            .collect();
        }
        // A few frames: egui sizes windows on the first and fades them in.
        for _ in 0..5 {
            let out = ui.run_headless((w, h), |ctx| {
                menu.draw(ctx, &mut s, opts.menu_online, &rows);
            });
            ui.skip(out);
        }
        let out = ui.run_headless((w, h), |ctx| {
            menu.draw(ctx, &mut s, opts.menu_online, &rows);
        });
        ui.paint(&off.device, &off.queue, off.color_view(), (w, h), out);
    }
    let px = off.read_pixels();
    if let Some(path) = &opts.screenshot {
        offscreen::save_png(std::path::Path::new(path), w, h, &px)?;
        println!("SCREENSHOT {path}");
    }
    // Checks: not blank, floor visible.
    let distinct = offscreen::distinct_colors(&px);
    let floor = offscreen::linear_to_srgb8(world.colors[0]);
    let floor_frac = offscreen::color_fraction(&px, floor, 2);
    let classic = style.is_classic();
    println!(
        "IMAGE distinct colours {distinct}, floor {floor:?} {:.1}%",
        floor_frac * 100.0
    );
    if distinct < 4 {
        return Err(format!("render looks blank ({distinct} colours)"));
    }
    // A textured theme has no single floor colour; it must show many (filtered) colours instead.
    if !classic && distinct < 50 {
        return Err(format!(
            "theme {} looks flat ({distinct} colours)",
            style.name()
        ));
    }
    let total = (w * h) as f64;
    // The pause menu sits in front of the opponent in the --menu check.
    for (name, c) in if opts.menu.is_some() {
        vec![]
    } else {
        vec![
            ("head", colors.head),
            ("upper", colors.upper),
            ("lower", colors.lower),
        ]
    } {
        let srgb = offscreen::linear_to_srgb8(c);
        let px_count = (offscreen::color_fraction(&px, srgb, 2) * total).round() as u64;
        println!("IMAGE enemy {name} {srgb:?}: {px_count} px");
        // The humanoid is slimmer: its head is a few pixels.
        let min = if opts.s.models.enemy == models::ModelKind::Classic {
            50
        } else {
            20
        };
        if opts.s.models.forced_colors && px_count < min {
            return Err(format!(
                "forced enemy {name} colour not visible ({px_count} px)"
            ));
        }
    }
    if opts.menu.is_some() {
        let rgb = ui::MENU_PANEL;
        let px_count = (offscreen::color_fraction(&px, rgb, 2) * total).round() as u64;
        println!("IMAGE menu panel {rgb:?}: {px_count} px");
        if px_count < 5_000 {
            return Err(format!("pause menu not drawn ({px_count} px)"));
        }
        println!("MENU CHECK OK");
        // The menu covers the screen centre (crosshair, beam, damage numbers).
        return Ok(());
    }
    let mut hud_checks = vec![
        ("crosshair", hud_settings.crosshair.color, 20),
        ("hud text", hud_settings.text_color, 100),
    ];
    if !numbers.items.is_empty() {
        hud_checks.push(("damage numbers", hud_settings.damage_color, 10));
    } else if frames >= 60 {
        return Err("expected live damage numbers after firing at the opponent".into());
    }
    for (name, c, min_px) in hud_checks {
        let srgb = offscreen::linear_to_srgb8(c);
        let px_count = (offscreen::color_fraction(&px, srgb, 2) * total).round() as u64;
        println!("IMAGE {name} {srgb:?}: {px_count} px");
        if px_count < min_px {
            return Err(format!("{name} not visible ({px_count} px)"));
        }
    }
    let fx = &opts.s.effects;
    let mut fx_checks = vec![];
    if fx.beam_style != BeamStyle::None {
        fx_checks.push(("beam", fx.beam_color, 50));
    } else {
        let srgb = offscreen::linear_to_srgb8(fx.beam_color);
        let px_count = (offscreen::color_fraction(&px, srgb, 2) * total).round() as u64;
        println!("IMAGE beam (style none) {srgb:?}: {px_count} px");
        if px_count > 0 {
            return Err(format!("beam style none still drew {px_count} beam pixels"));
        }
    }
    if fx.sparks && fx.spark_size > 0.0 {
        fx_checks.push(("sparks", fx.spark_color, 10));
    } else {
        let srgb = offscreen::linear_to_srgb8(fx.spark_color);
        let px_count = (offscreen::color_fraction(&px, srgb, 2) * total).round() as u64;
        println!("IMAGE sparks (off) {srgb:?}: {px_count} px");
        if px_count > 0 {
            return Err(format!("sparks off still drew {px_count} spark pixels"));
        }
    }
    for (name, c, min_px) in fx_checks {
        let srgb = offscreen::linear_to_srgb8(c);
        let px_count = (offscreen::color_fraction(&px, srgb, 2) * total).round() as u64;
        println!("IMAGE {name} {srgb:?}: {px_count} px");
        if px_count < min_px {
            return Err(format!("{name} not visible ({px_count} px)"));
        }
    }
    if classic && floor_frac < 0.05 {
        return Err(format!(
            "floor colour covers only {:.2}%",
            floor_frac * 100.0
        ));
    }
    println!("RENDER CHECK OK");
    Ok(())
}

/// Where the game runs: offline (local sim, optional bot) or online.
/// One instance per process, so the variant size difference is irrelevant.
#[allow(clippy::large_enum_variant)]
enum Session {
    Local {
        s: LocalSession,
        bot: Option<(Bot, PlayerId)>,
    },
    Net(NetSession),
    Demo(Box<demo_play::DemoPlayer>),
}

impl Session {
    fn frame_view(&self) -> Option<FrameView> {
        match self {
            Session::Local { s, .. } => Some(s.frame_view()),
            Session::Net(n) => n.frame_view().cloned(),
            Session::Demo(d) => d.frame_view(),
        }
    }

    fn start_recording(&mut self) -> Result<(), String> {
        match self {
            Session::Local { s, .. } => {
                s.start_recording();
                Ok(())
            }
            Session::Net(n) => n.start_recording(),
            Session::Demo(_) => Err("can't record while playing a demo".into()),
        }
    }

    fn stop_recording(&mut self) -> Option<lgduel_net::demo::Demo> {
        match self {
            Session::Local { s, .. } => s.stop_recording(),
            Session::Net(n) => n.stop_recording(),
            Session::Demo(_) => None,
        }
    }

    fn world_cfg(&self) -> (&World, Option<&GameConfig>) {
        match self {
            Session::Local { s, .. } => (&s.game.world, Some(&s.game.cfg)),
            Session::Net(n) => (n.world(), n.config()),
            Session::Demo(d) => {
                let (w, c) = d.world_cfg();
                (w, Some(c))
            }
        }
    }
}

/// A server running on a background thread of this process.
struct HostHandle {
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
    /// "LG-XXXX" once the rendezvous service has given one.
    join_code: std::sync::Arc<std::sync::Mutex<Option<String>>>,
}

impl HostHandle {
    /// Start a server with the host settings; returns once it is listening (or failed).
    fn start(
        h: &settings::HostSettings,
        rendezvous: Option<String>,
    ) -> Result<(HostHandle, std::net::SocketAddr), String> {
        let mut cfg = lgduel_server::ServerConfig::default();
        if h.join_code
            && let Some(r) = rendezvous
        {
            cfg.server.rendezvous = r;
        }
        cfg.server.name = h.name.clone();
        cfg.server.port = h.port;
        cfg.server.map = h.map.clone();
        cfg.game.rules.mode = if h.classic {
            lgduel_sim::config::DuelMode::Classic
        } else {
            lgduel_sim::config::DuelMode::Vamp
        };
        cfg.game.rules.lifesteal_percent = h.lifesteal_percent;
        cfg.game.rules.hit_mode = if h.anti_lag {
            lgduel_sim::config::HitMode::AntiLag
        } else {
            lgduel_sim::config::HitMode::NoAntiLag
        };
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (tx, rx) = std::sync::mpsc::channel();
        let flag = stop.clone();
        let join_code = std::sync::Arc::new(std::sync::Mutex::new(None));
        let code_out = join_code.clone();
        let thread = std::thread::spawn(move || {
            let admin = lgduel_server::host::Admin {
                join_code: Some(code_out),
                ..Default::default()
            };
            let _ = lgduel_server::host::run_config(cfg, None, None, &flag, admin, |r| {
                let _ = tx.send(r);
            });
        });
        match rx.recv_timeout(Duration::from_secs(3)) {
            Ok(Ok(addr)) => Ok((
                HostHandle {
                    stop,
                    thread: Some(thread),
                    join_code,
                },
                addr,
            )),
            Ok(Err(e)) => Err(e),
            Err(_) => Err("server did not start".into()),
        }
    }
}

impl Drop for HostHandle {
    fn drop(&mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// Shipped servers (official) followed by the player's own.
fn server_entries(s: &Settings) -> Vec<(settings::ServerEntry, bool)> {
    let mut v: Vec<_> = browser::shipped_servers()
        .into_iter()
        .map(|e| (e, true))
        .collect();
    for e in &s.servers {
        if !v.iter().any(|(o, _)| o.address == e.address) {
            v.push((e.clone(), false));
        }
    }
    v
}

/// Offline practice session per the options (bot from practice settings).
fn new_practice(opts: &Options, world: &World) -> (Session, lgduel_sim::math::Angle) {
    let s = LocalSession::new(GameConfig::default(), world.clone(), 1);
    let bot = opts.bot.map(|(pattern, aim)| {
        let id = s.game.players.iter().find(|p| p.id != s.me).unwrap().id;
        println!("PRACTICE vs bot: pattern {pattern:?}, aim {aim:?}");
        (
            Bot::new(BotConfig {
                pattern,
                aim,
                ..BotConfig::default()
            }),
            id,
        )
    });
    let yaw = s.game.player(s.me).unwrap().yaw;
    (Session::Local { s, bot }, yaw)
}

fn resolve(addr: &str) -> Result<std::net::SocketAddr, String> {
    use std::net::ToSocketAddrs;
    let addr = addr.trim();
    // "host" or "1.2.3.4" without a port → default port. (IPv6 needs [addr]:port.)
    let owned;
    let addr = if !addr.contains(':') {
        owned = format!("{addr}:27960");
        owned.as_str()
    } else {
        addr
    };
    addr.to_socket_addrs()
        .map_err(|e| format!("{addr}: {e}"))?
        .next()
        .ok_or(format!("{addr}: no address"))
}

/// Headless online smoke run: connect, play for `secs` (camera tracks the
/// first opponent), render offscreen, check the opponent is drawn in the forced
/// colours. Proves network + snapshots + interpolation + prediction + render.
fn run_headless_online(opts: &Options) -> Result<(), String> {
    let world = load_world(&opts.s.practice.map);
    let (server, mut n) = match &opts.join {
        Some(code) => (
            code.clone(),
            join_session(&opts.s, code, opts.force_relay, world)?,
        ),
        None => {
            let addr = opts.connect.clone().unwrap_or_default();
            let server = resolve(&addr)?;
            (
                addr,
                NetSession::connect(server, &opts.s.name, world).map_err(|e| e.to_string())?,
            )
        }
    };
    let (w, h) = opts.s.video.size;
    let off = offscreen::Offscreen::with_samples(w, h, opts.s.video.msaa)?;
    let style = theme_style(opts);
    let mut r = Renderer::with_samples(
        &off.device,
        offscreen::FORMAT,
        render::sane_samples(opts.s.video.msaa),
    );
    let colors = EnemyColors::default();
    let hud_settings = HudSettings::default();
    let start = Instant::now();
    let mut aim = (lgduel_sim::math::Angle::ZERO, lgduel_sim::math::Angle::ZERO);
    let mut frames = 0u64;
    let mut seen_opponent_frames = 0u64;
    let mut asked_spectate = false;
    let mut recording = false;
    while start.elapsed().as_secs_f64() < opts.secs {
        if opts.record.is_some() && !recording {
            recording = n.start_recording().is_ok();
        }
        if opts.spectate && !asked_spectate && n.client.player().is_some() {
            n.set_spectate(true);
            asked_spectate = true;
        }
        let a = aim;
        let fire = if start.elapsed().as_secs_f64() > 1.0 {
            lgduel_sim::usercmd::buttons::FIRE
        } else {
            0
        };
        n.update(|t| UserCmd {
            tick: t,
            pitch: a.0,
            yaw: a.1,
            buttons: fire,
            ..UserCmd::default()
        });
        if let Some(fv) = n.frame_view()
            && let Some(o) = fv.others.iter().find(|o| o.alive)
        {
            aim = look_angles(o.origin + Vec3::new(0.0, 0.0, 8.0) - fv.eye);
            seen_opponent_frames += 1;
        }
        frames += 1;
        std::thread::sleep(Duration::from_millis(2));
    }
    if let Some(e) = &n.map_error {
        return Err(format!("can't join {server}: {e}"));
    }
    if let (Some(path), Some(demo)) = (&opts.record, n.stop_recording()) {
        std::fs::write(path, demo.encode()).map_err(|e| format!("{path}: {e}"))?;
        println!(
            "DEMO RECORDED {path}: {} frames (online)",
            demo.frames.len()
        );
    }
    // The picture is checked for the opponent's colours: if the session ended
    // right after a frag (opponent dead / respawning), keep playing idle up to
    // 2 s until they're back, aimed at.
    let wait = Instant::now();
    while wait.elapsed() < Duration::from_secs(2)
        && !n
            .frame_view()
            .is_some_and(|fv| fv.others.iter().any(|o| o.alive))
    {
        n.update(|t| UserCmd {
            tick: t,
            pitch: aim.0,
            yaw: aim.1,
            ..UserCmd::default()
        });
        std::thread::sleep(Duration::from_millis(2));
    }
    if let Some(fv) = n.frame_view()
        && let Some(o) = fv.others.iter().find(|o| o.alive)
    {
        aim = look_angles(o.origin + Vec3::new(0.0, 0.0, 8.0) - fv.eye);
    }
    let fv = n
        .frame_view()
        .cloned()
        .ok_or(format!("never got a frame from {server}: {:?}", n.state()))?;
    let world = n.world().clone();
    println!("MAP {} ({:016x})", world.name, world.content_hash());
    r.set_world(&off.device, &off.queue, &world, &style);
    let cfg = n.config().cloned().ok_or("no config from server")?;
    let cam = Camera {
        eye: fv.eye,
        pitch: fv.spectate.map_or(aim.0, |s| s.pitch),
        yaw: fv.spectate.map_or(aim.1, |s| s.yaw),
        fov: opts.s.video.fov,
    };
    println!("STATUS {}", fv.status.as_deref().unwrap_or("-"));
    let mut seed = 0;
    let stats = off.render(
        &mut r,
        10,
        |_| {
            seed += 1;
            let scene = scene::scene_mesh(
                &fv,
                &world,
                &cfg,
                &cam,
                fv.spectate.is_some_and(|s| s.firing),
                &colors,
                &opts.s.models,
                &opts.s.effects,
                seed,
            );
            let hud = scene::hud_mesh(
                &fv,
                &cam,
                (w, h),
                &DamageNumbers::default(),
                &hud_settings,
                None,
                None,
            );
            (cam, scene, hud)
        },
        style.sky(),
    );
    let px = off.read_pixels();
    if let Some(path) = &opts.screenshot {
        offscreen::save_png(std::path::Path::new(path), w, h, &px)?;
        println!("SCREENSHOT {path}");
    }
    let pred = n
        .predictor
        .as_ref()
        .map(|p| p.stats.clone())
        .unwrap_or_default();
    println!(
        "ONLINE {:?} | frames {frames}, opponent visible in {seen_opponent_frames} | rtt {:.1} ms | interp delay {:.2} ticks | prediction checked {} mean {:.4} | hitreg {:.1}% of {} | HUD {:?}",
        n.state(),
        n.client.conn.stats.rtt_us / 1000.0,
        n.interp.settings.delay_ticks,
        pred.checked,
        pred.mean_error(),
        n.hitreg.stats.agreement_pct(),
        n.hitreg.stats.shots(),
        fv.hud
    );
    if let Some(s) = stats.summary() {
        println!("FRAMESTATS {s} | offscreen online");
    }
    let total = (w * h) as f64;
    for (name, c) in [
        ("head", colors.head),
        ("upper", colors.upper),
        ("lower", colors.lower),
    ] {
        let srgb = offscreen::linear_to_srgb8(c);
        let count = (offscreen::color_fraction(&px, srgb, 2) * total).round() as u64;
        println!("IMAGE remote opponent {name} {srgb:?}: {count} px");
        if count < 30 {
            return Err(format!(
                "remote opponent {name} colour not visible ({count} px)"
            ));
        }
    }
    let g = fv.hud.net.ok_or("no net-graph data online")?;
    println!(
        "NETGRAPH ping {:.1} ms +-{:.1} | loss {:.2}% | tick {} Hz | snapshots {:.0}/s | interp {:.1} ms | in {:.1} KB/s out {:.1} KB/s | rubber-bands {} | hitreg {:.1}%",
        g.ping_ms,
        g.jitter_ms,
        g.loss_pct,
        g.tick_hz,
        g.snap_hz,
        g.interp_delay_ms,
        g.in_kbps,
        g.out_kbps,
        g.rubber_bands,
        g.hitreg_pct
    );
    if g.tick_hz != cfg.tick_rate_hz
        || !(100.0..=140.0).contains(&g.snap_hz)
        || g.ping_ms <= 0.0
        || g.in_kbps <= 0.0
        || g.out_kbps <= 0.0
    {
        return Err(format!("net-graph values implausible: {g:?}"));
    }
    let ng = offscreen::linear_to_srgb8(hud_settings.net_graph_color);
    let ng_px = (offscreen::color_fraction(&px, ng, 2) * total).round() as u64;
    println!("IMAGE net-graph {ng:?}: {ng_px} px");
    if ng_px < 200 {
        return Err(format!("net-graph not drawn ({ng_px} px)"));
    }
    n.disconnect();
    println!("ONLINE CHECK OK");
    Ok(())
}

struct App {
    opts: Options,
    world: World,
    /// When the app started (main-menu camera orbit).
    started: Instant,
    /// Server browser (created when the Online screen is first shown).
    browser: Option<browser::Browser>,
    /// Server running inside this process ("Host a game").
    host: Option<HostHandle>,
    /// None = main menu.
    session: Option<Session>,
    connect_started: Option<Instant>,
    look: MouseLook,
    keys: Keys,
    /// Alt held (for Alt+Enter).
    alt: bool,
    grabbed: bool,

    audio: audio::Audio,

    numbers: DamageNumbers,
    fights: fight::FightTracker,
    window: Option<Arc<Window>>,
    gfx: Option<Gfx>,
    renderer: Option<Renderer>,
    /// How the world is drawn (the map theme).
    style: theme::ThemeStyle,
    ui: Option<ui::Ui>,
    menu: menu::Menu,
    depth: Option<render::Targets>,
    stats: FrameStats,
    last_frame: Option<Instant>,
    frames: u64,
    error: Option<String>,
    /// `quit` typed in the console (handled at the end of the frame).
    quit_requested: bool,
    /// Demo file being recorded to (recording starts once a session is running).
    recording: Option<std::path::PathBuf>,
    recording_started: bool,
    /// The end of the demo being played was reported.
    demo_reported: bool,
    /// A challenge was pending last frame (the ping plays once when one arrives).
    was_challenged: bool,
}

impl App {
    fn set_grab(&mut self, grab: bool) {
        let grab = grab && self.session.is_some() && !self.menu.is_open();
        let Some(w) = &self.window else { return };
        if grab {
            let ok = w
                .set_cursor_grab(CursorGrabMode::Locked)
                .or_else(|_| w.set_cursor_grab(CursorGrabMode::Confined))
                .is_ok();
            w.set_cursor_visible(!ok);
            self.grabbed = ok;
        } else {
            let _ = w.set_cursor_grab(CursorGrabMode::None);
            w.set_cursor_visible(true);
            self.grabbed = false;
        }
    }

    /// Push edited settings into the running game (called while the settings screen is open).
    /// Rebuild the world style when the theme setting changed.
    fn refresh_style(&mut self) {
        if self.style.requested != self.opts.s.video.theme {
            self.load_style();
        }
    }

    fn import_theme_file(&mut self, file: &std::path::Path, name: Option<&str>) {
        let msg = import_theme(&mut self.opts, file, name);
        println!("THEMEPACK {msg}");
        self.menu.console.print(&msg);
        self.menu.share.report = vec![msg];
        self.menu.dirty = true;
        self.load_style();
    }

    fn share_preset(&mut self, name: &str) {
        let lines = apply_preset(&mut self.opts, name);
        for l in &lines {
            self.menu.console.print(l);
        }
        self.menu.share.report = lines;
        self.menu.dirty = true;
        self.apply_settings();
        self.load_style();
    }

    /// Import a share code (preview or apply) and report in the Share tab and console.
    fn share_import(&mut self, code: &str, controls: bool, apply: bool) {
        let lines = import_profile(&mut self.opts, code, controls, apply);
        for l in &lines {
            self.menu.console.print(l);
        }
        self.menu.share.report = lines;
        if apply {
            self.menu.dirty = true;
            self.apply_settings();
            self.load_style();
        }
    }

    /// (Re)load the theme named in the settings and tell the player about problems.
    fn load_style(&mut self) {
        self.style = theme_style(&self.opts);
        let msg = format!("theme: {}", self.style.name());
        println!("THEME {msg}");
        self.menu.console.print(&msg);
        for p in &self.style.problems {
            self.menu.console.print(p);
        }
        self.menu.themes = theme::available(&theme_dirs(&self.opts));
    }

    fn apply_settings(&mut self) {
        let s = &self.opts.s;
        self.look.settings = s.mouse;
        if (self.audio.volume > 0.0) != (s.audio.volume > 0.0)
            && !self.audio.is_live()
            && s.audio.volume > 0.0
        {
            self.audio = audio::Audio::open(s.audio.volume);
        }
        self.audio.volume = s.audio.volume;
        if let Some(g) = &mut self.gfx {
            g.set_vsync(s.video.vsync);
        }
        self.apply_fullscreen();
        let s = &self.opts.s;
        if let Some(w) = &self.window
            && !s.video.fullscreen
            && w.fullscreen().is_none()
        {
            let want = winit::dpi::PhysicalSize::new(s.video.size.0, s.video.size.1);
            if w.inner_size() != want {
                let _ = w.request_inner_size(want);
            }
        }
    }

    /// Write settings to disk if they changed.
    fn save_settings(&mut self) {
        if !self.menu.dirty {
            return;
        }
        self.menu.dirty = false;
        if let Some(path) = &self.opts.settings_path
            && let Err(e) = settings::save(path, &self.opts.s)
        {
            eprintln!("lgduel: could not save settings to {}: {e}", path.display());
        }
    }

    /// Switch borderless fullscreen on/off and remember it.
    fn toggle_fullscreen(&mut self) {
        self.opts.s.video.fullscreen = !self.opts.s.video.fullscreen;
        self.apply_fullscreen();
        self.menu.dirty = true;
        self.save_settings();
    }

    fn apply_fullscreen(&self) {
        if let Some(w) = &self.window {
            let want = self.opts.s.video.fullscreen;
            if want != w.fullscreen().is_some() {
                w.set_fullscreen(want.then_some(winit::window::Fullscreen::Borderless(None)));
            }
        }
    }

    fn toggle_menu(&mut self) {
        if self.session.is_none()
            || matches!(
                self.menu.screen,
                menu::Screen::Connecting | menu::Screen::Settings
            )
        {
            match self.menu.back() {
                Some(menu::Action::SettingsClosed) => self.save_settings(),
                Some(menu::Action::CancelConnect) => self.leave_session(None, menu::Screen::Online),
                _ => {}
            }
            return;
        }
        if !self.menu.is_open() {
            self.menu.screen = menu::Screen::Pause;
            self.keys = Keys::default();
            self.set_grab(false);
        } else {
            self.save_settings();
            self.menu.screen = menu::Screen::Closed;
            self.set_grab(true);
        }
    }

    fn toggle_console(&mut self) {
        let c = &mut self.menu.console;
        c.open = !c.open;
        if c.open {
            if c.lines.is_empty() {
                c.print("LGDuel console - help lists commands, cvars lists settings");
            }
            self.keys = Keys::default();
            self.set_grab(false);
        } else {
            self.save_settings();
            self.set_grab(true);
        }
    }

    /// Run a console line: settings change live (and are saved), some commands act.
    fn console_line(&mut self, line: &str) {
        let dir = self
            .opts
            .settings_path
            .as_ref()
            .and_then(|p| p.parent())
            .map(|p| p.to_path_buf());
        self.menu.console.print(&format!("] {line}"));
        let (out, action, changed) = console::run(&mut self.opts.s, line, dir.as_deref());
        for l in out {
            self.menu.console.print(&l);
        }
        if changed {
            self.menu.dirty = true;
            self.apply_settings();
        }
        let acted = action.is_some();
        match action {
            Some(console::ConsoleAction::Record(name)) => {
                match console::demo_path(dir.as_deref(), &name) {
                    Ok(p) if self.session.is_some() => {
                        self.save_recording();
                        self.menu
                            .console
                            .print(&format!("recording to {}", p.display()));
                        self.recording = Some(p);
                    }
                    Ok(_) => self.menu.console.print("start a game first"),
                    Err(e) => self.menu.console.print(&e),
                }
            }
            Some(console::ConsoleAction::ReloadTheme) => self.load_style(),
            Some(console::ConsoleAction::Preset(name)) => self.share_preset(&name),
            Some(console::ConsoleAction::ExportTheme(name)) => {
                let msg = export_theme(&self.opts, &name);
                self.menu.console.print(&msg);
            }
            Some(console::ConsoleAction::ImportTheme { file, name }) => {
                self.import_theme_file(std::path::Path::new(&file), name.as_deref());
            }
            Some(console::ConsoleAction::ExportProfile { controls }) => {
                let code = profile::export(&self.opts.s, &theme_dirs(&self.opts), controls);
                self.menu.console.print(&code);
            }
            Some(console::ConsoleAction::ImportProfile(code)) => {
                self.share_import(&code, false, true);
            }
            Some(console::ConsoleAction::StopRecord) => {
                if self.recording.is_some() {
                    self.save_recording();
                } else {
                    self.menu.console.print("not recording");
                }
            }
            Some(console::ConsoleAction::PlayDemo(name)) => {
                match console::demo_path(dir.as_deref(), &name) {
                    Ok(p) => self.play_demo(&p),
                    Err(e) => self.menu.console.print(&e),
                }
            }
            Some(console::ConsoleAction::Connect(a)) => {
                self.leave_session(None, menu::Screen::Closed);
                self.start_online(&a);
            }
            Some(console::ConsoleAction::Join(code)) => {
                self.leave_session(None, menu::Screen::Closed);
                self.start_code(&code);
            }
            Some(console::ConsoleAction::Practice) => {
                self.leave_session(None, menu::Screen::Closed);
                self.start_practice();
            }
            Some(console::ConsoleAction::Disconnect) => {
                self.leave_session(None, menu::Screen::Main)
            }
            Some(console::ConsoleAction::Quit) => self.quit_requested = true,
            None => {}
        }
        if acted && self.session.is_some() && self.menu.screen == menu::Screen::Closed {
            self.menu.console.open = false;
            self.set_grab(true);
        }
    }

    fn key(&mut self, code: KeyCode, down: bool) {
        match code {
            KeyCode::KeyW => self.keys.forward = down,
            KeyCode::KeyS => self.keys.back = down,
            KeyCode::KeyA => self.keys.left = down,
            KeyCode::KeyD => self.keys.right = down,
            KeyCode::Space => self.keys.jump = down,
            KeyCode::ControlLeft | KeyCode::KeyC => self.keys.crouch = down,
            KeyCode::Escape if down && self.menu.console.open => self.toggle_console(),
            KeyCode::Escape if down => self.toggle_menu(),
            KeyCode::Backquote if down => self.toggle_console(),
            KeyCode::F3 if down => self.opts.s.hud.net_graph = !self.opts.s.hud.net_graph,
            KeyCode::F11 if down => self.toggle_fullscreen(),
            KeyCode::Enter | KeyCode::NumpadEnter if down && self.alt => self.toggle_fullscreen(),
            _ => {}
        }
    }

    /// Advance the local sim by the real time since the last frame.
    fn simulate(&mut self, elapsed: Duration) {
        if self.opts.synthetic_input {
            self.keys.forward = true;
            self.keys.right = true;
            self.look.apply(2.0, 0.0);
        }
        let keys = self.keys;
        let look = self.look;
        let Some(session) = &mut self.session else {
            return;
        };
        match session {
            Session::Local { s, bot } => {
                s.advance(
                    elapsed.as_micros() as u64,
                    |t| keys.cmd(t, &look),
                    |g, _| match bot {
                        Some((b, id)) => vec![(*id, b.think(g, *id))],
                        None => vec![],
                    },
                );
            }
            Session::Net(n) => n.update(|t| keys.cmd(t, &look)),
            Session::Demo(d) => {
                d.advance(elapsed.as_micros() as u64);
                if let demo_play::DemoPlayer::Local(r) = d.as_ref()
                    && let Some(v) = &r.verdict
                    && !self.demo_reported
                {
                    self.demo_reported = true;
                    println!("DEMO end: {v:?}");
                }
            }
        }
        if self.recording.is_some()
            && let Some(s) = &mut self.session
            && !self.recording_started
        {
            self.recording_started = s.start_recording().is_ok();
        }
        self.check_connection();
        if let Some(fv) = self.session.as_ref().and_then(Session::frame_view) {
            let hits = scene::track_hits(&fv, &mut self.numbers);
            for _ in 0..hits {
                let a = &self.opts.s.audio;
                let pitch = if a.hit_pitch_by_damage {
                    1.0 + self.numbers.items.last().map_or(0, |d| d.amount.min(70)) as f32 / 140.0
                } else {
                    1.0
                };
                self.audio.play(audio::Sound::Hit, a.hit_volume, pitch);
            }
            if self.opts.s.audio.kill_sound {
                for e in &fv.events {
                    if let lgduel_sim::game::GameEvent::Kill { killer, .. } = e
                        && *killer == fv.me
                    {
                        self.audio.play(audio::Sound::Kill, 1.0, 1.0);
                    }
                }
            }
            let tick_hz = self
                .session
                .as_ref()
                .and_then(|s| s.world_cfg().1.map(|c| c.tick_rate_hz))
                .unwrap_or(125);
            let had_summary = self.fights.summary().is_some();
            self.fights.update(&fv, tick_hz, elapsed.as_secs_f32());
            if self.opts.s.audio.kill_sound
                && !had_summary
                && let Some(f) = self.fights.summary()
            {
                let s = if f.won {
                    audio::Sound::FightWon
                } else {
                    audio::Sound::FightLost
                };
                self.audio.play(s, 1.0, 1.0);
            }
            let challenged =
                matches!(&self.session, Some(Session::Net(n)) if n.incoming_challenge().is_some());
            if challenged && !self.was_challenged && self.opts.s.audio.ui_sounds {
                self.audio.play(audio::Sound::Challenge, 1.0, 1.0);
            }
            self.was_challenged = challenged;
        }
        self.numbers.update(elapsed.as_secs_f32());
    }

    fn finish(&mut self, el: &ActiveEventLoop) {
        self.end_session();
        if let Some(s) = self.stats.summary() {
            let (present, size) = self
                .gfx
                .as_ref()
                .map(|g| {
                    (
                        format!("{:?}", g.config.present_mode),
                        format!("{}x{}", g.config.width, g.config.height),
                    )
                })
                .unwrap_or_default();
            println!("FRAMESTATS {s} | present mode {present} | {size}");
        }
        el.exit();
    }

    /// Start an offline practice duel with the practice settings.
    fn start_practice(&mut self) {
        self.save_settings();
        self.opts.bot = practice_bot(&self.opts.s.practice);
        self.world = load_world(&self.opts.s.practice.map);
        let (session, yaw) = new_practice(&self.opts, &self.world);
        self.look = MouseLook::new(self.opts.s.mouse, yaw);
        self.session = Some(session);
        self.numbers = DamageNumbers::default();
        self.menu.screen = menu::Screen::Closed;
        self.set_grab(true);
    }

    /// Run a server in this process and join it.
    fn start_host(&mut self) {
        self.save_settings();
        self.host = None;
        match HostHandle::start(&self.opts.s.host, rendezvous_of(&self.opts.s)) {
            Ok((handle, addr)) => {
                let port = addr.port();
                let lan = lgduel_server::host::lan_ip()
                    .map_or("your LAN IP".to_string(), |ip| ip.to_string());
                let info = format!("Hosting: friends connect to {lan}:{port}");
                println!("HOST {info}");
                self.host = Some(handle);
                self.menu.hosting_info = Some(info);
                self.start_online(&format!("127.0.0.1:{port}"));
            }
            Err(e) => {
                println!("MENU Can't host: {e}");
                self.menu.message = Some(format!("Can't host: {e}"));
                self.menu.screen = menu::Screen::Host;
            }
        }
    }

    /// Begin joining by code; the Connecting screen shows the progress.
    fn start_code(&mut self, code: &str) {
        match join_session(
            &self.opts.s,
            code,
            self.opts.force_relay,
            self.world.clone(),
        ) {
            Ok(n) => {
                self.save_settings();
                println!("ONLINE joining {code} as {}", self.opts.s.name);
                self.session = Some(Session::Net(n));
                self.menu.connecting_to = code.trim().to_uppercase();
                self.menu.screen = menu::Screen::Connecting;
                self.connect_started = Some(Instant::now());
                self.numbers = DamageNumbers::default();
            }
            Err(e) => {
                self.menu.message = Some(format!("Can't join: {e}"));
                self.menu.screen = menu::Screen::Online;
            }
        }
    }

    /// Begin connecting to `addr`; the Connecting screen shows until accepted.
    fn start_online(&mut self, addr: &str) {
        let result = resolve(addr).and_then(|server| {
            NetSession::connect(server, &self.opts.s.name, self.world.clone())
                .map_err(|e| e.to_string())
        });
        match result {
            Ok(n) => {
                println!("ONLINE connecting to {addr} as {}", self.opts.s.name);
                self.session = Some(Session::Net(n));
                self.menu.connecting_to = addr.to_string();
                self.menu.screen = menu::Screen::Connecting;
                self.connect_started = Some(Instant::now());
                self.numbers = DamageNumbers::default();
            }
            Err(e) => {
                self.menu.message = Some(format!("Can't connect: {e}"));
                self.menu.screen = menu::Screen::Online;
            }
        }
    }

    /// Connecting → playing, or back to the menu on reject / timeout / lost server.
    fn check_connection(&mut self) {
        let Some(Session::Net(n)) = &self.session else {
            return;
        };
        // Joining by code: relay fallback can take a few seconds longer.
        let patience = if n.join_progress().is_some() { 12 } else { 6 };
        let problem = match n.state() {
            _ if n.join_failed().is_some() => Some(format!(
                "Can't join {}: {}",
                self.menu.connecting_to,
                n.join_failed().unwrap_or_default()
            )),
            _ if n.map_error.is_some() => Some(format!(
                "Can't join: {}",
                n.map_error.as_deref().unwrap_or_default()
            )),
            lgduel_net::client::ClientState::Rejected(r) => Some(format!("Server refused: {r}")),
            lgduel_net::client::ClientState::Disconnected(r) => Some(format!("Disconnected: {r}")),
            _ if self.menu.screen == menu::Screen::Connecting
                && self
                    .connect_started
                    .is_some_and(|t| t.elapsed() > Duration::from_secs(patience)) =>
            {
                Some(format!(
                    "No answer from {} (is the server running and the UDP port open?)",
                    self.menu.connecting_to
                ))
            }
            _ if n.server_silent(5.0) => Some("Lost connection to the server".to_string()),
            _ => None,
        };
        if let Some(msg) = problem {
            self.leave_session(Some(msg), menu::Screen::Online);
            return;
        }
        if self.menu.screen == menu::Screen::Connecting && n.frame_view().is_some() {
            println!("ONLINE joined {}", self.menu.connecting_to);
            self.menu.screen = menu::Screen::Closed;
            self.connect_started = None;
            self.set_grab(true);
        }
    }

    /// Stop the current game and return to a menu screen.
    fn leave_session(&mut self, message: Option<String>, to: menu::Screen) {
        if let Some(msg) = &message {
            println!("MENU {msg}");
        }
        self.end_session();
        // Leaving also stops a game this PC was hosting.
        self.host = None;
        self.menu.hosting_info = None;
        self.session = None;
        self.menu.message = message;
        self.menu.screen = to;
        self.keys = Keys::default();
        self.set_grab(false);
    }

    /// Print the session report and disconnect (if online).
    fn end_session(&mut self) {
        let Some(session) = &mut self.session else {
            return;
        };
        match session {
            Session::Local { s, bot } => {
                if let Some((_, id)) = bot {
                    let b = s.game.player(*id).unwrap();
                    println!(
                        "BOT acc {:.1}% dmg {} kills {} | you: dmg dealt {} taken {} W-L {}-{}",
                        b.stats.accuracy_pct(),
                        b.stats.damage_dealt,
                        b.stats.kills,
                        b.stats.damage_taken,
                        b.stats.damage_dealt,
                        b.stats.fights_lost,
                        b.stats.fights_won
                    );
                }
                let p = s.game.player(s.me).unwrap();
                println!(
                    "LOCAL tick {} origin ({:.1}, {:.1}, {:.1}) speed {:.1} yaw {:.2}",
                    s.game.tick.0,
                    p.pm.origin.x,
                    p.pm.origin.y,
                    p.pm.origin.z,
                    p.pm.velocity.length_xy(),
                    self.look.yaw.degrees()
                );
            }
            Session::Net(n) => {
                if let Some(fv) = n.frame_view() {
                    println!("ONLINE {:?} | HUD {:?}", n.state(), fv.hud);
                }
                n.disconnect();
            }
            Session::Demo(_) => {}
        }
        self.save_recording();
    }

    /// Write the demo being recorded (if any) to its file.
    fn save_recording(&mut self) {
        let Some(path) = self.recording.take() else {
            return;
        };
        self.recording_started = false;
        let Some(demo) = self.session.as_mut().and_then(Session::stop_recording) else {
            return;
        };
        let msg = match path
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|_| std::fs::write(&path, demo.encode()))
        {
            Ok(()) => format!(
                "demo saved: {} ({} frames)",
                path.display(),
                demo.frames.len()
            ),
            Err(e) => format!("demo not saved: {}: {e}", path.display()),
        };
        println!("DEMO {msg}");
        self.menu.console.print(&msg);
    }

    /// Play a demo file (replaces the current session).
    fn play_demo(&mut self, path: &std::path::Path) {
        let opened = std::fs::read(path)
            .map_err(|e| format!("{}: {e}", path.display()))
            .and_then(|b| demo_play::DemoPlayer::open(&b));
        match opened {
            Ok(d) => {
                self.leave_session(None, menu::Screen::Closed);
                println!("DEMO playing {}", path.display());
                self.session = Some(Session::Demo(Box::new(d)));
                self.demo_reported = false;
                self.set_grab(true);
            }
            Err(e) => self.menu.console.print(&format!("can't play demo: {e}")),
        }
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let attrs = Window::default_attributes().with_title("LGDuel");
        // Application id: desktop files and icons match windows by it (`lgduel.desktop`).
        #[cfg(target_os = "linux")]
        let attrs = {
            use winit::platform::x11::WindowAttributesExtX11;
            WindowAttributesExtX11::with_name(attrs, "lgduel", "LGDuel")
        };
        #[cfg(target_os = "linux")]
        let attrs = {
            use winit::platform::wayland::WindowAttributesExtWayland;
            WindowAttributesExtWayland::with_name(attrs, "lgduel", "lgduel")
        };
        let attrs = attrs
            .with_window_icon(winit::window::Icon::from_rgba(icon::icon_rgba(64), 64, 64).ok())
            .with_inner_size(winit::dpi::PhysicalSize::new(
                self.opts.s.video.size.0,
                self.opts.s.video.size.1,
            ))
            .with_fullscreen(
                self.opts
                    .s
                    .video
                    .fullscreen
                    .then_some(winit::window::Fullscreen::Borderless(None)),
            );
        let window = match el.create_window(attrs) {
            Ok(w) => Arc::new(w),
            Err(e) => {
                self.error = Some(format!("create window: {e}"));
                el.exit();
                return;
            }
        };
        match Gfx::new(window.clone(), self.opts.s.video.vsync) {
            Ok(g) => {
                println!(
                    "GPU {} ({:?}), {}x{}, present {:?}",
                    g.adapter_info.name,
                    g.adapter_info.backend,
                    g.config.width,
                    g.config.height,
                    g.config.present_mode
                );
                let samples = render::sane_samples(self.opts.s.video.msaa);
                let mut r = Renderer::with_samples(&g.device, g.config.format, samples);
                r.set_world(&g.device, &g.queue, &self.world, &self.style);
                self.depth = Some(render::targets(
                    &g.device,
                    g.config.width,
                    g.config.height,
                    g.config.format,
                    samples,
                ));
                self.renderer = Some(r);
                self.ui = Some(ui::Ui::new(&g.device, g.config.format, Some(&window)));
                self.gfx = Some(g);
            }
            Err(e) => {
                self.error = Some(e);
                el.exit();
                return;
            }
        }
        window.request_redraw();
        self.window = Some(window);
        if !self.opts.synthetic_input {
            self.set_grab(true);
        }
    }

    fn device_event(&mut self, _el: &ActiveEventLoop, _id: DeviceId, event: DeviceEvent) {
        // Raw device counts: no OS acceleration or cursor clamping.
        if let DeviceEvent::MouseMotion { delta } = event
            && self.grabbed
        {
            self.look.apply(delta.0, delta.1);
        }
    }

    fn window_event(&mut self, el: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        if self.menu.is_open()
            && let (Some(ui), Some(w)) = (&mut self.ui, &self.window)
        {
            let consumed = ui.on_window_event(w, &event);
            let esc = matches!(&event, WindowEvent::KeyboardInput { event: k, .. }
                if matches!(k.physical_key, PhysicalKey::Code(KeyCode::Escape | KeyCode::Backquote)));
            let game_input = matches!(
                event,
                WindowEvent::KeyboardInput { .. } | WindowEvent::MouseInput { .. }
            );
            if (consumed || game_input) && !esc {
                if let Some(w) = &self.window {
                    w.request_redraw();
                }
                return;
            }
        }
        match event {
            WindowEvent::CloseRequested => self.finish(el),
            WindowEvent::DroppedFile(path) => {
                if path
                    .extension()
                    .is_some_and(|e| e.eq_ignore_ascii_case("lgtheme"))
                {
                    self.import_theme_file(&path, None);
                } else {
                    self.menu
                        .console
                        .print("only .lgtheme files can be dropped here");
                }
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if let PhysicalKey::Code(code) = event.physical_key {
                    self.key(code, event.state == ElementState::Pressed);
                }
            }
            WindowEvent::MouseInput {
                state,
                button: MouseButton::Left,
                ..
            } => {
                if !self.grabbed && state == ElementState::Pressed {
                    self.set_grab(true);
                } else {
                    self.keys.fire = state == ElementState::Pressed;
                }
            }
            WindowEvent::ModifiersChanged(mods) => {
                self.alt = mods.state().alt_key();
            }
            WindowEvent::Focused(false) => {
                self.keys = Keys::default();
                self.set_grab(false);
            }
            WindowEvent::Resized(size) => {
                if let Some(g) = &mut self.gfx {
                    g.resize(size.width, size.height);
                    self.depth = Some(render::targets(
                        &g.device,
                        g.config.width,
                        g.config.height,
                        g.config.format,
                        render::sane_samples(self.opts.s.video.msaa),
                    ));
                }
            }
            WindowEvent::RedrawRequested => {
                if let Some(cap) = self.opts.s.video.fps_cap
                    && let Some(last) = self.last_frame
                {
                    let target = Duration::from_secs_f64(1.0 / cap.max(1) as f64);
                    let elapsed = last.elapsed();
                    if elapsed < target {
                        std::thread::sleep(target - elapsed);
                    }
                }
                let now = Instant::now();
                if let Some(last) = self.last_frame {
                    self.stats.record(now - last);
                    self.simulate(now - last);
                }
                self.last_frame = Some(now);
                self.refresh_style();
                let fv = self.session.as_ref().and_then(Session::frame_view);
                let (world, cfg) = match &self.session {
                    Some(s) => s.world_cfg(),
                    None => (&self.world, None),
                };
                let cam = match &fv {
                    // Spectating: through the watched player's eyes.
                    Some(f) => {
                        scene::view_camera(f, self.look.pitch, self.look.yaw, self.opts.s.video.fov)
                    }
                    // Main menu: slow orbit around the arena.
                    None => {
                        let t = self.started.elapsed().as_secs_f64() * 0.08;
                        let eye =
                            Vec3::new((t.cos() * 640.0) as f32, (t.sin() * 640.0) as f32, 260.0);
                        let (pitch, yaw) = look_angles(Vec3::new(0.0, 0.0, 40.0) - eye);
                        Camera {
                            eye,
                            pitch,
                            yaw,
                            fov: self.opts.s.video.fov,
                        }
                    }
                };
                let dynamic = match (&fv, cfg) {
                    (Some(fv), Some(cfg)) => scene::scene_mesh(
                        fv,
                        world,
                        cfg,
                        &cam,
                        fv.spectate.map_or(self.keys.fire, |s| s.firing),
                        &self.opts.s.enemy_colors,
                        &self.opts.s.models,
                        &self.opts.s.effects,
                        self.frames as u32,
                    ),
                    _ => vec![],
                };
                if let (Some(g), Some(r), Some(depth)) =
                    (&mut self.gfx, &mut self.renderer, &self.depth)
                {
                    let size = (g.config.width, g.config.height);
                    r.set_world(&g.device, &g.queue, world, &self.style);
                    r.set_dynamic(&g.device, &g.queue, &dynamic);
                    let hud = fv
                        .as_ref()
                        .map(|fv| {
                            let k = self.keys;
                            let demo_input = match &self.session {
                                Some(Session::Demo(d)) => Some(d.input_view()),
                                _ => None,
                            };
                            let input = match fv.spectate {
                                Some(sv) => hud::InputView {
                                    jump: sv.buttons & lgduel_sim::usercmd::buttons::JUMP != 0,
                                    crouch: sv.buttons & lgduel_sim::usercmd::buttons::CROUCH != 0,
                                    fire: sv.firing,
                                    ..Default::default()
                                },
                                None => hud::InputView {
                                    forward: k.forward,
                                    back: k.back,
                                    left: k.left,
                                    right: k.right,
                                    jump: k.jump,
                                    crouch: k.crouch,
                                    fire: k.fire,
                                },
                            };
                            // Demos always show the recorded inputs.
                            let hud_s = crate::hud::HudSettings {
                                input_display: self.opts.s.hud.input_display
                                    || demo_input.is_some(),
                                ..self.opts.s.hud
                            };
                            scene::hud_mesh(
                                fv,
                                &cam,
                                size,
                                &self.numbers,
                                &hud_s,
                                self.fights.summary(),
                                Some(demo_input.unwrap_or(input)),
                            )
                        })
                        .unwrap_or_default();
                    r.set_hud(&g.device, &g.queue, &hud);
                    let mut action = None;
                    let entries = server_entries(&self.opts.s);
                    let rows = if self.menu.screen == menu::Screen::Online {
                        let b = self.browser.get_or_insert_with(browser::Browser::new);
                        b.set_master(rendezvous_of(&self.opts.s));
                        b.update(&entries);
                        b.rows(&entries)
                    } else {
                        Vec::new()
                    };
                    let ui_out = match (&mut self.ui, &self.window) {
                        (Some(ui), Some(w)) if self.menu.is_open() => {
                            let online = matches!(self.session, Some(Session::Net(_)));
                            self.menu.has_rendezvous = rendezvous_of(&self.opts.s).is_some();
                            self.menu.connect_detail = match &self.session {
                                Some(Session::Net(n)) => n.join_progress(),
                                _ => None,
                            };
                            if let Some(h) = &self.host
                                && let Some(code) = h.join_code.lock().unwrap().clone()
                                && !self
                                    .menu
                                    .hosting_info
                                    .as_ref()
                                    .is_some_and(|i| i.contains(&code))
                            {
                                println!("HOST join code {code}");
                                self.menu.hosting_info = Some(format!(
                                    "{} | join code {code}",
                                    self.menu.hosting_info.clone().unwrap_or_default()
                                ));
                            }
                            self.menu.spectating = match &self.session {
                                Some(Session::Net(n)) => Some(n.spectating()),
                                _ => None,
                            };
                            (self.menu.players, self.menu.invite) = match &self.session {
                                Some(Session::Net(n)) => (player_rows(n), n.incoming_challenge()),
                                _ => (Vec::new(), None),
                            };
                            let (menu, s) = (&mut self.menu, &mut self.opts.s);
                            ui.set_ui_scale(s.video.ui_scale);
                            ui.set_accent(s.ui.accent);
                            Some(
                                ui.run_windowed(w, |ctx| action = menu.draw(ctx, s, online, &rows)),
                            )
                        }
                        _ => None,
                    };
                    let sky = self.style.sky();
                    let ui = &mut self.ui;
                    g.frame(|d, q, v| {
                        r.draw(d, q, v, depth, size, &cam, sky);
                        if let (Some(ui), Some(out)) = (ui.as_mut(), ui_out) {
                            ui.paint(d, q, v, size, out);
                        }
                    });
                    if self.menu.dirty {
                        self.apply_settings();
                    }
                    if action.is_some()
                        && self.opts.s.audio.ui_sounds
                        && !matches!(action, Some(menu::Action::ConsoleLine(_)))
                    {
                        self.audio.play(audio::Sound::UiClick, 1.0, 1.0);
                    }
                    match action {
                        Some(menu::Action::Resume) => self.set_grab(true),
                        Some(menu::Action::TestSound) => self.audio.play_hit(),
                        Some(menu::Action::Challenge(id)) => {
                            if let Some(Session::Net(n)) = &mut self.session {
                                n.challenge(id);
                            }
                        }
                        Some(menu::Action::Respond { from, accept }) => {
                            if let Some(Session::Net(n)) = &mut self.session {
                                n.respond(from, accept);
                            }
                            if accept {
                                self.menu.screen = menu::Screen::Closed;
                                self.set_grab(true);
                            }
                        }
                        Some(menu::Action::Spectate(on)) => {
                            if let Some(Session::Net(n)) = &mut self.session {
                                n.set_spectate(on);
                            }
                            self.menu.screen = menu::Screen::Closed;
                            self.set_grab(true);
                        }
                        Some(menu::Action::SettingsClosed) => self.save_settings(),
                        Some(menu::Action::Disconnect) => {
                            self.leave_session(None, menu::Screen::Main)
                        }
                        Some(menu::Action::CancelConnect) => {
                            self.leave_session(None, menu::Screen::Online)
                        }
                        Some(menu::Action::StartPractice) => self.start_practice(),
                        Some(menu::Action::Connect(addr)) => self.start_online(&addr),
                        Some(menu::Action::JoinCode(code)) => self.start_code(&code),
                        Some(menu::Action::ShareExport { controls }) => {
                            self.menu.share.code =
                                profile::export(&self.opts.s, &theme_dirs(&self.opts), controls);
                        }
                        Some(menu::Action::Preset(name)) => self.share_preset(&name),
                        Some(menu::Action::SharePreview { code, controls }) => {
                            self.share_import(&code, controls, false)
                        }
                        Some(menu::Action::ShareApply { code, controls }) => {
                            self.share_import(&code, controls, true)
                        }
                        Some(menu::Action::ConsoleLine(line)) => {
                            self.console_line(&line);
                            if self.quit_requested {
                                self.finish(el);
                                return;
                            }
                        }
                        Some(menu::Action::StartHost) => self.start_host(),
                        Some(menu::Action::RefreshServers) => {
                            if let Some(b) = &mut self.browser {
                                b.refresh(&entries);
                            }
                        }
                        Some(menu::Action::AddServer { name, address }) => {
                            if !self.opts.s.servers.iter().any(|e| e.address == address) {
                                self.opts
                                    .s
                                    .servers
                                    .push(settings::ServerEntry { name, address });
                                self.menu.dirty = true;
                                self.save_settings();
                                if let Some(b) = &mut self.browser {
                                    b.refresh(&server_entries(&self.opts.s));
                                }
                            }
                        }
                        Some(menu::Action::RemoveServer(address)) => {
                            self.opts.s.servers.retain(|e| e.address != address);
                            self.menu.dirty = true;
                            self.save_settings();
                            if let Some(b) = &mut self.browser {
                                b.forget(&address);
                            }
                        }
                        Some(menu::Action::Quit) => {
                            self.finish(el);
                            return;
                        }
                        None => {}
                    }
                }
                self.frames += 1;
                if self.opts.frames.is_some_and(|n| self.frames >= n) {
                    self.finish(el);
                    return;
                }
                if let Some(w) = &self.window {
                    w.request_redraw();
                }
            }
            _ => {}
        }
    }
}

fn main() {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    if argv.first().map(String::as_str) == Some("host") {
        let result = lgduel_server::host::parse_args(argv.into_iter().skip(1))
            .and_then(|a| lgduel_server::host::run(&a));
        if let Err(e) = result {
            eprintln!("lgduel host: {e}");
            std::process::exit(1);
        }
        return;
    }
    // Settings file: windowed runs load (and create) it; headless runs and
    // `--no-settings` use defaults so tests never depend on a player's file.
    let headless = argv.iter().any(|a| a == "--headless");
    let no_settings = argv.iter().any(|a| a == "--no-settings");
    let explicit = argv
        .iter()
        .position(|a| a == "--settings")
        .and_then(|i| argv.get(i + 1))
        .map(std::path::PathBuf::from);
    let path = if headless || no_settings {
        None
    } else {
        explicit.or_else(settings::default_path)
    };
    let base = match &path {
        Some(p) => {
            let (s, warning) = settings::load(p);
            if let Some(w) = warning {
                eprintln!("lgduel: settings: {w}");
            } else if !p.exists()
                && let Err(e) = settings::save(p, &s)
            {
                eprintln!("lgduel: could not write {}: {e}", p.display());
            }
            Options::with_settings(s, path.clone())
        }
        None => Options::default(),
    };
    let mut opts = match parse_args(argv, base) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("lgduel: {e}");
            std::process::exit(2);
        }
    };
    if opts.headless && opts.play_demo.is_some() {
        if let Err(e) = run_headless_demo(&opts) {
            eprintln!("lgduel: {e}");
            std::process::exit(1);
        }
        return;
    }
    if opts.headless && opts.record.is_some() && opts.connect.is_none() && opts.join.is_none() {
        if let Err(e) = run_headless_record(&opts) {
            eprintln!("lgduel: {e}");
            std::process::exit(1);
        }
        return;
    }
    if opts.headless && (opts.connect.is_some() || opts.join.is_some()) {
        if let Err(e) = run_headless_online(&opts) {
            eprintln!("lgduel: {e}");
            std::process::exit(1);
        }
        return;
    }
    if opts.headless {
        if let Err(e) = run_headless(&opts) {
            eprintln!("lgduel: {e}");
            std::process::exit(1);
        }
        return;
    }
    let event_loop = EventLoop::new().expect("event loop");
    event_loop.set_control_flow(ControlFlow::Poll);
    let audio = if opts.s.audio.volume > 0.0 {
        audio::Audio::open(opts.s.audio.volume)
    } else {
        audio::Audio::silent("muted")
    };
    println!("AUDIO {}", audio.description);
    if opts.sound_test {
        if !audio.is_live() {
            eprintln!("lgduel: no audio output");
            std::process::exit(1);
        }
        for _ in 0..5 {
            audio.play_hit();
            std::thread::sleep(Duration::from_millis(120));
        }
        for s in audio::Sound::ALL {
            audio.play(s, 1.0, 1.0);
            std::thread::sleep(Duration::from_millis(450));
        }
        std::thread::sleep(Duration::from_millis(200));
        println!(
            "SOUND TEST played 5 hit sounds and all {} sounds",
            audio::Sound::ALL.len()
        );
        return;
    }
    let world = load_world(&opts.s.practice.map);
    let mut first_menu = menu::Menu::default();
    let mut connect_to = None;
    let (session, start_yaw) = if let Some(addr) = &opts.connect {
        connect_to = Some(addr.clone());
        (None, lgduel_sim::math::Angle::ZERO)
    } else if opts.practice || opts.synthetic_input {
        let (s, yaw) = new_practice(&opts, &world);
        (Some(s), yaw)
    } else {
        first_menu.screen = menu::Screen::Main;
        (None, lgduel_sim::math::Angle::ZERO)
    };
    let look = MouseLook::new(opts.s.mouse, start_yaw);
    println!(
        "MOUSE {} cm/360 @ {} dpi = {:.5} deg/count (raw input, sub-count accumulation)",
        opts.s.mouse.cm_per_360,
        opts.s.mouse.dpi,
        opts.s.mouse.degrees_per_count()
    );
    if let Some(f) = opts.import_theme.clone() {
        let msg = import_theme(&mut opts, std::path::Path::new(&f), None);
        println!("THEMEPACK {msg}");
    }
    if let Some(code) = opts.import_profile.clone() {
        for l in import_profile(&mut opts, &code, false, true) {
            println!("PROFILE {l}");
        }
    }
    let opts_record = opts.record.clone();
    let style = theme_style(&opts);
    let mut app = App {
        style,
        world,
        session,
        connect_started: None,
        started: Instant::now(),
        browser: None,
        host: None,
        look,
        keys: Keys::default(),
        alt: false,
        grabbed: false,

        audio,

        numbers: DamageNumbers::default(),
        fights: fight::FightTracker::default(),
        opts,
        window: None,
        gfx: None,
        renderer: None,
        ui: None,
        menu: first_menu,
        depth: None,
        stats: FrameStats::default(),
        last_frame: None,
        frames: 0,
        error: None,
        quit_requested: false,
        recording: opts_record.map(std::path::PathBuf::from),
        recording_started: false,
        demo_reported: false,
        was_challenged: false,
    };
    app.menu.themes = theme::available(&theme_dirs(&app.opts));
    for p in &app.style.problems {
        app.menu.console.print(p);
    }
    if let Some(p) = app.opts.play_demo.clone() {
        app.play_demo(std::path::Path::new(&p));
    } else if let Some(code) = app.opts.join.clone() {
        app.start_code(&code);
    } else if let Some(addr) = connect_to {
        app.start_online(&addr);
    } else if app.opts.host_game {
        app.start_host();
    }
    if let Err(e) = event_loop.run_app(&mut app) {
        eprintln!("lgduel: event loop: {e}");
        std::process::exit(1);
    }
    if let Some(e) = app.error {
        eprintln!("lgduel: {e}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(s: &str) -> Result<Options, String> {
        parse_args(args(s), Options::default())
    }

    fn args(s: &str) -> Vec<String> {
        s.split_whitespace().map(String::from).collect()
    }

    #[test]
    fn parses_args() {
        assert_eq!(parse("").unwrap(), Options::default());
        let o = parse("--frames 500 --vsync --fps-cap 240 --size 1920x1080 --fov 300").unwrap();
        assert_eq!(o.frames, Some(500));
        assert!(o.s.video.vsync);
        assert_eq!(o.s.video.fps_cap, Some(240));
        assert_eq!(o.s.video.size, (1920, 1080));
        assert_eq!(o.s.video.fov, 130.0);
        let o = parse("--headless --screenshot a.png").unwrap();
        assert!(o.headless && o.screenshot.as_deref() == Some("a.png"));
        assert!(parse("--frames").is_err());
        assert!(parse("--size 10").is_err());
        assert!(parse("--bogus").is_err());
    }
}
