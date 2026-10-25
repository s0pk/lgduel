//! Join codes without port forwarding: a public rendezvous service tells a
//! joining client the host's public address (both then send to each other,
//! which opens their NATs: "hole punching"), and relays the traffic when a
//! direct path can't be opened (symmetric NATs, strict firewalls).
//!
//! `RelayTransport` wraps the game's socket so the server and client code
//! above it don't change: the host sees relayed clients at virtual addresses,
//! and a joining client always talks to `JOIN_ADDR`, whichever path is used.
//!
//! Datagrams start with `RDV_MAGIC`. Requests a stranger can send are padded
//! to at least the size of their answer (no amplification).

use crate::transport::Transport;
use crate::wire::{Reader, Writer};
use std::collections::VecDeque;
use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

pub const RDV_MAGIC: u8 = 0xFE;
/// Default rendezvous / relay port.
pub const RDV_PORT: u16 = 27950;
/// Size every stranger-sendable request is padded to.
pub const RDV_REQUEST_SIZE: usize = 40;

/// Join-code alphabet: no 0/O, 1/I/L.
const ALPHABET: &[u8] = b"23456789ABCDEFGHJKMNPQRSTUVWXYZ";
/// Number of distinct codes (31^4).
pub const CODE_SPACE: u32 = 31 * 31 * 31 * 31;

/// "LG-7KQ2" for code index `c`.
pub fn format_code(c: u32) -> String {
    let mut c = c % CODE_SPACE;
    let mut s = [0u8; 4];
    for ch in s.iter_mut().rev() {
        *ch = ALPHABET[(c % 31) as usize];
        c /= 31;
    }
    format!("LG-{}", std::str::from_utf8(&s).unwrap())
}

/// Accepts "LG-7KQ2", "lg 7kq2", "7KQ2".
pub fn parse_code(s: &str) -> Option<u32> {
    let s: String = s
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect::<String>()
        .to_ascii_uppercase();
    let s = s.strip_prefix("LG").filter(|r| r.len() == 4).unwrap_or(&s);
    if s.len() != 4 {
        return None;
    }
    s.bytes().try_fold(0u32, |acc, b| {
        let d = ALPHABET.iter().position(|a| *a == b)? as u32;
        Some(acc * 31 + d)
    })
}

/// Digits in a direct code (31^11 > 2^54: 32-bit address, 16-bit port, 6-bit check).
const DIRECT_DIGITS: usize = 11;

fn direct_check(v: u64) -> u64 {
    v.wrapping_mul(0x9E37_79B9_7F4A_7C15) >> 58
}

/// "LG-4F7K-Q29M-XTR": the host's public address and port in a code, for hosts
/// that are reachable directly (router opened the port, public IP): no
/// rendezvous service needed.
pub fn format_direct(a: std::net::SocketAddrV4) -> String {
    let v = (u64::from(u32::from(*a.ip())) << 16) | u64::from(a.port());
    let mut c = (v << 6) | direct_check(v);
    let mut s = [0u8; DIRECT_DIGITS];
    for ch in s.iter_mut().rev() {
        *ch = ALPHABET[(c % 31) as usize];
        c /= 31;
    }
    let s = std::str::from_utf8(&s).unwrap();
    format!("LG-{}-{}-{}", &s[..4], &s[4..8], &s[8..])
}

/// Accepts what `format_direct` makes, in any case and with any separators.
/// Typos are caught by the check bits.
pub fn parse_direct(s: &str) -> Option<std::net::SocketAddrV4> {
    let s: String = s
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect::<String>()
        .to_ascii_uppercase();
    let s = s
        .strip_prefix("LG")
        .filter(|r| r.len() == DIRECT_DIGITS)
        .unwrap_or(&s);
    if s.len() != DIRECT_DIGITS {
        return None;
    }
    let c = s.bytes().try_fold(0u64, |acc, b| {
        let d = ALPHABET.iter().position(|a| *a == b)? as u64;
        Some(acc * 31 + d)
    })?;
    let (v, check) = (c >> 6, c & 63);
    if v >> 48 != 0 || check != direct_check(v) {
        return None;
    }
    let (ip, port) = (Ipv4Addr::from((v >> 16) as u32), v as u16);
    (port != 0).then(|| std::net::SocketAddrV4::new(ip, port))
}

#[derive(Debug, Clone, PartialEq)]
pub enum Rdv {
    /// Host → rendezvous (also every few seconds as keepalive). `listed`:
    /// show this server in everyone's browser (master list).
    Register {
        nonce: u64,
        listed: bool,
    },
    Registered {
        nonce: u64,
        code: u32,
    },
    /// Client → rendezvous.
    Lookup {
        nonce: u64,
        code: u32,
    },
    Found {
        nonce: u64,
        host: SocketAddr,
    },
    NotFound {
        nonce: u64,
    },
    /// Rendezvous → host: this client is about to connect (punch toward it).
    Incoming {
        client: SocketAddr,
    },
    /// Opens the sender's NAT toward the receiver; otherwise ignored.
    Punch,
    /// Client → rendezvous: relay me to the host with this code.
    RelayOpen {
        nonce: u64,
        code: u32,
    },
    RelayReady {
        nonce: u64,
        session: u32,
    },
    /// Game datagram through the relay, either direction.
    RelayData {
        session: u32,
        payload: Vec<u8>,
    },
    /// Browser → master: the public server list. First with `cookie` 0; the
    /// master answers with a `ListChallenge` cookie proving the address, and
    /// only a request carrying it gets the (large) list.
    ListRequest {
        nonce: u64,
        cookie: u64,
    },
    ListChallenge {
        nonce: u64,
        cookie: u64,
    },
    List {
        nonce: u64,
        servers: Vec<SocketAddr>,
    },
}

/// Most servers in one `List` datagram.
pub const MAX_LIST: usize = 100;

fn write_addr(w: &mut Writer, a: SocketAddr) {
    match a.ip() {
        IpAddr::V4(v) => {
            w.u8(4);
            w.raw(&v.octets());
        }
        IpAddr::V6(v) => {
            w.u8(6);
            w.raw(&v.octets());
        }
    }
    w.u16(a.port());
}

fn read_addr(r: &mut Reader) -> Option<SocketAddr> {
    let ip = match r.u8()? {
        4 => {
            let mut b = [0u8; 4];
            for x in &mut b {
                *x = r.u8()?;
            }
            IpAddr::from(b)
        }
        6 => {
            let mut b = [0u8; 16];
            for x in &mut b {
                *x = r.u8()?;
            }
            IpAddr::from(b)
        }
        _ => return None,
    };
    Some(SocketAddr::new(ip, r.u16()?))
}

impl Rdv {
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new();
        w.u8(RDV_MAGIC);
        let pad = match self {
            Self::Register { nonce, listed } => {
                w.u8(1);
                w.u64(*nonce);
                w.u8(*listed as u8);
                true
            }
            Self::Registered { nonce, code } => {
                w.u8(2);
                w.u64(*nonce);
                w.u32(*code);
                false
            }
            Self::Lookup { nonce, code } => {
                w.u8(3);
                w.u64(*nonce);
                w.u32(*code);
                true
            }
            Self::Found { nonce, host } => {
                w.u8(4);
                w.u64(*nonce);
                write_addr(&mut w, *host);
                false
            }
            Self::NotFound { nonce } => {
                w.u8(5);
                w.u64(*nonce);
                false
            }
            Self::Incoming { client } => {
                w.u8(6);
                write_addr(&mut w, *client);
                false
            }
            Self::Punch => {
                w.u8(7);
                false
            }
            Self::RelayOpen { nonce, code } => {
                w.u8(8);
                w.u64(*nonce);
                w.u32(*code);
                true
            }
            Self::RelayReady { nonce, session } => {
                w.u8(9);
                w.u64(*nonce);
                w.u32(*session);
                false
            }
            Self::RelayData { session, payload } => {
                w.u8(10);
                w.u32(*session);
                w.raw(payload);
                false
            }
            Self::ListRequest { nonce, cookie } => {
                w.u8(11);
                w.u64(*nonce);
                w.u64(*cookie);
                true
            }
            Self::ListChallenge { nonce, cookie } => {
                w.u8(12);
                w.u64(*nonce);
                w.u64(*cookie);
                false
            }
            Self::List { nonce, servers } => {
                w.u8(13);
                w.u64(*nonce);
                let n = servers.len().min(MAX_LIST);
                w.u8(n as u8);
                for a in &servers[..n] {
                    write_addr(&mut w, *a);
                }
                false
            }
        };
        if pad {
            let n = RDV_REQUEST_SIZE.saturating_sub(w.buf.len());
            w.raw(&vec![0; n]);
        }
        w.finish()
    }

    pub fn decode(data: &[u8]) -> Option<Rdv> {
        let mut r = Reader::new(data);
        if r.u8()? != RDV_MAGIC {
            return None;
        }
        Some(match r.u8()? {
            1 => Self::Register {
                nonce: r.u64()?,
                listed: r.u8()? != 0,
            },
            2 => Self::Registered {
                nonce: r.u64()?,
                code: r.u32()?,
            },
            3 => Self::Lookup {
                nonce: r.u64()?,
                code: r.u32()?,
            },
            4 => Self::Found {
                nonce: r.u64()?,
                host: read_addr(&mut r)?,
            },
            5 => Self::NotFound { nonce: r.u64()? },
            6 => Self::Incoming {
                client: read_addr(&mut r)?,
            },
            7 => Self::Punch,
            8 => Self::RelayOpen {
                nonce: r.u64()?,
                code: r.u32()?,
            },
            9 => Self::RelayReady {
                nonce: r.u64()?,
                session: r.u32()?,
            },
            10 => Self::RelayData {
                session: r.u32()?,
                payload: r.rest().to_vec(),
            },
            11 => Self::ListRequest {
                nonce: r.u64()?,
                cookie: r.u64()?,
            },
            12 => Self::ListChallenge {
                nonce: r.u64()?,
                cookie: r.u64()?,
            },
            13 => {
                let nonce = r.u64()?;
                let n = (r.u8()? as usize).min(MAX_LIST);
                let mut servers = Vec::with_capacity(n);
                for _ in 0..n {
                    servers.push(read_addr(&mut r)?);
                }
                Self::List { nonce, servers }
            }
            _ => return None,
        })
    }
}

/// Where a joining client always sends its game traffic (the wrapper maps
/// it to the host's real address or to the relay).
pub const JOIN_ADDR: SocketAddr = SocketAddr::new(
    IpAddr::V6(Ipv6Addr::new(0xfd00, 0x4c47, 0, 0, 0, 0, 0, 1)),
    1,
);

/// Virtual address of relayed client `session` on the host.
fn session_addr(session: u32) -> SocketAddr {
    let [a, b] = [(session >> 16) as u16, session as u16];
    SocketAddr::new(
        IpAddr::V6(Ipv6Addr::new(0xfd00, 0x4c47, 0, 1, 0, 0, a, b)),
        1,
    )
}

fn addr_session(a: SocketAddr) -> Option<u32> {
    match a.ip() {
        IpAddr::V6(v) => {
            let s = v.segments();
            (s[..4] == [0xfd00, 0x4c47, 0, 1]).then(|| ((s[6] as u32) << 16) | s[7] as u32)
        }
        IpAddr::V4(_) => None,
    }
}

/// How long a joining client waits for the host directly before asking the relay.
pub const DIRECT_TIMEOUT_US: u64 = 2_500_000;
const RESEND_US: u64 = 500_000;
const GIVE_UP_US: u64 = 6_000_000;
const REGISTER_EVERY_US: u64 = 5_000_000;

/// Joining client's progress.
#[derive(Debug, Clone, PartialEq)]
pub enum JoinState {
    /// Asking the rendezvous where the host is.
    LookingUp,
    /// Talking to the host's public address.
    Direct {
        host: SocketAddr,
        heard: bool,
    },
    /// Direct path didn't open; asking for a relay.
    OpeningRelay,
    Relayed {
        session: u32,
    },
    Failed(String),
}

enum Mode {
    Plain,
    Host {
        code: Option<u32>,
        last_register: Option<u64>,
        nonce: u64,
        listed: bool,
    },
    Join {
        code: u32,
        /// Skip the direct attempt (testing the relay on real sockets).
        force_relay: bool,
        state: JoinState,
        /// When the current state began / the last request went out.
        since: u64,
        last_sent: Option<u64>,
        nonce: u64,
    },
}

/// Game socket wrapper adding rendezvous, hole punching and relaying.
pub struct RelayTransport<T: Transport> {
    pub inner: T,
    rdv: Option<SocketAddr>,
    mode: Mode,
    now_us: u64,
    /// Relayed traffic routed: (datagrams in, datagrams out).
    pub relayed: (u64, u64),
}

impl<T: Transport> RelayTransport<T> {
    /// No rendezvous: a plain socket.
    pub fn plain(inner: T) -> Self {
        Self {
            inner,
            rdv: None,
            mode: Mode::Plain,
            now_us: 0,
            relayed: (0, 0),
        }
    }

    /// A host: registers with `rdv` for a join code and accepts relayed clients.
    pub fn host(inner: T, rdv: SocketAddr, nonce: u64) -> Self {
        Self {
            mode: Mode::Host {
                code: None,
                last_register: None,
                nonce,
                listed: false,
            },
            rdv: Some(rdv),
            ..Self::plain(inner)
        }
    }

    /// A client joining `code`: send game traffic to `JOIN_ADDR`.
    pub fn join(inner: T, rdv: SocketAddr, code: u32, nonce: u64) -> Self {
        Self {
            mode: Mode::Join {
                code,
                force_relay: false,
                state: JoinState::LookingUp,
                since: 0,
                last_sent: None,
                nonce,
            },
            rdv: Some(rdv),
            ..Self::plain(inner)
        }
    }

    /// Host: also appear in the public server list.
    pub fn listed(mut self) -> Self {
        if let Mode::Host { listed, .. } = &mut self.mode {
            *listed = true;
        }
        self
    }

    /// Go straight to the relay once the code is found (no direct attempt).
    pub fn forcing_relay(mut self) -> Self {
        if let Mode::Join { force_relay, .. } = &mut self.mode {
            *force_relay = true;
        }
        self
    }

    /// The host's join code once registered.
    pub fn code(&self) -> Option<u32> {
        match self.mode {
            Mode::Host { code, .. } => code,
            _ => None,
        }
    }

    pub fn join_state(&self) -> Option<&JoinState> {
        match &self.mode {
            Mode::Join { state, .. } => Some(state),
            _ => None,
        }
    }

    fn rdv_send(&mut self, m: &Rdv) {
        if let Some(r) = self.rdv {
            let _ = self.inner.send_to(r, &m.encode());
        }
    }

    /// Advance timers (registration keepalive, lookup/relay retries, fallback).
    pub fn tick(&mut self, now_us: u64) {
        self.now_us = now_us;
        let mut out = None;
        match &mut self.mode {
            Mode::Plain => {}
            Mode::Host {
                last_register,
                nonce,
                listed,
                ..
            } => {
                if last_register.is_none_or(|t| now_us.saturating_sub(t) >= REGISTER_EVERY_US) {
                    *last_register = Some(now_us);
                    out = Some(Rdv::Register {
                        nonce: *nonce,
                        listed: *listed,
                    });
                }
            }
            Mode::Join {
                code,
                state,
                since,
                last_sent,
                nonce,
                ..
            } => {
                let due = last_sent.is_none_or(|t| now_us.saturating_sub(t) >= RESEND_US);
                let waited = now_us.saturating_sub(*since);
                match state {
                    JoinState::LookingUp if waited > GIVE_UP_US && last_sent.is_some() => {
                        *state = JoinState::Failed("the rendezvous service didn't answer".into());
                    }
                    JoinState::LookingUp if due => {
                        if last_sent.is_none() {
                            *since = now_us;
                        }
                        out = Some(Rdv::Lookup {
                            nonce: *nonce,
                            code: *code,
                        });
                    }
                    JoinState::Direct { heard: false, .. } if waited > DIRECT_TIMEOUT_US => {
                        *state = JoinState::OpeningRelay;
                        *since = now_us;
                        out = Some(Rdv::RelayOpen {
                            nonce: *nonce,
                            code: *code,
                        });
                    }
                    JoinState::OpeningRelay if waited > GIVE_UP_US => {
                        *state =
                            JoinState::Failed("no direct path and the relay didn't answer".into());
                    }
                    JoinState::OpeningRelay if due => {
                        out = Some(Rdv::RelayOpen {
                            nonce: *nonce,
                            code: *code,
                        });
                    }
                    _ => {}
                }
                if out.is_some() {
                    *last_sent = Some(now_us);
                }
            }
        }
        if let Some(m) = out {
            self.rdv_send(&m);
        }
    }

    /// Handle a rendezvous datagram; game data it carries is returned.
    fn handle_rdv(&mut self, m: Rdv) -> Option<(SocketAddr, Vec<u8>)> {
        let now = self.now_us;
        match (&mut self.mode, m) {
            (Mode::Host { code, nonce, .. }, Rdv::Registered { nonce: n, code: c })
                if n == *nonce =>
            {
                *code = Some(c);
            }
            (Mode::Host { .. }, Rdv::Incoming { client }) => {
                // Opens our NAT toward the client so its connect gets through.
                let _ = self.inner.send_to(client, &Rdv::Punch.encode());
            }
            (Mode::Host { .. }, Rdv::RelayData { session, payload }) => {
                self.relayed.0 += 1;
                return Some((session_addr(session), payload));
            }
            (
                Mode::Join {
                    state,
                    since,
                    nonce,
                    force_relay,
                    last_sent,
                    ..
                },
                m,
            ) => match m {
                Rdv::Found { nonce: n, host } if n == *nonce && *state == JoinState::LookingUp => {
                    *since = now;
                    if *force_relay {
                        // Relay right away: tick sends RelayOpen.
                        *state = JoinState::OpeningRelay;
                        *last_sent = None;
                    } else {
                        *state = JoinState::Direct { host, heard: false };
                        let _ = self.inner.send_to(host, &Rdv::Punch.encode());
                    }
                }
                Rdv::NotFound { nonce: n } if n == *nonce => {
                    *state = JoinState::Failed("no game with that join code".into());
                }
                Rdv::RelayReady { nonce: n, session }
                    if n == *nonce && *state == JoinState::OpeningRelay =>
                {
                    *state = JoinState::Relayed { session };
                }
                Rdv::RelayData { session, payload } if *state == JoinState::Relayed { session } => {
                    self.relayed.0 += 1;
                    return Some((JOIN_ADDR, payload));
                }
                _ => {}
            },
            _ => {}
        }
        None
    }
}

impl<T: Transport> Transport for RelayTransport<T> {
    fn local_addr(&self) -> SocketAddr {
        self.inner.local_addr()
    }

    fn send_to(&mut self, to: SocketAddr, data: &[u8]) -> io::Result<()> {
        match &self.mode {
            Mode::Host { .. } => match (addr_session(to), self.rdv) {
                (Some(session), Some(r)) => {
                    self.relayed.1 += 1;
                    let m = Rdv::RelayData {
                        session,
                        payload: data.to_vec(),
                    };
                    self.inner.send_to(r, &m.encode())
                }
                _ => self.inner.send_to(to, data),
            },
            Mode::Join { state, .. } if to == JOIN_ADDR => match state {
                JoinState::Direct { host, .. } => {
                    let host = *host;
                    self.inner.send_to(host, data)
                }
                JoinState::Relayed { session } => {
                    let m = Rdv::RelayData {
                        session: *session,
                        payload: data.to_vec(),
                    };
                    self.relayed.1 += 1;
                    match self.rdv {
                        Some(r) => self.inner.send_to(r, &m.encode()),
                        None => Ok(()),
                    }
                }
                // Not connected yet: the game layer resends.
                _ => Ok(()),
            },
            _ => self.inner.send_to(to, data),
        }
    }

    fn recv(&mut self) -> Option<(SocketAddr, Vec<u8>)> {
        let mut queue: VecDeque<(SocketAddr, Vec<u8>)> = VecDeque::new();
        while queue.is_empty() {
            let (from, data) = self.inner.recv()?;
            if Some(from) == self.rdv {
                if let Some(m) = Rdv::decode(&data)
                    && let Some(d) = self.handle_rdv(m)
                {
                    queue.push_back(d);
                }
                continue;
            }
            if data.first() == Some(&RDV_MAGIC) {
                continue; // a peer's punch
            }
            if let Mode::Join { state, .. } = &mut self.mode {
                match state {
                    JoinState::Direct { host, heard } if *host == from => {
                        *heard = true;
                        queue.push_back((JOIN_ADDR, data));
                    }
                    _ => {} // not our host
                }
                continue;
            }
            queue.push_back((from, data));
        }
        queue.pop_front()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_round_trip_and_parse_loosely() {
        for c in [0, 1, 30, 31, 12_345, CODE_SPACE - 1] {
            let s = format_code(c);
            assert_eq!(s.len(), 7);
            assert_eq!(parse_code(&s), Some(c), "{s}");
            assert_eq!(parse_code(&s[3..]), Some(c));
            assert_eq!(parse_code(&s.to_lowercase().replace('-', " ")), Some(c));
        }
        assert_eq!(parse_code("LG-0000"), None, "0 is not in the alphabet");
        assert_eq!(parse_code("LG-ABC"), None);
        assert_eq!(parse_code(""), None);
    }

    #[test]
    fn direct_codes_round_trip_and_catch_typos() {
        use std::net::SocketAddrV4;
        for a in [
            "203.0.113.7:27960",
            "255.255.255.255:65535",
            "1.2.3.4:1",
            "0.0.0.0:80",
        ] {
            let a: SocketAddrV4 = a.parse().unwrap();
            let s = format_direct(a);
            assert_eq!(s.len(), 3 + 11 + 2, "{s}");
            assert_eq!(parse_direct(&s), Some(a), "{s}");
            assert_eq!(parse_direct(&s[3..]), Some(a));
            assert_eq!(parse_direct(&s.to_lowercase().replace('-', " ")), Some(a));
            assert_eq!(
                parse_code(&s),
                None,
                "a direct code is not a rendezvous code"
            );
        }
        // Any single wrong character is caught (the check bits cover the address).
        let good = format_direct("198.51.100.23:27960".parse().unwrap());
        let mut undetected = 0;
        for i in 3..good.len() {
            if good.as_bytes()[i] == b'-' {
                continue;
            }
            for d in ALPHABET {
                let mut b = good.clone().into_bytes();
                if b[i] == *d {
                    continue;
                }
                b[i] = *d;
                if parse_direct(std::str::from_utf8(&b).unwrap()).is_some() {
                    undetected += 1;
                }
            }
        }
        assert!(
            undetected <= 11 * 30 / 20,
            "{undetected} of 330 typos slipped through"
        );
        assert_eq!(parse_direct("LG-7KQ2"), None);
        assert_eq!(parse_direct(""), None);
        assert_eq!(parse_direct("0000-0000-000"), None);
    }

    #[test]
    fn messages_round_trip_and_requests_are_padded() {
        let v4: SocketAddr = "203.0.113.7:40001".parse().unwrap();
        let v6: SocketAddr = "[2001:db8::1]:27960".parse().unwrap();
        for m in [
            Rdv::Register {
                nonce: 1,
                listed: true,
            },
            Rdv::ListRequest {
                nonce: 4,
                cookie: 0,
            },
            Rdv::ListChallenge {
                nonce: 4,
                cookie: 99,
            },
            Rdv::List {
                nonce: 4,
                servers: vec![v4, v6],
            },
            Rdv::Registered { nonce: 1, code: 77 },
            Rdv::Lookup { nonce: 2, code: 77 },
            Rdv::Found { nonce: 2, host: v4 },
            Rdv::Found { nonce: 2, host: v6 },
            Rdv::NotFound { nonce: 2 },
            Rdv::Incoming { client: v6 },
            Rdv::Punch,
            Rdv::RelayOpen { nonce: 3, code: 77 },
            Rdv::RelayReady {
                nonce: 3,
                session: 9,
            },
            Rdv::RelayData {
                session: 9,
                payload: vec![1, 2, 3],
            },
        ] {
            assert_eq!(Rdv::decode(&m.encode()), Some(m.clone()), "{m:?}");
        }
        // Anything a stranger can ask is at least as big as any answer.
        // (The List itself only goes to an address that proved itself with the cookie.)
        let biggest_answer = Rdv::Found { nonce: 0, host: v6 }.encode().len().max(
            Rdv::ListChallenge {
                nonce: 0,
                cookie: 0,
            }
            .encode()
            .len(),
        );
        for req in [
            Rdv::ListRequest {
                nonce: 0,
                cookie: 0,
            },
            Rdv::Register {
                nonce: 0,
                listed: false,
            },
            Rdv::Lookup { nonce: 0, code: 0 },
            Rdv::RelayOpen { nonce: 0, code: 0 },
        ] {
            assert!(req.encode().len() >= biggest_answer);
        }
        assert_eq!(addr_session(session_addr(0xDEAD_BEEF)), Some(0xDEAD_BEEF));
        assert_eq!(addr_session(JOIN_ADDR), None);
        assert_eq!(addr_session(v4), None);
    }
}
