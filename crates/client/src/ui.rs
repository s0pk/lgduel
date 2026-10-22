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
    accent: [f32; 3],
}

/// Fill colour of menu windows. The headless menu check counts pixels of exactly
/// this colour, so the style and the check share the constant.
pub const MENU_PANEL: [u8; 3] = [27, 27, 27];

/// The menu look: dark panels, rounded corners, roomy buttons, one accent colour
/// (`accent`, linear 0..1) for selections and hover.
pub fn apply_style(ctx: &egui::Context, accent: [f32; 3]) {
    let to8 = |c: f32| (c.clamp(0.0, 1.0) * 255.0).round() as u8;
    let a = egui::Color32::from_rgb(to8(accent[0]), to8(accent[1]), to8(accent[2]));
    let dim = |c: egui::Color32, k: f32| {
        egui::Color32::from_rgb(
            (c.r() as f32 * k) as u8,
            (c.g() as f32 * k) as u8,
            (c.b() as f32 * k) as u8,
        )
    };
    let mut v = egui::Visuals::dark();
    v.window_fill = egui::Color32::from_rgb(MENU_PANEL[0], MENU_PANEL[1], MENU_PANEL[2]);
    v.panel_fill = v.window_fill;
    v.selection.bg_fill = dim(a, 0.55);
    v.selection.stroke = egui::Stroke::new(1.0, a);
    v.hyperlink_color = a;
    v.widgets.hovered.bg_stroke = egui::Stroke::new(1.0, a);
    v.widgets.hovered.weak_bg_fill = dim(a, 0.30);
    v.widgets.active.weak_bg_fill = dim(a, 0.45);
    let r = egui::CornerRadius::same(6);
    v.window_corner_radius = r;
    v.widgets.noninteractive.corner_radius = r;
    v.widgets.inactive.corner_radius = r;
    v.widgets.hovered.corner_radius = r;
    v.widgets.active.corner_radius = r;
    ctx.set_theme(egui::Theme::Dark);
    ctx.set_visuals_of(egui::Theme::Dark, v);
    ctx.style_mut_of(egui::Theme::Dark, |s| {
        s.spacing.button_padding = egui::vec2(10.0, 5.0);
        s.spacing.item_spacing = egui::vec2(8.0, 7.0);
    });
}

impl Ui {
    /// Change the accent colour (re-styles only when it changed).
    pub fn set_accent(&mut self, accent: [f32; 3]) {
        if self.accent != accent {
            self.accent = accent;
            apply_style(&self.ctx, accent);
        }
    }

    pub fn new(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        window: Option<&Window>,
    ) -> Self {
        let ctx = egui::Context::default();
        let accent = crate::settings::UiSettings::default().accent;
        apply_style(&ctx, accent);
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
            accent,
        }
    }

    /// Menu size multiplier (egui zoom factor).
    pub fn set_ui_scale(&mut self, scale: f32) {
        if (self.ctx.zoom_factor() - scale).abs() > 1e-3 {
            self.ctx.set_zoom_factor(scale);
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
