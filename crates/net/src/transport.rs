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
        })))
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
        n.in_flight
            .insert((at, seq), (self.addr, to, data.to_vec()));
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
