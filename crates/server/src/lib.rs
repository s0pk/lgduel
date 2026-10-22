//! Dedicated server: authoritative fixed-tick simulation behind the network
//! protocol. Generic over `Transport`, so the same code runs on real UDP and on
//! the deterministic in-memory network used by tests.

use lgduel_net::PROTOCOL_VERSION;
pub mod host;

use lgduel_net::conn::Connection;
use lgduel_net::protocol::{Connectionless, Control, InputBuffer, Inputs, cookie};
use lgduel_net::snapshot::{Snapshot, SnapshotHistory};
use lgduel_net::transport::Transport;
use lgduel_sim::config::{ConfigError, GameConfig, HitMode};
use lgduel_sim::game::{Game, GameEvent, PlayerId};
use lgduel_sim::tick::Tick;
use lgduel_sim::usercmd::UserCmd;
use lgduel_sim::world::{Aabb, World};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::net::SocketAddr;

/// `[server]` section of the server config file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ServerSettings {
    pub name: String,
    pub bind: String,
    pub port: u16,
    pub max_clients: u8,
    /// Send a snapshot every N ticks (1 = every tick).
    pub snapshot_every: u32,
    /// Drop a client after this long without packets.
    pub timeout_ms: u32,
    /// Built-in map name or a path to a map TOML file.
    pub map: String,
}

impl Default for ServerSettings {
    fn default() -> Self {
        Self {
            name: "LGDuel server".into(),
            bind: "0.0.0.0".into(),
            port: 27960,
            max_clients: 16,
            snapshot_every: 1,
            timeout_ms: 5000,
            map: "arena_flat".into(),
        }
    }
}

/// Whole server config file: `[server]` plus the gameplay sections of `GameConfig`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ServerConfig {
    pub server: ServerSettings,
    pub game: GameConfig,
}

impl ServerConfig {
    pub fn from_toml_str(s: &str) -> Result<Self, ConfigError> {
        let mut table: toml::Table =
            toml::from_str(s).map_err(|e| ConfigError::Parse(e.to_string()))?;
        let server = match table.remove("server") {
            Some(v) => v
                .try_into()
                .map_err(|e: toml::de::Error| ConfigError::Parse(e.to_string()))?,
            None => ServerSettings::default(),
        };
        let rest = toml::to_string(&table).map_err(|e| ConfigError::Parse(e.to_string()))?;
        let game = GameConfig::from_toml_str(&rest)?;
        let cfg = Self { server, game };
        if cfg.server.snapshot_every == 0 || cfg.server.max_clients == 0 {
            return Err(ConfigError::Invalid(
                "snapshot_every and max_clients must be > 0".into(),
            ));
        }
        Ok(cfg)
    }
}

struct Client {
    addr: SocketAddr,
    conn: Connection,
    player: PlayerId,
    name: String,
    inputs: InputBuffer,
    history: SnapshotHistory,
    acked: Option<Tick>,
    last_heard_us: u64,
    last_cmd: Option<UserCmd>,
    last_consumed: Option<Tick>,
    pending_events: Vec<GameEvent>,
    /// Ticks simulated with a repeated (missing) command.
    pub missing: u64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ClientInfo {
    pub player: PlayerId,
    pub rtt_ms: f64,
    pub loss: f64,
    pub missing_cmds: u64,
    pub buffered_cmds: usize,
}

pub struct Server<T: Transport> {
    pub transport: T,
    pub game: Game,
    pub settings: ServerSettings,
    clients: Vec<Client>,
    secret: u64,
    /// Log lines for the operator (connects, disconnects).
    pub log: Vec<String>,
    /// Rewind buffer: (snapshot tick, hitboxes of living players) per tick.
    rewind: VecDeque<(u32, Vec<(PlayerId, Aabb)>)>,
    /// (name, final stats) of clients that left.
    departed: Vec<(String, lgduel_sim::game::PlayerStats, u64)>,
    /// Anti-lag diagnostics: how far back shots were rewound (ticks), and how
    /// many requests exceeded `max_rewind_ms` and were clamped.
    pub rewind_stats: RewindStats,
    /// `World::content_hash` of the map (sent in the accept).
    map_hash: u64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct RewindStats {
    pub shots_rewound: u64,
    pub sum_rewind_ticks: f64,
    pub clamped: u64,
}

/// Hitbox of `id` at fractional snapshot tick `t`, interpolated between the two
/// recorded ticks around it (matching what the client's interpolation showed).
fn rewound_box(
    history: &VecDeque<(u32, Vec<(PlayerId, Aabb)>)>,
    id: PlayerId,
    t: f64,
) -> Option<Aabb> {
    let lo = t.floor() as u32;
    let frac = (t - t.floor()) as f32;
    let find = |tick: u32| {
        history
            .iter()
            .find(|(ht, _)| *ht == tick)
            .and_then(|(_, b)| b.iter().find(|(p, _)| *p == id).map(|(_, a)| *a))
    };
    let a = find(lo)?;
    let b = find(lo + 1).unwrap_or(a);
    let lerp = |x: lgduel_sim::math::Vec3, y: lgduel_sim::math::Vec3| x + (y - x) * frac;
    Some(Aabb::new(lerp(a.min, b.min), lerp(a.max, b.max)))
}

impl<T: Transport> Server<T> {
    pub fn new(transport: T, cfg: ServerConfig, world: World, secret: u64) -> Self {
        Self {
            transport,
            map_hash: world.content_hash(),
            game: Game::new(cfg.game, world),
            settings: cfg.server,
            clients: Vec::new(),
            secret,
            log: Vec::new(),
            rewind: VecDeque::new(),
            departed: Vec::new(),
            rewind_stats: RewindStats::default(),
        }
    }

    /// Per-player stats as a TOML document (written at shutdown with `--stats-file`).
    /// Includes players that already left (their last known stats).
    pub fn stats_toml(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!(
            "# LGDuel server stats
ticks = {}
mode = \"{:?}\"
lifesteal_percent = {}
hit_mode = \"{:?}\"
",
            self.game.tick.0,
            self.game.cfg.rules.mode,
            self.game.cfg.rules.lifesteal_percent,
            self.game.cfg.rules.hit_mode
        ));
        let current = self.clients.iter().filter_map(|c| {
            self.game
                .player(c.player)
                .map(|p| (c.name.clone(), p.stats, c.missing))
        });
        for (name, s, missing) in self.departed.iter().cloned().chain(current) {
            out.push_str(&format!(
                "
[[players]]
name = {name:?}
shots = {}
hits = {}
accuracy_pct = {:.2}
damage_dealt = {}
damage_taken = {}
healed = {}
lifesteal_earned = {}
kills = {}
deaths = {}
fights_won = {}
fights_lost = {}
fight_ticks = {}
missing_cmds = {missing}
",
                s.shots,
                s.hits,
                s.accuracy_pct(),
                s.damage_dealt,
                s.damage_taken,
                s.healed,
                s.lifesteal_earned,
                s.kills,
                s.deaths,
                s.fights_won,
                s.fights_lost,
                s.fight_ticks
            ));
        }
        out
    }

    pub fn client_count(&self) -> usize {
        self.clients.len()
    }

    pub fn clients(&self) -> Vec<ClientInfo> {
        self.clients
            .iter()
            .map(|c| ClientInfo {
                player: c.player,
                rtt_ms: c.conn.stats.rtt_us / 1000.0,
                loss: c.conn.stats.loss(),
                missing_cmds: c.missing,
                buffered_cmds: c.inputs.buffered(),
            })
            .collect()
    }

    /// Read and handle every pending datagram.
    pub fn poll(&mut self, now_us: u64) {
        while let Some((from, data)) = self.transport.recv() {
            if data.first() == Some(&lgduel_net::protocol::CONNECTIONLESS) {
                if let Some(m) = Connectionless::decode(&data) {
                    self.connectionless(now_us, from, m);
                }
                continue;
            }
            let Some(i) = self.clients.iter().position(|c| c.addr == from) else {
                continue;
            };
            let Some(rx) = self.clients[i].conn.on_packet(now_us, &data) else {
                continue;
            };
            let c = &mut self.clients[i];
            c.last_heard_us = now_us;
            if let Some(inp) = Inputs::decode(&rx.unreliable) {
                if let Some(t) = inp.ack_snapshot
                    && c.acked.is_none_or(|a| t > a)
                {
                    c.acked = Some(t);
                }
                for (i, cmd) in inp.cmds.into_iter().enumerate() {
                    c.inputs
                        .push_with_view(cmd, inp.views.get(i).copied().flatten());
                }
            }
            let mut leave = None;
            for m in rx.reliable {
                if let Some(Control::Disconnect { reason }) = Control::decode(&m) {
                    leave = Some(reason);
                }
            }
            if let Some(reason) = leave {
                self.drop_client(i, &format!("left ({reason})"));
            }
        }
        // Timeouts.
        let timeout = self.settings.timeout_ms as u64 * 1000;
        let mut i = 0;
        while i < self.clients.len() {
            if now_us.saturating_sub(self.clients[i].last_heard_us) > timeout {
                self.drop_client(i, "timed out");
            } else {
                i += 1;
            }
        }
    }

    fn reply(&mut self, to: SocketAddr, m: Connectionless) {
        let _ = self.transport.send_to(to, &m.encode());
    }

    /// Public description for the server browser.
    pub fn info(&self) -> lgduel_net::protocol::ServerInfo {
        let r = &self.game.cfg.rules;
        lgduel_net::protocol::ServerInfo {
            protocol: PROTOCOL_VERSION,
            name: self.settings.name.clone(),
            map: self.game.world.name.clone(),
            players: self.clients.len().min(255) as u8,
            max_players: self.settings.max_clients,
            classic: r.mode == lgduel_sim::config::DuelMode::Classic,
            lifesteal_percent: r.lifesteal_percent.min(100) as u8,
            anti_lag: r.hit_mode == HitMode::AntiLag,
            tick_rate: self.game.cfg.tick_rate_hz.min(u16::MAX as u32) as u16,
        }
    }

    fn connectionless(&mut self, now_us: u64, from: SocketAddr, m: Connectionless) {
        match m {
            Connectionless::InfoRequest { nonce } => {
                let info = self.info();
                self.reply(from, Connectionless::InfoResponse { nonce, info });
            }
            Connectionless::ConnectRequest { version, nonce } => {
                if version != PROTOCOL_VERSION {
                    let reason = format!("protocol {version} != server {PROTOCOL_VERSION}");
                    self.reply(from, Connectionless::Reject { reason });
                    return;
                }
                let c = cookie(self.secret, &from, nonce);
                self.reply(from, Connectionless::Challenge { nonce, cookie: c });
            }
            Connectionless::ConnectResponse {
                version,
                nonce,
                cookie: ck,
                name,
            } => {
                if version != PROTOCOL_VERSION || ck != cookie(self.secret, &from, nonce) {
                    return; // spoofed or stale: ignore silently
                }
                // Re-sent response from an already-connected client: just re-accept.
                if let Some(c) = self.clients.iter().find(|c| c.addr == from) {
                    let player = c.player.0;
                    self.accept(from, player);
                    return;
                }
                if self.clients.len() >= self.settings.max_clients as usize {
                    self.reply(
                        from,
                        Connectionless::Reject {
                            reason: "server full".into(),
                        },
                    );
                    return;
                }
                let player = self.game.add_player();
                self.log.push(format!(
                    "{name} connected from {from} as player {}",
                    player.0
                ));
                self.clients.push(Client {
                    addr: from,
                    conn: Connection::new(),
                    player,
                    name,
                    inputs: InputBuffer::default(),
                    history: SnapshotHistory::new(64),
                    acked: None,
                    last_heard_us: now_us,
                    last_cmd: None,
                    last_consumed: None,
                    pending_events: Vec::new(),
                    missing: 0,
                });
                self.accept(from, player.0);
            }
            _ => {}
        }
    }

    fn accept(&mut self, to: SocketAddr, player: u8) {
        let m = Connectionless::Accept {
            player,
            tick_rate: self.game.cfg.tick_rate_hz,
            config_toml: self.game.cfg.to_toml_string(),
            map: self.game.world.name.clone(),
            map_hash: self.map_hash,
        };
        self.reply(to, m);
    }

    fn drop_client(&mut self, i: usize, why: &str) {
        let c = self.clients.remove(i);
        if let Some(p) = self.game.player(c.player) {
            self.departed.push((c.name.clone(), p.stats, c.missing));
        }
        self.game.remove_player(c.player);
        self.log
            .push(format!("{} (player {}) {why}", c.name, c.player.0));
    }

    /// Simulate one tick and send snapshots.
    pub fn tick(&mut self, now_us: u64) {
        let tick = self.game.tick;
        let mut cmds = Vec::with_capacity(self.clients.len());
        let mut views: Vec<(PlayerId, f64)> = Vec::new();
        for c in &mut self.clients {
            let cmd = match c.inputs.take_with_view(tick) {
                Some((cmd, view)) => {
                    c.last_consumed = Some(tick);
                    c.last_cmd = Some(cmd);
                    if let Some(v) = view {
                        views.push((c.player, v));
                    }
                    cmd
                }
                None => {
                    // Missing (late/lost): repeat the last real command for this tick.
                    // Ticks before the client's first command don't count as missing.
                    c.missing += u64::from(c.last_cmd.is_some());
                    UserCmd {
                        tick,
                        ..c.last_cmd.unwrap_or_default()
                    }
                }
            };
            cmds.push((c.player, cmd));
        }
        // Anti-lag: test each shooter's beam against targets where that shooter saw them.
        let anti_lag = self.game.cfg.rules.hit_mode == HitMode::AntiLag;
        let max_rewind_ticks =
            self.game.cfg.rules.max_rewind_ms as f64 * self.game.cfg.tick_rate_hz as f64 / 1000.0;
        // The newest recorded state is the current one (snapshot tick == game.tick).
        let now_t = tick.0 as f64;
        let clamped_views: Vec<(PlayerId, f64)> = views
            .iter()
            .map(|(p, v)| (*p, v.clamp(now_t - max_rewind_ticks, now_t)))
            .collect();
        let history = &self.rewind;
        let rewind = |shooter: PlayerId, target: PlayerId| -> Option<Aabb> {
            if !anti_lag {
                return None;
            }
            let (_, v) = clamped_views.iter().find(|(p, _)| *p == shooter)?;
            rewound_box(history, target, *v)
        };
        let shots_before: u64 = self.game.players.iter().map(|p| p.stats.shots as u64).sum();
        self.game.step_with(&cmds, &rewind);
        if anti_lag {
            let shots_after: u64 = self.game.players.iter().map(|p| p.stats.shots as u64).sum();
            for (p, v) in &views {
                if shots_after > shots_before
                    && self
                        .game
                        .events
                        .iter()
                        .any(|e| matches!(e, GameEvent::Fire { shooter, .. } if shooter == p))
                {
                    self.rewind_stats.shots_rewound += 1;
                    self.rewind_stats.sum_rewind_ticks +=
                        (now_t - v.clamp(now_t - max_rewind_ticks, now_t)).max(0.0);
                    if *v < now_t - max_rewind_ticks {
                        self.rewind_stats.clamped += 1;
                    }
                }
            }
        }
        // Record hitboxes for future rewinds, labelled like snapshots.
        let boxes = self
            .game
            .players
            .iter()
            .filter(|p| p.alive)
            .map(|p| (p.id, self.game.hitbox(p)))
            .collect();
        self.rewind.push_back((self.game.tick.0, boxes));
        while self.rewind.len() > 128 {
            self.rewind.pop_front();
        }
        let send = self
            .game
            .tick
            .0
            .is_multiple_of(self.settings.snapshot_every);
        for c in &mut self.clients {
            c.pending_events.extend(self.game.events.iter().cloned());
            if !send {
                continue;
            }
            let snap = Snapshot::capture(
                &self.game,
                std::mem::take(&mut c.pending_events),
                c.last_consumed,
            );
            let base = c.acked.and_then(|t| c.history.get(t));
            let payload = snap.encode_delta(base);
            c.history.push(snap);
            let pkt = c.conn.build_packet(now_us, &payload);
            let _ = self.transport.send_to(c.addr, &pkt);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lgduel_net::protocol::INPUT_REDUNDANCY;
    use lgduel_net::transport::{LinkConditions, MemEndpoint, MemNet};
    use lgduel_sim::math::Vec3;
    use lgduel_sim::world::builtin;

    fn world(cfg: &GameConfig) -> World {
        let p = &cfg.physics;
        World::from_toml_str(
            builtin::ARENA_FLAT,
            (
                Vec3::from_array(p.player_mins),
                Vec3::from_array(p.player_maxs),
            ),
        )
        .unwrap()
    }

    #[test]
    fn config_file_sections() {
        let cfg = ServerConfig::from_toml_str(
            "[server]\nname = \"x\"\nport = 1234\n\n[rules]\nlifesteal_percent = 50\nhit_mode = \"no_anti_lag\"\n",
        )
        .unwrap();
        assert_eq!(cfg.server.name, "x");
        assert_eq!(cfg.server.port, 1234);
        assert_eq!(cfg.server.max_clients, 16);
        assert_eq!(cfg.game.rules.lifesteal_percent, 50);
        assert_eq!(
            ServerConfig::from_toml_str("").unwrap(),
            ServerConfig::default()
        );
        assert!(ServerConfig::from_toml_str("[server]\nprot = 1\n").is_err());
        assert!(ServerConfig::from_toml_str("[rules]\nlifesteal_percent = 500\n").is_err());
        assert!(ServerConfig::from_toml_str("[server]\nsnapshot_every = 0\n").is_err());
        let example = include_str!("../../../config/server.example.toml");
        let parsed = ServerConfig::from_toml_str(example).unwrap();
        assert_eq!(parsed, ServerConfig::default(), "example file = defaults");
    }

    /// Minimal scripted client for server tests.
    struct TestClient {
        ep: MemEndpoint,
        server: SocketAddr,
        conn: Connection,
        player: Option<u8>,
        cmds: Vec<UserCmd>,
        snaps: SnapshotHistory,
        newest: Option<Tick>,
    }

    impl TestClient {
        fn new(net: &MemNet, port: u16, server: SocketAddr) -> Self {
            Self {
                ep: net.endpoint(port),
                server,
                conn: Connection::new(),
                player: None,
                cmds: vec![],
                snaps: SnapshotHistory::new(64),
                newest: None,
            }
        }
        fn connect_request(&mut self) {
            let m = Connectionless::ConnectRequest {
                version: PROTOCOL_VERSION,
                nonce: 42,
            };
            self.ep.send_to(self.server, &m.encode()).unwrap();
        }
        fn pump(&mut self, now_us: u64) {
            while let Some((_, d)) = self.ep.recv() {
                if let Some(m) = Connectionless::decode(&d) {
                    match m {
                        Connectionless::Challenge { nonce, cookie } => {
                            let r = Connectionless::ConnectResponse {
                                version: PROTOCOL_VERSION,
                                nonce,
                                cookie,
                                name: "t".into(),
                            };
                            self.ep.send_to(self.server, &r.encode()).unwrap();
                        }
                        Connectionless::Accept { player, .. } => self.player = Some(player),
                        _ => {}
                    }
                    continue;
                }
                if let Some(rx) = self.conn.on_packet(now_us, &d) {
                    let base = Snapshot::delta_base(&rx.unreliable).flatten();
                    let b = base.and_then(|t| self.snaps.get(t));
                    if let Some(s) = Snapshot::decode_delta(&rx.unreliable, b) {
                        self.newest = Some(self.newest.map_or(s.tick, |n| n.max(s.tick)));
                        self.snaps.push(s);
                    }
                }
            }
        }
        fn send_input(&mut self, now_us: u64, cmd: UserCmd) {
            self.cmds.push(cmd);
            let from = self.cmds.len().saturating_sub(INPUT_REDUNDANCY);
            let inp = Inputs {
                ack_snapshot: self.newest,
                cmds: self.cmds[from..].to_vec(),
                views: vec![],
            };
            let pkt = self.conn.build_packet(now_us, &inp.encode());
            self.ep.send_to(self.server, &pkt).unwrap();
        }
    }

    fn setup(c: LinkConditions) -> (MemNet, Server<MemEndpoint>, TestClient) {
        let net = MemNet::new(9);
        net.set_conditions(c);
        let cfg = ServerConfig::default();
        let server = Server::new(net.endpoint(27960), cfg.clone(), world(&cfg.game), 0x5EC2E7);
        let client = TestClient::new(&net, 5000, server.transport.local_addr());
        (net, server, client)
    }

    #[test]
    fn handshake_inputs_and_delta_snapshots() {
        let (net, mut server, mut client) = setup(LinkConditions {
            latency_us: 10_000,
            ..Default::default()
        });
        client.connect_request();
        let mut t = 0u64;
        while client.player.is_none() && t < 1_000_000 {
            t += 1_000;
            net.advance_to(t);
            server.poll(t);
            client.pump(t);
        }
        let me = PlayerId(client.player.expect("accepted"));
        assert_eq!(server.client_count(), 1);
        // Run 2 s: the client sends "walk forward" for ticks well ahead of the server.
        for _ in 0..250 {
            let lead = Tick(server.game.tick.0 + 3);
            client.send_input(
                t,
                UserCmd {
                    tick: lead,
                    forward: 127,
                    ..UserCmd::default()
                },
            );
            t += 8_000;
            net.advance_to(t);
            server.poll(t);
            server.tick(t);
            client.pump(t);
        }
        let snap = client.snaps.latest().expect("snapshots arrive");
        let p = snap.player(me).unwrap();
        assert!(
            p.pm.velocity.length_xy() > 300.0,
            "server applied our inputs: {:?}",
            p.pm.velocity
        );
        assert_eq!(server.clients()[0].missing_cmds, 0);
        assert!(snap.tick.0 > 240);
        assert!(server.clients()[0].rtt_ms > 15.0, "{:?}", server.clients());
    }

    #[test]
    fn stats_toml_parses_and_keeps_departed_players() {
        let (net, mut server, mut client) = setup(LinkConditions::default());
        client.connect_request();
        for t in 1..20 {
            net.advance_to(t);
            server.poll(t);
            client.pump(t);
        }
        server.tick(20);
        server.poll(10_000_000); // times out → departed
        let doc: toml::Table = toml::from_str(&server.stats_toml()).unwrap();
        let players = doc["players"].as_array().unwrap();
        assert_eq!(players.len(), 1);
        assert_eq!(players[0]["name"].as_str(), Some("t"));
        assert_eq!(doc["lifesteal_percent"].as_integer(), Some(100));
    }

    #[test]
    fn spoofed_response_and_wrong_version_rejected() {
        let (net, mut server, mut client) = setup(LinkConditions::default());
        // Response with a made-up cookie: ignored.
        let r = Connectionless::ConnectResponse {
            version: PROTOCOL_VERSION,
            nonce: 1,
            cookie: 123,
            name: "x".into(),
        };
        client.ep.send_to(client.server, &r.encode()).unwrap();
        net.advance_to(1);
        server.poll(1);
        assert_eq!(server.client_count(), 0);
        // Wrong protocol version: explicit reject.
        let m = Connectionless::ConnectRequest {
            version: PROTOCOL_VERSION + 1,
            nonce: 1,
        };
        client.ep.send_to(client.server, &m.encode()).unwrap();
        net.advance_to(2);
        server.poll(2);
        let (_, d) = client.ep.recv().unwrap();
        assert!(matches!(
            Connectionless::decode(&d),
            Some(Connectionless::Reject { .. })
        ));
    }

    #[test]
    fn timeout_and_disconnect_remove_player() {
        let (net, mut server, mut client) = setup(LinkConditions::default());
        client.connect_request();
        for t in 1..20 {
            net.advance_to(t);
            server.poll(t);
            client.pump(t);
        }
        assert_eq!(server.client_count(), 1);
        assert_eq!(server.game.players.len(), 1);
        server.poll(6_000_000);
        assert_eq!(server.client_count(), 0);
        assert_eq!(server.game.players.len(), 0);
        assert!(server.log.iter().any(|l| l.contains("timed out")));
    }

    #[test]
    fn redundant_inputs_cover_loss() {
        let (net, mut server, mut client) = setup(LinkConditions {
            latency_us: 20_000,
            jitter_us: 5_000,
            loss: 0.05,
        });
        client.connect_request();
        let mut t = 0;
        while client.player.is_none() {
            t += 1_000;
            net.advance_to(t);
            server.poll(t);
            client.pump(t);
            client.connect_request();
        }
        for _ in 0..1250 {
            let lead = Tick(server.game.tick.0 + 6);
            client.send_input(
                t,
                UserCmd {
                    tick: lead,
                    right: 127,
                    ..UserCmd::default()
                },
            );
            t += 8_000;
            net.advance_to(t);
            server.poll(t);
            server.tick(t);
            client.pump(t);
        }
        let info = server.clients()[0];
        // 5% loss with 8× redundancy: a tick's command is lost only if 8 packets in a row are.
        assert_eq!(info.missing_cmds, 0, "{info:?}");
        assert!(info.loss > 0.02 && info.loss < 0.1, "{info:?}");
    }
}
