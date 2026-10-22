//! LGDuel client.
//!
//! `lgduel [--frames N] [--vsync] [--fps-cap N] [--size WxH]`
//! `lgduel --headless --screenshot out.png [--frames N] [--size WxH]`
//!
//! `--frames N` renders N frames, prints frame-time stats and exits (perf runs).
//! `--headless` renders offscreen (no window), saves a screenshot and checks
//! that it rendered (non-blank, expected colours present); exit code 1 if not.

mod frame_stats;
mod gfx;
mod offscreen;
mod render;

use frame_stats::FrameStats;
use gfx::Gfx;
use lgduel_sim::config::GameConfig;
use lgduel_sim::math::{Vec3, look_angles};
use lgduel_sim::world::{World, builtin};
use render::{Camera, Renderer};
use std::sync::Arc;
use std::time::{Duration, Instant};
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
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

/// Camera at the first spawn's eye, looking at the second spawn.
fn spawn_camera(w: &World, fov: f32) -> Camera {
    let eye = w.spawns[0].origin + Vec3::new(0.0, 0.0, GameConfig::default().physics.view_height);
    let (pitch, yaw) = look_angles(w.spawns[1].origin - eye);
    Camera {
        eye,
        pitch,
        yaw,
        fov,
    }
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
    let cam = spawn_camera(&world, opts.fov);
    let frames = opts.frames.unwrap_or(120) as u32;
    let stats = off.render(&mut r, frames, |_| (cam, vec![]), CLEAR);
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
    fn finish(&mut self, el: &ActiveEventLoop) {
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
    }

    fn window_event(&mut self, el: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => self.finish(el),
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
                }
                self.last_frame = Some(now);
                let cam = spawn_camera(&self.world, self.opts.fov);
                if let (Some(g), Some(r), Some(depth)) =
                    (&mut self.gfx, &self.renderer, &self.depth)
                {
                    let size = (g.config.width, g.config.height);
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
    let mut app = App {
        world: load_world(),
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
