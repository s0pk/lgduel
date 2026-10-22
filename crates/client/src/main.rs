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
mod effects;
mod fight;
mod font;
mod frame_stats;
mod gfx;
mod hud;
mod input;
mod local;
mod menu;
mod net_session;
mod offscreen;
mod players;
mod render;
mod scene;
mod settings;
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

const CLEAR: wgpu::Color = wgpu::Color {
    r: 0.02,
    g: 0.02,
    b: 0.03,
    a: 1.0,
};

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
            menu_online: false,
            summary_demo: false,
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
                    "settings" => menu::Screen::Settings,
                    "main" => menu::Screen::Main,
                    "practice" => menu::Screen::Practice,
                    "online" => menu::Screen::Online,
                    "host" => menu::Screen::Host,
                    other => return Err(format!("--menu must be pause|settings, got {other}")),
                })
            }
            "--screenshot" => o.screenshot = Some(val("--screenshot")?),
            "--version" => {
                println!(
                    "lgduel {} (sim tick {} Hz, protocol v{})",
                    env!("CARGO_PKG_VERSION"),
                    lgduel_sim::DEFAULT_TICK_RATE_HZ,
                    lgduel_net::PROTOCOL_VERSION
                );
                std::process::exit(0);
            }
            other => return Err(format!("unknown argument: {other}")),
        }
    }
    Ok(o)
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
    let off = offscreen::Offscreen::new(w, h)?;
    println!(
        "GPU {} ({:?}) offscreen {w}x{h}",
        off.adapter_info.name, off.adapter_info.backend
    );
    let world = load_world(&opts.s.practice.map);
    println!("MAP {} ({:016x})", world.name, world.content_hash());
    let mut r = Renderer::new(&off.device, offscreen::FORMAT);
    r.set_world(&off.device, &world);
    // Local player at spawn 1, an idle opponent at spawn 2; look at the opponent's chest.
    // Opponent placed 400u in front (mid LG range) so effects are a few pixels wide.
    // Knockback off so the opponent stays put and the picture is stable frame to frame.
    let mut cfg = GameConfig::default();
    cfg.lg.knockback = 0.0;
    let mut session = LocalSession::new(cfg, world.clone(), 1);
    let me = session.game.player(session.me).unwrap().pm.origin;
    let enemy_id = session.game.players[1].id;
    session.teleport(enemy_id, me + Vec3::new(400.0, 0.0, 0.0));
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
    let hud_settings = HudSettings::default();
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
                &opts.s.effects,
                i,
            );
            let hud = scene::hud_mesh(&fv, &cam, (w, h), &numbers, &hud_settings, demo.as_ref());
            (cam, scene, hud)
        },
        CLEAR,
    );
    println!("SIM hits {total_hits}, HUD {:?}", session.frame_view().hud);
    if let Some(s) = stats.summary() {
        println!("FRAMESTATS {s} | offscreen (GPU wait per frame)");
    }
    if let Some(screen) = opts.menu {
        let mut ui = ui::Ui::new(&off.device, offscreen::FORMAT, None);
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
            b.refresh(&entries);
            let t = Instant::now();
            while t.elapsed() < Duration::from_millis(500) {
                b.update(&entries);
                std::thread::sleep(Duration::from_millis(5));
            }
        }
        let rows = b.rows(&entries);
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
    println!(
        "IMAGE distinct colours {distinct}, floor {floor:?} {:.1}%",
        floor_frac * 100.0
    );
    if distinct < 4 {
        return Err(format!("render looks blank ({distinct} colours)"));
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
        if px_count < 50 {
            return Err(format!(
                "forced enemy {name} colour not visible ({px_count} px)"
            ));
        }
    }
    if opts.menu.is_some() {
        let fill = egui::Visuals::dark().window_fill;
        let rgb = [fill.r(), fill.g(), fill.b()];
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
    if floor_frac < 0.05 {
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
}

impl Session {
    fn frame_view(&self) -> Option<FrameView> {
        match self {
            Session::Local { s, .. } => Some(s.frame_view()),
            Session::Net(n) => n.frame_view().cloned(),
        }
    }

    fn world_cfg(&self) -> (&World, Option<&GameConfig>) {
        match self {
            Session::Local { s, .. } => (&s.game.world, Some(&s.game.cfg)),
            Session::Net(n) => (n.world(), n.config()),
        }
    }
}

/// A server running on a background thread of this process.
struct HostHandle {
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl HostHandle {
    /// Start a server with the host settings; returns once it is listening (or failed).
    fn start(h: &settings::HostSettings) -> Result<(HostHandle, std::net::SocketAddr), String> {
        let mut cfg = lgduel_server::ServerConfig::default();
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
        let thread = std::thread::spawn(move || {
            let _ = lgduel_server::host::run_config(cfg, None, None, &flag, |r| {
                let _ = tx.send(r);
            });
        });
        match rx.recv_timeout(Duration::from_secs(3)) {
            Ok(Ok(addr)) => Ok((
                HostHandle {
                    stop,
                    thread: Some(thread),
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
    let server = resolve(opts.connect.as_deref().unwrap())?;
    let mut n = NetSession::connect(server, &opts.s.name, load_world(&opts.s.practice.map))
        .map_err(|e| e.to_string())?;
    let (w, h) = opts.s.video.size;
    let off = offscreen::Offscreen::new(w, h)?;
    let mut r = Renderer::new(&off.device, offscreen::FORMAT);
    let colors = EnemyColors::default();
    let hud_settings = HudSettings::default();
    let start = Instant::now();
    let mut aim = (lgduel_sim::math::Angle::ZERO, lgduel_sim::math::Angle::ZERO);
    let mut frames = 0u64;
    let mut seen_opponent_frames = 0u64;
    let mut asked_spectate = false;
    while start.elapsed().as_secs_f64() < opts.secs {
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
    let fv = n
        .frame_view()
        .cloned()
        .ok_or(format!("never got a frame from {server}: {:?}", n.state()))?;
    let world = n.world().clone();
    println!("MAP {} ({:016x})", world.name, world.content_hash());
    r.set_world(&off.device, &world);
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
            );
            (cam, scene, hud)
        },
        CLEAR,
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
    ui: Option<ui::Ui>,
    menu: menu::Menu,
    depth: Option<wgpu::TextureView>,
    stats: FrameStats,
    last_frame: Option<Instant>,
    frames: u64,
    error: Option<String>,
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

    fn key(&mut self, code: KeyCode, down: bool) {
        match code {
            KeyCode::KeyW => self.keys.forward = down,
            KeyCode::KeyS => self.keys.back = down,
            KeyCode::KeyA => self.keys.left = down,
            KeyCode::KeyD => self.keys.right = down,
            KeyCode::Space => self.keys.jump = down,
            KeyCode::ControlLeft | KeyCode::KeyC => self.keys.crouch = down,
            KeyCode::Escape if down => self.toggle_menu(),
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
        }
        self.check_connection();
        if let Some(fv) = self.session.as_ref().and_then(Session::frame_view) {
            for _ in 0..scene::track_hits(&fv, &mut self.numbers) {
                self.audio.play_hit();
            }
            let tick_hz = self
                .session
                .as_ref()
                .and_then(|s| s.world_cfg().1.map(|c| c.tick_rate_hz))
                .unwrap_or(125);
            self.fights.update(&fv, tick_hz, elapsed.as_secs_f32());
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
        match HostHandle::start(&self.opts.s.host) {
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
        let problem = match n.state() {
            _ if n.map_error.is_some() => Some(format!(
                "Can't join: {}",
                n.map_error.as_deref().unwrap_or_default()
            )),
            lgduel_net::client::ClientState::Rejected(r) => Some(format!("Server refused: {r}")),
            lgduel_net::client::ClientState::Disconnected(r) => Some(format!("Disconnected: {r}")),
            _ if self.menu.screen == menu::Screen::Connecting
                && self
                    .connect_started
                    .is_some_and(|t| t.elapsed() > Duration::from_secs(6)) =>
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
        }
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let attrs = Window::default_attributes()
            .with_title("LGDuel")
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
                let mut r = Renderer::new(&g.device, g.config.format);
                r.set_world(&g.device, &self.world);
                self.depth = Some(render::depth_texture(
                    &g.device,
                    g.config.width,
                    g.config.height,
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
                if k.physical_key == PhysicalKey::Code(KeyCode::Escape));
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
                    self.depth = Some(render::depth_texture(
                        &g.device,
                        g.config.width,
                        g.config.height,
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
                let fv = self.session.as_ref().and_then(Session::frame_view);
                let (world, cfg) = match &self.session {
                    Some(s) => s.world_cfg(),
                    None => (&self.world, None),
                };
                let cam = match &fv {
                    // Spectating: through the watched player's eyes.
                    Some(f) => Camera {
                        eye: f.eye,
                        pitch: f.spectate.map_or(self.look.pitch, |s| s.pitch),
                        yaw: f.spectate.map_or(self.look.yaw, |s| s.yaw),
                        fov: self.opts.s.video.fov,
                    },
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
                        &self.opts.s.effects,
                        self.frames as u32,
                    ),
                    _ => vec![],
                };
                if let (Some(g), Some(r), Some(depth)) =
                    (&mut self.gfx, &mut self.renderer, &self.depth)
                {
                    let size = (g.config.width, g.config.height);
                    r.set_world(&g.device, world);
                    r.set_dynamic(&g.device, &g.queue, &dynamic);
                    let hud = fv
                        .as_ref()
                        .map(|fv| {
                            scene::hud_mesh(
                                fv,
                                &cam,
                                size,
                                &self.numbers,
                                &self.opts.s.hud,
                                self.fights.summary(),
                            )
                        })
                        .unwrap_or_default();
                    r.set_hud(&g.device, &g.queue, &hud);
                    let mut action = None;
                    let entries = server_entries(&self.opts.s);
                    let rows = if self.menu.screen == menu::Screen::Online {
                        let b = self.browser.get_or_insert_with(browser::Browser::new);
                        b.update(&entries);
                        b.rows(&entries)
                    } else {
                        Vec::new()
                    };
                    let ui_out = match (&mut self.ui, &self.window) {
                        (Some(ui), Some(w)) if self.menu.is_open() => {
                            let online = matches!(self.session, Some(Session::Net(_)));
                            self.menu.spectating = match &self.session {
                                Some(Session::Net(n)) => Some(n.spectating()),
                                _ => None,
                            };
                            (self.menu.players, self.menu.invite) = match &self.session {
                                Some(Session::Net(n)) => (player_rows(n), n.incoming_challenge()),
                                _ => (Vec::new(), None),
                            };
                            let (menu, s) = (&mut self.menu, &mut self.opts.s);
                            Some(
                                ui.run_windowed(w, |ctx| action = menu.draw(ctx, s, online, &rows)),
                            )
                        }
                        _ => None,
                    };
                    let ui = &mut self.ui;
                    g.frame(|d, q, v| {
                        r.draw(d, q, v, depth, size, &cam, CLEAR);
                        if let (Some(ui), Some(out)) = (ui.as_mut(), ui_out) {
                            ui.paint(d, q, v, size, out);
                        }
                    });
                    if self.menu.dirty {
                        self.apply_settings();
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
    let opts = match parse_args(argv, base) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("lgduel: {e}");
            std::process::exit(2);
        }
    };
    if opts.headless && opts.connect.is_some() {
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
        std::thread::sleep(Duration::from_millis(200));
        println!("SOUND TEST played 5 hit sounds");
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
    let mut app = App {
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
    };
    if let Some(addr) = connect_to {
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
