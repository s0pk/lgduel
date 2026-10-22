//! Mouse look and key state → `UserCmd`.
//!
//! Mouse deltas are raw device counts (no OS acceleration). Sensitivity is
//! cm/360 at a given DPI. Fractions of an angle unit are carried between events
//! so no motion is ever lost (sub-pixel accumulation).

use lgduel_sim::math::Angle;
use lgduel_sim::tick::Tick;
use lgduel_sim::usercmd::{PITCH_LIMIT_DEG, UserCmd, buttons};

const UNITS_PER_TURN: f64 = 4_294_967_296.0;

#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct MouseSettings {
    pub dpi: f64,
    /// Centimetres of mouse travel for a 360° turn.
    pub cm_per_360: f64,
    pub invert_pitch: bool,
}

impl Default for MouseSettings {
    fn default() -> Self {
        Self {
            dpi: 800.0,
            cm_per_360: 30.0,
            invert_pitch: false,
        }
    }
}

impl MouseSettings {
    /// Angle units per mouse count.
    pub fn units_per_count(&self) -> f64 {
        let counts_per_turn = self.cm_per_360 / 2.54 * self.dpi;
        UNITS_PER_TURN / counts_per_turn
    }

    /// Degrees per count (Quake's `m_yaw * sensitivity` equivalent).
    pub fn degrees_per_count(&self) -> f64 {
        360.0 / (self.cm_per_360 / 2.54 * self.dpi)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MouseLook {
    pub settings: MouseSettings,
    pub yaw: Angle,
    pub pitch: Angle,
    rem_yaw: f64,
    rem_pitch: f64,
}

impl MouseLook {
    pub fn new(settings: MouseSettings, yaw: Angle) -> Self {
        Self {
            settings,
            yaw,
            pitch: Angle::ZERO,
            rem_yaw: 0.0,
            rem_pitch: 0.0,
        }
    }

    /// Apply a raw mouse delta in counts (+x right, +y down).
    pub fn apply(&mut self, dx: f64, dy: f64) {
        let upc = self.settings.units_per_count();
        // Moving right turns right = yaw decreases (yaw is counter-clockwise).
        let y = -dx * upc + self.rem_yaw;
        let whole_y = y.trunc();
        self.rem_yaw = y - whole_y;
        self.yaw = Angle(self.yaw.0.wrapping_add(whole_y as i64 as u32));

        let sign = if self.settings.invert_pitch {
            -1.0
        } else {
            1.0
        };
        // Moving down looks down = pitch increases.
        let p = sign * dy * upc + self.rem_pitch;
        let whole_p = p.trunc();
        self.rem_pitch = p - whole_p;
        let limit = Angle::from_degrees(PITCH_LIMIT_DEG).0 as i64;
        let pitch = (self.pitch.0 as i32 as i64 + whole_p as i64).clamp(-limit, limit);
        if pitch.abs() == limit {
            self.rem_pitch = 0.0;
        }
        self.pitch = Angle(pitch as i32 as u32);
    }
}

/// Movement/action key state.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Keys {
    pub forward: bool,
    pub back: bool,
    pub left: bool,
    pub right: bool,
    pub jump: bool,
    pub crouch: bool,
    pub fire: bool,
}

impl Keys {
    pub fn cmd(&self, tick: Tick, look: &MouseLook) -> UserCmd {
        let mut b = 0;
        if self.jump {
            b |= buttons::JUMP;
        }
        if self.crouch {
            b |= buttons::CROUCH;
        }
        if self.fire {
            b |= buttons::FIRE;
        }
        UserCmd::from_keys(
            tick,
            look.pitch,
            look.yaw,
            self.forward,
            self.back,
            self.left,
            self.right,
            b,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_turn_returns_to_start() {
        let s = MouseSettings {
            dpi: 800.0,
            cm_per_360: 30.0,
            invert_pitch: false,
        };
        let mut m = MouseLook::new(s, Angle::from_degrees(10.0));
        let counts = 30.0 / 2.54 * 800.0; // 9448.8…
        // Feed in uneven chunks, including fractional counts.
        let mut left = counts;
        let mut step: f64 = 0.37;
        while left > 0.0 {
            let d = step.min(left);
            m.apply(d, 0.0);
            left -= d;
            step = (step * 1.7) % 13.0 + 0.01;
        }
        let diff = (m.yaw.0 as i64 - Angle::from_degrees(10.0).0 as i64).abs();
        assert!(diff <= 2, "drift {diff} units after a full turn");
    }

    #[test]
    fn subpixel_motion_is_not_lost() {
        let s = MouseSettings {
            dpi: 400.0,
            cm_per_360: 60.0,
            invert_pitch: false,
        };
        let mut a = MouseLook::new(s, Angle::ZERO);
        let mut b = MouseLook::new(s, Angle::ZERO);
        for _ in 0..10_000 {
            a.apply(0.1, 0.0);
        }
        b.apply(1000.0, 0.0);
        assert!((a.yaw.0 as i64 - b.yaw.0 as i64).abs() <= 2);
    }

    #[test]
    fn directions_and_pitch_clamp() {
        let mut m = MouseLook::new(MouseSettings::default(), Angle::ZERO);
        m.apply(100.0, 0.0);
        assert!(m.yaw.signed_degrees() < 0.0, "right turns clockwise");
        m.apply(0.0, 100.0);
        assert!(m.pitch.signed_degrees() > 0.0, "down looks down");
        m.apply(0.0, 1e9);
        assert!((m.pitch.signed_degrees() - 89.0).abs() < 1e-6);
        m.apply(0.0, -1e9);
        assert!((m.pitch.signed_degrees() + 89.0).abs() < 1e-6);
        let mut inv = MouseLook::new(
            MouseSettings {
                invert_pitch: true,
                ..Default::default()
            },
            Angle::ZERO,
        );
        inv.apply(0.0, 100.0);
        assert!(inv.pitch.signed_degrees() < 0.0);
    }

    #[test]
    fn sensitivity_math() {
        let s = MouseSettings {
            dpi: 1600.0,
            cm_per_360: 25.4,
            invert_pitch: false,
        };
        // 25.4 cm = 10 in → 16000 counts per turn.
        assert!((s.degrees_per_count() - 360.0 / 16000.0).abs() < 1e-12);
    }

    #[test]
    fn keys_to_cmd() {
        let m = MouseLook::new(MouseSettings::default(), Angle::from_degrees(45.0));
        let k = Keys {
            forward: true,
            left: true,
            fire: true,
            ..Default::default()
        };
        let c = k.cmd(Tick(3), &m);
        assert_eq!((c.forward, c.right), (127, -127));
        assert!(c.pressed(buttons::FIRE) && !c.pressed(buttons::JUMP));
        assert_eq!(c.yaw, m.yaw);
    }
}
