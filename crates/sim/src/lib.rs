//! Deterministic game simulation shared by client and server.
//!
//! Pure: no I/O, no rendering, no wall clocks.

pub mod config;
pub mod math;
pub mod tick;
pub mod trace;
pub mod usercmd;
pub mod world;

/// Default fixed simulation rate in Hz (server-configurable).
pub const DEFAULT_TICK_RATE_HZ: u32 = 125;

/// Duration of one fixed tick in microseconds for a given rate.
pub fn tick_micros(rate_hz: u32) -> u32 {
    1_000_000 / rate_hz
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_tick_is_8ms() {
        assert_eq!(tick_micros(DEFAULT_TICK_RATE_HZ), 8_000);
    }
}
