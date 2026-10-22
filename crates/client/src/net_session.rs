//! Online session: NetClient + prediction (own movement) + interpolation
//! (everyone else), producing the same `FrameView` the offline session does.

use crate::hud::{HudInfo, NetGraph};
use crate::scene::{FrameView, OtherView};
use lgduel_net::client::{ClientState, NetClient};
use lgduel_net::hitreg::HitReg;
use lgduel_net::interp::{InterpSettings, Interpolator};
use lgduel_net::predict::Predictor;
use lgduel_net::snapshot::Snapshot;
use lgduel_net::transport::UdpTransport;
use lgduel_sim::config::GameConfig;
use lgduel_sim::game::{GameEvent, hitscan};
use lgduel_sim::math::{Vec3, angle_vectors};
use lgduel_sim::tick::Tick;
use lgduel_sim::usercmd::{UserCmd, buttons};
use lgduel_sim::world::{Aabb, World};
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
    /// Client-vs-server hit verdict agreement for our LG cells.
    pub hitreg: HitReg,
    /// When the last snapshot arrived (µs since session start).
    last_snapshot_us: Option<u64>,
    /// Receive times of recent snapshots (for the measured snapshot rate).
    snap_times: std::collections::VecDeque<u64>,
    /// (time, bytes in, bytes out) at the start of the bandwidth window.
    bw_mark: (u64, u64, u64),
    /// Set when the server's map can't be loaded here (we disconnect).
    pub map_error: Option<String>,
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
            hitreg: HitReg::default(),
            last_snapshot_us: None,
            snap_times: std::collections::VecDeque::new(),
            bw_mark: (0, 0, 0),
            map_error: None,
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
        if self.map_error.is_some() {
            return;
        }
        if self.predictor.is_none()
            && let Some((name, hash)) = self.client.map.clone()
            && self.world.content_hash() != hash
        {
            let hull = (
                Vec3::from_array(cfg.physics.player_mins),
                Vec3::from_array(cfg.physics.player_maxs),
            );
            match lgduel_sim::world::builtin::matching(&name, hash, hull) {
                Ok(w) => self.world = w,
                Err(e) => {
                    self.map_error = Some(e);
                    self.client.disconnect(now);
                    return;
                }
            }
        }
        self.interp.settings.tick_hz = cfg.tick_rate_hz as f64;
        let predictor = self
            .predictor
            .get_or_insert_with(|| Predictor::new(me, self.world.clone(), cfg.clone()));
        let mut events: Vec<GameEvent> = Vec::new();
        for snap in self.client.take_snapshots() {
            self.interp.push(snap.tick.0, &snap.players);
            self.hitreg.on_snapshot(me, &snap);
            self.last_snapshot_us = Some(now);
            self.snap_times.push_back(now);
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
        let Some(snap) = self.latest.clone() else {
            return;
        };
        let Some(mine) = snap.player(me).cloned() else {
            // Not in an arena (queued): idle commands keep the connection and clock alive.
            for t in self.client.advance(now) {
                self.client.send_cmd(
                    now,
                    UserCmd {
                        tick: t,
                        ..UserCmd::default()
                    },
                );
            }
            self.view = None;
            return;
        };
        // Remote players as rendered this frame (also what our own beam is judged against).
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
        let mins = Vec3::from_array(cfg.physics.player_mins);
        let targets: Vec<_> = others
            .iter()
            .filter(|o| o.alive)
            .map(|o| {
                let mut maxs = Vec3::from_array(cfg.physics.player_maxs);
                if o.ducked {
                    maxs.z = cfg.physics.crouch_maxs_z;
                }
                (o.id, Aabb::at(o.origin, mins, maxs))
            })
            .collect();
        for t in self.client.advance(now) {
            let cmd = cmd_for(t);
            self.client.send_cmd_with_view(now, cmd, Some(view_tick));
            predictor.predict(&cmd);
            if cmd.pressed(buttons::FIRE) && mine.alive {
                let (f, _, _) = angle_vectors(cmd.pitch, cmd.yaw);
                let eye = predictor.state.eye(&cfg.physics);
                let hs = hitscan(&self.world, eye, f, cfg.lg.range, &targets);
                self.hitreg.record(t.0, hs.target.is_some());
            }
        }
        while self
            .snap_times
            .front()
            .is_some_and(|t| now.saturating_sub(*t) > 1_000_000)
        {
            self.snap_times.pop_front();
        }
        let st = self.client.conn.stats;
        if now.saturating_sub(self.bw_mark.0) >= 2_000_000 {
            self.bw_mark = (now, st.bytes_received, st.bytes_sent);
        }
        let bw_secs = (now.saturating_sub(self.bw_mark.0) as f32 / 1e6).max(0.001);
        let net = NetGraph {
            ping_ms: (self.client.rtt_us() / 1000.0) as f32,
            jitter_ms: (st.rtt_var_us / 1000.0) as f32,
            loss_pct: (st.loss() * 100.0) as f32,
            tick_hz: cfg.tick_rate_hz,
            snap_hz: self.snap_times.len() as f32,
            interp_delay_ms: (self.interp.settings.delay_ticks * 1000.0 / cfg.tick_rate_hz as f64)
                as f32,
            in_kbps: (st.bytes_received - self.bw_mark.1) as f32 / 1024.0 / bw_secs,
            out_kbps: (st.bytes_sent - self.bw_mark.2) as f32 / 1024.0 / bw_secs,
            rubber_bands: predictor.stats.rubber_bands,
            hitreg_pct: self.hitreg.stats.agreement_pct() as f32,
        };
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
                net: Some(net),
            },
            events,
        });
    }

    /// True if we were receiving snapshots and the server went quiet for `secs`.
    pub fn server_silent(&self, secs: f64) -> bool {
        self.last_snapshot_us
            .is_some_and(|t| self.now_us().saturating_sub(t) as f64 > secs * 1e6)
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
