//! Fixed-rate tick scheduling. The sim never reads a clock: the caller feeds
//! elapsed real time and runs as many ticks as this returns.

use serde::{Deserialize, Serialize};

/// Simulation tick index.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
)]
pub struct Tick(pub u32);

impl Tick {
    pub fn next(self) -> Tick {
        Tick(self.0.wrapping_add(1))
    }
}

/// Converts elapsed microseconds into whole ticks with no rounding drift,
/// even when the rate does not divide one second (e.g. 60 Hz).
#[derive(Debug, Clone)]
pub struct FixedStepper {
    rate_hz: u32,
    /// Accumulated time in units of 1/(1e6 * rate) s, always < 1e6.
    acc: u64,
    /// Upper bound on ticks returned per `advance` (avoids a catch-up spiral after a stall).
    max_catch_up: u32,
    /// Next tick to run.
    tick: Tick,
}

const MICROS: u64 = 1_000_000;

impl FixedStepper {
    pub fn new(rate_hz: u32) -> Self {
        assert!(rate_hz > 0);
        Self {
            rate_hz,
            acc: 0,
            max_catch_up: rate_hz.div_ceil(4).max(1),
            tick: Tick(0),
        }
    }

    pub fn with_max_catch_up(mut self, ticks: u32) -> Self {
        self.max_catch_up = ticks.max(1);
        self
    }

    pub fn rate_hz(&self) -> u32 {
        self.rate_hz
    }

    /// Next tick that will run.
    pub fn tick(&self) -> Tick {
        self.tick
    }

    /// Add elapsed time; returns the range of ticks to simulate now
    /// (`start..start + count`). Time beyond the catch-up cap is dropped.
    pub fn advance(&mut self, elapsed_us: u64) -> (Tick, u32) {
        let start = self.tick;
        self.acc += elapsed_us * self.rate_hz as u64;
        let whole = self.acc / MICROS;
        self.acc %= MICROS;
        let count = if whole > self.max_catch_up as u64 {
            self.acc = 0;
            self.max_catch_up
        } else {
            whole as u32
        };
        self.tick = Tick(start.0.wrapping_add(count));
        (start, count)
    }

    /// Fraction of the way to the next tick, in [0, 1), for render interpolation.
    pub fn alpha(&self) -> f32 {
        (self.acc as f64 / MICROS as f64) as f32
    }

    /// Tick duration in seconds as the sim uses it.
    pub fn dt(&self) -> f32 {
        1.0 / self.rate_hz as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_at_125hz() {
        let mut s = FixedStepper::new(125);
        assert_eq!(s.advance(7_999), (Tick(0), 0));
        assert_eq!(s.advance(1), (Tick(0), 1));
        assert_eq!(s.advance(16_000), (Tick(1), 2));
        assert_eq!(s.tick(), Tick(3));
        assert_eq!(s.alpha(), 0.0);
    }

    #[test]
    fn no_drift_at_60hz() {
        let mut s = FixedStepper::new(60).with_max_catch_up(1_000);
        let mut total = 0;
        // 1 hour of 1 ms frames.
        for _ in 0..3_600_000 {
            total += s.advance(1_000).1;
        }
        assert_eq!(total, 60 * 3600);
    }

    #[test]
    fn irregular_frames_sum_exactly() {
        let mut s = FixedStepper::new(125).with_max_catch_up(10_000);
        let frames = [1_u64, 3_333, 17, 999_999, 2, 6_648];
        let total: u32 = frames.iter().map(|&f| s.advance(f).1).sum();
        let us: u64 = frames.iter().sum();
        assert_eq!(total as u64, us * 125 / 1_000_000);
    }

    #[test]
    fn stall_is_capped() {
        let mut s = FixedStepper::new(125).with_max_catch_up(10);
        assert_eq!(s.advance(5_000_000).1, 10);
        assert_eq!(s.alpha(), 0.0);
        assert_eq!(s.advance(8_000).1, 1);
    }

    #[test]
    fn alpha_tracks_partial_tick() {
        let mut s = FixedStepper::new(125);
        s.advance(4_000);
        assert_eq!(s.alpha(), 0.5);
    }
}
