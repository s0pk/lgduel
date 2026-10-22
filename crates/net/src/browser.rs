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
    next_nonce: u64,
    pub results: HashMap<SocketAddr, PingResult>,
}

impl<T: Transport> Pinger<T> {
    pub fn new(transport: T, seed: u64) -> Self {
        Self {
            transport,
            pending: HashMap::new(),
            next_nonce: seed | 1,
            results: HashMap::new(),
        }
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
            if let Some(Connectionless::InfoResponse { nonce, info }) =
                Connectionless::decode(&data)
                && let Some((addr, sent)) = self.pending.get(&nonce).copied()
                && addr == from
            {
                self.pending.remove(&nonce);
                let rtt_ms = now_us.saturating_sub(sent) as f64 / 1000.0;
                self.results.insert(addr, PingResult { info, rtt_ms });
                answered.push(addr);
            }
        }
        self.pending
            .retain(|_, (_, sent)| now_us.saturating_sub(*sent) < timeout_us);
        answered
    }

    pub fn waiting(&self, addr: SocketAddr) -> bool {
        self.pending.values().any(|(a, _)| *a == addr)
    }
}
