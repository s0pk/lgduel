//! Textures for the world renderer: fixed-size RGBA images collected into one
//! 2D texture array (layer 0 is always plain white, so an untextured surface is
//! drawn as exactly its vertex colour).

use std::collections::HashMap;

/// Every texture is TEX x TEX pixels (user images are resized to this).
pub const TEX: u32 = 256;
/// Mip levels down to 1x1.
pub const MIPS: u32 = 9;
/// Most layers in the array (extra textures fall back to layer 0).
#[allow(dead_code)] // used by themes (next milestone)
pub const MAX_LAYERS: usize = 64;

#[derive(Debug, Clone, PartialEq)]
pub struct Image {
    /// TEX*TEX*4 bytes, sRGB RGBA8.
    pub rgba: Vec<u8>,
}

impl Image {
    pub fn solid(c: [u8; 4]) -> Image {
        Image {
            rgba: c
                .iter()
                .copied()
                .cycle()
                .take((TEX * TEX * 4) as usize)
                .collect(),
        }
    }
}

pub fn white() -> Image {
    Image::solid([255, 255, 255, 255])
}

/// Mip chain, level 0 first: each level is a 2x2 box filter of the previous one.
pub fn mip_chain(img: &Image) -> Vec<Vec<u8>> {
    let mut levels = vec![img.rgba.clone()];
    let mut size = TEX as usize;
    while size > 1 {
        let prev = levels.last().unwrap();
        let next_size = size / 2;
        let mut next = vec![0u8; next_size * next_size * 4];
        for y in 0..next_size {
            for x in 0..next_size {
                for c in 0..4 {
                    let at = |dx: usize, dy: usize| {
                        prev[((y * 2 + dy) * size + x * 2 + dx) * 4 + c] as u32
                    };
                    next[(y * next_size + x) * 4 + c] =
                        ((at(0, 0) + at(1, 0) + at(0, 1) + at(1, 1) + 2) / 4) as u8;
                }
            }
        }
        levels.push(next);
        size = next_size;
    }
    levels
}

/// The images that go into the GPU array, by layer.
#[derive(Debug, Clone, PartialEq)]
pub struct TextureSet {
    pub layers: Vec<Image>,
    pub index: HashMap<String, u32>,
}

impl Default for TextureSet {
    fn default() -> Self {
        Self::new()
    }
}

impl TextureSet {
    /// Layer 0 = white.
    pub fn new() -> Self {
        Self {
            layers: vec![white()],
            index: HashMap::new(),
        }
    }

    /// Layer for `name`, adding `img` the first time. Past `MAX_LAYERS`: layer 0.
    #[allow(dead_code)] // used by themes (next milestone)
    pub fn add(&mut self, name: &str, img: Image) -> u32 {
        if let Some(i) = self.index.get(name) {
            return *i;
        }
        if self.layers.len() >= MAX_LAYERS {
            return 0;
        }
        let i = self.layers.len() as u32;
        self.layers.push(img);
        self.index.insert(name.to_string(), i);
        i
    }
}

pub struct GpuTextures {
    pub view: wgpu::TextureView,
    pub sampler: wgpu::Sampler,
}

/// Upload every layer with its mips.
pub fn upload(set: &TextureSet, device: &wgpu::Device, queue: &wgpu::Queue) -> GpuTextures {
    let layers = set.layers.len() as u32;
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("world textures"),
        size: wgpu::Extent3d {
            width: TEX,
            height: TEX,
            depth_or_array_layers: layers,
        },
        mip_level_count: MIPS,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    for (layer, img) in set.layers.iter().enumerate() {
        for (mip, data) in mip_chain(img).iter().enumerate() {
            let size = (TEX >> mip).max(1);
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &texture,
                    mip_level: mip as u32,
                    origin: wgpu::Origin3d {
                        x: 0,
                        y: 0,
                        z: layer as u32,
                    },
                    aspect: wgpu::TextureAspect::All,
                },
                data,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(4 * size),
                    rows_per_image: Some(size),
                },
                wgpu::Extent3d {
                    width: size,
                    height: size,
                    depth_or_array_layers: 1,
                },
            );
        }
    }
    let view = texture.create_view(&wgpu::TextureViewDescriptor {
        dimension: Some(wgpu::TextureViewDimension::D2Array),
        ..Default::default()
    });
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("world sampler"),
        address_mode_u: wgpu::AddressMode::Repeat,
        address_mode_v: wgpu::AddressMode::Repeat,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        mipmap_filter: wgpu::MipmapFilterMode::Linear,
        ..Default::default()
    });
    GpuTextures { view, sampler }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn white_layer_zero_and_dedupe() {
        let mut s = TextureSet::new();
        assert_eq!(s.layers.len(), 1);
        assert!(s.layers[0].rgba.iter().all(|b| *b == 255));
        let a = s.add("a", Image::solid([10, 20, 30, 255]));
        assert_eq!(a, 1);
        assert_eq!(
            s.add("a", Image::solid([1, 1, 1, 255])),
            1,
            "same name, same layer"
        );
        assert_eq!(s.layers.len(), 2);
        for i in 0..100 {
            s.add(&format!("t{i}"), white());
        }
        assert_eq!(s.layers.len(), MAX_LAYERS);
        assert_eq!(s.add("overflow", white()), 0);
    }

    #[test]
    fn mip_chain_sizes_and_averages() {
        let mut img = white();
        // Left half black, right half white: the 1x1 mip is mid grey.
        for y in 0..TEX as usize {
            for x in 0..TEX as usize / 2 {
                let i = (y * TEX as usize + x) * 4;
                img.rgba[i..i + 3].copy_from_slice(&[0, 0, 0]);
            }
        }
        let m = mip_chain(&img);
        assert_eq!(m.len(), MIPS as usize);
        for (i, level) in m.iter().enumerate() {
            let s = (TEX >> i) as usize;
            assert_eq!(level.len(), s * s * 4);
        }
        let last = &m[MIPS as usize - 1];
        assert!((126..=129).contains(&last[0]), "{}", last[0]);
        assert_eq!(last[3], 255);
        // A white image stays white at every level.
        assert!(
            mip_chain(&white())
                .iter()
                .all(|l| l.iter().all(|b| *b == 255))
        );
    }
}
