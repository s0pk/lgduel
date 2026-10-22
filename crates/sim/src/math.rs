//! Deterministic math for the simulation.
//!
//! Policy (see DECISIONS D2): only IEEE-754 basic ops (+ - * / sqrt), which are
//! correctly rounded on every platform; no `mul_add`, no libm transcendentals.
//! Angles are integer fractions of a full turn so the client sends exact values
//! and trig is computed by our own fixed routine.

use serde::{Deserialize, Serialize};
use std::ops::{Add, AddAssign, Mul, Neg, Sub, SubAssign};

/// 3-vector in Quake coordinates: +x forward, +y left, +z up.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct Vec3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

impl Vec3 {
    pub const ZERO: Vec3 = Vec3::new(0.0, 0.0, 0.0);

    pub const fn new(x: f32, y: f32, z: f32) -> Self {
        Self { x, y, z }
    }

    pub const fn from_array(a: [f32; 3]) -> Self {
        Self::new(a[0], a[1], a[2])
    }

    pub fn to_array(self) -> [f32; 3] {
        [self.x, self.y, self.z]
    }

    pub fn dot(self, o: Vec3) -> f32 {
        self.x * o.x + self.y * o.y + self.z * o.z
    }

    pub fn cross(self, o: Vec3) -> Vec3 {
        Vec3::new(
            self.y * o.z - self.z * o.y,
            self.z * o.x - self.x * o.z,
            self.x * o.y - self.y * o.x,
        )
    }

    pub fn length(self) -> f32 {
        self.dot(self).sqrt()
    }

    /// Horizontal (xy) speed.
    pub fn length_xy(self) -> f32 {
        (self.x * self.x + self.y * self.y).sqrt()
    }

    /// Unit vector and original length; zero vector stays zero.
    pub fn normalize_len(self) -> (Vec3, f32) {
        let len = self.length();
        if len == 0.0 {
            (Vec3::ZERO, 0.0)
        } else {
            let inv = 1.0 / len;
            (self * inv, len)
        }
    }

    pub fn normalize(self) -> Vec3 {
        self.normalize_len().0
    }

    /// `self + d * s`, evaluated as two separately rounded ops (no FMA).
    pub fn mad(self, s: f32, d: Vec3) -> Vec3 {
        self + d * s
    }

    pub fn component(self, axis: usize) -> f32 {
        match axis {
            0 => self.x,
            1 => self.y,
            2 => self.z,
            _ => panic!("axis {axis} out of range"),
        }
    }

    pub fn set_component(&mut self, axis: usize, v: f32) {
        match axis {
            0 => self.x = v,
            1 => self.y = v,
            2 => self.z = v,
            _ => panic!("axis {axis} out of range"),
        }
    }
}

impl Add for Vec3 {
    type Output = Vec3;
    fn add(self, o: Vec3) -> Vec3 {
        Vec3::new(self.x + o.x, self.y + o.y, self.z + o.z)
    }
}

impl AddAssign for Vec3 {
    fn add_assign(&mut self, o: Vec3) {
        *self = *self + o;
    }
}

impl Sub for Vec3 {
    type Output = Vec3;
    fn sub(self, o: Vec3) -> Vec3 {
        Vec3::new(self.x - o.x, self.y - o.y, self.z - o.z)
    }
}

impl SubAssign for Vec3 {
    fn sub_assign(&mut self, o: Vec3) {
        *self = *self - o;
    }
}

impl Mul<f32> for Vec3 {
    type Output = Vec3;
    fn mul(self, s: f32) -> Vec3 {
        Vec3::new(self.x * s, self.y * s, self.z * s)
    }
}

impl Neg for Vec3 {
    type Output = Vec3;
    fn neg(self) -> Vec3 {
        Vec3::new(-self.x, -self.y, -self.z)
    }
}

/// Angle as a fraction of a full turn: `u32::MAX + 1` units = 360°.
/// Wraps naturally; interpret as signed for pitch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub struct Angle(pub u32);

const TURN: f64 = 4_294_967_296.0;

impl Angle {
    pub const ZERO: Angle = Angle(0);

    /// Nearest representable angle; any real input wraps into a turn.
    pub fn from_degrees(deg: f64) -> Angle {
        let turns = deg / 360.0;
        let frac = turns - turns.floor();
        Angle(((frac * TURN).round() as u64 & 0xFFFF_FFFF) as u32)
    }

    /// Degrees in [0, 360).
    pub fn degrees(self) -> f64 {
        self.0 as f64 * 360.0 / TURN
    }

    /// Degrees in [-180, 180).
    pub fn signed_degrees(self) -> f64 {
        self.0 as i32 as f64 * 360.0 / TURN
    }

    pub fn wrapping_add(self, o: Angle) -> Angle {
        Angle(self.0.wrapping_add(o.0))
    }

    /// Clamp a signed angle (e.g. pitch) to [-limit, limit].
    pub fn clamp_signed(self, limit: Angle) -> Angle {
        let l = limit.0 as i32;
        Angle((self.0 as i32).clamp(-l, l) as u32)
    }

    /// (sin, cos) computed deterministically from the integer angle.
    pub fn sin_cos(self) -> (f32, f32) {
        let (s, c) = sin_cos_turn(self.0);
        // `+ 0.0` turns -0.0 into +0.0 so cardinal angles give clean zeros.
        (s as f32 + 0.0, c as f32 + 0.0)
    }
}

/// Deterministic sin/cos of `a / 2^32` turns. Octant reduction on the integer,
/// then fixed-degree Taylor polynomials on [0, π/4] in f64 (error < 1e-15,
/// far below f32 resolution). Uses only + and *.
fn sin_cos_turn(a: u32) -> (f64, f64) {
    const OCTANT: u32 = 1 << 29;
    let octant = a >> 29;
    let rem = a & (OCTANT - 1);
    // Within each octant measure from the nearest multiple of 90° so the
    // polynomial argument stays in [0, π/4].
    let mirrored = octant & 1 == 1;
    let r = if mirrored { OCTANT - rem } else { rem };
    let x = r as f64 * (std::f64::consts::TAU / TURN);
    let (s, c) = poly_sin_cos(x);
    // Base quadrant values for angle q*90° + t (t = ±x).
    let (s, c) = if mirrored { (c, s) } else { (s, c) };
    match octant {
        0 => (s, c),
        1 => (s, c),
        2 => (c, -s),
        3 => (c, -s),
        4 => (-s, -c),
        5 => (-s, -c),
        6 => (-c, s),
        _ => (-c, s),
    }
}

fn poly_sin_cos(x: f64) -> (f64, f64) {
    let x2 = x * x;
    // sin: x - x^3/3! + ... - x^15/15!
    let mut s = 0.0;
    let mut c = 0.0;
    const SIN: [f64; 8] = [
        1.0,
        -1.0 / 6.0,
        1.0 / 120.0,
        -1.0 / 5040.0,
        1.0 / 362_880.0,
        -1.0 / 39_916_800.0,
        1.0 / 6_227_020_800.0,
        -1.0 / 1_307_674_368_000.0,
    ];
    const COS: [f64; 9] = [
        1.0,
        -1.0 / 2.0,
        1.0 / 24.0,
        -1.0 / 720.0,
        1.0 / 40_320.0,
        -1.0 / 3_628_800.0,
        1.0 / 479_001_600.0,
        -1.0 / 87_178_291_200.0,
        1.0 / 20_922_789_888_000.0,
    ];
    for k in SIN.iter().rev() {
        s = s * x2 + k;
    }
    for k in COS.iter().rev() {
        c = c * x2 + k;
    }
    (s * x, c)
}

/// Deterministic `atan2(y, x)` in radians, (-π, π], using only + - * / sqrt.
/// Two half-angle reductions bring the argument under tan(π/16), then a fixed
/// Taylor series (error < 1e-15).
pub fn atan2_det(y: f64, x: f64) -> f64 {
    use std::f64::consts::{FRAC_PI_2, PI};
    if x == 0.0 && y == 0.0 {
        return 0.0;
    }
    let (ax, ay) = (x.abs(), y.abs());
    // Angle of (ax, ay) in [0, π/2].
    let swap = ay > ax;
    let mut t = if swap { ax / ay } else { ay / ax };
    // atan(t) = 2 atan(t / (1 + sqrt(1 + t²))), applied twice.
    for _ in 0..2 {
        t /= 1.0 + (1.0 + t * t).sqrt();
    }
    let t2 = t * t;
    let mut s = 0.0;
    for k in (0..12).rev() {
        let term = 1.0 / (2 * k + 1) as f64;
        s = s * t2 + if k % 2 == 0 { term } else { -term };
    }
    let mut a = 4.0 * s * t;
    if swap {
        a = FRAC_PI_2 - a;
    }
    match (x < 0.0, y < 0.0) {
        (false, false) => a,
        (true, false) => PI - a,
        (true, true) => a - PI,
        (false, true) => -a,
    }
}

impl Angle {
    /// Nearest angle to `rad` radians (wraps).
    pub fn from_radians(rad: f64) -> Angle {
        Angle::from_degrees(rad * (180.0 / std::f64::consts::PI))
    }
}

/// (pitch, yaw) that look along `dir` (positive pitch = down). Deterministic.
pub fn look_angles(dir: Vec3) -> (Angle, Angle) {
    let yaw = atan2_det(dir.y as f64, dir.x as f64);
    let flat = (dir.x as f64 * dir.x as f64 + dir.y as f64 * dir.y as f64).sqrt();
    let pitch = -atan2_det(dir.z as f64, flat);
    (Angle::from_radians(pitch), Angle::from_radians(yaw))
}

/// View basis from pitch (positive = look down) and yaw, roll = 0.
/// Returns (forward, right, up), matching Quake's AngleVectors convention.
pub fn angle_vectors(pitch: Angle, yaw: Angle) -> (Vec3, Vec3, Vec3) {
    let (sp, cp) = pitch.sin_cos();
    let (sy, cy) = yaw.sin_cos();
    let forward = Vec3::new(cp * cy, cp * sy, -sp);
    let right = Vec3::new(sy, -cy, 0.0);
    let up = Vec3::new(sp * cy, sp * sy, cp);
    (forward, right, up)
}

/// Stable 64-bit FNV-1a hasher for state hashes. Unlike `std::hash`, the
/// output is fixed forever, so hashes can be stored in golden tests and demos.
#[derive(Debug, Clone)]
pub struct StateHasher(u64);

impl Default for StateHasher {
    fn default() -> Self {
        Self(0xcbf2_9ce4_8422_2325)
    }
}

impl StateHasher {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn bytes(&mut self, b: &[u8]) {
        for &x in b {
            self.0 ^= x as u64;
            self.0 = self.0.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }

    pub fn u32(&mut self, v: u32) {
        self.bytes(&v.to_le_bytes());
    }

    pub fn u64(&mut self, v: u64) {
        self.bytes(&v.to_le_bytes());
    }

    /// Hashes the exact bit pattern (so -0.0 != 0.0, NaNs by payload).
    pub fn f32(&mut self, v: f32) {
        self.u32(v.to_bits());
    }

    pub fn vec3(&mut self, v: Vec3) {
        self.f32(v.x);
        self.f32(v.y);
        self.f32(v.z);
    }

    pub fn finish(&self) -> u64 {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_cardinal_angles() {
        for (deg, s, c) in [
            (0.0, 0.0, 1.0),
            (90.0, 1.0, 0.0),
            (180.0, 0.0, -1.0),
            (270.0, -1.0, 0.0),
        ] {
            let (gs, gc) = Angle::from_degrees(deg).sin_cos();
            assert_eq!((gs.abs(), gc.abs()), (f32::abs(s), f32::abs(c)), "{deg}");
            assert_eq!(gs.signum() * s.abs(), s, "sin sign at {deg}");
            assert_eq!(gc.signum() * c.abs(), c, "cos sign at {deg}");
        }
    }

    #[test]
    fn matches_std_trig() {
        let mut worst = 0.0f64;
        for i in 0..100_000u32 {
            let a = Angle(i.wrapping_mul(2_654_435_761));
            let rad = a.0 as f64 * std::f64::consts::TAU / TURN;
            let (s, c) = sin_cos_turn(a.0);
            worst = worst.max((s - rad.sin()).abs()).max((c - rad.cos()).abs());
        }
        assert!(worst < 1e-14, "worst error {worst}");
    }

    #[test]
    fn degrees_round_trip() {
        for deg in [-720.5, -90.0, 0.0, 33.3, 89.0, 359.999] {
            let a = Angle::from_degrees(deg);
            let back = a.degrees();
            let want = deg.rem_euclid(360.0);
            assert!((back - want).abs() < 1e-6, "{deg}: {back}");
        }
        assert_eq!(Angle::from_degrees(-90.0).signed_degrees(), -90.0);
    }

    #[test]
    fn pitch_clamp() {
        let lim = Angle::from_degrees(89.0);
        let a = Angle::from_degrees(-95.0).clamp_signed(lim);
        assert!((a.signed_degrees() + 89.0).abs() < 1e-6);
        let b = Angle::from_degrees(30.0).clamp_signed(lim);
        assert_eq!(b, Angle::from_degrees(30.0));
    }

    #[test]
    fn view_basis_orthonormal() {
        for (p, y) in [(0.0, 0.0), (30.0, 45.0), (-80.0, 200.0), (89.0, 359.0)] {
            let (f, r, u) = angle_vectors(Angle::from_degrees(p), Angle::from_degrees(y));
            for v in [f, r, u] {
                assert!((v.length() - 1.0).abs() < 1e-6);
            }
            assert!(f.dot(r).abs() < 1e-6 && f.dot(u).abs() < 1e-6 && r.dot(u).abs() < 1e-6);
        }
        // yaw 90 looks along +y (left); the right-hand side is then +x.
        let (f, r, _) = angle_vectors(Angle::ZERO, Angle::from_degrees(90.0));
        assert_eq!(f, Vec3::new(0.0, 1.0, 0.0));
        assert_eq!(r, Vec3::new(1.0, 0.0, 0.0));
        // positive pitch looks down
        let (f, _, _) = angle_vectors(Angle::from_degrees(90.0), Angle::ZERO);
        assert_eq!(f.z, -1.0);
    }

    #[test]
    fn hasher_is_stable() {
        // FNV-1a test vector.
        let mut h = StateHasher::new();
        h.bytes(b"a");
        assert_eq!(h.finish(), 0xaf63_dc4c_8601_ec8c);
        let mut a = StateHasher::new();
        a.f32(0.0);
        let mut b = StateHasher::new();
        b.f32(-0.0);
        assert_ne!(a.finish(), b.finish());
    }

    #[test]
    fn atan2_matches_std() {
        let mut worst = 0.0f64;
        let mut seed = 12345u64;
        for _ in 0..100_000 {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            let y = ((seed >> 11) as f64 / (1u64 << 53) as f64) * 2000.0 - 1000.0;
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            let x = ((seed >> 11) as f64 / (1u64 << 53) as f64) * 2000.0 - 1000.0;
            worst = worst.max((atan2_det(y, x) - y.atan2(x)).abs());
        }
        assert!(worst < 1e-13, "worst {worst}");
        assert_eq!(atan2_det(0.0, 0.0), 0.0);
        assert_eq!(atan2_det(1.0, 0.0), std::f64::consts::FRAC_PI_2);
        assert_eq!(atan2_det(0.0, -1.0), std::f64::consts::PI);
    }

    #[test]
    fn look_angles_round_trip() {
        for d in [
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::new(0.3, -0.8, 0.2),
            Vec3::new(-5.0, 2.0, -3.0),
        ] {
            let (p, y) = look_angles(d);
            let (f, _, _) = angle_vectors(p, y);
            assert!((f - d.normalize()).length() < 1e-6, "{d:?} -> {f:?}");
        }
    }

    #[test]
    fn vec_ops() {
        let a = Vec3::new(3.0, 4.0, 0.0);
        assert_eq!(a.length(), 5.0);
        assert_eq!(a.normalize_len(), (Vec3::new(0.6, 0.8, 0.0), 5.0));
        assert_eq!(Vec3::ZERO.normalize(), Vec3::ZERO);
        assert_eq!(
            Vec3::new(1.0, 0.0, 0.0).cross(Vec3::new(0.0, 1.0, 0.0)),
            Vec3::new(0.0, 0.0, 1.0)
        );
        assert_eq!(
            a.mad(2.0, Vec3::new(1.0, 1.0, 1.0)),
            Vec3::new(5.0, 6.0, 2.0)
        );
    }
}
