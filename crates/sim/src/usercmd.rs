//! Per-tick player input. This exact struct is what the client sends and the
//! server simulates, so prediction sees identical values on both sides.

use crate::math::Angle;
use crate::tick::Tick;
use serde::{Deserialize, Serialize};

/// Button bit flags.
pub mod buttons {
    pub const JUMP: u8 = 1 << 0;
    pub const CROUCH: u8 = 1 << 1;
    pub const FIRE: u8 = 1 << 2;
}

/// Max magnitude of a move axis (Quake's ±127 convention).
pub const MOVE_MAX: i8 = 127;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct UserCmd {
    /// Tick this command is for.
    pub tick: Tick,
    /// Pitch, positive = look down, clamped to ±`PITCH_LIMIT_DEG`.
    pub pitch: Angle,
    pub yaw: Angle,
    /// -127..=127, positive = forward.
    pub forward: i8,
    /// -127..=127, positive = right.
    pub right: i8,
    pub buttons: u8,
}

/// Pitch limit in degrees (Quake clamps just short of straight up/down).
pub const PITCH_LIMIT_DEG: f64 = 89.0;

impl UserCmd {
    /// Build from keyboard state; opposite keys cancel.
    #[allow(clippy::too_many_arguments)]
    pub fn from_keys(
        tick: Tick,
        pitch: Angle,
        yaw: Angle,
        fwd: bool,
        back: bool,
        left: bool,
        right: bool,
        buttons: u8,
    ) -> Self {
        let axis = |pos: bool, neg: bool| match (pos, neg) {
            (true, false) => MOVE_MAX,
            (false, true) => -MOVE_MAX,
            _ => 0,
        };
        Self {
            tick,
            pitch,
            yaw,
            forward: axis(fwd, back),
            right: axis(right, left),
            buttons,
        }
        .sanitized()
    }

    /// Enforce invariants on untrusted input (server side): pitch clamped,
    /// move axes never -128, unknown buttons cleared.
    pub fn sanitized(mut self) -> Self {
        self.pitch = self
            .pitch
            .clamp_signed(Angle::from_degrees(PITCH_LIMIT_DEG));
        self.forward = self.forward.max(-MOVE_MAX);
        self.right = self.right.max(-MOVE_MAX);
        self.buttons &= buttons::JUMP | buttons::CROUCH | buttons::FIRE;
        self
    }

    pub fn pressed(&self, button: u8) -> bool {
        self.buttons & button != 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_map_to_axes() {
        let c = UserCmd::from_keys(
            Tick(5),
            Angle::ZERO,
            Angle::ZERO,
            true,
            false,
            true,
            true,
            buttons::JUMP,
        );
        assert_eq!((c.forward, c.right), (127, 0));
        assert!(c.pressed(buttons::JUMP) && !c.pressed(buttons::FIRE));
        let c = UserCmd::from_keys(
            Tick(5),
            Angle::ZERO,
            Angle::ZERO,
            false,
            true,
            true,
            false,
            0,
        );
        assert_eq!((c.forward, c.right), (-127, -127));
    }

    #[test]
    fn sanitize_untrusted() {
        let c = UserCmd {
            tick: Tick(1),
            pitch: Angle::from_degrees(120.0),
            yaw: Angle::from_degrees(10.0),
            forward: i8::MIN,
            right: i8::MIN,
            buttons: 0xFF,
        }
        .sanitized();
        assert!((c.pitch.signed_degrees() - 89.0).abs() < 1e-6);
        assert_eq!((c.forward, c.right), (-127, -127));
        assert_eq!(c.buttons, 0b111);
        assert_eq!(c.yaw, Angle::from_degrees(10.0));
    }
}
