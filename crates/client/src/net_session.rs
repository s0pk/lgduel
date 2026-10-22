//! Online session: NetClient + prediction (own movement) + interpolation
//! (everyone else), producing the same `FrameView` the offline session does.

use crate::hud::HudInfo;
use crate::scene::{FrameView, OtherView};
use lgduel_net::client::{ClientState, NetClient};
use lgduel_net::interp::{InterpSettings, Interpolator};
use lgduel_net::predict::Predictor;
use lgduel_net::snapshot::Snapshot;
use lgduel_net::transport::UdpTransport;
use lgduel_sim::config::GameConfig;
use lgduel_sim::game::GameEvent;
use lgduel_sim::tick::Tick;
use lgduel_sim::usercmd::UserCmd;
use lgduel_sim::world::World;
use std::net::SocketAddr;
use std::time::Instant;

pub struct NetSession {
    pub client: NetClient<UdpTransport>,
    start: Instant,
    world: World,
    pub predictor: Option<Predictor>,
    pub interp: Interpolator,
    latest: Option<Snapshot>,
    view: Option<FrameView>,
    last_us: u64,
}

impl NetSession {
    pub fn connect(server: SocketAddr, name: &str, world: World) -> std::io::Result<Self> {
        let transport = UdpTransport::bind("0.0.0.0:0".parse().unwrap())?;
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(1, |d| d.as_nanos() as u64);
        Ok(Self {
            client: NetClient::new(transport, server, name, nonce),
            start: Instant::now(),
            world,
            predictor: None,
            interp: Interpolator::new(InterpSettings::default()),
            latest: None,
            view: None,
            last_us: 0,
        })
    }

    pub fn now_us(&self) -> u64 {
        self.start.elapsed().as_micros() as u64
    }

    pub fn state(&self) -> &ClientState {
        &self.client.state
    }

    pub fn config(&self) -> Option<&GameConfig> {
        self.client.config.as_ref()
    }

    pub fn world(&self) -> &World {
        &self.world
    }

    /// Poll the network, reconcile, send this frame's commands, rebuild the view.
    pub fn update(&mut self, mut cmd_for: impl FnMut(Tick) -> UserCmd) {
        let now = self.now_us();
        let frame_dt = now.saturating_sub(self.last_us) as f32 / 1e6;
        self.last_us = now;
        self.client.update(now);
        let (Some(me), Some(cfg)) = (self.client.player(), self.client.config.clone()) else {
            return;
        };
        self.interp.settings.tick_hz = cfg.tick_rate_hz as f64;
        let predictor = self
            .predictor
            .get_or_insert_with(|| Predictor::new(me, self.world.clone(), cfg.clone()));
        let mut events: Vec<GameEvent> = Vec::new();
        for snap in self.client.take_snapshots() {
            self.interp.push(snap.tick.0, &snap.players);
            events.extend(snap.events.iter().cloned());
            let newest = self.latest.as_ref().is_none_or(|l| snap.tick > l.tick);
            if newest {
                predictor.on_snapshot(&snap, &self.client.sent_cmds);
                self.latest = Some(snap);
            }
        }
        predictor.update_smoothing(frame_dt);
        self.interp
            .set_target_delay(self.client.recommended_interp_delay());
        let Some(clock) = self.client.snapshot_clock(now) else {
            return;
        };
        let view_tick = clock - self.interp.settings.delay_ticks;
        for t in self.client.advance(now) {
            let cmd = cmd_for(t);
            self.client.send_cmd_with_view(now, cmd, Some(view_tick));
            predictor.predict(&cmd);
        }
        let Some(snap) = &self.latest else { return };
        let Some(mine) = snap.player(me) else { return };
        let mut others = Vec::new();
        for p in snap.players.iter().filter(|p| p.id != me) {
            if let Some(v) = self.interp.sample(p.id, clock) {
                others.push(OtherView {
                    id: p.id,
                    origin: v.origin,
                    pitch: v.pitch,
                    yaw: v.yaw,
                    ducked: v.ducked,
                    alive: v.alive,
                    buttons: v.buttons,
                });
            }
        }
        let rate = cfg.tick_rate_hz as f32;
        self.view = Some(FrameView {
            me,
            me_alive: mine.alive,
            eye: predictor.eye(),
            others,
            hud: HudInfo {
                health: mine.health,
                accuracy_pct: mine.stats.accuracy_pct(),
                damage_dealt: mine.stats.damage_dealt,
                damage_taken: mine.stats.damage_taken,
                wins: mine.stats.fights_won,
                losses: mine.stats.fights_lost,
                fight_secs: snap
                    .fight_start
                    .map(|s| snap.tick.0.saturating_sub(s.0) as f32 / rate),
                opponent_health: snap.players.iter().find(|p| p.id != me).map(|p| p.health),
            },
            events,
        });
    }

    /// The frame to draw (None until connected and the first snapshot arrived).
    pub fn frame_view(&self) -> Option<&FrameView> {
        self.view.as_ref()
    }

    pub fn disconnect(&mut self) {
        let now = self.now_us();
        self.client.disconnect(now);
    }
}
