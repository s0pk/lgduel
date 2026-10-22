//! Builds a TrueType font in memory from LGDuel's own 5×7 pixel glyphs
//! (`font.rs`), so the egui UI needs no third-party font files. Each lit pixel
//! becomes a square contour. Lowercase maps to the uppercase glyphs.
//!
//! Tables: cmap (format 12), glyf, head, hhea, hmtx, loca (long), maxp, post.

use crate::font::{GLYPH_H, GLYPH_W, chars, glyph_rows};

/// Font units per pixel of the 5×7 grid.
const PX: i16 = 128;
const UNITS_PER_EM: u16 = 1024; // 8 px: 7 rows + 1 row gap
const ASCENDER: i16 = 8 * PX;
const DESCENDER: i16 = -2 * PX;
const ADVANCE: u16 = ((GLYPH_W as i16 + 1) * PX) as u16;

struct Glyph {
    /// Squares as (x0, y0) in font units (each PX × PX).
    squares: Vec<(i16, i16)>,
}

impl Glyph {
    fn from_rows(rows: &[&str; GLYPH_H]) -> Glyph {
        let mut squares = Vec::new();
        for (r, row) in rows.iter().enumerate() {
            for (c, b) in row.bytes().enumerate() {
                if b == b'#' {
                    // Bottom row (r = 6) sits on the baseline.
                    let x0 = c as i16 * PX + PX / 2;
                    let y0 = (GLYPH_H as i16 - 1 - r as i16) * PX;
                    squares.push((x0, y0));
                }
            }
        }
        Glyph { squares }
    }

    fn encode(&self, out: &mut Vec<u8>) {
        if self.squares.is_empty() {
            return; // empty glyph: zero-length entry in loca
        }
        let xs = self.squares.iter().map(|s| s.0);
        let ys = self.squares.iter().map(|s| s.1);
        let (x_min, x_max) = (xs.clone().min().unwrap(), xs.max().unwrap() + PX);
        let (y_min, y_max) = (ys.clone().min().unwrap(), ys.max().unwrap() + PX);
        put16(out, self.squares.len() as u16); // numberOfContours
        for v in [x_min, y_min, x_max, y_max] {
            put16(out, v as u16);
        }
        for i in 0..self.squares.len() {
            put16(out, (i * 4 + 3) as u16); // endPtsOfContours
        }
        put16(out, 0); // instructionLength
        let n = self.squares.len() * 4;
        out.extend(std::iter::repeat_n(0x01u8, n)); // all points on-curve, 16-bit deltas
        // Clockwise (outer) contour per square: up the left edge, right, down.
        let mut pts = Vec::with_capacity(n);
        for &(x, y) in &self.squares {
            pts.extend([(x, y), (x, y + PX), (x + PX, y + PX), (x + PX, y)]);
        }
        let (mut px, mut py) = (0i16, 0i16);
        for &(x, _) in &pts {
            put16(out, (x - px) as u16);
            px = x;
        }
        for &(_, y) in &pts {
            put16(out, (y - py) as u16);
            py = y;
        }
    }
}

fn put16(out: &mut Vec<u8>, v: u16) {
    out.extend_from_slice(&v.to_be_bytes());
}

fn put32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_be_bytes());
}

fn checksum(data: &[u8]) -> u32 {
    data.chunks(4).fold(0u32, |acc, c| {
        let mut b = [0u8; 4];
        b[..c.len()].copy_from_slice(c);
        acc.wrapping_add(u32::from_be_bytes(b))
    })
}

/// The complete font file.
pub fn build() -> Vec<u8> {
    // Glyph 0 = .notdef (a box), 1 = space, then every defined character.
    let notdef = Glyph::from_rows(&[
        "#####", "#   #", "#   #", "#   #", "#   #", "#   #", "#####",
    ]);
    let space = Glyph { squares: vec![] };
    let mut glyphs = vec![notdef, space];
    let mut map: Vec<(u32, u32)> = vec![(' ' as u32, 1)];
    for c in chars() {
        let id = glyphs.len() as u32;
        glyphs.push(Glyph::from_rows(&glyph_rows(c).expect("listed char")));
        map.push((c as u32, id));
        if c.is_ascii_uppercase() {
            map.push((c.to_ascii_lowercase() as u32, id));
        }
    }
    map.sort();
    let n = glyphs.len() as u16;

    // glyf + loca (long offsets, 4-byte aligned entries).
    let (mut glyf, mut loca) = (Vec::new(), Vec::new());
    let mut max_points = 0;
    let mut max_contours = 0;
    for g in &glyphs {
        put32(&mut loca, glyf.len() as u32);
        g.encode(&mut glyf);
        while !glyf.len().is_multiple_of(4) {
            glyf.push(0);
        }
        max_points = max_points.max(g.squares.len() * 4);
        max_contours = max_contours.max(g.squares.len());
    }
    put32(&mut loca, glyf.len() as u32);

    let mut cmap = Vec::new();
    put16(&mut cmap, 0); // version
    put16(&mut cmap, 1); // numTables
    put16(&mut cmap, 3); // platform: Windows
    put16(&mut cmap, 10); // encoding: Unicode full
    put32(&mut cmap, 12); // offset of subtable
    put16(&mut cmap, 12); // format 12
    put16(&mut cmap, 0);
    put32(&mut cmap, 16 + 12 * map.len() as u32);
    put32(&mut cmap, 0); // language
    put32(&mut cmap, map.len() as u32);
    for (code, id) in &map {
        put32(&mut cmap, *code);
        put32(&mut cmap, *code);
        put32(&mut cmap, *id);
    }

    let x_max = GLYPH_W as i16 * PX + PX / 2;
    let mut head = Vec::new();
    put32(&mut head, 0x0001_0000); // version
    put32(&mut head, 0x0001_0000); // fontRevision
    put32(&mut head, 0); // checksumAdjustment (filled below)
    put32(&mut head, 0x5F0F_3CF5); // magic
    put16(&mut head, 0x000B); // flags
    put16(&mut head, UNITS_PER_EM);
    head.extend_from_slice(&[0; 16]); // created, modified
    for v in [0, 0, x_max, GLYPH_H as i16 * PX] {
        put16(&mut head, v as u16);
    }
    put16(&mut head, 0); // macStyle
    put16(&mut head, 8); // lowestRecPPEM
    put16(&mut head, 2); // fontDirectionHint
    put16(&mut head, 1); // indexToLocFormat: long
    put16(&mut head, 0); // glyphDataFormat

    let mut hhea = Vec::new();
    put32(&mut hhea, 0x0001_0000);
    for v in [ASCENDER, DESCENDER, 0] {
        put16(&mut hhea, v as u16);
    }
    put16(&mut hhea, ADVANCE);
    for v in [0, 0, x_max, 1, 0, 0, 0, 0, 0, 0, 0] {
        put16(&mut hhea, v as u16); // minLSB, minRSB, xMaxExtent, caret…, reserved, metricDataFormat
    }
    put16(&mut hhea, n); // numberOfHMetrics

    let mut hmtx = Vec::new();
    for _ in 0..n {
        put16(&mut hmtx, ADVANCE);
        put16(&mut hmtx, (PX / 2) as u16);
    }

    let mut maxp = Vec::new();
    put32(&mut maxp, 0x0001_0000);
    put16(&mut maxp, n);
    put16(&mut maxp, max_points as u16);
    put16(&mut maxp, max_contours as u16);
    for v in [0u16, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0] {
        put16(&mut maxp, v);
    }

    let mut post = Vec::new();
    put32(&mut post, 0x0003_0000);
    put32(&mut post, 0); // italicAngle
    put16(&mut post, (-PX) as u16);
    put16(&mut post, (PX / 2) as u16);
    put32(&mut post, 1); // isFixedPitch
    post.extend_from_slice(&[0; 16]);

    // Table directory (tags sorted).
    let tables: [(&[u8; 4], Vec<u8>); 8] = [
        (b"cmap", cmap),
        (b"glyf", glyf),
        (b"head", head),
        (b"hhea", hhea),
        (b"hmtx", hmtx),
        (b"loca", loca),
        (b"maxp", maxp),
        (b"post", post),
    ];
    let num = tables.len() as u16;
    let entry_selector = 15 - num.leading_zeros() as u16; // floor(log2(num))
    let search_range = (1u16 << entry_selector) * 16;
    let mut out = Vec::new();
    put32(&mut out, 0x0001_0000);
    put16(&mut out, num);
    put16(&mut out, search_range);
    put16(&mut out, entry_selector);
    put16(&mut out, num * 16 - search_range);
    let mut offset = 12 + 16 * tables.len();
    let mut head_offset = 0;
    for (tag, data) in &tables {
        out.extend_from_slice(*tag);
        put32(&mut out, checksum(data));
        put32(&mut out, offset as u32);
        put32(&mut out, data.len() as u32);
        if *tag == b"head" {
            head_offset = offset;
        }
        offset += data.len().div_ceil(4) * 4;
    }
    for (_, data) in &tables {
        out.extend_from_slice(data);
        while !out.len().is_multiple_of(4) {
            out.push(0);
        }
    }
    let adjust = 0xB1B0_AFBAu32.wrapping_sub(checksum(&out));
    out[head_offset + 8..head_offset + 12].copy_from_slice(&adjust.to_be_bytes());
    out
}

/// Install the pixel font as egui's only font, at crisp pixel-multiple sizes.
pub fn install(ctx: &egui::Context) {
    let mut defs = egui::FontDefinitions::empty();
    defs.font_data.insert(
        "lgduel-pixel".into(),
        std::sync::Arc::new(egui::FontData::from_owned(build())),
    );
    for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
        defs.families.insert(family, vec!["lgduel-pixel".into()]);
    }
    ctx.set_fonts(defs);
    ctx.global_style_mut(|style| {
        use egui::{FontFamily::Proportional, FontId, TextStyle};
        style.text_styles = [
            (TextStyle::Small, FontId::new(16.0, Proportional)),
            (TextStyle::Body, FontId::new(16.0, Proportional)),
            (TextStyle::Button, FontId::new(16.0, Proportional)),
            (
                TextStyle::Monospace,
                FontId::new(16.0, egui::FontFamily::Monospace),
            ),
            (TextStyle::Heading, FontId::new(24.0, Proportional)),
        ]
        .into();
        style.spacing.item_spacing = egui::vec2(8.0, 8.0);
        style.spacing.button_padding = egui::vec2(10.0, 6.0);
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use skrifa::instance::{LocationRef, Size};
    use skrifa::raw::TableProvider;
    use skrifa::{FontRef, MetadataProvider};

    /// Parse with skrifa — the same font reader egui uses.
    #[test]
    fn generated_font_parses_and_maps_chars() {
        let bytes = build();
        let f = FontRef::new(&bytes).expect("valid TrueType");
        assert_eq!(f.head().unwrap().units_per_em(), UNITS_PER_EM);
        let cm = f.charmap();
        let a = cm.map('A').expect("A mapped");
        assert_eq!(cm.map('a'), Some(a), "lowercase uses uppercase glyph");
        assert!(cm.map(':').is_some() && cm.map('_').is_some() && cm.map('@').is_some());
        assert!(cm.map('€').is_none());
        let gm = f.glyph_metrics(Size::unscaled(), LocationRef::default());
        assert_eq!(gm.advance_width(a), Some(ADVANCE as f32));
        let b = gm.bounds(a).expect("A has bounds");
        assert_eq!(b.y_max - b.y_min, GLYPH_H as f32 * PX as f32);
        assert!(f.outline_glyphs().get(a).is_some());
    }

    #[test]
    fn every_listed_char_has_a_distinct_glyph() {
        let bytes = build();
        let f = FontRef::new(&bytes).unwrap();
        let cm = f.charmap();
        let ids: std::collections::BTreeSet<_> = chars().map(|c| cm.map(c).unwrap()).collect();
        assert_eq!(ids.len(), chars().count());
    }
}
