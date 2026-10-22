//! Datagram transports: real UDP, and an in-memory network with a manual clock
//! for deterministic tests (optionally with delay, jitter, loss and reordering).

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::io;
use std::net::{SocketAddr, UdpSocket};
use std::rc::Rc;

/// Largest datagram we ever send (stays under common MTUs).
pub const MAX_PACKET: usize = 1200;

pub trait Transport {
    fn local_addr(&self) -> SocketAddr;
    fn send_to(&mut self, to: SocketAddr, data: &[u8]) -> io::Result<()>;
    /// Next received datagram, if any (never blocks).
    fn recv(&mut self) -> Option<(SocketAddr, Vec<u8>)>;
}

/// Non-blocking UDP socket.
pub struct UdpTransport {
    socket: UdpSocket,
    addr: SocketAddr,
    buf: Vec<u8>,
}

impl UdpTransport {
    pub fn bind(addr: SocketAddr) -> io::Result<Self> {
        let socket = UdpSocket::bind(addr)?;
        socket.set_nonblocking(true)?;
        let addr = socket.local_addr()?;
        Ok(Self {
            socket,
            addr,
            buf: vec![0; 2048],
        })
    }

    /// Allow sending to broadcast addresses (LAN discovery).
    pub fn set_broadcast(&self, on: bool) -> io::Result<()> {
        self.socket.set_broadcast(on)
    }
}

impl Transport for UdpTransport {
    fn local_addr(&self) -> SocketAddr {
        self.addr
    }

    fn send_to(&mut self, to: SocketAddr, data: &[u8]) -> io::Result<()> {
        self.socket.send_to(data, to).map(|_| ())
    }

    fn recv(&mut self) -> Option<(SocketAddr, Vec<u8>)> {
        loop {
            match self.socket.recv_from(&mut self.buf) {
                Ok((n, from)) => return Some((from, self.buf[..n].to_vec())),
                // Windows reports ICMP port-unreachable from an earlier send as a
                // recv error (WSAECONNRESET); it says nothing about this socket.
                Err(e) if e.kind() == io::ErrorKind::ConnectionReset => continue,
                Err(_) => return None,
            }
        }
    }
}

/// Link conditions for the in-memory network (applied per direction).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct LinkConditions {
    /// One-way delay, µs.
    pub latency_us: u64,
    /// Uniform extra delay in [-jitter, +jitter] µs (clamped at 0 total).
    pub jitter_us: u64,
    /// Probability [0,1] a datagram is dropped.
    pub loss: f64,
}

struct MemNetInner {
    now_us: u64,
    conditions: LinkConditions,
    rng: u64,
    /// (deliver_at, sequence) → (from, to, data). The sequence keeps ordering stable.
    in_flight: BTreeMap<(u64, u64), (SocketAddr, SocketAddr, Vec<u8>)>,
    seq: u64,
    inboxes: BTreeMap<SocketAddr, Vec<(SocketAddr, Vec<u8>)>>,
    sent: u64,
    dropped: u64,
    /// Simulated home routers, each with one endpoint behind it.
    nats: Vec<NatBox>,
    /// Datagrams a NAT refused (unsolicited).
    nat_dropped: u64,
}

/// How a simulated NAT maps and filters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NatKind {
    /// One public port per endpoint; only peers it has sent to may answer
    /// (port-restricted cone: most home routers; hole punching works).
    Cone,
    /// A new public port for every destination (hole punching fails: relay).
    Symmetric,
}

struct NatBox {
    kind: NatKind,
    public_ip: [u8; 4],
    private: SocketAddr,
    /// (destination, public port used for it).
    mappings: Vec<(SocketAddr, u16)>,
    next_port: u16,
}

impl NatBox {
    fn public_port_for(&mut self, dest: SocketAddr) -> u16 {
        if let Some((_, p)) = self.mappings.iter().find(|(d, _)| *d == dest) {
            return *p;
        }
        let port = match (self.kind, self.mappings.first()) {
            (NatKind::Cone, Some((_, p))) => *p,
            _ => {
                self.next_port += 1;
                self.next_port
            }
        };
        self.mappings.push((dest, port));
        port
    }
}

/// Shared in-memory network. Single-threaded and fully deterministic.
#[derive(Clone)]
pub struct MemNet(Rc<RefCell<MemNetInner>>);

impl MemNet {
    pub fn new(seed: u64) -> Self {
        Self(Rc::new(RefCell::new(MemNetInner {
            now_us: 0,
            conditions: LinkConditions::default(),
            rng: seed | 1,
            in_flight: BTreeMap::new(),
            seq: 0,
            inboxes: BTreeMap::new(),
            sent: 0,
            dropped: 0,
            nats: Vec::new(),
            nat_dropped: 0,
        })))
    }

    /// An endpoint behind its own NAT with public address `public_ip`. Its
    /// local address is private (192.168.x); peers see the NAT's mapping.
    pub fn endpoint_behind_nat(&self, port: u16, public_ip: [u8; 4], kind: NatKind) -> MemEndpoint {
        let addr = SocketAddr::from(([192, 168, public_ip[3], 2], port));
        let mut n = self.0.borrow_mut();
        n.inboxes.entry(addr).or_default();
        n.nats.push(NatBox {
            kind,
            public_ip,
            private: addr,
            mappings: Vec::new(),
            next_port: 40_000,
        });
        drop(n);
        MemEndpoint {
            net: self.clone(),
            addr,
        }
    }

    /// Datagrams dropped by simulated NATs (unsolicited inbound).
    pub fn nat_dropped(&self) -> u64 {
        self.0.borrow().nat_dropped
    }

    pub fn set_conditions(&self, c: LinkConditions) {
        self.0.borrow_mut().conditions = c;
    }

    pub fn now_us(&self) -> u64 {
        self.0.borrow().now_us
    }

    /// Advance the clock, delivering everything due by then into inboxes.
    pub fn advance_to(&self, t_us: u64) {
        let mut n = self.0.borrow_mut();
        n.now_us = n.now_us.max(t_us);
        let now = n.now_us;
        let due: Vec<(u64, u64)> = n.in_flight.range(..(now + 1, 0)).map(|(k, _)| *k).collect();
        for k in due {
            let (from, to, data) = n.in_flight.remove(&k).unwrap();
            // Into a NAT: forward only to a mapping that has sent to `from`.
            let nat = n
                .nats
                .iter()
                .find(|b| SocketAddr::from((b.public_ip, to.port())).ip() == to.ip());
            let to = match nat {
                Some(b) => match b
                    .mappings
                    .iter()
                    .any(|(dest, port)| *port == to.port() && *dest == from)
                {
                    true => b.private,
                    false => {
                        n.nat_dropped += 1;
                        continue;
                    }
                },
                None => to,
            };
            n.inboxes.entry(to).or_default().push((from, data));
        }
    }

    pub fn endpoint(&self, port: u16) -> MemEndpoint {
        let addr = SocketAddr::from(([10, 0, 0, 1], port));
        self.0.borrow_mut().inboxes.entry(addr).or_default();
        MemEndpoint {
            net: self.clone(),
            addr,
        }
    }

    /// (datagrams sent, datagrams dropped).
    pub fn counters(&self) -> (u64, u64) {
        let n = self.0.borrow();
        (n.sent, n.dropped)
    }
}

impl MemNetInner {
    fn rand_unit(&mut self) -> f64 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        (self.rng >> 11) as f64 / (1u64 << 53) as f64
    }
}

pub struct MemEndpoint {
    net: MemNet,
    addr: SocketAddr,
}

impl Transport for MemEndpoint {
    fn local_addr(&self) -> SocketAddr {
        self.addr
    }

    fn send_to(&mut self, to: SocketAddr, data: &[u8]) -> io::Result<()> {
        let mut n = self.net.0.borrow_mut();
        n.sent += 1;
        let c = n.conditions;
        if c.loss > 0.0 && n.rand_unit() < c.loss {
            n.dropped += 1;
            return Ok(());
        }
        let jitter = if c.jitter_us > 0 {
            ((n.rand_unit() * 2.0 - 1.0) * c.jitter_us as f64) as i64
        } else {
            0
        };
        let at = (n.now_us as i64 + c.latency_us as i64 + jitter).max(n.now_us as i64) as u64;
        n.seq += 1;
        let seq = n.seq;
        // Out of a NAT: the source becomes the NAT's public mapping for `to`.
        let from = match n.nats.iter_mut().find(|b| b.private == self.addr) {
            Some(b) => SocketAddr::from((b.public_ip, b.public_port_for(to))),
            None => self.addr,
        };
        n.in_flight.insert((at, seq), (from, to, data.to_vec()));
        drop(n);
        if c.latency_us == 0 && c.jitter_us == 0 {
            let now = self.net.now_us();
            self.net.advance_to(now);
        }
        Ok(())
    }

    fn recv(&mut self) -> Option<(SocketAddr, Vec<u8>)> {
        let mut n = self.net.0.borrow_mut();
        let inbox = n.inboxes.get_mut(&self.addr)?;
        if inbox.is_empty() {
            None
        } else {
            Some(inbox.remove(0))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn udp_loopback() {
        let mut a = UdpTransport::bind("127.0.0.1:0".parse().unwrap()).unwrap();
        let mut b = UdpTransport::bind("127.0.0.1:0".parse().unwrap()).unwrap();
        a.send_to(b.local_addr(), b"ping").unwrap();
        let mut got = None;
        for _ in 0..200 {
            if let Some(m) = b.recv() {
                got = Some(m);
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        let (from, data) = got.expect("loopback datagram");
        assert_eq!(from, a.local_addr());
        assert_eq!(data, b"ping");
        assert!(b.recv().is_none());
    }

    #[test]
    fn simulated_nats_map_and_filter() {
        let net = MemNet::new(3);
        let mut server = net.endpoint(100);
        let mut cone = net.endpoint_behind_nat(5000, [203, 0, 113, 7], NatKind::Cone);
        let mut sym = net.endpoint_behind_nat(5000, [198, 51, 100, 9], NatKind::Symmetric);
        let mut other = net.endpoint(200);
        // Outbound: the server sees the NAT's public address, not the private one.
        cone.send_to(server.local_addr(), b"hi").unwrap();
        let (from, _) = server.recv().unwrap();
        assert_eq!(from.ip(), std::net::IpAddr::from([203, 0, 113, 7]));
        // The answer comes back in; an unsolicited sender is dropped.
        server.send_to(from, b"back").unwrap();
        assert_eq!(cone.recv().map(|m| m.1), Some(b"back".to_vec()));
        other.send_to(from, b"spam").unwrap();
        assert!(cone.recv().is_none());
        assert_eq!(net.nat_dropped(), 1);
        // Cone: same public port toward every destination.
        cone.send_to(other.local_addr(), b"x").unwrap();
        assert_eq!(other.recv().unwrap().0, from);
        // Symmetric: a new public port per destination.
        sym.send_to(server.local_addr(), b"a").unwrap();
        sym.send_to(other.local_addr(), b"b").unwrap();
        let (s1, _) = server.recv().unwrap();
        let (s2, _) = other.recv().unwrap();
        assert_eq!(s1.ip(), s2.ip());
        assert_ne!(s1.port(), s2.port());
    }

    #[test]
    fn memnet_latency_and_order() {
        let net = MemNet::new(1);
        net.set_conditions(LinkConditions {
            latency_us: 50_000,
            ..Default::default()
        });
        let mut a = net.endpoint(1);
        let mut b = net.endpoint(2);
        a.send_to(b.local_addr(), b"1").unwrap();
        a.send_to(b.local_addr(), b"2").unwrap();
        net.advance_to(49_999);
        assert!(b.recv().is_none());
        net.advance_to(50_000);
        assert_eq!(b.recv().unwrap().1, b"1");
        assert_eq!(b.recv().unwrap().1, b"2");
    }

    #[test]
    fn memnet_loss_rate_and_determinism() {
        let run = |seed| {
            let net = MemNet::new(seed);
            net.set_conditions(LinkConditions {
                loss: 0.05,
                ..Default::default()
            });
            let mut a = net.endpoint(1);
            let b = net.endpoint(2);
            for _ in 0..20_000 {
                a.send_to(b.local_addr(), b"x").unwrap();
            }
            net.counters()
        };
        let (sent, dropped) = run(7);
        assert_eq!(sent, 20_000);
        let rate = dropped as f64 / sent as f64;
        assert!((0.04..0.06).contains(&rate), "{rate}");
        assert_eq!(run(7), (sent, dropped));
    }

    #[test]
    fn memnet_jitter_reorders() {
        let net = MemNet::new(3);
        net.set_conditions(LinkConditions {
            latency_us: 20_000,
            jitter_us: 20_000,
            loss: 0.0,
        });
        let mut a = net.endpoint(1);
        let mut b = net.endpoint(2);
        for i in 0..100u8 {
            a.send_to(b.local_addr(), &[i]).unwrap();
            net.advance_to(net.now_us() + 1_000);
        }
        net.advance_to(1_000_000);
        let got: Vec<u8> = std::iter::from_fn(|| b.recv().map(|m| m.1[0])).collect();
        assert_eq!(got.len(), 100);
        assert!(
            got.windows(2).any(|w| w[1] < w[0]),
            "jitter should reorder some datagrams"
        );
    }
}
