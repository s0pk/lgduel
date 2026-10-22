//! Online session: NetClient + prediction (own movement) + interpolation
//! (everyone else), producing the same `FrameView` the offline session does.

use crate::hud::{HudInfo, NetGraph};
use crate::scene::{FrameView, OtherView, SpectateView};
use lgduel_net::client::{ClientState, NetClient};
use lgduel_net::hitreg::HitReg;
use lgduel_net::interp::{InterpSettings, Interpolator};
use lgduel_net::predict::Predictor;
use lgduel_net::rendezvous::{JOIN_ADDR, JoinState, RelayTransport};
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
    pub client: NetClient<RelayTransport<UdpTransport>>,
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
    /// Player we watch while queued / spectating.
    follow: Option<lgduel_sim::game::PlayerId>,
    /// Latest server message and when it arrived (shown for a few seconds).
    toast: Option<(String, u64)>,
}

fn nonce() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(1, |d| d.as_nanos() as u64)
}

/// How long a server message stays on screen.
const TOAST_US: u64 = 4_000_000;

impl NetSession {
    pub fn connect(server: SocketAddr, name: &str, world: World) -> std::io::Result<Self> {
        let transport = RelayTransport::plain(UdpTransport::bind("0.0.0.0:0".parse().unwrap())?);
        Ok(Self::with(transport, server, name, world))
    }

    /// Join the game with this code through the rendezvous service.
    pub fn connect_code(
        rendezvous: SocketAddr,
        code: u32,
        force_relay: bool,
        name: &str,
        world: World,
    ) -> std::io::Result<Self> {
        let udp = UdpTransport::bind("0.0.0.0:0".parse().unwrap())?;
        let mut t = RelayTransport::join(udp, rendezvous, code, nonce());
        if force_relay {
            t = t.forcing_relay();
        }
        Ok(Self::with(t, JOIN_ADDR, name, world))
    }

    /// Join-code progress for the Connecting screen (None when not joining by code).
    pub fn join_progress(&self) -> Option<String> {
        Some(match self.client.transport.join_state()? {
            JoinState::LookingUp => "Looking up the join code...".into(),
            JoinState::Direct { heard: false, .. } => "Connecting directly...".into(),
            JoinState::Direct { heard: true, .. } => "Connected directly".into(),
            JoinState::OpeningRelay => "No direct path; trying the relay...".into(),
            JoinState::Relayed { .. } => "Connected through the relay".into(),
            JoinState::Failed(e) => e.clone(),
        })
    }

    /// Why joining by code failed, if it did.
    pub fn join_failed(&self) -> Option<String> {
        match self.client.transport.join_state()? {
            JoinState::Failed(e) => Some(e.clone()),
            _ => None,
        }
    }

    fn with(
        transport: RelayTransport<UdpTransport>,
        server: SocketAddr,
        name: &str,
        world: World,
    ) -> Self {
        Self {
            client: NetClient::new(transport, server, name, nonce()),
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
            follow: None,
            toast: None,
        }
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
        self.client.transport.tick(now);
        self.client.update(now);
        while let Some(m) = self.client.messages.pop_front() {
            println!("SERVER {m}");
            self.toast = Some((m, now));
        }
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
        let mine = snap.player(me).cloned();
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
        let Some(mine) = mine else {
            // Not in an arena (queued or spectating): idle commands keep the
            // connection and clock alive; watch one of the players.
            for t in self.client.advance(now) {
                self.client.send_cmd(
                    now,
                    UserCmd {
                        tick: t,
                        ..UserCmd::default()
                    },
                );
            }
            self.view = self.spectator_view(&snap, others, events, &cfg, net);
            return;
        };
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
        let rate = cfg.tick_rate_hz as f32;
        let eye = predictor.eye();
        let status = self.status_text();
        self.view = Some(FrameView {
            me,
            me_alive: mine.alive,
            eye,
            spectate: None,
            status,
            stats: snap.players.iter().map(|p| (p.id, p.stats)).collect(),
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

    /// First-person view of a watched player (the one followed before, else
    /// the first living one).
    fn spectator_view(
        &mut self,
        snap: &Snapshot,
        mut others: Vec<OtherView>,
        events: Vec<GameEvent>,
        cfg: &GameConfig,
        net: NetGraph,
    ) -> Option<FrameView> {
        let i = self
            .follow
            .and_then(|f| others.iter().position(|o| o.id == f))
            .or_else(|| others.iter().position(|o| o.alive))
            .or((!others.is_empty()).then_some(0))?;
        let f = others.remove(i);
        self.follow = Some(f.id);
        let p = snap.player(f.id)?;
        let h = if f.ducked {
            cfg.physics.crouch_view_height
        } else {
            cfg.physics.view_height
        };
        let rate = cfg.tick_rate_hz as f32;
        Some(FrameView {
            me: f.id,
            me_alive: f.alive,
            eye: f.origin + Vec3::new(0.0, 0.0, h),
            hud: HudInfo {
                health: p.health,
                accuracy_pct: p.stats.accuracy_pct(),
                damage_dealt: p.stats.damage_dealt,
                damage_taken: p.stats.damage_taken,
                wins: p.stats.fights_won,
                losses: p.stats.fights_lost,
                fight_secs: snap
                    .fight_start
                    .map(|s| snap.tick.0.saturating_sub(s.0) as f32 / rate),
                opponent_health: others
                    .first()
                    .and_then(|o| snap.player(o.id))
                    .map(|o| o.health),
                net: Some(net),
            },
            others,
            events,
            spectate: Some(SpectateView {
                pitch: f.pitch,
                yaw: f.yaw,
                firing: f.buttons & buttons::FIRE != 0,
                buttons: f.buttons,
            }),
            status: self.status_text(),
            stats: snap.players.iter().map(|p| (p.id, p.stats)).collect(),
        })
    }

    /// The server's arena/queue status as one HUD line (a fresh server
    /// message or an incoming challenge take its place).
    pub fn status_text(&self) -> Option<String> {
        if let Some((m, t)) = &self.toast
            && self.last_us.saturating_sub(*t) < TOAST_US
        {
            return Some(m.clone());
        }
        let s = self.client.status.as_ref()?;
        if let Some((_, name)) = &s.challenge_from {
            return Some(format!(
                "{} CHALLENGES YOU - ESC TO ANSWER",
                name.to_uppercase()
            ));
        }
        let base = self.place_text(s)?;
        Some(if s.challenge_to.is_empty() {
            base
        } else {
            format!("{base} - CHALLENGED {}", s.challenge_to.to_uppercase())
        })
    }

    fn place_text(&self, s: &lgduel_net::protocol::ArenaStatus) -> Option<String> {
        let watch = s.watching as u32 + 1;
        Some(if s.spectating {
            format!("SPECTATING ARENA {watch} - ESC > PLAY TO JOIN")
        } else if s.queue_pos > 0 {
            format!("QUEUE #{} - WATCHING ARENA {watch}", s.queue_pos)
        } else {
            let a = s.arena? as u32 + 1;
            if s.opponent_bot {
                format!("ARENA {a} - PRACTICE BOT UNTIL AN OPPONENT JOINS")
            } else if s.opponent.is_empty() {
                format!("ARENA {a} - WAITING FOR AN OPPONENT")
            } else {
                format!("ARENA {a} - VS {}", s.opponent.to_uppercase())
            }
        })
    }

    /// Everyone on the server.
    pub fn roster(&self) -> &[lgduel_net::protocol::RosterEntry] {
        &self.client.roster
    }

    /// Pending challenge to us: (player id, name).
    pub fn incoming_challenge(&self) -> Option<(u8, String)> {
        self.client.status.as_ref()?.challenge_from.clone()
    }

    pub fn challenge(&mut self, target: u8) {
        self.client.challenge(target);
    }

    pub fn respond(&mut self, from: u8, accept: bool) {
        self.client.respond(from, accept);
    }

    /// Watching instead of playing (as last reported by the server).
    pub fn spectating(&self) -> bool {
        self.client.status.as_ref().is_some_and(|s| s.spectating)
    }

    /// Ask the server to let us watch (`true`) or play (`false`).
    pub fn set_spectate(&mut self, on: bool) {
        self.client.set_spectate(on);
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
