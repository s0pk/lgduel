//! End-of-fight summary: when a fight involving the viewed player ends, compare
//! both players' stats with where they stood when the fight began.

use crate::scene::FrameView;
use lgduel_sim::game::{GameEvent, PlayerId, PlayerStats};

/// One finished fight, from the viewed player's side.
#[derive(Debug, Clone, PartialEq)]
pub struct FightSummary {
    pub won: bool,
    pub secs: f32,
    /// LG accuracy during this fight (percent), own and opponent's.
    pub acc: f32,
    pub opp_acc: f32,
    pub dealt: u32,
    pub taken: u32,
}

impl FightSummary {
    /// The lines the HUD shows.
    pub fn lines(&self) -> [String; 3] {
        [
            format!(
                "{} IN {:.1} SEC",
                if self.won { "YOU WON" } else { "YOU LOST" },
                self.secs
            ),
            format!("LG {:.0}% VS {:.0}%", self.acc, self.opp_acc),
            format!("DMG {} / {}", self.dealt, self.taken),
        ]
    }
}

/// How long a summary stays on screen.
pub const SHOW_SECS: f32 = 4.0;

#[derive(Debug, Default)]
pub struct FightTracker {
    /// Everyone's stats when the current fight began.
    base: Vec<(PlayerId, PlayerStats)>,
    /// Latest summary and how long it has been shown.
    pub shown: Option<(FightSummary, f32)>,
}

fn since(now: PlayerStats, then: PlayerStats) -> PlayerStats {
    PlayerStats {
        shots: now.shots.saturating_sub(then.shots),
        hits: now.hits.saturating_sub(then.hits),
        damage_dealt: now.damage_dealt.saturating_sub(then.damage_dealt),
        damage_taken: now.damage_taken.saturating_sub(then.damage_taken),
        ..PlayerStats::default()
    }
}

impl FightTracker {
    /// Feed one frame (`dt` seconds since the last one).
    pub fn update(&mut self, fv: &FrameView, tick_hz: u32, dt: f32) {
        if let Some((_, t)) = &mut self.shown {
            *t += dt;
            if *t > SHOW_SECS {
                self.shown = None;
            }
        }
        let stat = |list: &[(PlayerId, PlayerStats)], id| {
            list.iter().find(|(p, _)| *p == id).map(|(_, s)| *s)
        };
        for e in &fv.events {
            let GameEvent::FightEnd {
                winner,
                loser,
                ticks,
            } = e
            else {
                continue;
            };
            let (won, opp) = match fv.me {
                m if m == *winner => (true, *loser),
                m if m == *loser => (false, *winner),
                _ => continue,
            };
            let (Some(me_now), Some(opp_now)) = (stat(&fv.stats, fv.me), stat(&fv.stats, opp))
            else {
                continue;
            };
            let me = since(me_now, stat(&self.base, fv.me).unwrap_or_default());
            let op = since(opp_now, stat(&self.base, opp).unwrap_or_default());
            self.shown = Some((
                FightSummary {
                    won,
                    secs: *ticks as f32 / tick_hz.max(1) as f32,
                    acc: me.accuracy_pct(),
                    opp_acc: op.accuracy_pct(),
                    dealt: me.damage_dealt,
                    taken: me.damage_taken,
                },
                0.0,
            ));
            self.base.clear();
        }
        // The next fight starts from here; players new to the list start now.
        for (id, s) in &fv.stats {
            if stat(&self.base, *id).is_none() {
                self.base.push((*id, *s));
            }
        }
        self.base
            .retain(|(id, _)| fv.stats.iter().any(|(p, _)| p == id));
    }

    pub fn summary(&self) -> Option<&FightSummary> {
        self.shown.as_ref().map(|(s, _)| s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hud::HudInfo;
    use lgduel_sim::math::Vec3;

    fn stats(shots: u32, hits: u32, dealt: u32, taken: u32) -> PlayerStats {
        PlayerStats {
            shots,
            hits,
            damage_dealt: dealt,
            damage_taken: taken,
            ..PlayerStats::default()
        }
    }

    fn frame(me: u8, a: PlayerStats, b: PlayerStats, events: Vec<GameEvent>) -> FrameView {
        FrameView {
            me: PlayerId(me),
            me_alive: true,
            eye: Vec3::ZERO,
            others: vec![],
            hud: HudInfo::default(),
            events,
            spectate: None,
            status: None,
            stats: vec![(PlayerId(0), a), (PlayerId(1), b)],
        }
    }

    #[test]
    fn summary_counts_only_this_fight() {
        let mut t = FightTracker::default();
        // Before the fight: earlier totals.
        t.update(
            &frame(0, stats(10, 5, 35, 70), stats(10, 2, 70, 35), vec![]),
            125,
            0.01,
        );
        assert!(t.summary().is_none());
        let end = GameEvent::FightEnd {
            winner: PlayerId(0),
            loser: PlayerId(1),
            ticks: 1000,
        };
        // This fight: we hit 10 of 20 (+70 dmg), they hit 4 of 20 (+28).
        t.update(
            &frame(
                0,
                stats(30, 15, 105, 98),
                stats(30, 6, 98, 105),
                vec![end.clone()],
            ),
            125,
            0.01,
        );
        let s = t.summary().unwrap().clone();
        assert_eq!(
            s,
            FightSummary {
                won: true,
                secs: 8.0,
                acc: 50.0,
                opp_acc: 20.0,
                dealt: 70,
                taken: 28,
            }
        );
        assert_eq!(s.lines()[0], "YOU WON IN 8.0 SEC");
        // Seen from the loser's side, and the next fight starts from zero.
        let mut l = FightTracker::default();
        l.update(
            &frame(1, stats(0, 0, 0, 0), stats(0, 0, 0, 0), vec![]),
            125,
            0.01,
        );
        l.update(
            &frame(1, stats(20, 10, 70, 28), stats(20, 4, 28, 70), vec![end]),
            125,
            0.01,
        );
        assert!(!l.summary().unwrap().won);
        assert_eq!(l.summary().unwrap().dealt, 28);
        // Disappears after a few seconds.
        for _ in 0..50 {
            t.update(
                &frame(0, stats(30, 15, 105, 98), stats(30, 6, 98, 105), vec![]),
                125,
                0.1,
            );
        }
        assert!(t.summary().is_none());
    }

    #[test]
    fn other_players_fights_are_ignored() {
        let mut t = FightTracker::default();
        let end = GameEvent::FightEnd {
            winner: PlayerId(5),
            loser: PlayerId(6),
            ticks: 100,
        };
        t.update(
            &frame(0, stats(1, 1, 7, 0), stats(1, 0, 0, 7), vec![end]),
            125,
            0.01,
        );
        assert!(t.summary().is_none());
    }
}
