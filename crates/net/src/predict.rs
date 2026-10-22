//! Client-side prediction of the local player's movement.
//!
//! The newest authoritative snapshot gives our state after the server applied
//! our command `last_cmd`. We replay every command we sent after that through
//! the same deterministic `pmove`, so our view moves the instant we press a
//! key. When a new snapshot arrives we redo this from it (reconciliation). If
//! the result differs from what we had shown, the visible difference is kept
//! as an offset that decays over ~100 ms instead of snapping the camera.

use crate::snapshot::Snapshot;
use lgduel_sim::config::GameConfig;
use lgduel_sim::game::PlayerId;
use lgduel_sim::math::Vec3;
use lgduel_sim::pmove::{PlayerMove, pmove};
use lgduel_sim::tick::Tick;
use lgduel_sim::usercmd::UserCmd;
use lgduel_sim::world::World;
use std::collections::{BTreeMap, VecDeque};

/// Corrections larger than this snap instead of smoothing (teleports, respawns).
const SNAP_DISTANCE: f32 = 64.0;
/// Fraction of the visual error removed per second (exponential decay rate).
const SMOOTHING_PER_SEC: f32 = 15.0;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct PredictionStats {
    /// Ticks for which a prediction was later checked against the server.
    pub checked: u64,
    /// Sum / max of |predicted - authoritative| origin, units.
    pub sum_error: f64,
    pub max_error: f32,
    /// Checked ticks with any error at all.
    pub mispredicted: u64,
    /// Corrections caused by things the client cannot predict (being hit →
    /// knockback, dying, respawning). Not counted as prediction error.
    pub external: u64,
    /// Rubber-band detector: largest jump between the shown position before and
    /// after a reconcile, excluding external corrections (units).
    pub max_correction: f32,
    /// Reconciles whose correction exceeded `RUBBER_BAND_UNITS`.
    pub rubber_bands: u64,
}

/// A correction this large (visible "snap back") counts as a rubber-band event.
pub const RUBBER_BAND_UNITS: f32 = 1.0;

impl PredictionStats {
    pub fn mean_error(&self) -> f64 {
        if self.checked == 0 {
            0.0
        } else {
            self.sum_error / self.checked as f64
        }
    }
}

pub struct Predictor {
    pub me: PlayerId,
    pub world: World,
    pub cfg: GameConfig,
    /// Predicted state after the newest command applied.
    pub state: PlayerMove,
    /// Tick of the newest command applied to `state` (None until a snapshot).
    pub state_tick: Option<Tick>,
    alive: bool,
    /// Predicted origin after each command tick (for error measurement).
    history: BTreeMap<u32, Vec3>,
    /// Visual correction offset (decays to zero).
    pub smooth_offset: Vec3,
    pub stats: PredictionStats,
    /// (damage taken, deaths, alive) at the last reconciled snapshot.
    last_seen: Option<(u32, u32, bool)>,
}

impl Predictor {
    pub fn new(me: PlayerId, world: World, cfg: GameConfig) -> Self {
        Self {
            me,
            world,
            cfg,
            state: PlayerMove::default(),
            state_tick: None,
            alive: false,
            history: BTreeMap::new(),
            smooth_offset: Vec3::ZERO,
            stats: PredictionStats::default(),
            last_seen: None,
        }
    }

    fn dt(&self) -> f32 {
        1.0 / self.cfg.tick_rate_hz as f32
    }

    /// Apply one new local command on top of the prediction.
    pub fn predict(&mut self, cmd: &UserCmd) {
        let dt = self.dt();
        if self.state_tick.is_none_or(|t| cmd.tick > t) && self.state_tick.is_some() {
            if self.alive {
                let c = cmd.sanitized();
                pmove(&mut self.state, &c, &self.world, &self.cfg.physics, dt);
            }
            self.state_tick = Some(cmd.tick);
            self.history.insert(cmd.tick.0, self.state.origin);
        }
    }

    /// Reconcile with a new authoritative snapshot; `sent` = all commands we
    /// sent, oldest first.
    pub fn on_snapshot(&mut self, snap: &Snapshot, sent: &VecDeque<UserCmd>) {
        let dt = self.dt();
        let Some(me) = snap.player(self.me) else {
            return;
        };
        // Hit (knockback), killed or respawned since the last snapshot? Then a
        // correction is expected and isn't a prediction error.
        let seen = (me.stats.damage_taken, me.stats.deaths, me.alive);
        let external = self.last_seen.is_some_and(|s| s != seen) || !me.alive;
        self.last_seen = Some(seen);
        // Measure: what did we predict for the command the server just applied?
        if external {
            self.stats.external += 1;
        } else if let Some(last) = snap.last_cmd
            && let Some(pred) = self.history.get(&last.0)
        {
            let err = (*pred - me.pm.origin).length();
            self.stats.checked += 1;
            self.stats.sum_error += err as f64;
            self.stats.max_error = self.stats.max_error.max(err);
            if err > 0.0 {
                self.stats.mispredicted += 1;
            }
        }
        if let Some(last) = snap.last_cmd {
            self.history.retain(|t, _| *t > last.0);
        }
        let shown_before = self.state_tick.map(|_| self.state.origin);
        // Rebuild from the authoritative state.
        self.state = me.pm;
        self.alive = me.alive;
        let from = snap.last_cmd;
        self.state_tick = Some(from.unwrap_or(Tick(snap.tick.0.saturating_sub(1))));
        for c in sent.iter().filter(|c| from.is_none_or(|f| c.tick > f)) {
            let c = c.sanitized();
            if self.alive {
                pmove(&mut self.state, &c, &self.world, &self.cfg.physics, dt);
            }
            self.state_tick = Some(c.tick);
            self.history.insert(c.tick.0, self.state.origin);
        }
        // Visual smoothing of the correction.
        if let Some(before) = shown_before {
            let jump = before + self.smooth_offset - self.state.origin;
            if !external {
                let correction = (before - self.state.origin).length();
                self.stats.max_correction = self.stats.max_correction.max(correction);
                if correction > RUBBER_BAND_UNITS {
                    self.stats.rubber_bands += 1;
                }
            }
            self.smooth_offset = if jump.length() > SNAP_DISTANCE || !self.alive {
                Vec3::ZERO
            } else {
                jump
            };
        }
    }

    /// Decay the visual correction; call once per rendered frame.
    pub fn update_smoothing(&mut self, frame_dt: f32) {
        let k = (-SMOOTHING_PER_SEC * frame_dt).exp();
        self.smooth_offset = self.smooth_offset * k;
        if self.smooth_offset.length() < 0.01 {
            self.smooth_offset = Vec3::ZERO;
        }
    }

    /// Origin to render (prediction + decaying correction offset).
    pub fn render_origin(&self) -> Vec3 {
        self.state.origin + self.smooth_offset
    }

    pub fn eye(&self) -> Vec3 {
        let mut pm = self.state;
        pm.origin = self.render_origin();
        pm.eye(&self.cfg.physics)
    }
}
