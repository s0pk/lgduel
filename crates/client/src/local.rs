//! Offline session: runs the sim locally at its fixed tick rate, independent
//! of render rate, and interpolates the own view between ticks.

use lgduel_sim::config::GameConfig;
use lgduel_sim::game::{Game, GameEvent, PlayerId};
use lgduel_sim::math::Vec3;
use lgduel_sim::tick::{FixedStepper, Tick};
use lgduel_sim::usercmd::UserCmd;
use lgduel_sim::world::World;

/// Moves longer than this between two ticks are teleports (respawn): don't smear them.
const TELEPORT_DIST: f32 = 128.0;

pub struct LocalSession {
    pub game: Game,
    pub me: PlayerId,
    stepper: FixedStepper,
    /// (player, origin at previous tick, origin at current tick) for render interpolation.
    interp: Vec<(PlayerId, Vec3, Vec3)>,
    /// Game events from all ticks run by the most recent `advance`.
    pub events: Vec<GameEvent>,
}

impl LocalSession {
    /// New offline game with the local player; `extra` more idle players are added.
    pub fn new(cfg: GameConfig, world: World, extra: usize) -> Self {
        let rate = cfg.tick_rate_hz;
        let mut game = Game::new(cfg, world);
        let me = game.add_player();
        for _ in 0..extra {
            game.add_player();
        }
        let interp = game
            .players
            .iter()
            .map(|p| (p.id, p.pm.origin, p.pm.origin))
            .collect();
        Self {
            game,
            me,
            stepper: FixedStepper::new(rate),
            interp,
            events: Vec::new(),
        }
    }

    /// Advance by real elapsed time; `cmd_for` builds the local command for each tick
    /// and `others` supplies commands for other players. Returns ticks run.
    pub fn advance(
        &mut self,
        elapsed_us: u64,
        mut cmd_for: impl FnMut(Tick) -> UserCmd,
        mut others: impl FnMut(&Game, Tick) -> Vec<(PlayerId, UserCmd)>,
    ) -> u32 {
        let (start, n) = self.stepper.advance(elapsed_us);
        self.events.clear();
        for i in 0..n {
            let tick = Tick(start.0 + i);
            let mut cmds = others(&self.game, tick);
            cmds.push((self.me, cmd_for(tick)));
            self.game.step(&cmds);
            self.events.extend(self.game.events.iter().cloned());
            self.update_interp();
        }
        n
    }

    fn update_interp(&mut self) {
        let old = std::mem::take(&mut self.interp);
        for p in &self.game.players {
            let cur = p.pm.origin;
            let prev = old
                .iter()
                .find(|(id, _, _)| *id == p.id)
                .map(|(_, _, c)| *c)
                .filter(|c| (cur - *c).length() <= TELEPORT_DIST)
                .unwrap_or(cur);
            self.interp.push((p.id, prev, cur));
        }
    }

    /// Move a player instantly (no interpolation smear), e.g. for scripted scenes.
    pub fn teleport(&mut self, id: PlayerId, origin: Vec3) {
        if let Some(p) = self.game.player_mut(id) {
            p.pm.origin = origin;
        }
        for e in &mut self.interp {
            if e.0 == id {
                e.1 = origin;
                e.2 = origin;
            }
        }
    }

    /// Origin of `id` interpolated between the last two ticks.
    pub fn origin(&self, id: PlayerId) -> Vec3 {
        let a = self.alpha();
        self.interp
            .iter()
            .find(|(pid, _, _)| *pid == id)
            .map(|(_, prev, cur)| *prev + (*cur - *prev) * a)
            .unwrap_or_else(|| self.game.player(id).map_or(Vec3::ZERO, |p| p.pm.origin))
    }

    /// What to draw this frame (same shape as the online session's view).
    pub fn frame_view(&self) -> crate::scene::FrameView {
        let g = &self.game;
        let me = g.player(self.me).unwrap();
        crate::scene::FrameView {
            me: self.me,
            me_alive: me.alive,
            eye: self.eye(),
            others: g
                .players
                .iter()
                .filter(|p| p.id != self.me)
                .map(|p| crate::scene::OtherView {
                    id: p.id,
                    origin: self.origin(p.id),
                    pitch: p.pitch,
                    yaw: p.yaw,
                    ducked: p.pm.ducked,
                    alive: p.alive,
                    buttons: p.buttons,
                })
                .collect(),
            hud: crate::hud::HudInfo {
                health: me.health,
                accuracy_pct: me.stats.accuracy_pct(),
                damage_dealt: me.stats.damage_dealt,
                damage_taken: me.stats.damage_taken,
                wins: me.stats.fights_won,
                losses: me.stats.fights_lost,
                fight_secs: g
                    .fight_start
                    .map(|s| (g.tick.0 - s.0) as f32 / g.cfg.tick_rate_hz as f32),
                opponent_health: g.players.iter().find(|p| p.id != self.me).map(|p| p.health),
                net: None,
            },
            events: self.events.clone(),
            spectate: None,
            status: None,
        }
    }

    /// Interpolation fraction between the last two ticks.
    pub fn alpha(&self) -> f32 {
        self.stepper.alpha()
    }

    /// Eye position interpolated between the last two ticks.
    pub fn eye(&self) -> Vec3 {
        let o = self.origin(self.me);
        let p = self.game.player(self.me).unwrap();
        o + (p.pm.eye(&self.game.cfg.physics) - p.pm.origin)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lgduel_sim::math::Angle;
    use lgduel_sim::world::builtin;

    fn session() -> LocalSession {
        let cfg = GameConfig::default();
        let p = &cfg.physics;
        let hull = (
            Vec3::from_array(p.player_mins),
            Vec3::from_array(p.player_maxs),
        );
        LocalSession::new(
            cfg,
            World::from_toml_str(builtin::ARENA_FLAT, hull).unwrap(),
            1,
        )
    }

    fn fwd(t: Tick) -> UserCmd {
        UserCmd {
            tick: t,
            forward: 127,
            yaw: Angle::ZERO,
            ..UserCmd::default()
        }
    }

    #[test]
    fn sim_rate_independent_of_frame_rate() {
        // Same 2 s of wall time at 1000 fps and at 37 fps → same number of ticks and same state.
        let mut fast = session();
        let mut slow = session();
        let mut tf = 0;
        for _ in 0..2000 {
            tf += fast.advance(1_000, fwd, |_, _| vec![]);
        }
        let mut ts = 0;
        let mut left = 2_000_000u64;
        while left > 0 {
            let d = left.min(27_027);
            ts += slow.advance(d, fwd, |_, _| vec![]);
            left -= d;
        }
        assert_eq!(tf, 250);
        assert_eq!(ts, 250);
        assert_eq!(fast.game.state_hash(), slow.game.state_hash());
    }

    #[test]
    fn other_players_interpolate_too() {
        let mut s = session();
        let other = s.game.players[1].id;
        let walk = |t: Tick| UserCmd {
            tick: t,
            forward: 127,
            ..UserCmd::default()
        };
        for _ in 0..50 {
            s.advance(8_000, fwd, |_, t| vec![(other, walk(t))]);
        }
        let a = s.origin(other);
        s.advance(4_000, fwd, |_, t| vec![(other, walk(t))]);
        let mid = s.origin(other);
        s.advance(4_000, fwd, |_, t| vec![(other, walk(t))]);
        let b = s.origin(other);
        assert!(
            (mid - (a + (b - a) * 0.5)).length() < 1e-3,
            "{a:?} {mid:?} {b:?}"
        );
    }

    #[test]
    fn firing_at_enemy_reports_own_damage_events() {
        use lgduel_sim::game::GameEvent;
        use lgduel_sim::math::look_angles;
        use lgduel_sim::usercmd::buttons;
        let mut s = session();
        let enemy = s.game.players[1].id;
        let me_pos = s.game.player(s.me).unwrap().pm.origin;
        s.teleport(enemy, me_pos + Vec3::new(300.0, 0.0, 0.0));
        let eye = s.eye();
        let (pitch, yaw) = look_angles(me_pos + Vec3::new(300.0, 0.0, 8.0) - eye);
        let fire = |t: Tick| UserCmd {
            tick: t,
            pitch,
            yaw,
            buttons: buttons::FIRE,
            ..UserCmd::default()
        };
        // One frame spanning 0.1 s: several ticks, several hits, all reported.
        s.advance(100_000, fire, |_, _| vec![]);
        let hits = s
            .events
            .iter()
            .filter(|e| matches!(e, GameEvent::Damage { attacker, .. } if *attacker == s.me))
            .count();
        assert_eq!(hits, 2, "100 ms of LG at 50 ms refire = 2 cells, both hit");
        s.advance(8_000, fire, |_, _| vec![]);
        assert!(s.events.len() <= 2, "events reset each advance");
    }

    #[test]
    fn offline_practice_vs_bot_plays_out() {
        use lgduel_bot::{AimSkill, Bot, BotConfig, StrafePattern};
        let mut s = session();
        let bot_id = s.game.players[1].id;
        let mut bot = Bot::new(BotConfig {
            pattern: StrafePattern::RandomAd {
                min_ms: 150,
                max_ms: 600,
            },
            aim: AimSkill::MEDIUM,
            ..BotConfig::default()
        });
        // Local player strafes A/D every 0.5 s and never shoots; frames at an uneven ~240 fps.
        let mut frame = 0u64;
        let mut t_us = 0u64;
        while t_us < 20_000_000 {
            let dt = 4_000 + (frame % 3) * 170;
            let strafe = if (t_us / 500_000).is_multiple_of(2) {
                127
            } else {
                -127
            };
            s.advance(
                dt,
                |t| UserCmd {
                    tick: t,
                    right: strafe,
                    ..UserCmd::default()
                },
                |g, _| vec![(bot_id, bot.think(g, bot_id))],
            );
            t_us += dt;
            frame += 1;
        }
        let b = s.game.player(bot_id).unwrap().stats;
        let me = s.game.player(s.me).unwrap().stats;
        assert_eq!(
            s.game.tick.0, 2500,
            "20 s at 125 Hz regardless of frame timing"
        );
        // The bot only fires while the target is alive (respawn delay 1 s per frag).
        assert!(b.shots > 200 && b.hits > 50, "{b:?}");
        assert!(
            b.fights_won >= 2 && me.fights_lost == b.fights_won,
            "{b:?} {me:?}"
        );
    }

    #[test]
    fn eye_interpolates_between_ticks() {
        let mut s = session();
        for _ in 0..100 {
            s.advance(8_000, fwd, |_, _| vec![]);
        }
        let before = s.eye();
        s.advance(4_000, fwd, |_, _| vec![]); // half a tick: no sim step
        let mid = s.eye();
        s.advance(4_000, fwd, |_, _| vec![]); // completes the tick
        let after = s.eye();
        assert!(
            mid.x > before.x - 1e-3 && mid.x < after.x,
            "{before:?} {mid:?} {after:?}"
        );
        assert_eq!(s.alpha(), 0.0);
    }
}
