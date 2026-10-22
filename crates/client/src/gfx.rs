//! GPU setup and presentation (wgpu).

use std::sync::Arc;
use winit::window::Window;

pub struct Gfx {
    pub surface: wgpu::Surface<'static>,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub config: wgpu::SurfaceConfiguration,
    pub adapter_info: wgpu::AdapterInfo,
}

/// Lowest-latency present mode available. Vsync off: Immediate (tearing, lowest
/// latency), else Mailbox, else Fifo. Vsync on: Fifo.
pub fn pick_present_mode(supported: &[wgpu::PresentMode], vsync: bool) -> wgpu::PresentMode {
    use wgpu::PresentMode::*;
    let prefs: &[wgpu::PresentMode] = if vsync {
        &[Fifo]
    } else {
        &[Immediate, Mailbox, Fifo]
    };
    prefs
        .iter()
        .copied()
        .find(|m| supported.contains(m))
        .unwrap_or(Fifo)
}

impl Gfx {
    pub fn new(window: Arc<Window>, vsync: bool) -> Result<Self, String> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let surface = instance
            .create_surface(window.clone())
            .map_err(|e| format!("create surface: {e}"))?;
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: Some(&surface),
            ..Default::default()
        }))
        .map_err(|e| format!("no GPU adapter: {e}"))?;
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("lgduel"),
            ..Default::default()
        }))
        .map_err(|e| format!("request device: {e}"))?;
        let caps = surface.get_capabilities(&adapter);
        let format = caps
            .formats
            .iter()
            .copied()
            .find(|f| f.is_srgb())
            .unwrap_or(caps.formats[0]);
        let size = window.inner_size();
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode: pick_present_mode(&caps.present_modes, vsync),
            desired_maximum_frame_latency: 1,
            alpha_mode: caps.alpha_modes[0],
            view_formats: vec![],
            color_space: wgpu::SurfaceColorSpace::Auto,
        };
        surface.configure(&device, &config);
        Ok(Self {
            surface,
            device,
            queue,
            config,
            adapter_info: adapter.get_info(),
        })
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        self.config.width = width;
        self.config.height = height;
        self.surface.configure(&self.device, &self.config);
    }

    /// Acquire, draw via `draw`, present. Returns false if the frame was skipped.
    pub fn frame(
        &mut self,
        draw: impl FnOnce(&wgpu::Device, &wgpu::Queue, &wgpu::TextureView),
    ) -> bool {
        let tex = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t) => t,
            wgpu::CurrentSurfaceTexture::Suboptimal(t) => {
                self.surface.configure(&self.device, &self.config);
                t
            }
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                self.surface.configure(&self.device, &self.config);
                return false;
            }
            _ => return false,
        };
        let view = tex
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        draw(&self.device, &self.queue, &view);
        self.queue.present(tex);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wgpu::PresentMode::*;

    #[test]
    fn present_mode_preference() {
        assert_eq!(
            pick_present_mode(&[Fifo, Mailbox, Immediate], false),
            Immediate
        );
        assert_eq!(pick_present_mode(&[Fifo, Mailbox], false), Mailbox);
        assert_eq!(pick_present_mode(&[Fifo], false), Fifo);
        assert_eq!(pick_present_mode(&[Fifo, Immediate], true), Fifo);
    }
}
