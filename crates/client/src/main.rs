//! LGDuel client.
//!
//! `lgduel [--frames N] [--vsync] [--fps-cap N] [--size WxH]`
//! `lgduel --headless --screenshot out.png [--frames N] [--size WxH]`
//!
//! `--frames N` renders N frames, prints frame-time stats and exits (perf runs).
//! `--headless` renders offscreen (no window), saves a screenshot and checks
//! that it rendered (non-blank, expected colours present); exit code 1 if not.

mod audio;
mod effects;
mod font;
mod frame_stats;
mod gfx;
mod hud;
mod input;
mod local;
mod offscreen;
mod players;
mod render;

use effects::{BeamStyle, EffectSettings};
use frame_stats::FrameStats;
use gfx::Gfx;
use hud::{DamageNumbers, HudInfo, HudSettings};
use input::{Keys, MouseLook, MouseSettings};
use lgduel_bot::{AimSkill, Bot, BotConfig, StrafePattern};
use lgduel_sim::config::GameConfig;
use lgduel_sim::game::{GameEvent, PlayerId, hitscan};
use lgduel_sim::math::{Angle, Vec3, angle_vectors, look_angles};
use lgduel_sim::usercmd::{UserCmd, buttons};
use lgduel_sim::world::{World, builtin};
use local::LocalSession;
use players::EnemyColors;
use render::{Camera, Renderer};
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
    frames: Option<u64>,
    vsync: bool,
    fps_cap: Option<u32>,
    size: (u32, u32),
    headless: bool,
    screenshot: Option<String>,
    fov: f32,
    mouse: MouseSettings,
    /// Feed scripted input (hold forward+right, turn right) for unattended runs.
    synthetic_input: bool,
    effects: EffectSettings,
    /// 0 = muted.
    volume: f32,
    sound_test: bool,
    /// Offline opponent: None = idle target dummy.
    bot: Option<(StrafePattern, AimSkill)>,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            frames: None,
            vsync: false,
            fps_cap: None,
            size: (1280, 720),
            headless: false,
            screenshot: None,
            fov: 110.0,
            mouse: MouseSettings::default(),
            synthetic_input: false,
            effects: EffectSettings::default(),
            volume: 0.7,
            sound_test: false,
            bot: Some((
                StrafePattern::RandomAd {
                    min_ms: 150,
                    max_ms: 600,
                },
                AimSkill::MEDIUM,
            )),
        }
    }
}

fn parse_args(args: impl IntoIterator<Item = String>) -> Result<Options, String> {
    let mut o = Options::default();
    let mut it = args.into_iter();
    while let Some(a) = it.next() {
        let mut val = |name: &str| it.next().ok_or(format!("{name} needs a value"));
        let num = |name: &str, v: String| {
            v.parse::<u64>()
                .map_err(|_| format!("{name} needs a number"))
        };
        match a.as_str() {
            "--frames" => o.frames = Some(num("--frames", val("--frames")?)?),
            "--fps-cap" => o.fps_cap = Some(num("--fps-cap", val("--fps-cap")?)? as u32),
            "--fov" => {
                o.fov = val("--fov")?
                    .parse::<f32>()
                    .map_err(|_| "--fov needs a number".to_string())?
                    .clamp(10.0, 130.0)
            }
            "--size" => {
                let v = val("--size")?;
                let (w, h) = v.split_once('x').ok_or("--size needs WxH")?;
                o.size = (
                    w.parse().map_err(|_| "--size needs WxH")?,
                    h.parse().map_err(|_| "--size needs WxH")?,
                );
            }
            "--sens-cm" => {
                o.mouse.cm_per_360 = val("--sens-cm")?
                    .parse::<f64>()
                    .ok()
                    .filter(|v| *v > 0.0)
                    .ok_or("--sens-cm needs a positive number")?
            }
            "--dpi" => {
                o.mouse.dpi = val("--dpi")?
                    .parse::<f64>()
                    .ok()
                    .filter(|v| *v > 0.0)
                    .ok_or("--dpi needs a positive number")?
            }
            "--invert" => o.mouse.invert_pitch = true,
            "--beam" => {
                let v = val("--beam")?;
                o.effects.beam_style = BeamStyle::parse(&v)
                    .ok_or(format!("--beam must be none|thin|thick|zigzag, got {v}"))?
            }
            "--no-sparks" => o.effects.sparks = false,
            "--no-sound" => o.volume = 0.0,
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
                o.volume = val("--volume")?
                    .parse::<f32>()
                    .ok()
                    .filter(|v| (0.0..=1.0).contains(v))
                    .ok_or("--volume needs a number in 0..=1")?
            }
            "--spark-size" => {
                o.effects.spark_size = val("--spark-size")?
                    .parse::<f32>()
                    .ok()
                    .filter(|v| (0.0..=32.0).contains(v))
                    .ok_or("--spark-size needs a number in 0..=32")?
            }
            "--synthetic-input" => o.synthetic_input = true,
            "--vsync" => o.vsync = true,
            "--headless" => o.headless = true,
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

fn load_world() -> World {
    let p = GameConfig::default().physics;
    let hull = (
        Vec3::from_array(p.player_mins),
        Vec3::from_array(p.player_maxs),
    );
    World::from_toml_str(builtin::ARENA_FLAT, hull).expect("builtin map")
}

/// Beam from `eye` along (pitch, yaw), drawn from a muzzle offset; sparks where it hits.
#[allow(clippy::too_many_arguments)]
fn push_lg(
    v: &mut Vec<render::Vertex>,
    session: &LocalSession,
    shooter: PlayerId,
    eye: Vec3,
    pitch: Angle,
    yaw: Angle,
    view_eye: Vec3,
    fx: &EffectSettings,
    seed: u32,
) {
    let g = &session.game;
    let targets: Vec<_> = g
        .players
        .iter()
        .filter(|p| p.alive && p.id != shooter)
        .map(|p| {
            let (mins, maxs) = p.pm.hull(&g.cfg.physics);
            (
                p.id,
                lgduel_sim::world::Aabb::at(session.origin(p.id), mins, maxs),
            )
        })
        .collect();
    let (f, r, u) = angle_vectors(pitch, yaw);
    let hs = hitscan(&g.world, eye, f, g.cfg.lg.range, &targets);
    let (mf, mr, md) = effects::MUZZLE;
    let muzzle = eye + f * mf + r * mr - u * md;
    effects::push_beam(v, muzzle, hs.end, view_eye, fx, seed);
    if (hs.end - eye).length() < g.cfg.lg.range - 0.5 {
        effects::push_sparks(v, hs.end, f, view_eye, fx, seed);
    }
}

/// Dynamic geometry: opponents, their beams, and (if firing) the own beam from `cam`.
fn scene_mesh(
    session: &LocalSession,
    cam: &Camera,
    firing: bool,
    colors: &EnemyColors,
    fx: &EffectSettings,
    seed: u32,
) -> Vec<render::Vertex> {
    let mut v = Vec::new();
    let phys = &session.game.cfg.physics;
    for p in &session.game.players {
        if p.id == session.me || !p.alive {
            continue;
        }
        let origin = session.origin(p.id);
        players::push_player(&mut v, origin, p.yaw, p.pm.ducked, phys, colors);
        if p.buttons & buttons::FIRE != 0 {
            let eye = origin + (p.pm.eye(phys) - p.pm.origin);
            push_lg(
                &mut v, session, p.id, eye, p.pitch, p.yaw, cam.eye, fx, seed,
            );
        }
    }
    let me_alive = session.game.player(session.me).is_some_and(|p| p.alive);
    if firing && me_alive {
        push_lg(
            &mut v, session, session.me, cam.eye, cam.pitch, cam.yaw, cam.eye, fx, seed,
        );
    }
    v
}

/// HUD numbers for the local player.
fn hud_info(session: &LocalSession) -> HudInfo {
    let g = &session.game;
    let me = g.player(session.me).unwrap();
    HudInfo {
        health: me.health,
        accuracy_pct: me.stats.accuracy_pct(),
        damage_dealt: me.stats.damage_dealt,
        damage_taken: me.stats.damage_taken,
        wins: me.stats.fights_won,
        losses: me.stats.fights_lost,
        fight_secs: g
            .fight_start
            .map(|s| (g.tick.0 - s.0) as f32 / g.cfg.tick_rate_hz as f32),
        opponent_health: g
            .players
            .iter()
            .find(|p| p.id != session.me)
            .map(|p| p.health),
    }
}

/// Turn the last advance's own hits into floating damage numbers; returns hit count.
fn track_hits(session: &LocalSession, numbers: &mut DamageNumbers) -> usize {
    let mut last_end = None;
    let mut hits = 0;
    for e in &session.events {
        match e {
            GameEvent::Fire { shooter, end, .. } if *shooter == session.me => last_end = Some(*end),
            GameEvent::Damage {
                attacker, amount, ..
            } if *attacker == session.me => {
                if let Some(p) = last_end {
                    numbers.add(p, *amount);
                }
                hits += 1;
            }
            _ => {}
        }
    }
    hits
}

fn hud_mesh(
    session: &LocalSession,
    cam: &Camera,
    size: (u32, u32),
    numbers: &DamageNumbers,
    s: &HudSettings,
) -> Vec<render::Vertex> {
    let mut v = Vec::new();
    let screen = (size.0 as f32, size.1 as f32);
    let vp = cam.view_proj(screen.0 / screen.1.max(1.0));
    hud::build(&mut v, screen, &vp, &hud_info(session), numbers, s);
    v
}

fn run_headless(opts: &Options) -> Result<(), String> {
    let (w, h) = opts.size;
    let off = offscreen::Offscreen::new(w, h)?;
    println!(
        "GPU {} ({:?}) offscreen {w}x{h}",
        off.adapter_info.name, off.adapter_info.backend
    );
    let world = load_world();
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
        fov: opts.fov,
    };
    let colors = EnemyColors::default();
    let hud_settings = HudSettings::default();
    let mut numbers = DamageNumbers::default();
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
            total_hits += track_hits(&session, &mut numbers);
            numbers.update(VIRTUAL_FRAME_US as f32 / 1e6);
            let scene = scene_mesh(&session, &cam, true, &colors, &opts.effects, i);
            let hud = hud_mesh(&session, &cam, (w, h), &numbers, &hud_settings);
            (cam, scene, hud)
        },
        CLEAR,
    );
    println!("SIM hits {total_hits}, HUD {:?}", hud_info(&session));
    if let Some(s) = stats.summary() {
        println!("FRAMESTATS {s} | offscreen (GPU wait per frame)");
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
    for (name, c) in [
        ("head", colors.head),
        ("upper", colors.upper),
        ("lower", colors.lower),
    ] {
        let srgb = offscreen::linear_to_srgb8(c);
        let px_count = (offscreen::color_fraction(&px, srgb, 2) * total).round() as u64;
        println!("IMAGE enemy {name} {srgb:?}: {px_count} px");
        if px_count < 50 {
            return Err(format!(
                "forced enemy {name} colour not visible ({px_count} px)"
            ));
        }
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
    let fx = &opts.effects;
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

struct App {
    opts: Options,
    world: World,
    session: LocalSession,
    look: MouseLook,
    keys: Keys,
    grabbed: bool,
    colors: EnemyColors,
    audio: audio::Audio,
    hud: HudSettings,
    numbers: DamageNumbers,
    /// Offline opponent and its player id.
    bot: Option<(Bot, PlayerId)>,
    window: Option<Arc<Window>>,
    gfx: Option<Gfx>,
    renderer: Option<Renderer>,
    depth: Option<wgpu::TextureView>,
    stats: FrameStats,
    last_frame: Option<Instant>,
    frames: u64,
    error: Option<String>,
}

impl App {
    fn set_grab(&mut self, grab: bool) {
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

    fn key(&mut self, code: KeyCode, down: bool) {
        match code {
            KeyCode::KeyW => self.keys.forward = down,
            KeyCode::KeyS => self.keys.back = down,
            KeyCode::KeyA => self.keys.left = down,
            KeyCode::KeyD => self.keys.right = down,
            KeyCode::Space => self.keys.jump = down,
            KeyCode::ControlLeft | KeyCode::KeyC => self.keys.crouch = down,
            KeyCode::Escape if down => self.set_grab(false),
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
        let bot = &mut self.bot;
        self.session.advance(
            elapsed.as_micros() as u64,
            |t| keys.cmd(t, &look),
            |g, _| match bot {
                Some((b, id)) => vec![(*id, b.think(g, *id))],
                None => vec![],
            },
        );
        for _ in 0..track_hits(&self.session, &mut self.numbers) {
            self.audio.play_hit();
        }
        self.numbers.update(elapsed.as_secs_f32());
    }

    fn finish(&mut self, el: &ActiveEventLoop) {
        if let Some((_, id)) = &self.bot {
            let b = self.session.game.player(*id).unwrap();
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
        let p = self.session.game.player(self.session.me).unwrap();
        println!(
            "LOCAL tick {} origin ({:.1}, {:.1}, {:.1}) speed {:.1} yaw {:.2}",
            self.session.game.tick.0,
            p.pm.origin.x,
            p.pm.origin.y,
            p.pm.origin.z,
            p.pm.velocity.length_xy(),
            self.look.yaw.degrees()
        );
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
}

impl ApplicationHandler for App {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let attrs = Window::default_attributes()
            .with_title("LGDuel")
            .with_inner_size(winit::dpi::PhysicalSize::new(
                self.opts.size.0,
                self.opts.size.1,
            ));
        let window = match el.create_window(attrs) {
            Ok(w) => Arc::new(w),
            Err(e) => {
                self.error = Some(format!("create window: {e}"));
                el.exit();
                return;
            }
        };
        match Gfx::new(window.clone(), self.opts.vsync) {
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
                if let Some(cap) = self.opts.fps_cap
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
                let cam = Camera {
                    eye: self.session.eye(),
                    pitch: self.look.pitch,
                    yaw: self.look.yaw,
                    fov: self.opts.fov,
                };
                let dynamic = scene_mesh(
                    &self.session,
                    &cam,
                    self.keys.fire,
                    &self.colors,
                    &self.opts.effects,
                    self.frames as u32,
                );
                if let (Some(g), Some(r), Some(depth)) =
                    (&mut self.gfx, &mut self.renderer, &self.depth)
                {
                    let size = (g.config.width, g.config.height);
                    r.set_dynamic(&g.device, &g.queue, &dynamic);
                    let hud = hud_mesh(&self.session, &cam, size, &self.numbers, &self.hud);
                    r.set_hud(&g.device, &g.queue, &hud);
                    g.frame(|d, q, v| r.draw(d, q, v, depth, size, &cam, CLEAR));
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
    let opts = match parse_args(std::env::args().skip(1)) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("lgduel: {e}");
            std::process::exit(2);
        }
    };
    if opts.headless {
        if let Err(e) = run_headless(&opts) {
            eprintln!("lgduel: {e}");
            std::process::exit(1);
        }
        return;
    }
    let event_loop = EventLoop::new().expect("event loop");
    event_loop.set_control_flow(ControlFlow::Poll);
    let audio = if opts.volume > 0.0 {
        audio::Audio::open(opts.volume)
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
    let world = load_world();
    let session = LocalSession::new(GameConfig::default(), world.clone(), 1);
    let bot = opts.bot.map(|(pattern, aim)| {
        let id = session
            .game
            .players
            .iter()
            .find(|p| p.id != session.me)
            .unwrap()
            .id;
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
    let me = session.game.player(session.me).unwrap();
    let look = MouseLook::new(opts.mouse, me.yaw);
    println!(
        "MOUSE {} cm/360 @ {} dpi = {:.5} deg/count (raw input, sub-count accumulation)",
        opts.mouse.cm_per_360,
        opts.mouse.dpi,
        opts.mouse.degrees_per_count()
    );
    let mut app = App {
        world,
        session,
        look,
        keys: Keys::default(),
        grabbed: false,
        colors: EnemyColors::default(),
        audio,
        hud: HudSettings::default(),
        numbers: DamageNumbers::default(),
        bot,
        opts,
        window: None,
        gfx: None,
        renderer: None,
        depth: None,
        stats: FrameStats::default(),
        last_frame: None,
        frames: 0,
        error: None,
    };
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

    fn args(s: &str) -> Vec<String> {
        s.split_whitespace().map(String::from).collect()
    }

    #[test]
    fn parses_args() {
        assert_eq!(parse_args(args("")).unwrap(), Options::default());
        let o = parse_args(args(
            "--frames 500 --vsync --fps-cap 240 --size 1920x1080 --fov 300",
        ))
        .unwrap();
        assert_eq!(o.frames, Some(500));
        assert!(o.vsync);
        assert_eq!(o.fps_cap, Some(240));
        assert_eq!(o.size, (1920, 1080));
        assert_eq!(o.fov, 130.0);
        let o = parse_args(args("--headless --screenshot a.png")).unwrap();
        assert!(o.headless && o.screenshot.as_deref() == Some("a.png"));
        assert!(parse_args(args("--frames")).is_err());
        assert!(parse_args(args("--size 10")).is_err());
        assert!(parse_args(args("--bogus")).is_err());
    }
}
