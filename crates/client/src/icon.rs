//! The app icon, drawn in code (original artwork): a dark rounded square with a
//! lightning bolt in the beam blue and a small crosshair-pink dot. Pure
//! rasterisation with 4x4 supersampling, so every platform produces the same bytes.

/// Beam blue (the default beam colour) and crosshair pink, 0..255.
const BOLT: [f32; 3] = [140.0, 204.0, 255.0];
const DOT: [f32; 3] = [255.0, 51.0, 153.0];
const BODY: [f32; 3] = [24.0, 26.0, 34.0];

/// The bolt as a polygon in 0..1 icon coordinates (y down).
const BOLT_POLY: [(f32, f32); 6] = [
    (0.60, 0.08),
    (0.28, 0.54),
    (0.46, 0.54),
    (0.38, 0.93),
    (0.74, 0.43),
    (0.55, 0.43),
];

fn inside_polygon(p: (f32, f32), poly: &[(f32, f32)]) -> bool {
    let mut inside = false;
    let mut j = poly.len() - 1;
    for i in 0..poly.len() {
        let (a, b) = (poly[i], poly[j]);
        if (a.1 > p.1) != (b.1 > p.1) && p.0 < (b.0 - a.0) * (p.1 - a.1) / (b.1 - a.1) + a.0 {
            inside = !inside;
        }
        j = i;
    }
    inside
}

fn inside_rounded_square(p: (f32, f32), radius: f32) -> bool {
    let (x, y) = (p.0.clamp(0.0, 1.0), p.1.clamp(0.0, 1.0));
    if !(0.0..=1.0).contains(&p.0) || !(0.0..=1.0).contains(&p.1) {
        return false;
    }
    let cx = x.clamp(radius, 1.0 - radius);
    let cy = y.clamp(radius, 1.0 - radius);
    (x - cx).powi(2) + (y - cy).powi(2) <= radius * radius
}

/// RGBA8 (straight alpha), `size` x `size`.
pub fn icon_rgba(size: u32) -> Vec<u8> {
    const SS: u32 = 4;
    let mut out = Vec::with_capacity((size * size * 4) as usize);
    for py in 0..size {
        for px in 0..size {
            let (mut r, mut g, mut b, mut covered) = (0.0f32, 0.0f32, 0.0f32, 0u32);
            for sy in 0..SS {
                for sx in 0..SS {
                    let u = (px as f32 + (sx as f32 + 0.5) / SS as f32) / size as f32;
                    let v = (py as f32 + (sy as f32 + 0.5) / SS as f32) / size as f32;
                    if !inside_rounded_square((u, v), 0.18) {
                        continue;
                    }
                    let c = if (u - 0.76).powi(2) + (v - 0.78).powi(2) <= 0.075 * 0.075 {
                        DOT
                    } else if inside_polygon((u, v), &BOLT_POLY) {
                        BOLT
                    } else {
                        BODY
                    };
                    r += c[0];
                    g += c[1];
                    b += c[2];
                    covered += 1;
                }
            }
            if covered == 0 {
                out.extend_from_slice(&[0, 0, 0, 0]);
            } else {
                let n = covered as f32;
                let alpha = (covered * 255 + SS * SS / 2) / (SS * SS);
                out.extend_from_slice(&[
                    (r / n).round() as u8,
                    (g / n).round() as u8,
                    (b / n).round() as u8,
                    alpha as u8,
                ]);
            }
        }
    }
    out
}

/// The sizes written to `assets/icons`.
pub const SIZES: [u32; 3] = [48, 128, 256];

/// `(file name, PNG bytes)` for every icon size.
pub fn icon_files() -> Vec<(String, Vec<u8>)> {
    SIZES
        .iter()
        .map(|&s| {
            (
                format!("lgduel-{s}.png"),
                crate::textures::encode_png_rgba(s, s, &icon_rgba(s)),
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icon_shape_and_determinism() {
        let a = icon_rgba(64);
        assert_eq!(a.len(), 64 * 64 * 4);
        assert_eq!(a, icon_rgba(64));
        let px = |x: usize, y: usize| &a[(y * 64 + x) * 4..][..4];
        assert_eq!(px(0, 0)[3], 0, "corner is transparent");
        assert_eq!(px(63, 63)[3], 0);
        assert_eq!(px(32, 4)[3], 255, "edge middle is opaque");
        // The bolt's body is the beam blue; a spot far from the bolt is the dark body colour.
        assert_eq!(&px(38, 30)[..3], [140, 204, 255]);
        assert_eq!(&px(8, 32)[..3], [24, 26, 34]);
        // The pink dot.
        assert_eq!(&px(48, 50)[..3], [255, 51, 153]);
    }

    #[test]
    fn committed_icons_match_the_generator() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/icons");
        for (name, bytes) in icon_files() {
            assert_eq!(
                std::fs::read(dir.join(&name)).unwrap(),
                bytes,
                "{name} is stale: rerun lgduel --make-icons assets/icons"
            );
        }
    }

    #[test]
    fn desktop_file_is_valid() {
        let text = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packaging/lgduel.desktop"),
        )
        .unwrap();
        let get = |k: &str| {
            text.lines()
                .find_map(|l| l.strip_prefix(&format!("{k}=")))
                .unwrap_or_else(|| panic!("missing {k}"))
        };
        assert!(text.starts_with("[Desktop Entry]"));
        assert_eq!(get("Type"), "Application");
        assert_eq!(get("Exec"), "lgduel");
        assert_eq!(get("Icon"), "lgduel");
        assert_eq!(get("StartupWMClass"), "lgduel");
        assert!(get("Categories").ends_with(';'));
        assert_eq!(get("Terminal"), "false");
    }
}
