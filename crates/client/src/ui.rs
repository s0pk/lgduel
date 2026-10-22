//! egui integration: input from winit, painting on top of the frame with wgpu.

use egui_wgpu::{Renderer as EguiRenderer, RendererOptions, ScreenDescriptor};
use winit::window::Window;

pub struct Ui {
    pub ctx: egui::Context,
    renderer: EguiRenderer,
    /// Window input state (None when rendering headless).
    winit: Option<egui_winit::State>,
    /// Headless clock (seconds), advanced 0.1 s per frame so animations settle.
    headless_time: f64,
    /// Texture updates from frames that were run but not painted (e.g. the
    /// font atlas from a warm-up frame); applied at the next paint.
    pending_textures: egui::TexturesDelta,
}

impl Ui {
    pub fn new(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        window: Option<&Window>,
    ) -> Self {
        let ctx = egui::Context::default();
        ctx.set_visuals(egui::Visuals::dark());
        crate::ttf::install(&ctx);
        let winit = window.map(|w| {
            egui_winit::State::new(
                ctx.clone(),
                egui::ViewportId::ROOT,
                w,
                Some(w.scale_factor() as f32),
                None,
                None,
            )
        });
        Self {
            ctx,
            renderer: EguiRenderer::new(device, format, RendererOptions::default()),
            winit,
            headless_time: 0.0,
            pending_textures: egui::TexturesDelta::default(),
        }
    }

    /// Feed a window event; true if egui wants it (the game should ignore it).
    pub fn on_window_event(&mut self, window: &Window, event: &winit::event::WindowEvent) -> bool {
        self.winit
            .as_mut()
            .is_some_and(|s| s.on_window_event(window, event).consumed)
    }

    /// Run one UI frame from window input.
    pub fn run_windowed(
        &mut self,
        window: &Window,
        build: impl FnMut(&egui::Context),
    ) -> egui::FullOutput {
        let raw = self
            .winit
            .as_mut()
            .expect("windowed ui")
            .take_egui_input(window);
        let out = self.ctx.run_pass(raw, build);
        if let Some(s) = self.winit.as_mut() {
            s.handle_platform_output(window, out.platform_output.clone());
        }
        out
    }

    /// Run one UI frame with no input (headless rendering).
    pub fn run_headless(
        &mut self,
        size: (u32, u32),
        build: impl FnMut(&egui::Context),
    ) -> egui::FullOutput {
        self.headless_time += 0.1;
        let raw = egui::RawInput {
            time: Some(self.headless_time),
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(size.0 as f32, size.1 as f32),
            )),
            ..Default::default()
        };
        self.ctx.run_pass(raw, build)
    }

    /// Discard a frame's drawing but keep its texture updates for the next paint.
    pub fn skip(&mut self, out: egui::FullOutput) {
        self.pending_textures.append(out.textures_delta);
    }

    /// Paint a UI frame onto `view` (loading what is already there).
    pub fn paint(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        view: &wgpu::TextureView,
        size: (u32, u32),
        out: egui::FullOutput,
    ) {
        let ppp = out.pixels_per_point;
        let jobs = self.ctx.tessellate(out.shapes, ppp);
        let screen = ScreenDescriptor {
            size_in_pixels: [size.0, size.1],
            pixels_per_point: ppp,
        };
        let mut textures = std::mem::take(&mut self.pending_textures);
        textures.append(out.textures_delta);
        for (id, deltas) in &textures.set {
            for delta in deltas {
                self.renderer.update_texture(device, queue, *id, delta);
            }
        }
        let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("egui"),
        });
        let extra = self
            .renderer
            .update_buffers(device, queue, &mut enc, &jobs, &screen);
        {
            let pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("egui"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            let mut pass = pass.forget_lifetime();
            self.renderer.render(&mut pass, &jobs, &screen);
        }
        queue.submit(extra.into_iter().chain([enc.finish()]));
        for id in &textures.free {
            self.renderer.free_texture(id);
        }
    }
}
