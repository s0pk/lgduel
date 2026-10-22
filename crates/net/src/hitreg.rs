//! Hit-registration agreement: did the server agree with what the shooter saw?
//!
//! For every command with FIRE held, the client records whether its own beam
//! (from its predicted eye, against remote players at their interpolated
//! positions) hit someone. When the server's `Fire` event for that tick comes
//! back, the two verdicts are compared. With snapshots every tick, the events
//! in a snapshot are those of tick `snapshot.tick - 1`.

use crate::snapshot::Snapshot;
use lgduel_sim::game::{GameEvent, PlayerId};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct HitRegStats {
    /// Shots where client and server agreed (both hit or both missed).
    pub agree: u64,
    /// Client saw a hit, server says miss ("I hit him on my screen!").
    pub false_hit: u64,
    /// Client saw a miss, server says hit.
    pub false_miss: u64,
}

impl HitRegStats {
    pub fn shots(&self) -> u64 {
        self.agree + self.false_hit + self.false_miss
    }

    /// Agreement in percent (100 if nothing compared yet).
    pub fn agreement_pct(&self) -> f64 {
        let n = self.shots();
        if n == 0 {
            100.0
        } else {
            self.agree as f64 * 100.0 / n as f64
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct HitReg {
    /// Client-side verdict per command tick (true = saw a hit).
    seen: BTreeMap<u32, bool>,
    pub stats: HitRegStats,
}

impl HitReg {
    /// Record what the client saw for the command of `tick` (FIRE held).
    pub fn record(&mut self, tick: u32, saw_hit: bool) {
        self.seen.insert(tick, saw_hit);
        while self.seen.len() > 512 {
            self.seen.pop_first();
        }
    }

    /// Compare against the server's verdicts in a received snapshot.
    pub fn on_snapshot(&mut self, me: PlayerId, snap: &Snapshot) {
        let tick = snap.tick.0.wrapping_sub(1);
        for e in &snap.events {
            if let GameEvent::Fire {
                shooter, target, ..
            } = e
                && *shooter == me
                && let Some(saw) = self.seen.remove(&tick)
            {
                match (saw, target.is_some()) {
                    (a, b) if a == b => self.stats.agree += 1,
                    (true, false) => self.stats.false_hit += 1,
                    _ => self.stats.false_miss += 1,
                }
            }
        }
        // Verdicts older than this snapshot can no longer be matched.
        self.seen.retain(|t, _| *t >= tick);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lgduel_sim::math::Vec3;
    use lgduel_sim::tick::Tick;

    fn snap(tick: u32, target: Option<PlayerId>) -> Snapshot {
        Snapshot {
            tick: Tick(tick),
            fight_start: None,
            players: vec![],
            events: vec![GameEvent::Fire {
                shooter: PlayerId(0),
                start: Vec3::ZERO,
                end: Vec3::ZERO,
                target,
            }],
            last_cmd: None,
        }
    }

    #[test]
    fn classifies_outcomes() {
        let mut h = HitReg::default();
        h.record(10, true);
        h.record(11, true);
        h.record(12, false);
        h.record(13, false);
        h.on_snapshot(PlayerId(0), &snap(11, Some(PlayerId(1)))); // tick 10: hit/hit
        h.on_snapshot(PlayerId(0), &snap(12, None)); // tick 11: saw hit, server miss
        h.on_snapshot(PlayerId(0), &snap(13, Some(PlayerId(1)))); // tick 12: saw miss, server hit
        h.on_snapshot(PlayerId(0), &snap(14, None)); // tick 13: miss/miss
        assert_eq!(
            h.stats,
            HitRegStats {
                agree: 2,
                false_hit: 1,
                false_miss: 1
            }
        );
        assert_eq!(h.stats.agreement_pct(), 50.0);
        // Other players' shots are ignored.
        h.record(20, true);
        h.on_snapshot(PlayerId(5), &snap(21, None));
        assert_eq!(h.stats.shots(), 4);
    }
}
