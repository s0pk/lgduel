//! Offline session: runs the sim locally at its fixed tick rate, independent
//! of render rate, and interpolates the own view between ticks.

use lgduel_sim::config::GameConfig;
use lgduel_sim::game::{Game, PlayerId};
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
    prev_origin: Vec3,
    cur_origin: Vec3,
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
        let o = game.player(me).unwrap().pm.origin;
        Self {
            game,
            me,
            stepper: FixedStepper::new(rate),
            prev_origin: o,
            cur_origin: o,
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
        for i in 0..n {
            let tick = Tick(start.0 + i);
            let mut cmds = others(&self.game, tick);
            cmds.push((self.me, cmd_for(tick)));
            self.prev_origin = self.cur_origin;
            self.game.step(&cmds);
            self.cur_origin = self.game.player(self.me).unwrap().pm.origin;
            if (self.cur_origin - self.prev_origin).length() > TELEPORT_DIST {
                self.prev_origin = self.cur_origin;
            }
        }
        n
    }

    /// Interpolation fraction between the last two ticks.
    pub fn alpha(&self) -> f32 {
        self.stepper.alpha()
    }

    /// Eye position interpolated between the last two ticks.
    pub fn eye(&self) -> Vec3 {
        let a = self.alpha();
        let o = self.prev_origin + (self.cur_origin - self.prev_origin) * a;
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
