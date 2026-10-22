//! Interpolation of other players between snapshots.
//!
//! Remote players are shown `delay` ticks behind the *snapshot clock*
//! (`NetClient::snapshot_clock`: the newest snapshot tick that could have
//! arrived by now on the fastest observed path), blended between the two
//! received snapshots that bracket that render time. With a delay of a couple of snapshot intervals, a late or lost
//! snapshot just widens the bracket instead of stalling the player.
//! Extrapolation (guessing past the newest snapshot) is off by default; when
//! the buffer runs dry the player holds at the newest known position, unless
//! `max_extrapolation` allows a short velocity-based guess.

use lgduel_sim::game::{Player, PlayerId};
use lgduel_sim::math::{Angle, Vec3};
use std::collections::VecDeque;

/// Teleports (respawn) are not blended across.
const TELEPORT_DIST: f32 = 128.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct InterpSettings {
    /// How far behind the estimated server time to render, in ticks.
    pub delay_ticks: f64,
    /// Max ticks to extrapolate past the newest snapshot (0 = off).
    pub max_extrapolation_ticks: f64,
    /// Server tick rate (converts velocity to per-tick motion when extrapolating).
    pub tick_hz: f64,
    /// Adapt `delay_ticks` toward 1 + measured arrival spread (see `set_target_delay`).
    pub adaptive: bool,
}

impl Default for InterpSettings {
    fn default() -> Self {
        Self {
            delay_ticks: 3.0,
            max_extrapolation_ticks: 0.0,
            tick_hz: 125.0,
            adaptive: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RemoteView {
    pub origin: Vec3,
    pub pitch: Angle,
    pub yaw: Angle,
    pub ducked: bool,
    pub alive: bool,
    pub buttons: u8,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct InterpStats {
    pub samples: u64,
    /// Render times that fell past the newest snapshot (buffer ran dry).
    pub starved: u64,
}

/// Snapshot positions of remote players, by tick.
#[derive(Debug, Clone, Default)]
pub struct Interpolator {
    pub settings: InterpSettings,
    /// (tick, players) — ascending tick order.
    frames: VecDeque<(u32, Vec<Player>)>,
    pub stats: InterpStats,
    target_delay: Option<f64>,
    /// Minimum delay learned from starvation; decays ~1 tick per second.
    starve_floor: f64,
    last_sample_tick: Option<f64>,
}

fn lerp_angle(a: Angle, b: Angle, t: f64) -> Angle {
    // Shortest way around, in angle units.
    let d = b.0.wrapping_sub(a.0) as i32 as f64;
    Angle(a.0.wrapping_add((d * t).round() as i64 as u32))
}

fn view(p: &Player) -> RemoteView {
    RemoteView {
        origin: p.pm.origin,
        pitch: p.pitch,
        yaw: p.yaw,
        ducked: p.pm.ducked,
        alive: p.alive,
        buttons: p.buttons,
    }
}

impl Interpolator {
    pub fn new(settings: InterpSettings) -> Self {
        Self {
            settings,
            ..Default::default()
        }
    }

    /// Add a received snapshot's players (any order; duplicates ignored).
    pub fn push(&mut self, tick: u32, players: &[Player]) {
        let pos = self.frames.partition_point(|(t, _)| *t < tick);
        if self.frames.get(pos).is_some_and(|(t, _)| *t == tick) {
            return;
        }
        self.frames.insert(pos, (tick, players.to_vec()));
        while self.frames.len() > 128 {
            self.frames.pop_front();
        }
    }

    /// Desired delay (ticks) when `adaptive`. The delay grows toward it at up
    /// to 25% of elapsed time (running dry is visible) and shrinks at ≤ 5%
    /// (a slightly large buffer is not); starving also raises the target.
    pub fn set_target_delay(&mut self, ticks: f64) {
        self.target_delay = Some(ticks.max(0.0));
    }

    pub fn newest_tick(&self) -> Option<u32> {
        self.frames.back().map(|(t, _)| *t)
    }

    /// Where `id` should be drawn at estimated server time `server_tick`
    /// (fractional ticks).
    pub fn sample(&mut self, id: PlayerId, server_tick: f64) -> Option<RemoteView> {
        if self.settings.adaptive
            && let Some(target) = self.target_delay
        {
            let elapsed = self
                .last_sample_tick
                .map_or(0.0, |l| (server_tick - l).max(0.0));
            self.starve_floor = (self.starve_floor - elapsed / self.settings.tick_hz).max(0.0);
            let target = target.max(self.starve_floor);
            self.settings.delay_ticks +=
                (target - self.settings.delay_ticks).clamp(-0.05 * elapsed, 0.25 * elapsed);
        }
        self.last_sample_tick = Some(server_tick);
        let t = server_tick - self.settings.delay_ticks;
        self.stats.samples += 1;
        let find = |f: &(u32, Vec<Player>)| f.1.iter().find(|p| p.id == id).cloned();
        // Bracketing frames containing this player.
        let after = self.frames.iter().position(|(ft, _)| *ft as f64 >= t);
        match after {
            Some(0) => self.frames.front().and_then(find).map(|p| view(&p)),
            Some(i) => {
                let (ta, pa) = (self.frames[i - 1].0, find(&self.frames[i - 1]));
                let (tb, pb) = (self.frames[i].0, find(&self.frames[i]));
                match (pa, pb) {
                    (Some(a), Some(b)) => {
                        let f = ((t - ta as f64) / (tb - ta) as f64).clamp(0.0, 1.0);
                        if (b.pm.origin - a.pm.origin).length() > TELEPORT_DIST
                            || a.alive != b.alive
                        {
                            return Some(view(if f < 1.0 { &a } else { &b }));
                        }
                        let ff = f as f32;
                        Some(RemoteView {
                            origin: a.pm.origin + (b.pm.origin - a.pm.origin) * ff,
                            pitch: lerp_angle(a.pitch, b.pitch, f),
                            yaw: lerp_angle(a.yaw, b.yaw, f),
                            ducked: if f < 0.5 { a.pm.ducked } else { b.pm.ducked },
                            alive: b.alive,
                            buttons: if f < 0.5 { a.buttons } else { b.buttons },
                        })
                    }
                    (_, Some(b)) => Some(view(&b)),
                    (Some(a), None) => Some(view(&a)),
                    (None, None) => None,
                }
            }
            None => {
                // Render time is past the newest snapshot.
                self.stats.starved += 1;
                if self.settings.adaptive {
                    let short = t - self.newest_tick()? as f64;
                    self.starve_floor = self
                        .starve_floor
                        .max(self.settings.delay_ticks + short + 0.5);
                }
                let (nt, players) = self.frames.back()?;
                let p = players.iter().find(|p| p.id == id)?;
                let mut v = view(p);
                let ahead = (t - *nt as f64).min(self.settings.max_extrapolation_ticks);
                if ahead > 0.0 && p.alive {
                    v.origin += p.pm.velocity * (ahead / self.settings.tick_hz) as f32;
                }
                Some(v)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lgduel_sim::game::PlayerStats;
    use lgduel_sim::pmove::PlayerMove;

    fn player(x: f32, yaw_deg: f64) -> Player {
        Player {
            id: PlayerId(1),
            pm: PlayerMove {
                origin: Vec3::new(x, 0.0, 24.0),
                velocity: Vec3::new(125.0, 0.0, 0.0),
                ..Default::default()
            },
            pitch: Angle::ZERO,
            yaw: Angle::from_degrees(yaw_deg),
            health: 100,
            armor: 0,
            alive: true,
            respawn_tick: None,
            weapon_time_us: 0,
            lifesteal_rem: 0,
            buttons: 0,
            frags: 0,
            stats: PlayerStats::default(),
        }
    }

    #[test]
    fn blends_between_bracketing_snapshots() {
        let mut i = Interpolator::new(InterpSettings {
            delay_ticks: 2.0,
            max_extrapolation_ticks: 0.0,
            tick_hz: 125.0,
            adaptive: false,
        });
        i.push(10, &[player(0.0, 350.0)]);
        i.push(11, &[player(1.0, 10.0)]);
        i.push(12, &[player(2.0, 30.0)]);
        // server 12.5 - delay 2 = 10.5 → halfway between 10 and 11.
        let v = i.sample(PlayerId(1), 12.5).unwrap();
        assert!((v.origin.x - 0.5).abs() < 1e-6);
        // Yaw 350° → 10° goes the short way through 0°.
        assert!(
            (v.yaw.degrees() - 0.0).abs() < 1e-3 || (v.yaw.degrees() - 360.0).abs() < 1e-3,
            "{}",
            v.yaw.degrees()
        );
        assert_eq!(i.stats.starved, 0);
    }

    #[test]
    fn lost_snapshot_widens_bracket() {
        let mut i = Interpolator::new(InterpSettings {
            delay_ticks: 2.0,
            max_extrapolation_ticks: 0.0,
            tick_hz: 125.0,
            adaptive: false,
        });
        i.push(10, &[player(0.0, 0.0)]);
        i.push(14, &[player(4.0, 0.0)]); // 11..13 lost
        i.push(15, &[player(5.0, 0.0)]);
        let v = i.sample(PlayerId(1), 13.0).unwrap(); // t = 11
        assert!((v.origin.x - 1.0).abs() < 1e-6);
    }

    #[test]
    fn out_of_order_and_duplicates() {
        let mut i = Interpolator::default();
        i.push(12, &[player(2.0, 0.0)]);
        i.push(10, &[player(0.0, 0.0)]);
        i.push(11, &[player(1.0, 0.0)]);
        i.push(11, &[player(99.0, 0.0)]);
        assert_eq!(i.newest_tick(), Some(12));
        let v = i.sample(PlayerId(1), 14.0).unwrap(); // delay 3 → t = 11
        assert_eq!(v.origin.x, 1.0);
    }

    #[test]
    fn starved_holds_unless_extrapolation_enabled() {
        let mut i = Interpolator::new(InterpSettings {
            delay_ticks: 1.0,
            max_extrapolation_ticks: 0.0,
            tick_hz: 125.0,
            adaptive: false,
        });
        i.push(10, &[player(0.0, 0.0)]);
        let v = i.sample(PlayerId(1), 15.0).unwrap();
        assert_eq!(v.origin.x, 0.0);
        assert_eq!(i.stats.starved, 1);
        i.settings.max_extrapolation_ticks = 2.0;
        let v = i.sample(PlayerId(1), 15.0).unwrap(); // capped at 2 ticks × 125 u/s / 125 Hz
        assert!((v.origin.x - 2.0).abs() < 1e-5, "{}", v.origin.x);
    }

    #[test]
    fn teleport_not_smeared() {
        let mut i = Interpolator::new(InterpSettings {
            delay_ticks: 0.0,
            max_extrapolation_ticks: 0.0,
            tick_hz: 125.0,
            adaptive: false,
        });
        i.push(10, &[player(0.0, 0.0)]);
        i.push(11, &[player(500.0, 0.0)]);
        let v = i.sample(PlayerId(1), 10.5).unwrap();
        assert!(v.origin.x == 0.0 || v.origin.x == 500.0);
    }
}
