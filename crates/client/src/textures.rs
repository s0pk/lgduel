//! Textures for the world renderer: fixed-size RGBA images collected into one
//! 2D texture array (layer 0 is always plain white, so an untextured surface is
//! drawn as exactly its vertex colour).

use std::collections::HashMap;
use std::path::{Component, Path};

/// Every texture is TEX x TEX pixels (user images are resized to this).
pub const TEX: u32 = 256;
/// Mip levels down to 1x1.
pub const MIPS: u32 = 9;
/// Most layers in the array (extra textures fall back to layer 0).
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

/// Most bytes of a texture file.
pub const MAX_FILE_BYTES: u64 = 4 * 1024 * 1024;
/// Largest accepted image side.
pub const MAX_SIDE: u32 = 2048;

/// Resize RGBA8 `w`x`h` to TEX x TEX: box filter when shrinking, nearest when growing.
pub fn resize_to_tex(rgba: &[u8], w: u32, h: u32) -> Image {
    let (w, h, t) = (w as usize, h as usize, TEX as usize);
    let mut out = vec![0u8; t * t * 4];
    for y in 0..t {
        let (y0, y1) = (y * h / t, (y * h / t + 1).max((y + 1) * h / t));
        for x in 0..t {
            let (x0, x1) = (x * w / t, (x * w / t + 1).max((x + 1) * w / t));
            let mut sum = [0u32; 4];
            for sy in y0..y1 {
                for sx in x0..x1 {
                    for (c, s) in sum.iter_mut().enumerate() {
                        *s += rgba[(sy * w + sx) * 4 + c] as u32;
                    }
                }
            }
            let n = ((y1 - y0) * (x1 - x0)) as u32;
            for c in 0..4 {
                out[(y * t + x) * 4 + c] = ((sum[c] + n / 2) / n) as u8;
            }
        }
    }
    Image { rgba: out }
}

/// Decode a PNG (any colour type, 8 or 16 bit) into a TEX x TEX image.
/// Rejects images wider or taller than `MAX_SIDE` before allocating.
pub fn decode_png(bytes: &[u8]) -> Result<Image, String> {
    let mut dec = png::Decoder::new(std::io::Cursor::new(bytes));
    dec.set_transformations(png::Transformations::normalize_to_color8());
    let mut r = dec
        .read_info()
        .map_err(|e| format!("not a readable PNG ({e})"))?;
    let (w, h) = (r.info().width, r.info().height);
    if w == 0 || h == 0 || w > MAX_SIDE || h > MAX_SIDE {
        return Err(format!(
            "{w}x{h} pixels (largest allowed {MAX_SIDE}x{MAX_SIDE})"
        ));
    }
    let mut buf = vec![0u8; r.output_buffer_size().ok_or("image too large")?];
    let frame = r
        .next_frame(&mut buf)
        .map_err(|e| format!("PNG data damaged ({e})"))?;
    let data = &buf[..frame.buffer_size()];
    let n = (w * h) as usize;
    let mut rgba = Vec::with_capacity(n * 4);
    match frame.color_type {
        png::ColorType::Grayscale => data
            .iter()
            .for_each(|g| rgba.extend_from_slice(&[*g, *g, *g, 255])),
        png::ColorType::GrayscaleAlpha => data
            .as_chunks::<2>()
            .0
            .iter()
            .for_each(|p| rgba.extend_from_slice(&[p[0], p[0], p[0], p[1]])),
        png::ColorType::Rgb => data
            .as_chunks::<3>()
            .0
            .iter()
            .for_each(|p| rgba.extend_from_slice(&[p[0], p[1], p[2], 255])),
        png::ColorType::Rgba => rgba.extend_from_slice(data),
        png::ColorType::Indexed => return Err("indexed PNG not expanded".into()),
    }
    if rgba.len() != n * 4 {
        return Err("PNG data has the wrong size".into());
    }
    Ok(resize_to_tex(&rgba, w, h))
}

/// 8-bit RGBA PNG bytes (the app icons).
pub fn encode_png_rgba(w: u32, h: u32, rgba: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut enc = png::Encoder::new(&mut out, w, h);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header()
        .and_then(|mut w| w.write_image_data(rgba))
        .expect("PNG encoding to memory");
    out
}

/// 8-bit RGB PNG bytes (used to write the example theme).
pub fn encode_png_rgb(w: u32, h: u32, rgb: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut enc = png::Encoder::new(&mut out, w, h);
    enc.set_color(png::ColorType::Rgb);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header()
        .and_then(|mut w| w.write_image_data(rgb))
        .expect("PNG encoding to memory");
    out
}

/// A texture file name is relative, inside the theme folder (no `..`, no drive or root).
pub fn safe_relative(name: &str) -> bool {
    let p = Path::new(name);
    // ':' and '\' are rejected outright so the answer is the same on every OS.
    !name.is_empty()
        && !name.contains(':')
        && !name.contains('\\')
        && !p.is_absolute()
        && p.components().all(|c| matches!(c, Component::Normal(_)))
}

/// Load `name` (a PNG) from the theme folder `dir`.
pub fn load_png_file(dir: &Path, name: &str) -> Result<Image, String> {
    if !safe_relative(name) {
        return Err("file names must be relative paths inside the theme folder".into());
    }
    let path = dir.join(name);
    let meta = std::fs::metadata(&path).map_err(|_| "not found".to_string())?;
    if meta.len() > MAX_FILE_BYTES {
        return Err(format!(
            "file is {} KB (largest allowed {} KB)",
            meta.len() / 1024,
            MAX_FILE_BYTES / 1024
        ));
    }
    let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
    decode_png(&bytes)
}

pub struct GpuTextures {
    pub view: wgpu::TextureView,
    pub sampler: wgpu::Sampler,
}

/// Upload every layer with its mips.
pub fn upload(set: &TextureSet, device: &wgpu::Device, queue: &wgpu::Queue) -> GpuTextures {
    // At least two layers: with one, OpenGL makes a plain 2D texture and an array
    // view of it samples black. The spare layer is never used.
    let layers = set.layers.len().max(2) as u32;
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

    fn encode(w: u32, h: u32, color: png::ColorType, data: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut enc = png::Encoder::new(&mut out, w, h);
        enc.set_color(color);
        enc.set_depth(png::BitDepth::Eight);
        enc.write_header().unwrap().write_image_data(data).unwrap();
        out
    }

    #[test]
    fn png_decode_all_colour_types_and_sizes() {
        // 2x2 RGB: red, green / blue, white → nearest upscale keeps each quadrant's colour.
        let rgb = encode(
            2,
            2,
            png::ColorType::Rgb,
            &[255, 0, 0, 0, 255, 0, 0, 0, 255, 255, 255, 255],
        );
        let img = decode_png(&rgb).unwrap();
        assert_eq!(img.rgba.len(), (TEX * TEX * 4) as usize);
        let px = |x: usize, y: usize| &img.rgba[(y * TEX as usize + x) * 4..][..4];
        assert_eq!(px(0, 0), [255, 0, 0, 255]);
        assert_eq!(px(255, 0), [0, 255, 0, 255]);
        assert_eq!(px(0, 255), [0, 0, 255, 255]);
        assert_eq!(px(255, 255), [255, 255, 255, 255]);
        // Grey and grey+alpha and RGBA.
        let g = decode_png(&encode(1, 1, png::ColorType::Grayscale, &[77])).unwrap();
        assert_eq!(&g.rgba[..4], [77, 77, 77, 255]);
        let ga = decode_png(&encode(1, 1, png::ColorType::GrayscaleAlpha, &[50, 200])).unwrap();
        assert_eq!(&ga.rgba[..4], [50, 50, 50, 200]);
        let rgba = decode_png(&encode(1, 1, png::ColorType::Rgba, &[1, 2, 3, 4])).unwrap();
        assert_eq!(&rgba.rgba[..4], [1, 2, 3, 4]);
        // Shrinking averages: 512x256 half black / half white → left black, right white.
        let mut big = vec![0u8; 512 * 256 * 3];
        for y in 0..256 {
            for x in 256..512 {
                big[(y * 512 + x) * 3..][..3].copy_from_slice(&[255, 255, 255]);
            }
        }
        let half = decode_png(&encode(512, 256, png::ColorType::Rgb, &big)).unwrap();
        assert_eq!(&half.rgba[10 * 4..][..3], [0, 0, 0]);
        assert_eq!(&half.rgba[(TEX as usize - 10) * 4..][..3], [255, 255, 255]);
    }

    #[test]
    fn png_limits_and_garbage() {
        let huge = encode(
            MAX_SIDE + 1,
            1,
            png::ColorType::Grayscale,
            &vec![0; MAX_SIDE as usize + 1],
        );
        assert!(decode_png(&huge).unwrap_err().contains("largest allowed"));
        assert!(
            decode_png(b"not a png at all")
                .unwrap_err()
                .contains("not a readable PNG")
        );
        let mut cut = encode(8, 8, png::ColorType::Rgb, &[7u8; 8 * 8 * 3]);
        cut.truncate(cut.len() - 20);
        assert!(decode_png(&cut).is_err());
    }

    #[test]
    fn only_relative_paths_inside_the_folder() {
        for ok in ["floor.png", "tiles/floor.png"] {
            assert!(safe_relative(ok), "{ok}");
        }
        for bad in [
            "",
            "../x.png",
            "a/../../x.png",
            "/etc/x.png",
            "C:\\x.png",
            "C:/x.png",
            "a\\..\\x.png",
            "./x.png",
        ] {
            assert!(!safe_relative(bad), "{bad}");
        }
        let dir = std::env::temp_dir();
        assert!(
            load_png_file(&dir, "../x.png")
                .unwrap_err()
                .contains("relative")
        );
        assert_eq!(
            load_png_file(&dir, "definitely-missing.png").unwrap_err(),
            "not found"
        );
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
