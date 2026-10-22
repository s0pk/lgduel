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
    pub status: Status,
}

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
            .map(|t| Pinger::new(t, std::process::id() as u64));
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
                    status,
                }
            })
            .collect()
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
}
