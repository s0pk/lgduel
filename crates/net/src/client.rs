//! Client side of the protocol: handshake, snapshot reception, input sending
//! and clock alignment.
//!
//! **Clock alignment.** The client runs its own tick counter ahead of the
//! server so that the command for tick T arrives shortly *before* the server
//! simulates T. From each newest snapshot it estimates the server's current
//! tick (snapshot tick + time since arrival + half RTT) and targets
//! `server_tick + lead`, where lead covers half the RTT plus a jitter-based
//! safety margin. Small errors are absorbed by running the client clock up to
//! ±5% fast/slow; errors over 250 ms snap.

use crate::PROTOCOL_VERSION;
use crate::conn::Connection;
use crate::protocol::{CONNECTIONLESS, Connectionless, Control, INPUT_REDUNDANCY, Inputs};
use crate::snapshot::{Snapshot, SnapshotHistory};
use crate::transport::Transport;
use lgduel_sim::config::GameConfig;
use lgduel_sim::game::PlayerId;
use lgduel_sim::tick::Tick;
use lgduel_sim::usercmd::UserCmd;
use std::collections::VecDeque;
use std::net::SocketAddr;

const CONNECT_RETRY_US: u64 = 250_000;
const SNAP_THRESHOLD_US: f64 = 250_000.0;
const MAX_DILATION: f64 = 0.05;
/// Extra safety ticks on top of half-RTT + jitter margin.
const BASE_SAFETY_TICKS: f64 = 1.0;

#[derive(Debug, Clone, PartialEq)]
pub enum ClientState {
    Connecting,
    Connected { player: PlayerId },
    Rejected(String),
    Disconnected(String),
}

pub struct NetClient<T: Transport> {
    pub transport: T,
    pub server: SocketAddr,
    pub state: ClientState,
    pub name: String,
    pub conn: Connection,
    /// Gameplay config sent by the server on accept.
    pub config: Option<GameConfig>,
    nonce: u64,
    last_request_us: Option<u64>,
    pub snapshots: SnapshotHistory,
    /// Snapshots received since the last `take_snapshots`.
    fresh: Vec<Snapshot>,
    /// (snapshot tick, local receive time) of the newest snapshot.
    newest: Option<(Tick, u64)>,
    /// Commands sent (newest last) for redundancy and prediction.
    pub sent_cmds: VecDeque<UserCmd>,
    tick_us: f64,
    /// Client simulation clock in µs since tick 0 (fractional, dilated).
    clock_us: f64,
    clock_started: bool,
    /// Next client tick to generate a command for.
    next_tick: u32,
    last_update_us: Option<u64>,
    /// Observed spread of snapshot arrival times (µs), for the safety margin.
    arrival_jitter_us: f64,
    pub snaps_received: u64,
    /// When our ConnectResponse went out (handshake RTT measurement).
    response_sent_us: Option<u64>,
    /// RTT measured by the handshake, used until packet acks provide one.
    handshake_rtt_us: f64,
}

impl<T: Transport> NetClient<T> {
    pub fn new(transport: T, server: SocketAddr, name: &str, nonce: u64) -> Self {
        Self {
            transport,
            server,
            state: ClientState::Connecting,
            name: name.to_string(),
            conn: Connection::new(),
            config: None,
            nonce,
            last_request_us: None,
            snapshots: SnapshotHistory::new(64),
            fresh: Vec::new(),
            newest: None,
            sent_cmds: VecDeque::new(),
            tick_us: 8_000.0,
            clock_us: 0.0,
            clock_started: false,
            next_tick: 0,
            last_update_us: None,
            arrival_jitter_us: 0.0,
            snaps_received: 0,
            response_sent_us: None,
            handshake_rtt_us: 0.0,
        }
    }

    pub fn player(&self) -> Option<PlayerId> {
        match self.state {
            ClientState::Connected { player } => Some(player),
            _ => None,
        }
    }

    /// Poll the network: handshake, snapshots. Call every frame.
    pub fn update(&mut self, now_us: u64) {
        if self.state == ClientState::Connecting
            && self
                .last_request_us
                .is_none_or(|t| now_us.saturating_sub(t) >= CONNECT_RETRY_US)
        {
            let m = Connectionless::ConnectRequest {
                version: PROTOCOL_VERSION,
                nonce: self.nonce,
            };
            let _ = self.transport.send_to(self.server, &m.encode());
            self.last_request_us = Some(now_us);
        }
        while let Some((from, data)) = self.transport.recv() {
            if from != self.server {
                continue;
            }
            if data.first() == Some(&CONNECTIONLESS) {
                self.connectionless(now_us, &data);
                continue;
            }
            if self.player().is_none() {
                continue;
            }
            let Some(rx) = self.conn.on_packet(now_us, &data) else {
                continue;
            };
            for m in &rx.reliable {
                if let Some(Control::Disconnect { reason }) = Control::decode(m) {
                    self.state = ClientState::Disconnected(reason);
                }
            }
            let Some(base) = Snapshot::delta_base(&rx.unreliable) else {
                continue;
            };
            let base_snap = match base {
                Some(t) => match self.snapshots.get(t) {
                    Some(s) => Some(s.clone()),
                    None => continue, // baseline gone: wait for a newer one
                },
                None => None,
            };
            let Some(snap) = Snapshot::decode_delta(&rx.unreliable, base_snap.as_ref()) else {
                continue;
            };
            self.snaps_received += 1;
            self.on_snapshot(now_us, snap);
        }
    }

    fn connectionless(&mut self, now_us: u64, data: &[u8]) {
        match Connectionless::decode(data) {
            Some(Connectionless::Challenge { nonce, cookie })
                if self.state == ClientState::Connecting =>
            {
                let r = Connectionless::ConnectResponse {
                    version: PROTOCOL_VERSION,
                    nonce,
                    cookie,
                    name: self.name.clone(),
                };
                let _ = self.transport.send_to(self.server, &r.encode());
                self.response_sent_us = Some(now_us);
            }
            Some(Connectionless::Accept {
                player,
                tick_rate,
                config_toml,
            }) if self.state == ClientState::Connecting => {
                self.tick_us = 1_000_000.0 / tick_rate.max(1) as f64;
                if let Some(t) = self.response_sent_us {
                    self.handshake_rtt_us = now_us.saturating_sub(t) as f64;
                }
                self.config = GameConfig::from_toml_str(&config_toml).ok();
                self.state = ClientState::Connected {
                    player: PlayerId(player),
                };
            }
            Some(Connectionless::Reject { reason }) if self.state == ClientState::Connecting => {
                self.state = ClientState::Rejected(reason);
            }
            _ => {}
        }
    }

    fn on_snapshot(&mut self, now_us: u64, snap: Snapshot) {
        let is_newest = self.newest.is_none_or(|(t, _)| snap.tick > t);
        if is_newest {
            if let Some((t0, r0)) = self.newest {
                // Arrival jitter: deviation of the receive interval from the tick interval.
                let expect = (snap.tick.0 - t0.0) as f64 * self.tick_us;
                let actual = now_us.saturating_sub(r0) as f64;
                let dev = (actual - expect).abs();
                self.arrival_jitter_us += (dev - self.arrival_jitter_us) * 0.1;
            }
            self.newest = Some((snap.tick, now_us));
        }
        self.snapshots.push(snap.clone());
        self.fresh.push(snap);
    }

    /// Snapshots received since the last call (in arrival order).
    pub fn take_snapshots(&mut self) -> Vec<Snapshot> {
        std::mem::take(&mut self.fresh)
    }

    /// Smoothed RTT from packet acks, or the handshake measurement before the first ack.
    pub fn rtt_us(&self) -> f64 {
        if self.conn.stats.rtt_us > 0.0 {
            self.conn.stats.rtt_us
        } else {
            self.handshake_rtt_us
        }
    }

    /// Newest snapshot tick seen and the estimated server tick now (fractional).
    pub fn estimated_server_tick(&self, now_us: u64) -> Option<f64> {
        let (t, recv) = self.newest?;
        let one_way = self.rtt_us() / 2.0;
        Some(t.0 as f64 + (now_us.saturating_sub(recv) as f64 + one_way) / self.tick_us)
    }

    /// How many ticks ahead of the server the client aims to be.
    pub fn target_lead_ticks(&self) -> f64 {
        let one_way = self.rtt_us() / 2.0;
        let margin = 2.0 * self.conn.stats.rtt_var_us.max(self.arrival_jitter_us);
        (one_way + margin) / self.tick_us + BASE_SAFETY_TICKS
    }

    /// Advance the client clock by real `elapsed_us`; returns the ticks to
    /// generate commands for now (possibly none, possibly several).
    pub fn advance(&mut self, now_us: u64) -> Vec<Tick> {
        let elapsed = self.last_update_us.map_or(0, |t| now_us.saturating_sub(t)) as f64;
        self.last_update_us = Some(now_us);
        let Some(server_tick) = self.estimated_server_tick(now_us) else {
            return vec![]; // no snapshot yet: nothing to align to
        };
        let target_us = (server_tick + self.target_lead_ticks()) * self.tick_us;
        if !self.clock_started || (target_us - self.clock_us).abs() > SNAP_THRESHOLD_US {
            self.clock_us = target_us;
            self.next_tick = (target_us / self.tick_us).floor() as u32;
            self.clock_started = true;
        } else {
            // Close the gap gently: up to ±5% speed change.
            let err = target_us - self.clock_us;
            let scale = 1.0 + (err / (self.tick_us * 20.0)).clamp(-MAX_DILATION, MAX_DILATION);
            self.clock_us += elapsed * scale;
        }
        let mut due = Vec::new();
        while (self.next_tick as f64 + 1.0) * self.tick_us <= self.clock_us {
            due.push(Tick(self.next_tick));
            self.next_tick += 1;
        }
        due
    }

    /// Current client tick (next to generate) and fraction toward the one after.
    pub fn client_tick(&self) -> (Tick, f64) {
        let f = self.clock_us / self.tick_us;
        (Tick(self.next_tick), (f - f.floor()).clamp(0.0, 1.0))
    }

    /// Send `cmd` (plus the previous ones for redundancy) and ack the newest snapshot.
    pub fn send_cmd(&mut self, now_us: u64, cmd: UserCmd) {
        if self.player().is_none() {
            return;
        }
        self.sent_cmds.push_back(cmd);
        while self.sent_cmds.len() > 256 {
            self.sent_cmds.pop_front();
        }
        let from = self.sent_cmds.len().saturating_sub(INPUT_REDUNDANCY);
        let inp = Inputs {
            ack_snapshot: self.newest.map(|(t, _)| t),
            cmds: self.sent_cmds.iter().skip(from).copied().collect(),
        };
        let pkt = self.conn.build_packet(now_us, &inp.encode());
        let _ = self.transport.send_to(self.server, &pkt);
    }

    pub fn disconnect(&mut self, now_us: u64) {
        self.conn.send_reliable(
            Control::Disconnect {
                reason: "quit".into(),
            }
            .encode(),
        );
        let pkt = self.conn.build_packet(now_us, &[]);
        let _ = self.transport.send_to(self.server, &pkt);
        self.state = ClientState::Disconnected("quit".into());
    }
}
