//! Rendezvous + relay service (`lgduel-server relay`): hands out join codes to
//! hosts, tells joining clients where the host is (and the host who is coming,
//! so both punch through their NATs), and relays game traffic for pairs whose
//! NATs can't be punched. Stateless apart from short-lived tables; never
//! answers a stranger with more bytes than it received.

use lgduel_net::rendezvous::{CODE_SPACE, Rdv};
use lgduel_net::transport::Transport;
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};

/// Hosts that stop re-registering are forgotten after this long.
const HOST_TTL_US: u64 = 30_000_000;
/// Idle relay sessions are closed after this long.
const SESSION_TTL_US: u64 = 30_000_000;
/// Lookups / relay requests per source IP per minute.
const REQUESTS_PER_MINUTE: u32 = 30;
/// Relay budget per session and direction: bytes per second.
pub const RELAY_BYTES_PER_SEC: u64 = 256 * 1024;

struct Host {
    addr: SocketAddr,
    last_seen: u64,
    /// In the public server list.
    listed: bool,
}

struct Session {
    client: SocketAddr,
    host: SocketAddr,
    last_seen: u64,
    /// (window start µs, bytes client→host, bytes host→client) in this second.
    window: (u64, u64, u64),
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct RelayStats {
    pub registered: u64,
    pub lookups: u64,
    pub sessions_opened: u64,
    pub relayed_datagrams: u64,
    pub dropped: u64,
    pub lists_sent: u64,
}

pub struct Rendezvous<T: Transport> {
    pub transport: T,
    hosts: HashMap<u32, Host>,
    codes: HashMap<SocketAddr, u32>,
    sessions: HashMap<u32, Session>,
    requests: HashMap<IpAddr, (u32, u64)>,
    rng: u64,
    /// Key for list-request cookies.
    secret: u64,
    pub stats: RelayStats,
    pub log: Vec<String>,
}

impl<T: Transport> Rendezvous<T> {
    pub fn new(transport: T, seed: u64) -> Self {
        Self {
            transport,
            hosts: HashMap::new(),
            codes: HashMap::new(),
            sessions: HashMap::new(),
            requests: HashMap::new(),
            rng: seed | 1,
            secret: seed.rotate_left(29) ^ 0x5EC2_E7C0_0C1E,
            stats: RelayStats::default(),
            log: Vec::new(),
        }
    }

    fn random(&mut self) -> u64 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        self.rng
    }

    pub fn hosts(&self) -> usize {
        self.hosts.len()
    }

    /// Servers in the public list.
    pub fn listed(&self) -> usize {
        self.hosts.values().filter(|h| h.listed).count()
    }

    pub fn sessions(&self) -> usize {
        self.sessions.len()
    }

    /// Rate limit for lookups and relay requests (code guessing, floods).
    fn allow(&mut self, ip: IpAddr, now: u64) -> bool {
        let e = self.requests.entry(ip).or_insert((0, now));
        if now.saturating_sub(e.1) > 60_000_000 {
            *e = (0, now);
        }
        e.0 += 1;
        e.0 <= REQUESTS_PER_MINUTE
    }

    fn send(&mut self, to: SocketAddr, m: &Rdv) {
        let _ = self.transport.send_to(to, &m.encode());
    }

    pub fn poll(&mut self, now: u64) {
        while let Some((from, data)) = self.transport.recv() {
            let Some(m) = Rdv::decode(&data) else {
                continue;
            };
            match m {
                Rdv::Register { nonce, listed } => {
                    let code = match self.codes.get(&from) {
                        Some(c) => *c,
                        None => {
                            let mut c = (self.random() % CODE_SPACE as u64) as u32;
                            while self.hosts.contains_key(&c) {
                                c = (c + 1) % CODE_SPACE;
                            }
                            self.codes.insert(from, c);
                            self.stats.registered += 1;
                            self.log.push(format!(
                                "host {from} registered as {}",
                                lgduel_net::rendezvous::format_code(c)
                            ));
                            c
                        }
                    };
                    self.hosts.insert(
                        code,
                        Host {
                            addr: from,
                            last_seen: now,
                            listed,
                        },
                    );
                    self.send(from, &Rdv::Registered { nonce, code });
                }
                Rdv::Lookup { nonce, code } => {
                    if !self.allow(from.ip(), now) {
                        continue;
                    }
                    self.stats.lookups += 1;
                    match self.hosts.get(&code).map(|h| h.addr) {
                        Some(host) => {
                            self.send(from, &Rdv::Found { nonce, host });
                            self.send(host, &Rdv::Incoming { client: from });
                        }
                        None => self.send(from, &Rdv::NotFound { nonce }),
                    }
                }
                Rdv::RelayOpen { nonce, code } => {
                    if !self.allow(from.ip(), now) {
                        continue;
                    }
                    let Some(host) = self.hosts.get(&code).map(|h| h.addr) else {
                        self.send(from, &Rdv::NotFound { nonce });
                        continue;
                    };
                    // Re-sent request: same session.
                    let existing = self
                        .sessions
                        .iter()
                        .find(|(_, s)| s.client == from && s.host == host)
                        .map(|(id, _)| *id);
                    let session = match existing {
                        Some(id) => id,
                        None => {
                            let mut id = self.random() as u32;
                            while id == 0 || self.sessions.contains_key(&id) {
                                id = self.random() as u32;
                            }
                            self.sessions.insert(
                                id,
                                Session {
                                    client: from,
                                    host,
                                    last_seen: now,
                                    window: (now, 0, 0),
                                },
                            );
                            self.stats.sessions_opened += 1;
                            self.log
                                .push(format!("relay {from} <-> {host} (session {id:08x})"));
                            id
                        }
                    };
                    self.send(from, &Rdv::RelayReady { nonce, session });
                }
                Rdv::ListRequest { nonce, cookie } => {
                    let expected = lgduel_net::protocol::cookie(self.secret, &from, nonce);
                    if cookie != expected {
                        // Small answer to an unproven address.
                        self.send(
                            from,
                            &Rdv::ListChallenge {
                                nonce,
                                cookie: expected,
                            },
                        );
                        continue;
                    }
                    if !self.allow(from.ip(), now) {
                        continue;
                    }
                    let mut servers: Vec<SocketAddr> = self
                        .hosts
                        .values()
                        .filter(|h| h.listed)
                        .map(|h| h.addr)
                        .collect();
                    servers.sort();
                    servers.truncate(lgduel_net::rendezvous::MAX_LIST);
                    self.stats.lists_sent += 1;
                    self.send(from, &Rdv::List { nonce, servers });
                }
                Rdv::RelayData { session, payload } => {
                    let Some(s) = self.sessions.get_mut(&session) else {
                        self.stats.dropped += 1;
                        continue;
                    };
                    if now.saturating_sub(s.window.0) >= 1_000_000 {
                        s.window = (now, 0, 0);
                    }
                    let n = payload.len() as u64;
                    let to = if from == s.client {
                        s.window.1 += n;
                        (s.window.1 <= RELAY_BYTES_PER_SEC).then_some(s.host)
                    } else if from == s.host {
                        s.window.2 += n;
                        (s.window.2 <= RELAY_BYTES_PER_SEC).then_some(s.client)
                    } else {
                        None
                    };
                    let Some(to) = to else {
                        self.stats.dropped += 1;
                        continue;
                    };
                    s.last_seen = now;
                    self.stats.relayed_datagrams += 1;
                    self.send(to, &Rdv::RelayData { session, payload });
                }
                _ => {}
            }
        }
        // Expire silent hosts and idle sessions.
        let before = self.hosts.len();
        self.hosts
            .retain(|_, h| now.saturating_sub(h.last_seen) < HOST_TTL_US);
        if self.hosts.len() != before {
            let hosts = &self.hosts;
            self.codes.retain(|_, c| hosts.contains_key(c));
        }
        self.sessions
            .retain(|_, s| now.saturating_sub(s.last_seen) < SESSION_TTL_US);
        self.requests
            .retain(|_, (_, t)| now.saturating_sub(*t) < 120_000_000);
    }
}

/// Run the service on a real UDP port until `duration` (or forever).
pub fn run_relay(bind: &str, port: u16, duration: Option<f64>) -> Result<(), String> {
    use lgduel_net::transport::UdpTransport;
    let addr: SocketAddr = format!("{bind}:{port}")
        .parse()
        .map_err(|e| format!("bad bind address: {e}"))?;
    let t = UdpTransport::bind(addr).map_err(|e| format!("can't use port {port}: {e}"))?;
    println!(
        "lgduel relay (rendezvous + relay) listening on {}",
        t.local_addr()
    );
    let seed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(1, |d| d.as_nanos() as u64);
    let mut r = Rendezvous::new(t, seed);
    let start = std::time::Instant::now();
    let mut logged = 0;
    loop {
        r.poll(start.elapsed().as_micros() as u64);
        for line in &r.log[logged..] {
            println!("{line}");
        }
        logged = r.log.len();
        if duration.is_some_and(|d| start.elapsed().as_secs_f64() >= d) {
            println!(
                "relay shutting down: {:?}, {} hosts, {} sessions",
                r.stats,
                r.hosts(),
                r.sessions()
            );
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_micros(500));
    }
}
