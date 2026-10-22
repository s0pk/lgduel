//! A bot that plays over the network like a real client: it keeps a mirror of
//! the game from snapshots, decides with `Bot::think`, sends commands with its
//! view time (for anti-lag), and runs the same prediction as the real client so
//! it can report desyncs (prediction errors) at the end.

use crate::{Bot, BotConfig};
use lgduel_net::client::{ClientState, NetClient};
use lgduel_net::predict::{PredictionStats, Predictor};
use lgduel_net::transport::Transport;
use lgduel_sim::game::{Game, PlayerStats};
use lgduel_sim::world::World;
use std::net::SocketAddr;

pub struct NetBot<T: Transport> {
    pub client: NetClient<T>,
    pub bot: Bot,
    world: World,
    mirror: Option<Game>,
    predictor: Option<Predictor>,
    pub snapshots: u64,
}

impl<T: Transport> NetBot<T> {
    pub fn new(transport: T, server: SocketAddr, name: &str, cfg: BotConfig, world: World) -> Self {
        Self {
            client: NetClient::new(transport, server, name, cfg.seed ^ 0xB07),
            bot: Bot::new(cfg),
            world,
            mirror: None,
            predictor: None,
            snapshots: 0,
        }
    }

    pub fn state(&self) -> &ClientState {
        &self.client.state
    }

    /// Run one frame at `now_us`.
    pub fn step(&mut self, now_us: u64) {
        self.client.update(now_us);
        let (Some(me), Some(cfg)) = (self.client.player(), self.client.config.clone()) else {
            return;
        };
        let mirror = self
            .mirror
            .get_or_insert_with(|| Game::new(cfg.clone(), self.world.clone()));
        let predictor = self
            .predictor
            .get_or_insert_with(|| Predictor::new(me, self.world.clone(), cfg.clone()));
        for snap in self.client.take_snapshots() {
            self.snapshots += 1;
            if self
                .client
                .snapshots
                .latest()
                .is_some_and(|l| l.tick == snap.tick)
            {
                snap.apply(mirror);
                predictor.on_snapshot(&snap, &self.client.sent_cmds);
            }
        }
        if mirror.player(me).is_none() {
            return;
        }
        // The bot sees the newest snapshot; that is also its anti-lag view time.
        let view = self.client.snapshots.latest().map(|s| s.tick.0 as f64);
        for t in self.client.advance(now_us) {
            mirror.tick = t;
            let mut cmd = self.bot.think(mirror, me);
            cmd.tick = t;
            self.client.send_cmd_with_view(now_us, cmd, view);
            predictor.predict(&cmd);
        }
    }

    /// Forget prediction/late-command statistics so far (end of warm-up).
    pub fn reset_measurements(&mut self) {
        if let Some(p) = self.predictor.as_mut() {
            p.stats = PredictionStats::default();
        }
        self.client.late_cmds = 0;
    }

    pub fn prediction(&self) -> PredictionStats {
        self.predictor
            .as_ref()
            .map(|p| p.stats.clone())
            .unwrap_or_default()
    }

    /// Own stats as of the newest snapshot.
    pub fn stats(&self) -> Option<PlayerStats> {
        let me = self.client.player()?;
        self.client.snapshots.latest()?.player(me).map(|p| p.stats)
    }

    pub fn disconnect(&mut self, now_us: u64) {
        self.client.disconnect(now_us);
    }
}
