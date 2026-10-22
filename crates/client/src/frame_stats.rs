//! Frame-time statistics (p50/p99) for perf logging.

use std::time::Duration;

#[derive(Debug, Default, Clone)]
pub struct FrameStats {
    samples_us: Vec<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Summary {
    pub frames: usize,
    pub p50_ms: f64,
    pub p99_ms: f64,
    pub max_ms: f64,
    pub avg_fps: f64,
}

impl FrameStats {
    pub fn record(&mut self, d: Duration) {
        self.samples_us
            .push(d.as_micros().min(u32::MAX as u128) as u32);
    }

    /// Nearest-rank percentiles over all recorded frames.
    pub fn summary(&self) -> Option<Summary> {
        if self.samples_us.is_empty() {
            return None;
        }
        let mut s = self.samples_us.clone();
        s.sort_unstable();
        let rank = |p: f64| {
            let idx = ((p / 100.0) * s.len() as f64).ceil() as usize;
            s[idx.clamp(1, s.len()) - 1] as f64 / 1000.0
        };
        let total_us: u64 = s.iter().map(|&v| v as u64).sum();
        Some(Summary {
            frames: s.len(),
            p50_ms: rank(50.0),
            p99_ms: rank(99.0),
            max_ms: *s.last().unwrap() as f64 / 1000.0,
            avg_fps: if total_us == 0 {
                0.0
            } else {
                s.len() as f64 * 1e6 / total_us as f64
            },
        })
    }
}

impl std::fmt::Display for Summary {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "frames {} | frame time p50 {:.3} ms, p99 {:.3} ms, max {:.3} ms | avg {:.0} fps",
            self.frames, self.p50_ms, self.p99_ms, self.max_ms, self.avg_fps
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentiles() {
        let mut fs = FrameStats::default();
        assert!(fs.summary().is_none());
        for ms in 1..=100 {
            fs.record(Duration::from_millis(ms));
        }
        let s = fs.summary().unwrap();
        assert_eq!(s.frames, 100);
        assert_eq!(s.p50_ms, 50.0);
        assert_eq!(s.p99_ms, 99.0);
        assert_eq!(s.max_ms, 100.0);
        assert!((s.avg_fps - 100.0 / 5.05).abs() < 1e-9);
    }
}
