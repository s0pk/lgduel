//! LGDuel client.
//!
//! `lgduel [--frames N] [--vsync] [--fps-cap N]`
//! `--frames N` renders N frames, prints frame-time stats and exits (perf runs).

mod frame_stats;
mod gfx;

use frame_stats::FrameStats;
use gfx::Gfx;
use std::sync::Arc;
use std::time::{Duration, Instant};
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::{Window, WindowId};

#[derive(Debug, Clone, Default, PartialEq)]
struct Options {
    frames: Option<u64>,
    vsync: bool,
    fps_cap: Option<u32>,
}

fn parse_args(args: impl IntoIterator<Item = String>) -> Result<Options, String> {
    let mut o = Options::default();
    let mut it = args.into_iter();
    while let Some(a) = it.next() {
        let mut num = |name: &str| -> Result<u64, String> {
            it.next()
                .and_then(|v| v.parse().ok())
                .ok_or(format!("{name} needs a number"))
        };
        match a.as_str() {
            "--frames" => o.frames = Some(num("--frames")?),
            "--fps-cap" => o.fps_cap = Some(num("--fps-cap")? as u32),
            "--vsync" => o.vsync = true,
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

struct App {
    opts: Options,
    window: Option<Arc<Window>>,
    gfx: Option<Gfx>,
    stats: FrameStats,
    last_frame: Option<Instant>,
    frames: u64,
    error: Option<String>,
}

impl App {
    fn finish(&mut self, el: &ActiveEventLoop) {
        if let Some(s) = self.stats.summary() {
            let present = self
                .gfx
                .as_ref()
                .map(|g| format!("{:?}", g.config.present_mode))
                .unwrap_or_default();
            println!("FRAMESTATS {s} | present mode {present}");
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
            .with_inner_size(winit::dpi::PhysicalSize::new(1280, 720));
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
                }
            }
            WindowEvent::RedrawRequested => {
                let now = Instant::now();
                if let Some(cap) = self.opts.fps_cap
                    && let Some(last) = self.last_frame
                {
                    let target = Duration::from_secs_f64(1.0 / cap.max(1) as f64);
                    let elapsed = now - last;
                    if elapsed < target {
                        std::thread::sleep(target - elapsed);
                    }
                }
                let now = Instant::now();
                if let Some(last) = self.last_frame {
                    self.stats.record(now - last);
                }
                self.last_frame = Some(now);
                if let Some(g) = &mut self.gfx {
                    g.frame(|d, q, v| {
                        gfx::clear(
                            d,
                            q,
                            v,
                            wgpu::Color {
                                r: 0.05,
                                g: 0.06,
                                b: 0.08,
                                a: 1.0,
                            },
                        )
                    });
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
    let event_loop = EventLoop::new().expect("event loop");
    event_loop.set_control_flow(ControlFlow::Poll);
    let mut app = App {
        opts,
        window: None,
        gfx: None,
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
        let o = parse_args(args("--frames 500 --vsync --fps-cap 240")).unwrap();
        assert_eq!(
            o,
            Options {
                frames: Some(500),
                vsync: true,
                fps_cap: Some(240)
            }
        );
        assert!(parse_args(args("--frames")).is_err());
        assert!(parse_args(args("--bogus")).is_err());
    }
}
