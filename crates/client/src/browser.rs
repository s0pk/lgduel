//! Server browser: the shipped server list + the player's own entries, pinged
//! with the connectionless info query.

use crate::settings::ServerEntry;
use lgduel_net::browser::{PingResult, Pinger};
use lgduel_net::transport::UdpTransport;
use serde::Deserialize;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::time::{Duration, Instant};

/// Servers shipped with the game (`config/servers.toml`).
pub const SHIPPED: &str = include_str!("../../../config/servers.toml");

#[derive(Deserialize)]
struct ShippedFile {
    #[serde(default)]
    server: Vec<ServerEntry>,
}

pub fn shipped_servers() -> Vec<ServerEntry> {
    toml::from_str::<ShippedFile>(SHIPPED)
        .map(|f| f.server)
        .unwrap_or_default()
}

#[derive(Debug, Clone, PartialEq)]
pub enum Status {
    /// Query sent, no answer yet.
    Waiting,
    /// No answer within the timeout.
    NoAnswer,
    /// Address could not be resolved.
    BadAddress(String),
    Online(PingResult),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub name: String,
    pub address: String,
    /// Shipped entry (can't be removed).
    pub official: bool,
    /// Found on the local network (not in the list; can't be removed).
    pub lan: bool,
    pub status: Status,
}

/// Ports LAN discovery asks on (the default and the next few, for a second
/// server or a changed Host port).
pub const LAN_PORTS: [u16; 4] = [27960, 27961, 27962, 27963];

const TIMEOUT: Duration = Duration::from_secs(2);
const AUTO_REFRESH: Duration = Duration::from_secs(5);

pub struct Browser {
    pinger: Option<Pinger<UdpTransport>>,
    start: Instant,
    /// Hostname lookups run on background threads (DNS can block for seconds).
    lookups_tx: std::sync::mpsc::Sender<(String, Result<SocketAddr, String>)>,
    lookups_rx: std::sync::mpsc::Receiver<(String, Result<SocketAddr, String>)>,
    looking_up: std::collections::HashSet<String>,
    /// address text → resolved socket address (or error)
    resolved: HashMap<String, Result<SocketAddr, String>>,
    sent_at: HashMap<String, Instant>,
    last_refresh: Option<Instant>,
    /// Ports to discover servers on (broadcast on the LAN + this PC).
    lan_ports: Vec<u16>,
}

impl Default for Browser {
    fn default() -> Self {
        Self::new()
    }
}

impl Browser {
    pub fn new() -> Self {
        let pinger = UdpTransport::bind("0.0.0.0:0".parse().unwrap())
            .ok()
            .map(|t| {
                let _ = t.set_broadcast(true);
                Pinger::new(t, std::process::id() as u64)
            });
        let (lookups_tx, lookups_rx) = std::sync::mpsc::channel();
        Self {
            pinger,
            start: Instant::now(),
            lookups_tx,
            lookups_rx,
            looking_up: Default::default(),
            resolved: HashMap::new(),
            sent_at: HashMap::new(),
            last_refresh: None,
            lan_ports: LAN_PORTS.to_vec(),
        }
    }

    /// A browser that discovers on these ports instead of `LAN_PORTS`.
    #[cfg(test)]
    pub fn with_lan_ports(ports: &[u16]) -> Self {
        Self {
            lan_ports: ports.to_vec(),
            ..Self::new()
        }
    }

    fn now_us(&self) -> u64 {
        self.start.elapsed().as_micros() as u64
    }

    /// Query every entry again (unresolved names are looked up in the background
    /// and queried as soon as their address is known).
    pub fn refresh(&mut self, entries: &[(ServerEntry, bool)]) {
        for (e, _) in entries {
            match self.resolved.get(&e.address).cloned() {
                Some(Ok(addr)) => self.query(&e.address, addr),
                Some(Err(_)) => {}
                None => self.look_up(&e.address),
            }
        }
        // LAN discovery: broadcast, plus this PC directly (its own broadcasts
        // don't always come back to it).
        let now = self.now_us();
        if let Some(p) = self.pinger.as_mut() {
            for &port in &self.lan_ports {
                p.discover(SocketAddr::from(([255, 255, 255, 255], port)), now);
                p.discover(SocketAddr::from(([127, 0, 0, 1], port)), now);
            }
        }
        self.last_refresh = Some(Instant::now());
    }

    fn query(&mut self, text: &str, addr: SocketAddr) {
        let now = self.now_us();
        if let Some(p) = self.pinger.as_mut() {
            p.query(addr, now);
            self.sent_at.insert(text.to_string(), Instant::now());
        }
    }

    fn look_up(&mut self, text: &str) {
        if !self.looking_up.insert(text.to_string()) {
            return;
        }
        let (tx, text) = (self.lookups_tx.clone(), text.to_string());
        std::thread::spawn(move || {
            let r = crate::resolve(&text);
            let _ = tx.send((text, r));
        });
    }

    /// Call every frame while the browser is visible.
    pub fn update(&mut self, entries: &[(ServerEntry, bool)]) {
        if self.last_refresh.is_none_or(|t| t.elapsed() > AUTO_REFRESH) {
            self.refresh(entries);
        }
        while let Ok((text, r)) = self.lookups_rx.try_recv() {
            self.looking_up.remove(&text);
            if let Ok(addr) = r {
                self.query(&text, addr);
            }
            self.resolved.insert(text, r);
        }
        let now = self.now_us();
        if let Some(p) = self.pinger.as_mut() {
            p.poll(now, TIMEOUT.as_micros() as u64);
        }
    }

    pub fn rows(&self, entries: &[(ServerEntry, bool)]) -> Vec<Row> {
        entries
            .iter()
            .map(|(e, official)| {
                let status = match self.resolved.get(&e.address) {
                    Some(Err(err)) => Status::BadAddress(err.clone()),
                    Some(Ok(addr)) => {
                        match self.pinger.as_ref().and_then(|p| p.results.get(addr)) {
                            // A result older than the latest query is still shown until replaced.
                            Some(r) => Status::Online(r.clone()),
                            None if self
                                .sent_at
                                .get(&e.address)
                                .is_some_and(|t| t.elapsed() < TIMEOUT) =>
                            {
                                Status::Waiting
                            }
                            None => Status::NoAnswer,
                        }
                    }
                    None => Status::Waiting, // still resolving
                };
                Row {
                    name: e.name.clone(),
                    address: e.address.clone(),
                    official: *official,
                    lan: false,
                    status,
                }
            })
            .chain(self.lan_rows(entries))
            .collect()
    }

    /// Discovered servers that aren't already in the list. A server reached on
    /// several addresses (this PC's own: 127.0.0.1 and every network adapter)
    /// is one row, preferring 127.0.0.1.
    fn lan_rows(&self, entries: &[(ServerEntry, bool)]) -> Vec<Row> {
        let Some(p) = self.pinger.as_ref() else {
            return vec![];
        };
        let mut seen: Vec<u64> = entries
            .iter()
            .filter_map(|(e, _)| self.resolved.get(&e.address)?.as_ref().ok())
            .filter_map(|a| p.results.get(a).map(|r| r.info.instance))
            .collect();
        let mut found: Vec<&SocketAddr> = p.discovered.iter().collect();
        found.sort_by_key(|a| !a.ip().is_loopback());
        let mut rows = Vec::new();
        for addr in found {
            let Some(r) = p.results.get(addr) else {
                continue;
            };
            if seen.contains(&r.info.instance) {
                continue;
            }
            seen.push(r.info.instance);
            rows.push(Row {
                name: r.info.name.clone(),
                address: addr.to_string(),
                official: false,
                lan: true,
                status: Status::Online(r.clone()),
            });
        }
        rows
    }

    /// Forget a server's resolution (after the address list changed).
    pub fn forget(&mut self, address: &str) {
        self.resolved.remove(address);
        self.sent_at.remove(address);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shipped_list_parses() {
        let s = shipped_servers();
        assert!(!s.is_empty());
        assert!(
            s.iter()
                .all(|e| !e.name.is_empty() && e.address.contains(':'))
        );
    }

    #[test]
    fn rows_report_bad_address_and_live_server() {
        // A real local server answers the query.
        let cfg = lgduel_server::ServerConfig::default();
        let p = &cfg.game.physics;
        let world = lgduel_sim::world::World::from_toml_str(
            lgduel_sim::world::builtin::ARENA_FLAT,
            (
                lgduel_sim::math::Vec3::from_array(p.player_mins),
                lgduel_sim::math::Vec3::from_array(p.player_maxs),
            ),
        )
        .unwrap();
        let t = UdpTransport::bind("127.0.0.1:0".parse().unwrap()).unwrap();
        let port = lgduel_net::transport::Transport::local_addr(&t).port();
        let mut server = lgduel_server::Server::new(t, cfg, world, 1);
        let entries = vec![
            (
                ServerEntry {
                    name: "local".into(),
                    address: format!("127.0.0.1:{port}"),
                },
                false,
            ),
            (
                ServerEntry {
                    name: "bad".into(),
                    address: "no.such.host.invalid:1".into(),
                },
                false,
            ),
        ];
        let mut b = Browser::new();
        b.refresh(&entries);
        let start = Instant::now();
        let mut rows = b.rows(&entries);
        // Wait for the live server and for the bad name's lookup to fail (in the background).
        while start.elapsed() < Duration::from_secs(10)
            && !(matches!(rows[0].status, Status::Online(_))
                && matches!(rows[1].status, Status::BadAddress(_)))
        {
            server.poll(start.elapsed().as_micros() as u64);
            b.update(&entries);
            rows = b.rows(&entries);
            std::thread::sleep(Duration::from_millis(1));
        }
        match &rows[0].status {
            Status::Online(r) => {
                assert_eq!(r.info.max_players, 16);
                assert!(r.rtt_ms < 100.0);
            }
            other => panic!("{other:?}"),
        }
        assert!(
            matches!(rows[1].status, Status::BadAddress(_)),
            "{:?}",
            rows[1].status
        );
    }

    #[test]
    fn lan_discovery_finds_an_unlisted_server_once() {
        let mut cfg = lgduel_server::ServerConfig::default();
        cfg.server.name = "lan test".into();
        let p = &cfg.game.physics;
        let world = lgduel_sim::world::World::from_toml_str(
            lgduel_sim::world::builtin::ARENA_FLAT,
            (
                lgduel_sim::math::Vec3::from_array(p.player_mins),
                lgduel_sim::math::Vec3::from_array(p.player_maxs),
            ),
        )
        .unwrap();
        // Bound to every interface, like a real host: reachable on 127.0.0.1 and the LAN address.
        let t = UdpTransport::bind("0.0.0.0:0".parse().unwrap()).unwrap();
        let port = lgduel_net::transport::Transport::local_addr(&t).port();
        let mut server = lgduel_server::Server::new(t, cfg, world, 1);
        let mut b = Browser::with_lan_ports(&[port]);
        let entries = vec![];
        b.refresh(&entries);
        let start = Instant::now();
        let mut rows = b.rows(&entries);
        while start.elapsed() < Duration::from_secs(3) && rows.is_empty() {
            server.poll(start.elapsed().as_micros() as u64);
            b.update(&entries);
            rows = b.rows(&entries);
            std::thread::sleep(Duration::from_millis(1));
        }
        // Give a broadcast answer from the LAN address time to arrive too.
        let t = Instant::now();
        while t.elapsed() < Duration::from_millis(300) {
            server.poll(start.elapsed().as_micros() as u64);
            b.update(&entries);
            std::thread::sleep(Duration::from_millis(1));
        }
        let rows = b.rows(&entries);
        assert_eq!(rows.len(), 1, "one row for this PC's server: {rows:?}");
        assert!(rows[0].lan);
        assert_eq!(rows[0].name, "lan test");
        assert!(rows[0].address.ends_with(&format!(":{port}")));
        // Listed already (as 127.0.0.1): no extra LAN row.
        let listed = vec![(
            ServerEntry {
                name: "mine".into(),
                address: format!("127.0.0.1:{port}"),
            },
            false,
        )];
        b.refresh(&listed);
        let t = Instant::now();
        while t.elapsed() < Duration::from_secs(3)
            && !matches!(b.rows(&listed)[0].status, Status::Online(_))
        {
            server.poll(start.elapsed().as_micros() as u64);
            b.update(&listed);
            std::thread::sleep(Duration::from_millis(1));
        }
        let rows = b.rows(&listed);
        assert_eq!(rows.len(), 1, "{rows:?}");
        assert!(!rows[0].lan);
    }
}
