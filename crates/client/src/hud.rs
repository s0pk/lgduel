//! Heads-up display: crosshair, health, fight stats, damage numbers.
//! Built as screen-space quads (vertex positions already in NDC).

use crate::font::{GLYPH_H, GLYPH_W, glyph};
use crate::render::Vertex;
use glam::{Mat4, Vec4};
use lgduel_sim::math::Vec3;

#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct CrosshairStyle {
    /// Shape of the crosshair.
    pub shape: CrosshairShape,
    /// Arm length in pixels (0 = no arms).
    pub size: f32,
    /// Gap from centre to arm start, pixels.
    pub gap: f32,
    pub thickness: f32,
    pub dot: bool,
    pub color: [f32; 3],
    /// Draw a dark outline around the shape (readable on any background).
    pub outline: bool,
    pub outline_color: [f32; 3],
    /// Colour for a moment after your beam hits (same as `color` = no change).
    pub hit_color: [f32; 3],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CrosshairShape {
    /// Four arms (the classic look).
    Cross,
    /// Only the centre dot.
    Dot,
    /// A ring of radius `gap + size`.
    Circle,
    /// Cross without the top arm.
    TShape,
}

impl CrosshairShape {
    pub const ALL: [CrosshairShape; 4] = [
        CrosshairShape::Cross,
        CrosshairShape::Dot,
        CrosshairShape::Circle,
        CrosshairShape::TShape,
    ];

    pub fn name(self) -> &'static str {
        match self {
            CrosshairShape::Cross => "Cross",
            CrosshairShape::Dot => "Dot",
            CrosshairShape::Circle => "Circle",
            CrosshairShape::TShape => "T shape",
        }
    }
}

impl Default for CrosshairStyle {
    fn default() -> Self {
        Self {
            shape: CrosshairShape::Cross,
            size: 8.0,
            gap: 4.0,
            thickness: 2.0,
            dot: true,
            color: [1.0, 0.2, 0.6],
            outline: false,
            outline_color: [0.0, 0.0, 0.0],
            hit_color: [1.0, 0.2, 0.6],
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct HudSettings {
    pub crosshair: CrosshairStyle,
    pub text_color: [f32; 3],
    pub damage_numbers: bool,
    pub damage_color: [f32; 3],
    /// Pixel size of one font pixel for HUD text.
    pub text_scale: f32,
    /// Dark drop shadow behind text for contrast on bright walls.
    pub text_shadow: bool,
    pub net_graph: bool,
    pub net_graph_color: [f32; 3],
    /// Scale the HUD text with the window height instead of using `text_scale`.
    pub auto_scale: bool,
    /// Show pressed movement keys (W A S D, jump, crouch, fire) on screen.
    pub input_display: bool,
}

/// Keys to show in the input display.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct InputView {
    pub forward: bool,
    pub back: bool,
    pub left: bool,
    pub right: bool,
    pub jump: bool,
    pub crouch: bool,
    pub fire: bool,
}

impl Default for HudSettings {
    fn default() -> Self {
        Self {
            crosshair: CrosshairStyle::default(),
            text_color: [0.92, 0.92, 0.92],
            damage_numbers: true,
            damage_color: [1.0, 0.45, 0.05],
            text_scale: 3.0,
            auto_scale: false,
            text_shadow: true,
            net_graph: true,
            input_display: false,
            net_graph_color: [1.0, 0.85, 0.4],
        }
    }
}

/// Numbers the HUD shows (gathered from the game by the caller).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct HudInfo {
    pub health: i32,
    pub accuracy_pct: f32,
    pub damage_dealt: u32,
    pub damage_taken: u32,
    pub wins: u32,
    pub losses: u32,
    /// Seconds into the current fight, if one is running.
    pub fight_secs: Option<f32>,
    pub opponent_health: Option<i32>,
    /// Network statistics (online only).
    pub net: Option<NetGraph>,
}

/// Live connection numbers for the net-graph overlay.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct NetGraph {
    pub ping_ms: f32,
    /// RTT mean deviation (jitter), ms.
    pub jitter_ms: f32,
    pub loss_pct: f32,
    /// Server simulation rate.
    pub tick_hz: u32,
    /// Snapshots actually received per second (last 1 s).
    pub snap_hz: f32,
    pub interp_delay_ms: f32,
    pub in_kbps: f32,
    pub out_kbps: f32,
    /// Own-view snap-backs (> 1u) so far.
    pub rubber_bands: u64,
    /// Client/server LG hit verdict agreement, %.
    pub hitreg_pct: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DamageNumber {
    pub pos: Vec3,
    pub amount: u32,
    pub age: f32,
}

/// Floating damage numbers: rise from the hit point and vanish after `LIFETIME`.
#[derive(Debug, Default, Clone)]
pub struct DamageNumbers {
    pub items: Vec<DamageNumber>,
}

impl DamageNumbers {
    pub const LIFETIME: f32 = 0.6;
    pub const RISE: f32 = 40.0;

    /// Hits closer together than this merge into one running total.
    pub const MERGE: f32 = 0.25;

    /// Add a hit. A stream of hits shows as one rising total, not a stack of 7s.
    pub fn add(&mut self, pos: Vec3, amount: u32) {
        if let Some(last) = self.items.last_mut()
            && last.age < Self::MERGE
        {
            last.amount += amount;
            last.pos = pos;
            last.age = 0.0;
            return;
        }
        self.items.push(DamageNumber {
            pos,
            amount,
            age: 0.0,
        });
    }

    pub fn update(&mut self, dt: f32) {
        for d in &mut self.items {
            d.age += dt;
        }
        self.items.retain(|d| d.age < Self::LIFETIME);
    }
}

/// Screen (pixels, origin top-left) → NDC.
fn ndc(screen: (f32, f32), x: f32, y: f32) -> [f32; 3] {
    [x / screen.0 * 2.0 - 1.0, 1.0 - y / screen.1 * 2.0, 0.0]
}

pub fn push_rect(
    out: &mut Vec<Vertex>,
    screen: (f32, f32),
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    color: [f32; 3],
) {
    let p = [
        ndc(screen, x, y),
        ndc(screen, x + w, y),
        ndc(screen, x + w, y + h),
        ndc(screen, x, y + h),
    ];
    for i in [0, 1, 2, 0, 2, 3] {
        out.push(Vertex { pos: p[i], color });
    }
}

/// The HUD text scale for a window `height` pixels tall: `text_scale`, or with
/// `auto_scale` one font pixel per 1/360th of the height (720p: 2, 1080p: 3, 4K: 6).
pub fn effective_text_scale(s: &HudSettings, height: u32) -> f32 {
    if s.auto_scale {
        ((height as f32 / 360.0).round()).clamp(2.0, 8.0)
    } else {
        s.text_scale
    }
}

/// Text width in pixels at `scale`.
pub fn text_width(s: &str, scale: f32) -> f32 {
    let n = s.chars().count() as f32;
    if n == 0.0 {
        0.0
    } else {
        (n * (GLYPH_W as f32 + 1.0) - 1.0) * scale
    }
}

pub fn push_text(
    out: &mut Vec<Vertex>,
    screen: (f32, f32),
    x: f32,
    y: f32,
    scale: f32,
    color: [f32; 3],
    s: &str,
) {
    for (i, c) in s.chars().enumerate() {
        let gx = x + i as f32 * (GLYPH_W as f32 + 1.0) * scale;
        if let Some(px) = glyph(c) {
            for (cx, cy) in px {
                push_rect(
                    out,
                    screen,
                    gx + cx as f32 * scale,
                    y + cy as f32 * scale,
                    scale,
                    scale,
                    color,
                );
            }
        }
    }
}

const SHADOW: [f32; 3] = [0.0, 0.0, 0.0];

/// Text with an optional 1-font-pixel drop shadow.
#[allow(clippy::too_many_arguments)]
fn label(
    out: &mut Vec<Vertex>,
    screen: (f32, f32),
    x: f32,
    y: f32,
    scale: f32,
    color: [f32; 3],
    shadow: bool,
    s: &str,
) {
    if shadow {
        let d = (scale / 2.0).max(1.0);
        push_text(out, screen, x + d, y + d, scale, SHADOW, s);
    }
    push_text(out, screen, x, y, scale, color, s);
}

pub fn push_crosshair(out: &mut Vec<Vertex>, screen: (f32, f32), c: &CrosshairStyle) {
    push_crosshair_hit(out, screen, c, false);
}

/// Pixel rectangles (x, y, w, h) of a crosshair shape, each grown by `grow` on every side.
fn crosshair_rects(c: &CrosshairStyle, cx: f32, cy: f32, grow: f32) -> Vec<(f32, f32, f32, f32)> {
    let t = c.thickness.max(1.0);
    let h = (t / 2.0).floor();
    let (g, s) = (c.gap, c.size);
    let r = |x: f32, y: f32, w: f32, hh: f32| (x - grow, y - grow, w + 2.0 * grow, hh + 2.0 * grow);
    let mut v = Vec::new();
    let arms = c.size > 0.0 && matches!(c.shape, CrosshairShape::Cross | CrosshairShape::TShape);
    if arms {
        v.push(r(cx - g - s, cy - h, s, t)); // left
        v.push(r(cx + g, cy - h, s, t)); // right
        if c.shape == CrosshairShape::Cross {
            v.push(r(cx - h, cy - g - s, t, s)); // top
        }
        v.push(r(cx - h, cy + g, t, s)); // bottom
    }
    if c.shape == CrosshairShape::Dot {
        // The Dot shape is a real dot: `size` wide (at least the line thickness).
        let d = c.size.max(t);
        v.push(r(cx - (d / 2.0).floor(), cy - (d / 2.0).floor(), d, d));
    } else if c.dot {
        v.push(r(cx - h, cy - h, t, t));
    }
    v
}

/// A ring of radius `radius` (pixels), `thickness` wide, from 24 straight quads.
fn push_ring(
    out: &mut Vec<Vertex>,
    screen: (f32, f32),
    cx: f32,
    cy: f32,
    radius: f32,
    thickness: f32,
    color: [f32; 3],
) {
    const N: usize = 24;
    let (r0, r1) = (
        (radius - thickness / 2.0).max(0.0),
        radius + thickness / 2.0,
    );
    let pt = |i: usize, r: f32| {
        let a = i as f32 / N as f32 * std::f32::consts::TAU;
        (cx + a.cos() * r, cy + a.sin() * r)
    };
    for i in 0..N {
        let q = [pt(i, r0), pt(i, r1), pt(i + 1, r1), pt(i + 1, r0)];
        for k in [0, 1, 2, 0, 2, 3] {
            out.push(Vertex {
                pos: ndc(screen, q[k].0, q[k].1),
                color,
            });
        }
    }
}

/// The crosshair; `hit` draws it in `hit_color` (a beam just connected).
pub fn push_crosshair_hit(
    out: &mut Vec<Vertex>,
    screen: (f32, f32),
    c: &CrosshairStyle,
    hit: bool,
) {
    let (cx, cy) = ((screen.0 / 2.0).floor(), (screen.1 / 2.0).floor());
    let color = if hit { c.hit_color } else { c.color };
    let t = c.thickness.max(1.0);
    let ring = c.shape == CrosshairShape::Circle && c.size > 0.0;
    if c.outline {
        for (x, y, w, h) in crosshair_rects(c, cx, cy, 1.0) {
            push_rect(out, screen, x, y, w, h, c.outline_color);
        }
        if ring {
            push_ring(
                out,
                screen,
                cx,
                cy,
                c.gap + c.size,
                t + 2.0,
                c.outline_color,
            );
        }
    }
    for (x, y, w, h) in crosshair_rects(c, cx, cy, 0.0) {
        push_rect(out, screen, x, y, w, h, color);
    }
    if ring {
        push_ring(out, screen, cx, cy, c.gap + c.size, t, color);
    }
}

/// Project a world point to screen pixels; None if behind the camera.
pub fn project(view_proj: &Mat4, screen: (f32, f32), p: Vec3) -> Option<(f32, f32)> {
    let c = *view_proj * Vec4::new(p.x, p.y, p.z, 1.0);
    if c.w <= 0.0 {
        return None;
    }
    let (x, y) = (c.x / c.w, c.y / c.w);
    Some(((x + 1.0) * 0.5 * screen.0, (1.0 - y) * 0.5 * screen.1))
}

/// Pressed keys as a small key grid above the bottom centre of the screen.
pub fn input_display(out: &mut Vec<Vertex>, screen: (f32, f32), k: InputView, s: &HudSettings) {
    let unit = 9.0 * effective_text_scale(s, screen.1 as u32).max(1.0);
    let gap = unit * 0.15;
    let x0 = screen.0 * 0.5 - (unit * 1.5 + gap);
    let y0 = screen.1 - 16.0 - unit * 3.0 - gap * 2.0;
    let dim = [0.25, 0.25, 0.28];
    let ts = (effective_text_scale(s, screen.1 as u32) * 0.67).max(1.0);
    let key = |out: &mut Vec<Vertex>, x: f32, y: f32, w: f32, on: bool, text: &str| {
        let (fill, ink) = if on {
            (s.text_color, [0.05, 0.05, 0.05])
        } else {
            (dim, [0.6, 0.6, 0.6])
        };
        push_rect(out, screen, x, y, w, unit, fill);
        let tx = x + (w - text_width(text, ts)) * 0.5;
        let ty = y + (unit - GLYPH_H as f32 * ts) * 0.5;
        push_text(out, screen, tx, ty, ts, ink, text);
    };
    let col = |i: f32| x0 + i * (unit + gap);
    key(out, col(1.0), y0, unit, k.forward, "W");
    let y1 = y0 + unit + gap;
    key(out, col(0.0), y1, unit, k.left, "A");
    key(out, col(1.0), y1, unit, k.back, "S");
    key(out, col(2.0), y1, unit, k.right, "D");
    let y2 = y1 + unit + gap;
    let wide = (unit * 3.0 + gap * 2.0 - gap * 2.0) / 3.0;
    key(out, col(0.0), y2, wide, k.jump, "JP");
    key(out, col(0.0) + wide + gap, y2, wide, k.crouch, "CR");
    key(out, col(0.0) + (wide + gap) * 2.0, y2, wide, k.fire, "LG");
}

/// One centred line at the top (arena / queue / spectating status).
pub fn status_line(out: &mut Vec<Vertex>, screen: (f32, f32), text: &str, s: &HudSettings) {
    let sc = effective_text_scale(s, screen.1 as u32);
    let x = (screen.0 - text_width(text, sc)) * 0.5;
    label(
        out,
        screen,
        x.max(0.0),
        16.0,
        sc,
        s.text_color,
        s.text_shadow,
        text,
    );
}

/// Lines centred horizontally, starting a little above the crosshair
/// (end-of-fight summary).
pub fn centre_block(out: &mut Vec<Vertex>, screen: (f32, f32), lines: &[String], s: &HudSettings) {
    let sc = effective_text_scale(s, screen.1 as u32) * 1.25;
    let line = (GLYPH_H as f32 + 4.0) * sc;
    let top = screen.1 * 0.5 - 60.0 - line * lines.len() as f32;
    for (i, l) in lines.iter().enumerate() {
        let x = (screen.0 - text_width(l, sc)) * 0.5;
        label(
            out,
            screen,
            x.max(0.0),
            top + i as f32 * line,
            sc,
            s.text_color,
            s.text_shadow,
            l,
        );
    }
}

pub fn build(
    out: &mut Vec<Vertex>,
    screen: (f32, f32),
    view_proj: &Mat4,
    info: &HudInfo,
    numbers: &DamageNumbers,
    s: &HudSettings,
) {
    let hit = numbers.items.iter().any(|d| d.age < 0.15);
    push_crosshair_hit(out, screen, &s.crosshair, hit);
    let sc = effective_text_scale(s, screen.1 as u32);
    let line = (GLYPH_H as f32 + 3.0) * sc;
    let margin = 16.0;
    let sh = s.text_shadow;
    // Health, big, bottom left.
    let big = sc * 2.5;
    let health = info.health.max(0).to_string();
    let hy = screen.1 - margin - GLYPH_H as f32 * big;
    label(out, screen, margin, hy, big, s.text_color, sh, &health);
    // Fight stats, top left.
    let lines = [
        format!("LG {:.1}%", info.accuracy_pct),
        format!("DMG {} / {}", info.damage_dealt, info.damage_taken),
        format!("W-L {}-{}", info.wins, info.losses),
        match info.fight_secs {
            Some(t) => format!("FIGHT {t:.1} SEC"),
            None => "FIGHT -".to_string(),
        },
    ];
    for (i, l) in lines.iter().enumerate() {
        label(
            out,
            screen,
            margin,
            margin + i as f32 * line,
            sc,
            s.text_color,
            sh,
            l,
        );
    }
    // Opponent health, top right.
    if let Some(h) = info.opponent_health {
        let t = format!("OPP {}", h.max(0));
        let x = screen.0 - margin - text_width(&t, sc);
        label(out, screen, x, margin, sc, s.text_color, sh, &t);
    }
    if s.net_graph
        && let Some(n) = &info.net
    {
        let ns = (sc * 0.67).max(1.0);
        let nline = (GLYPH_H as f32 + 3.0) * ns;
        let rows = [
            format!("PING {:.0} MS +-{:.0}", n.ping_ms, n.jitter_ms),
            format!("LOSS {:.1}%", n.loss_pct),
            format!("TICK {} HZ SNAP {:.0} HZ", n.tick_hz, n.snap_hz),
            format!("INTERP {:.0} MS", n.interp_delay_ms),
            format!("IN {:.1} OUT {:.1} KB/S", n.in_kbps, n.out_kbps),
            format!("HITREG {:.0}% RB {}", n.hitreg_pct, n.rubber_bands),
        ];
        let width = rows.iter().map(|r| text_width(r, ns)).fold(0.0, f32::max);
        let x = screen.0 - margin - width;
        let y0 = screen.1 - margin - rows.len() as f32 * nline;
        for (i, r) in rows.iter().enumerate() {
            label(
                out,
                screen,
                x,
                y0 + i as f32 * nline,
                ns,
                s.net_graph_color,
                sh,
                r,
            );
        }
    }
    if s.damage_numbers {
        for d in &numbers.items {
            let rise = d.age / DamageNumbers::LIFETIME * DamageNumbers::RISE;
            if let Some((x, y)) =
                project(view_proj, screen, d.pos + Vec3::new(0.0, 0.0, 24.0 + rise))
            {
                let t = d.amount.to_string();
                let x = x - text_width(&t, sc) / 2.0;
                label(out, screen, x, y, sc, s.damage_color, sh, &t);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rect_maps_to_ndc() {
        let mut v = vec![];
        push_rect(&mut v, (200.0, 100.0), 0.0, 0.0, 200.0, 100.0, [1.0; 3]);
        assert_eq!(v[0].pos, [-1.0, 1.0, 0.0]);
        assert_eq!(v[2].pos, [1.0, -1.0, 0.0]);
    }

    #[test]
    fn crosshair_parts() {
        let mut v = vec![];
        push_crosshair(&mut v, (1920.0, 1080.0), &CrosshairStyle::default());
        assert_eq!(v.len(), 5 * 6);
        let mut v = vec![];
        push_crosshair(
            &mut v,
            (1920.0, 1080.0),
            &CrosshairStyle {
                size: 0.0,
                dot: false,
                ..Default::default()
            },
        );
        assert!(v.is_empty());
    }

    /// The crosshair as it was before shapes existed (arms + dot only).
    fn legacy_crosshair(out: &mut Vec<Vertex>, screen: (f32, f32), c: &CrosshairStyle) {
        let (cx, cy) = ((screen.0 / 2.0).floor(), (screen.1 / 2.0).floor());
        let t = c.thickness.max(1.0);
        let h = (t / 2.0).floor();
        if c.size > 0.0 {
            let (g, s) = (c.gap, c.size);
            push_rect(out, screen, cx - g - s, cy - h, s, t, c.color);
            push_rect(out, screen, cx + g, cy - h, s, t, c.color);
            push_rect(out, screen, cx - h, cy - g - s, t, s, c.color);
            push_rect(out, screen, cx - h, cy + g, t, s, c.color);
        }
        if c.dot {
            push_rect(out, screen, cx - h, cy - h, t, t, c.color);
        }
    }

    #[test]
    fn default_crosshair_is_unchanged() {
        let screen = (1920.0, 1080.0);
        for style in [
            CrosshairStyle::default(),
            CrosshairStyle {
                size: 14.0,
                gap: 0.0,
                thickness: 3.0,
                dot: false,
                ..Default::default()
            },
            CrosshairStyle {
                size: 0.0,
                ..Default::default()
            },
        ] {
            let (mut a, mut b) = (vec![], vec![]);
            push_crosshair(&mut a, screen, &style);
            legacy_crosshair(&mut b, screen, &style);
            assert_eq!(a, b);
        }
    }

    #[test]
    fn shapes_outline_and_hit_colour() {
        let screen = (1000.0, 600.0);
        let count = |s: CrosshairStyle| {
            let mut v = vec![];
            push_crosshair(&mut v, screen, &s);
            v
        };
        let base = CrosshairStyle {
            size: 10.0,
            gap: 5.0,
            thickness: 2.0,
            dot: false,
            ..Default::default()
        };
        let cross = count(base);
        let tee = count(CrosshairStyle {
            shape: CrosshairShape::TShape,
            ..base
        });
        let dot = count(CrosshairStyle {
            shape: CrosshairShape::Dot,
            ..base
        });
        let circle = count(CrosshairStyle {
            shape: CrosshairShape::Circle,
            ..base
        });
        assert_eq!(
            (cross.len(), tee.len(), dot.len(), circle.len()),
            (4 * 6, 3 * 6, 6, 24 * 6)
        );
        // Everything stays within size + gap + thickness of the centre.
        let reach = |v: &[Vertex]| {
            v.iter()
                .map(|p| {
                    let x = (p.pos[0] + 1.0) / 2.0 * screen.0 - 500.0;
                    let y = (1.0 - p.pos[1]) / 2.0 * screen.1 - 300.0;
                    x.abs().max(y.abs())
                })
                .fold(0.0, f32::max)
        };
        for v in [&cross, &tee, &dot, &circle] {
            assert!(reach(v) <= 10.0 + 5.0 + 2.0 + 0.01, "{}", reach(v));
        }
        // Outline adds geometry underneath in its own colour, and the main colour is on top.
        let outlined = count(CrosshairStyle {
            outline: true,
            ..base
        });
        assert_eq!(outlined.len(), cross.len() * 2);
        assert_eq!(outlined[0].color, [0.0, 0.0, 0.0]);
        assert_eq!(outlined.last().unwrap().color, base.color);
        let ring_outline = count(CrosshairStyle {
            shape: CrosshairShape::Circle,
            outline: true,
            ..base
        });
        assert_eq!(ring_outline.len(), circle.len() * 2);
        // Hit colour.
        let style = CrosshairStyle {
            hit_color: [0.0, 1.0, 0.0],
            ..base
        };
        let mut v = vec![];
        push_crosshair_hit(&mut v, screen, &style, true);
        assert!(v.iter().all(|p| p.color == [0.0, 1.0, 0.0]));
        let mut v = vec![];
        push_crosshair_hit(&mut v, screen, &style, false);
        assert!(v.iter().all(|p| p.color == style.color));
    }

    #[test]
    fn auto_scale_follows_the_window_height() {
        let mut s = HudSettings::default();
        assert_eq!(effective_text_scale(&s, 1080), 3.0, "manual by default");
        s.text_scale = 5.0;
        assert_eq!(effective_text_scale(&s, 2160), 5.0);
        s.auto_scale = true;
        for (h, want) in [
            (200, 2.0),
            (720, 2.0),
            (1080, 3.0),
            (1440, 4.0),
            (2160, 6.0),
            (9000, 8.0),
        ] {
            assert_eq!(effective_text_scale(&s, h), want, "{h}");
        }
    }

    #[test]
    fn text_width_and_quads() {
        assert_eq!(text_width("", 2.0), 0.0);
        assert_eq!(text_width("10", 2.0), (2.0 * 6.0 - 1.0) * 2.0);
        let mut v = vec![];
        push_text(&mut v, (100.0, 100.0), 0.0, 0.0, 1.0, [1.0; 3], "1");
        assert_eq!(v.len(), 6 * crate::font::glyph('1').unwrap().count());
    }

    #[test]
    fn rapid_hits_merge_into_one_total() {
        let mut d = DamageNumbers::default();
        for _ in 0..12 {
            d.add(Vec3::ZERO, 7);
            d.update(0.05);
        }
        assert_eq!(d.items.len(), 1);
        assert_eq!(d.items[0].amount, 84);
        d.update(0.3); // gap longer than MERGE
        d.add(Vec3::ZERO, 7);
        assert_eq!(d.items.len(), 2);
    }

    #[test]
    fn damage_numbers_expire() {
        let mut d = DamageNumbers::default();
        d.add(Vec3::ZERO, 7);
        d.update(0.3);
        assert_eq!(d.items.len(), 1);
        d.update(0.31);
        assert!(d.items.is_empty());
    }

    #[test]
    fn projection_centre_and_behind() {
        let cam = crate::render::Camera {
            eye: Vec3::ZERO,
            pitch: lgduel_sim::math::Angle::ZERO,
            yaw: lgduel_sim::math::Angle::ZERO,
            fov: 90.0,
        };
        let vp = cam.view_proj(2.0);
        let (x, y) = project(&vp, (200.0, 100.0), Vec3::new(100.0, 0.0, 0.0)).unwrap();
        assert!((x - 100.0).abs() < 1e-3 && (y - 50.0).abs() < 1e-3);
        assert!(project(&vp, (200.0, 100.0), Vec3::new(-100.0, 0.0, 0.0)).is_none());
    }
}
