//! Procedural RGB textures with a dark, worn, Quake-like look: stone blocks,
//! riveted metal plates, tread plate and ribbed ceiling panels. They are
//! original (generated from noise and simple shapes, nothing from any game) and
//! every one tiles seamlessly at `TEX` x `TEX`.
//!
//! Unlike the light grey textures of the other built-in themes, these carry
//! their own colour and are meant to be drawn with a white tint. They stay mid
//! to dark and low-contrast at a distance so enemies in bright forced colours
//! remain the most visible thing on screen.

use crate::textures::{Image, TEX};

fn hash(x: i32, y: i32, seed: u32) -> u32 {
    let mut h = (x as u32).wrapping_mul(0x27D4_EB2F)
        ^ (y as u32).wrapping_mul(0x1656_67B1)
        ^ seed.wrapping_mul(0x9E37_79B9);
    h ^= h >> 15;
    h = h.wrapping_mul(0x85EB_CA6B);
    h ^= h >> 13;
    h = h.wrapping_mul(0xC2B2_AE35);
    h ^ (h >> 16)
}

/// Uniform 0..1 per lattice point.
fn rnd(x: i32, y: i32, seed: u32) -> f32 {
    (hash(x, y, seed) & 0xFFFF) as f32 / 65535.0
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

fn smooth(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn step(edge0: f32, edge1: f32, x: f32) -> f32 {
    smooth((x - edge0) / (edge1 - edge0))
}

/// Smooth value noise in 0..1 with `cell` pixels per lattice cell; repeats every `TEX` pixels.
fn vnoise(px: f32, py: f32, cell: f32, seed: u32) -> f32 {
    let period = (TEX as f32 / cell).round().max(1.0) as i32;
    let (gx, gy) = (px / cell, py / cell);
    let (x0, y0) = (gx.floor() as i32, gy.floor() as i32);
    let (u, v) = (smooth(gx - x0 as f32), smooth(gy - y0 as f32));
    let at = |x: i32, y: i32| rnd(x.rem_euclid(period), y.rem_euclid(period), seed);
    lerp(
        lerp(at(x0, y0), at(x0 + 1, y0), u),
        lerp(at(x0, y0 + 1), at(x0 + 1, y0 + 1), u),
        v,
    )
}

/// Fractal noise in 0..1: `octaves` layers, each half the cell size and amplitude.
fn fbm(px: f32, py: f32, cell: f32, octaves: u32, seed: u32) -> f32 {
    let (mut sum, mut amp, mut norm, mut c) = (0.0, 1.0, 0.0, cell);
    for o in 0..octaves {
        sum += amp * vnoise(px, py, c.max(4.0), seed.wrapping_add(o * 101));
        norm += amp;
        amp *= 0.5;
        c *= 0.5;
    }
    sum / norm
}

fn mul(c: [f32; 3], k: f32) -> [f32; 3] {
    [c[0] * k, c[1] * k, c[2] * k]
}

fn mix(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [
        lerp(a[0], b[0], t),
        lerp(a[1], b[1], t),
        lerp(a[2], b[2], t),
    ]
}

/// Build an image from a function of the pixel centre (colours 0..1, sRGB).
fn paint(f: impl Fn(f32, f32) -> [f32; 3]) -> Image {
    let mut rgba = Vec::with_capacity((TEX * TEX * 4) as usize);
    for y in 0..TEX {
        for x in 0..TEX {
            let c = f(x as f32 + 0.5, y as f32 + 0.5);
            for ch in c {
                rgba.push((ch.clamp(0.0, 1.0) * 255.0 + 0.5) as u8);
            }
            rgba.push(255);
        }
    }
    Image { rgba }
}

/// Light from the top left: edges facing it brighten, the opposite ones darken.
/// `d_lit` / `d_shade` are distances (pixels) to the lit and shaded edges.
fn bevel(d_lit: f32, d_shade: f32, width: f32) -> f32 {
    0.32 * (1.0 - step(0.0, width, d_lit)) - 0.38 * (1.0 - step(0.0, width * 1.2, d_shade))
}

const GROOVE: [f32; 3] = [0.07, 0.065, 0.06];

/// Rows of stone blocks (two per row, rows offset by half a block).
pub fn stone_wall() -> Image {
    paint(|x, y| {
        let (block_w, block_h) = (128.0, 64.0);
        let row = (y / block_h).floor() as i32;
        let off = if row % 2 == 0 { 0.0 } else { block_w / 2.0 };
        let sx = x + off;
        let col = (sx / block_w).floor() as i32;
        let (lx, ly) = (sx.rem_euclid(block_w), y.rem_euclid(block_h));
        let d_edge = lx.min(ly).min(block_w - lx).min(block_h - ly);
        let id_x = col.rem_euclid(2);
        let tone = 0.86 + 0.28 * rnd(id_x, row, 7);
        let warm = rnd(id_x, row, 11);
        let base = mix([0.37, 0.33, 0.29], [0.40, 0.31, 0.25], warm);
        let grain = 0.68 + 0.64 * fbm(x, y, 32.0, 4, 3);
        let speck = 0.9 + 0.2 * rnd(x as i32, y as i32, 5);
        // Hairline cracks: thin dark ridges of the noise.
        let ridge = 1.0 - (2.0 * vnoise(x, y, 16.0, 61) - 1.0).abs();
        let stain = fbm(x, y, 64.0, 2, 9);
        let mut c = mul(base, tone * grain * speck);
        c = mix(c, [0.22, 0.16, 0.12], step(0.58, 0.8, stain) * 0.5);
        c = mul(c, 1.0 - 0.55 * step(0.965, 0.995, ridge));
        c = mul(
            c,
            1.0 + bevel(lx.min(ly), (block_w - lx).min(block_h - ly), 5.0),
        );
        // Ambient occlusion next to the mortar.
        c = mul(c, 0.72 + 0.28 * step(2.0, 11.0, d_edge));
        if d_edge < 2.5 {
            let m = 0.8 + 0.4 * rnd(x as i32, y as i32, 13);
            return mul(GROOVE, m);
        }
        c
    })
}

/// 128 px plates of dark steel with bolts, scratches and the odd rust stain.
pub fn metal_floor() -> Image {
    paint(|x, y| {
        let plate = 128.0;
        let (lx, ly) = (x.rem_euclid(plate), y.rem_euclid(plate));
        let (px, py) = ((x / plate).floor() as i32, (y / plate).floor() as i32);
        let d_edge = lx.min(ly).min(plate - lx).min(plate - ly);
        let tone = 0.9 + 0.2 * rnd(px.rem_euclid(2), py.rem_euclid(2), 21);
        let grain = 0.82 + 0.36 * fbm(x, y, 32.0, 3, 17);
        // Fine brushed streaks along x.
        let streak = 0.985 + 0.03 * vnoise(x, y * 8.0, 4.0, 23);
        let mut c = mul([0.27, 0.285, 0.32], tone * grain * streak);
        let rust = fbm(x, y, 64.0, 3, 29);
        c = mix(c, [0.40, 0.24, 0.14], step(0.6, 0.78, rust) * 0.65);
        c = mul(
            c,
            1.0 + bevel(lx.min(ly), (plate - lx).min(plate - ly), 5.0),
        );
        c = mul(c, 0.75 + 0.25 * step(2.0, 12.0, d_edge));
        // Bolts in the corners: a lit dome with a dark ring.
        for (bx, by) in [(15.0, 15.0), (113.0, 15.0), (15.0, 113.0), (113.0, 113.0)] {
            let (dx, dy) = (lx - bx, ly - by);
            let r = (dx * dx + dy * dy).sqrt();
            if r < 5.0 {
                let lit = (-(dx + dy) / (5.0 * 1.4)).clamp(-1.0, 1.0);
                c = mul([0.36, 0.37, 0.40], 0.85 + 0.45 * lit);
            } else if r < 7.5 {
                c = mul(c, 0.55);
            }
        }
        if d_edge < 2.0 {
            return mul(GROOVE, 0.9 + 0.2 * rnd(x as i32, y as i32, 31));
        }
        c
    })
}

/// Diamond tread plate for ramps and stairs.
pub fn tread_plate() -> Image {
    // Height of one lozenge at local (u, v) in -1..1, long axis along `a`.
    let lozenge = |u: f32, v: f32, flip: bool| {
        let (a, b) = if flip {
            (
                (u - v) * std::f32::consts::FRAC_1_SQRT_2,
                (u + v) * std::f32::consts::FRAC_1_SQRT_2,
            )
        } else {
            (
                (u + v) * std::f32::consts::FRAC_1_SQRT_2,
                (v - u) * std::f32::consts::FRAC_1_SQRT_2,
            )
        };
        let e = (a / 0.92).powi(2) + (b / 0.3).powi(2);
        (1.0 - e).max(0.0).sqrt()
    };
    paint(|x, y| {
        let cell = 32.0;
        let (cx, cy) = (x.rem_euclid(cell), y.rem_euclid(cell));
        let flip = ((x / cell).floor() as i32 + (y / cell).floor() as i32).rem_euclid(2) == 1;
        let (u, v) = ((cx - 16.0) / 16.0, (cy - 16.0) / 16.0);
        let h = lozenge(u, v, flip);
        let e = 0.1;
        let lit = lozenge(u + e, v + e, flip) - lozenge(u - e, v - e, flip);
        let grain = 0.8 + 0.4 * fbm(x, y, 32.0, 3, 41);
        let scuff = 0.9 + 0.2 * rnd(x as i32, y as i32, 43);
        let base = mix([0.20, 0.21, 0.24], [0.33, 0.34, 0.38], h.min(1.0));
        mul(base, grain * scuff * (1.0 + 0.9 * lit))
    })
}

/// Dark ribbed panels (128 px) with a faint grid.
pub fn ribbed_ceiling() -> Image {
    paint(|x, y| {
        let panel = 128.0;
        let (lx, ly) = (x.rem_euclid(panel), y.rem_euclid(panel));
        let d_edge = lx.min(ly).min(panel - lx).min(panel - ly);
        let grain = 0.72 + 0.56 * fbm(x, y, 32.0, 4, 51);
        // Vertical grime streaks and fine speckle.
        let grime = 0.88 + 0.12 * vnoise(x * 6.0, y, 8.0, 53);
        let speck = 0.93 + 0.14 * rnd(x as i32, y as i32, 57);
        let mut c = mul([0.21, 0.215, 0.24], grain * grime * speck);
        // Faint 32 px grid inside each panel.
        let g = (x.rem_euclid(32.0)).min(y.rem_euclid(32.0));
        if g < 1.0 {
            c = mul(c, 0.8);
        }
        // Raised rib frame with a bolt every 32 px.
        if d_edge < 9.0 {
            let lit = bevel(lx.min(ly), (panel - lx).min(panel - ly), 4.0);
            c = mul(mix(c, [0.30, 0.30, 0.34], 0.7), 1.0 + lit);
            let along = if lx.min(panel - lx) < ly.min(panel - ly) {
                ly
            } else {
                lx
            };
            let (bx, by) = (along.rem_euclid(32.0) - 16.0, d_edge - 4.5);
            let r = (bx * bx + by * by).sqrt();
            if r < 2.2 {
                c = mul(
                    [0.40, 0.41, 0.45],
                    0.9 + 0.3 * (-(bx + by) / 3.0).clamp(-1.0, 1.0),
                );
            } else if r < 3.4 {
                c = mul(c, 0.6);
            }
        }
        if d_edge < 1.5 {
            return mul(GROOVE, 1.2);
        }
        c
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all() -> [(&'static str, Image); 4] {
        [
            ("stone_wall", stone_wall()),
            ("metal_floor", metal_floor()),
            ("tread_plate", tread_plate()),
            ("ribbed_ceiling", ribbed_ceiling()),
        ]
    }

    #[test]
    fn textures_are_deterministic_dark_and_detailed() {
        for (name, img) in all() {
            assert_eq!(img.rgba.len(), (TEX * TEX * 4) as usize, "{name}");
            let px = img.rgba.as_chunks::<4>().0;
            assert!(px.iter().all(|p| p[3] == 255), "{name}: opaque");
            let luma = |p: &[u8; 4]| (p[0] as u32 * 3 + p[1] as u32 * 6 + p[2] as u32) / 10;
            let mean = px.iter().map(|p| luma(p) as u64).sum::<u64>() / px.len() as u64;
            // Mid to dark: enemies in bright colours stay the most visible thing.
            assert!((35..110).contains(&mean), "{name}: mean luma {mean}");
            // Detail: brightness varies across the image (a flat texture has a spread near 0).
            let mean_f = mean as f64;
            let spread = (px
                .iter()
                .map(|p| (luma(p) as f64 - mean_f).powi(2))
                .sum::<f64>()
                / px.len() as f64)
                .sqrt();
            assert!(spread > 6.0, "{name}: brightness spread only {spread:.1}");
            // Low saturation: nothing as vivid as a forced enemy colour.
            let vivid = px
                .iter()
                .filter(|p| {
                    let (mx, mn) = (p[..3].iter().max().unwrap(), p[..3].iter().min().unwrap());
                    mx - mn > 70
                })
                .count();
            assert!(vivid < px.len() / 50, "{name}: {vivid} vivid pixels");
        }
        assert_eq!(stone_wall(), stone_wall());
    }

    #[test]
    fn textures_tile_without_a_visible_seam() {
        // The jump across the wrap-around edge is no bigger than the typical jump
        // between neighbouring pixels inside the image.
        for (name, img) in all() {
            let n = TEX as usize;
            let at = |x: usize, y: usize| {
                let i = (y * n + x) * 4;
                [
                    img.rgba[i] as i32,
                    img.rgba[i + 1] as i32,
                    img.rgba[i + 2] as i32,
                ]
            };
            let diff = |a: [i32; 3], b: [i32; 3]| (0..3).map(|c| (a[c] - b[c]).abs()).sum::<i32>();
            let (mut inner, mut wrap) = (0i64, 0i64);
            for k in 0..n {
                for x in 1..n - 1 {
                    inner +=
                        diff(at(x, k), at(x + 1, k)) as i64 + diff(at(k, x), at(k, x + 1)) as i64;
                }
                wrap += diff(at(n - 1, k), at(0, k)) as i64 + diff(at(k, n - 1), at(k, 0)) as i64;
            }
            let per_inner = inner as f64 / (n * (n - 2) * 2) as f64;
            let per_wrap = wrap as f64 / (n * 2) as f64;
            assert!(
                per_wrap < per_inner * 2.5 + 6.0,
                "{name}: seam {per_wrap:.1} vs inner {per_inner:.1}"
            );
        }
    }

    /// `QUAKE_TEX_DIR=/some/dir cargo test dump_quake_textures -- --ignored` writes PNGs to look at.
    #[test]
    #[ignore]
    fn dump_quake_textures() {
        let dir = std::env::var("QUAKE_TEX_DIR").expect("set QUAKE_TEX_DIR");
        for (name, img) in all() {
            let png = crate::textures::encode_png_rgba(TEX, TEX, &img.rgba);
            // Tiled 2x2 so seams show.
            let mut big = vec![0u8; (TEX * 2 * TEX * 2 * 4) as usize];
            for y in 0..(TEX * 2) {
                for x in 0..(TEX * 2) {
                    let src = (((y % TEX) * TEX + x % TEX) * 4) as usize;
                    let dst = ((y * TEX * 2 + x) * 4) as usize;
                    big[dst..dst + 4].copy_from_slice(&img.rgba[src..src + 4]);
                }
            }
            std::fs::write(format!("{dir}/{name}.png"), png).unwrap();
            std::fs::write(
                format!("{dir}/{name}_tiled.png"),
                crate::textures::encode_png_rgba(TEX * 2, TEX * 2, &big),
            )
            .unwrap();
        }
    }
}
