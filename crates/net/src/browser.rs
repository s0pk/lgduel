//! Server-browser pinger: query many servers at once, collect their info and
//! round-trip time. Non-blocking; call `poll` every frame.

use crate::protocol::{Connectionless, ServerInfo};
use crate::transport::Transport;
use std::collections::HashMap;
use std::net::SocketAddr;

#[derive(Debug, Clone, PartialEq)]
pub struct PingResult {
    pub info: ServerInfo,
    pub rtt_ms: f64,
}

pub struct Pinger<T: Transport> {
    pub transport: T,
    /// nonce → (address, send time µs)
    pending: HashMap<u64, (SocketAddr, u64)>,
    /// Nonces of discovery queries: any server may answer, more than once.
    discovery: std::collections::HashSet<u64>,
    next_nonce: u64,
    pub results: HashMap<SocketAddr, PingResult>,
    /// Servers that answered a discovery query.
    pub discovered: std::collections::BTreeSet<SocketAddr>,
}

impl<T: Transport> Pinger<T> {
    pub fn new(transport: T, seed: u64) -> Self {
        Self {
            transport,
            pending: HashMap::new(),
            discovery: Default::default(),
            next_nonce: seed | 1,
            results: HashMap::new(),
            discovered: Default::default(),
        }
    }

    /// Send a discovery query to `addr` (a broadcast address, or a port on this
    /// machine): every server that answers is recorded in `discovered`.
    pub fn discover(&mut self, addr: SocketAddr, now_us: u64) {
        self.query(addr, now_us);
        self.discovery.insert(self.next_nonce);
    }

    /// Send an info query to `addr`.
    pub fn query(&mut self, addr: SocketAddr, now_us: u64) {
        self.next_nonce = self
            .next_nonce
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1);
        let nonce = self.next_nonce;
        let _ = self
            .transport
            .send_to(addr, &Connectionless::InfoRequest { nonce }.encode());
        self.pending.insert(nonce, (addr, now_us));
    }

    /// Collect answers; queries older than `timeout_us` are forgotten.
    /// Returns the addresses that answered in this call.
    pub fn poll(&mut self, now_us: u64, timeout_us: u64) -> Vec<SocketAddr> {
        let mut answered = Vec::new();
        while let Some((from, data)) = self.transport.recv() {
            let Some(Connectionless::InfoResponse { nonce, info }) = Connectionless::decode(&data)
            else {
                continue;
            };
            let Some((addr, sent)) = self.pending.get(&nonce).copied() else {
                continue;
            };
            let rtt_ms = now_us.saturating_sub(sent) as f64 / 1000.0;
            if self.discovery.contains(&nonce) {
                // Several servers may answer one discovery query: keep it open.
                self.discovered.insert(from);
            } else if addr == from {
                self.pending.remove(&nonce);
            } else {
                continue;
            }
            self.results.insert(from, PingResult { info, rtt_ms });
            answered.push(from);
        }
        self.pending
            .retain(|_, (_, sent)| now_us.saturating_sub(*sent) < timeout_us);
        let pending = &self.pending;
        self.discovery.retain(|n| pending.contains_key(n));
        answered
    }

    pub fn waiting(&self, addr: SocketAddr) -> bool {
        self.pending.values().any(|(a, _)| *a == addr)
    }
}
