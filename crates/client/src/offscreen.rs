//! Headless rendering: draw the scene into an offscreen texture (no window),
//! read it back, save a PNG and check the pixels. Used by check.sh.

use crate::frame_stats::FrameStats;
use crate::render::{Camera, Renderer, Vertex, depth_texture};
use std::path::Path;
use std::time::Instant;

pub const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

pub struct Offscreen {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub adapter_info: wgpu::AdapterInfo,
    pub size: (u32, u32),
    color: wgpu::Texture,
    color_view: wgpu::TextureView,
    depth_view: wgpu::TextureView,
}

impl Offscreen {
    pub fn new(width: u32, height: u32) -> Result<Self, String> {
        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            ..Default::default()
        }))
        .or_else(|_| {
            // No hardware adapter (e.g. CI): fall back to a software one.
            pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
                force_fallback_adapter: true,
                ..Default::default()
            }))
        })
        .map_err(|e| format!("no adapter: {e}"))?;
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("offscreen"),
            ..Default::default()
        }))
        .map_err(|e| format!("request device: {e}"))?;
        let color = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("offscreen color"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let color_view = color.create_view(&Default::default());
        let depth_view = depth_texture(&device, width, height);
        Ok(Self {
            device,
            queue,
            adapter_info: adapter.get_info(),
            size: (width, height),
            color,
            color_view,
            depth_view,
        })
    }

    /// Render `frames` frames (timed, waiting for the GPU each frame), return stats.
    pub fn render(
        &self,
        r: &mut Renderer,
        frames: u32,
        mut scene: impl FnMut(u32) -> (Camera, Vec<Vertex>),
        clear: wgpu::Color,
    ) -> FrameStats {
        let mut stats = FrameStats::default();
        for i in 0..frames {
            let t = Instant::now();
            let (cam, dynamic) = scene(i);
            r.set_dynamic(&self.device, &self.queue, &dynamic);
            r.draw(
                &self.device,
                &self.queue,
                &self.color_view,
                &self.depth_view,
                self.size,
                &cam,
                clear,
            );
            let _ = self.device.poll(wgpu::PollType::wait_indefinitely());
            stats.record(t.elapsed());
        }
        stats
    }

    /// Read back the colour target as tightly packed RGBA8 (sRGB-encoded).
    pub fn read_pixels(&self) -> Vec<u8> {
        let (w, h) = self.size;
        let unpadded = w * 4;
        let padded = unpadded.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
            * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let buf = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("readback"),
            size: (padded * h) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut enc = self.device.create_command_encoder(&Default::default());
        enc.copy_texture_to_buffer(
            self.color.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buf,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded),
                    rows_per_image: Some(h),
                },
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
        self.queue.submit([enc.finish()]);
        let slice = buf.slice(..);
        slice.map_async(wgpu::MapMode::Read, |r| r.expect("map readback"));
        let _ = self.device.poll(wgpu::PollType::wait_indefinitely());
        let data = slice.get_mapped_range().expect("mapped readback");
        let mut out = Vec::with_capacity((unpadded * h) as usize);
        for row in 0..h {
            let s = (row * padded) as usize;
            out.extend_from_slice(&data[s..s + unpadded as usize]);
        }
        out
    }
}

pub fn save_png(path: &Path, w: u32, h: u32, rgba: &[u8]) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let file = std::fs::File::create(path).map_err(|e| e.to_string())?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w, h);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut wr = enc.write_header().map_err(|e| e.to_string())?;
    wr.write_image_data(rgba).map_err(|e| e.to_string())
}

/// Linear [0,1] colour → 8-bit sRGB, as the GPU writes it into an sRGB target.
pub fn linear_to_srgb8(c: [f32; 3]) -> [u8; 3] {
    let f = |v: f32| {
        let v = v.clamp(0.0, 1.0);
        let s = if v <= 0.003_130_8 {
            v * 12.92
        } else {
            1.055 * v.powf(1.0 / 2.4) - 0.055
        };
        (s * 255.0).round() as u8
    };
    [f(c[0]), f(c[1]), f(c[2])]
}

/// Fraction of pixels within `tol` (per channel) of `rgb`.
pub fn color_fraction(rgba: &[u8], rgb: [u8; 3], tol: u8) -> f64 {
    let n = rgba.len() / 4;
    let hits = rgba
        .as_chunks::<4>()
        .0
        .iter()
        .filter(|p| (0..3).all(|i| p[i].abs_diff(rgb[i]) <= tol))
        .count();
    hits as f64 / n.max(1) as f64
}

/// Number of distinct colours (cheap "is it blank?" check).
pub fn distinct_colors(rgba: &[u8]) -> usize {
    let mut v: Vec<u32> = rgba
        .as_chunks::<4>()
        .0
        .iter()
        .map(|p| u32::from_le_bytes([p[0], p[1], p[2], 0]))
        .collect();
    v.sort_unstable();
    v.dedup();
    v.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn srgb_encoding() {
        assert_eq!(linear_to_srgb8([0.0, 1.0, 0.5]), [0, 255, 188]);
    }

    #[test]
    fn pixel_checks() {
        let img = [
            10, 20, 30, 255, 10, 20, 30, 255, 200, 0, 0, 255, 12, 21, 30, 255,
        ];
        assert_eq!(color_fraction(&img, [10, 20, 30], 0), 0.5);
        assert_eq!(color_fraction(&img, [10, 20, 30], 2), 0.75);
        assert_eq!(distinct_colors(&img), 3);
    }
}
