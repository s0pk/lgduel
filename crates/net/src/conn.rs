//! Per-peer packet layer.
//!
//! Every packet carries a 16-bit sequence number plus an ack of the newest
//! remote sequence and a 32-bit bitfield of the 32 before it, so each packet
//! acknowledges up to 33 packets (redundant acks survive loss). On top:
//! - an **unreliable** payload (snapshots, input bundles): delivered once, may
//!   arrive out of order (`newest` tells the caller);
//! - **reliable ordered** messages: kept in the send queue and repeated in
//!   outgoing packets until a packet carrying them is acked; delivered exactly
//!   once and in order.
//!
//! Header: magic u8, seq u16, ack u16, ack_bits u32, reliable count u8,
//! [msg id u16, len varint, bytes]*, then the unreliable payload.

use crate::transport::MAX_PACKET;
use crate::wire::{Reader, Writer};
use std::collections::{BTreeMap, VecDeque};

pub const MAGIC: u8 = 0x4C; // 'L'
const HEADER: usize = 1 + 2 + 2 + 4 + 1;
const SENT_HISTORY: usize = 1024;
/// Resend a reliable message if its last send is older than this (µs).
const MIN_RESEND_US: u64 = 30_000;

/// `a` is newer than `b` with 16-bit wrap-around.
pub fn seq_newer(a: u16, b: u16) -> bool {
    a != b && a.wrapping_sub(b) < 0x8000
}

#[derive(Debug, Clone, Default)]
struct Sent {
    seq: u16,
    time_us: u64,
    reliable: Vec<u16>,
    acked: bool,
    /// Counted as lost (fell out of the ack window unacked).
    lost: bool,
    valid: bool,
}

#[derive(Debug, Clone)]
struct Outgoing {
    id: u16,
    data: Vec<u8>,
    last_sent_us: Option<u64>,
}

/// What `on_packet` extracted from one datagram.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Received {
    pub seq: u16,
    /// This packet is newer than every packet received before it.
    pub newest: bool,
    pub unreliable: Vec<u8>,
    /// Reliable messages that became deliverable (in order).
    pub reliable: Vec<Vec<u8>>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ConnStats {
    pub packets_sent: u64,
    pub packets_received: u64,
    pub packets_acked: u64,
    /// Packets that fell out of the ack window without being acked.
    pub packets_lost: u64,
    /// Smoothed round-trip time, µs (0 until the first ack).
    pub rtt_us: f64,
    /// Smoothed mean deviation of RTT, µs (jitter estimate).
    pub rtt_var_us: f64,
    pub bytes_sent: u64,
    pub bytes_received: u64,
}

impl ConnStats {
    /// Fraction of sent packets known lost.
    pub fn loss(&self) -> f64 {
        let settled = self.packets_acked + self.packets_lost;
        if settled == 0 {
            0.0
        } else {
            self.packets_lost as f64 / settled as f64
        }
    }
}

#[derive(Debug, Clone)]
pub struct Connection {
    local_seq: u16,
    /// Newest remote sequence seen and bits for the 32 before it.
    remote_seq: Option<u16>,
    recv_bits: u32,
    sent: Vec<Sent>,
    rel_out: VecDeque<Outgoing>,
    next_rel_id: u16,
    rel_in_next: u16,
    rel_in_buf: BTreeMap<u16, Vec<u8>>,
    /// Newest local sequence the peer has acked (drives loss detection).
    newest_acked: Option<u16>,
    pub stats: ConnStats,
}

impl Default for Connection {
    fn default() -> Self {
        Self::new()
    }
}

impl Connection {
    pub fn new() -> Self {
        Self {
            local_seq: 0,
            remote_seq: None,
            recv_bits: 0,
            sent: vec![Sent::default(); SENT_HISTORY],
            rel_out: VecDeque::new(),
            next_rel_id: 0,
            rel_in_next: 0,
            rel_in_buf: BTreeMap::new(),
            newest_acked: None,
            stats: ConnStats::default(),
        }
    }

    /// Queue a reliable message (delivered once, in order).
    pub fn send_reliable(&mut self, data: Vec<u8>) {
        let id = self.next_rel_id;
        self.next_rel_id = self.next_rel_id.wrapping_add(1);
        self.rel_out.push_back(Outgoing {
            id,
            data,
            last_sent_us: None,
        });
    }

    /// Reliable messages not yet acknowledged.
    pub fn reliable_pending(&self) -> usize {
        self.rel_out.len()
    }

    /// Build the next datagram carrying `unreliable` (truncated to fit) and any
    /// reliable messages that are due.
    pub fn build_packet(&mut self, now_us: u64, unreliable: &[u8]) -> Vec<u8> {
        let seq = self.local_seq;
        self.local_seq = self.local_seq.wrapping_add(1);
        let mut w = Writer::new();
        w.u8(MAGIC);
        w.u16(seq);
        w.u16(self.remote_seq.unwrap_or(0xFFFF));
        w.u32(if self.remote_seq.is_some() {
            self.recv_bits
        } else {
            0
        });
        // Reliable messages: those never sent, or not resent within the RTT-based timeout.
        let resend_after = (self.stats.rtt_us * 1.25).max(MIN_RESEND_US as f64) as u64;
        let budget = MAX_PACKET - HEADER - unreliable.len().min(MAX_PACKET / 2);
        let mut rel = Writer::new();
        let mut ids = Vec::new();
        for m in self.rel_out.iter_mut() {
            let due = m
                .last_sent_us
                .is_none_or(|t| now_us.saturating_sub(t) >= resend_after);
            if !due || ids.len() == 255 {
                continue;
            }
            let size = 2 + 3 + m.data.len();
            if rel.buf.len() + size > budget {
                break;
            }
            rel.u16(m.id);
            rel.bytes(&m.data);
            m.last_sent_us = Some(now_us);
            ids.push(m.id);
        }
        w.u8(ids.len() as u8);
        w.raw(&rel.buf);
        let room = MAX_PACKET.saturating_sub(w.buf.len());
        w.raw(&unreliable[..unreliable.len().min(room)]);

        let slot = &mut self.sent[seq as usize % SENT_HISTORY];
        if slot.valid && !slot.acked && !slot.lost {
            self.stats.packets_lost += 1;
        }
        *slot = Sent {
            seq,
            time_us: now_us,
            reliable: ids,
            acked: false,
            lost: false,
            valid: true,
        };
        self.stats.packets_sent += 1;
        self.stats.bytes_sent += w.buf.len() as u64;
        w.finish()
    }

    /// Process a datagram from the peer. `None` if malformed or a duplicate.
    pub fn on_packet(&mut self, now_us: u64, data: &[u8]) -> Option<Received> {
        let mut r = Reader::new(data);
        if r.u8()? != MAGIC {
            return None;
        }
        let seq = r.u16()?;
        let ack = r.u16()?;
        let ack_bits = r.u32()?;
        let n_rel = r.u8()?;
        let mut rel = Vec::with_capacity(n_rel as usize);
        for _ in 0..n_rel {
            let id = r.u16()?;
            rel.push((id, r.bytes()?.to_vec()));
        }
        let unreliable = r.rest().to_vec();

        // Duplicate / too old?
        let newest = match self.remote_seq {
            None => true,
            Some(rs) => seq_newer(seq, rs),
        };
        if newest {
            match self.remote_seq {
                None => self.recv_bits = 0,
                Some(rs) => {
                    let shift = seq.wrapping_sub(rs) as u32;
                    self.recv_bits = if shift >= 32 {
                        0
                    } else {
                        (self.recv_bits << shift) | (1 << (shift - 1))
                    };
                }
            }
            self.remote_seq = Some(seq);
        } else {
            let back = self.remote_seq.unwrap().wrapping_sub(seq) as u32;
            if back == 0 || back > 32 || self.recv_bits & (1 << (back - 1)) != 0 {
                return None; // duplicate or outside the window
            }
            self.recv_bits |= 1 << (back - 1);
        }
        self.stats.packets_received += 1;
        self.stats.bytes_received += data.len() as u64;

        // Acks.
        if ack != 0xFFFF || ack_bits != 0 {
            self.ack(ack, now_us);
            for i in 0..32 {
                if ack_bits & (1 << i) != 0 {
                    self.ack(ack.wrapping_sub(i + 1), now_us);
                }
            }
            if self.newest_acked.is_none_or(|n| seq_newer(ack, n)) {
                self.newest_acked = Some(ack);
                self.detect_loss(ack);
            }
        }

        // Reliable in-order delivery.
        for (id, d) in rel {
            let ahead = id.wrapping_sub(self.rel_in_next);
            if ahead < 0x8000 {
                self.rel_in_buf.entry(id).or_insert(d);
            }
        }
        let mut reliable = Vec::new();
        while let Some(d) = self.rel_in_buf.remove(&self.rel_in_next) {
            reliable.push(d);
            self.rel_in_next = self.rel_in_next.wrapping_add(1);
        }
        Some(Received {
            seq,
            newest,
            unreliable,
            reliable,
        })
    }

    /// Packets more than 32 behind the newest ack can never be acked any more.
    fn detect_loss(&mut self, newest_ack: u16) {
        let edge = newest_ack.wrapping_sub(33);
        for back in 0..(SENT_HISTORY as u16 - 34) {
            let seq = edge.wrapping_sub(back);
            let slot = &mut self.sent[seq as usize % SENT_HISTORY];
            if !slot.valid || slot.seq != seq || slot.acked || slot.lost {
                break; // older ones were settled by earlier calls
            }
            slot.lost = true;
            self.stats.packets_lost += 1;
        }
    }

    fn ack(&mut self, seq: u16, now_us: u64) {
        let slot = &mut self.sent[seq as usize % SENT_HISTORY];
        if !slot.valid || slot.seq != seq || slot.acked {
            return;
        }
        slot.acked = true;
        if slot.lost {
            // Declared lost, then acked by a straggler after all.
            slot.lost = false;
            self.stats.packets_lost -= 1;
        }
        self.stats.packets_acked += 1;
        let sample = now_us.saturating_sub(slot.time_us) as f64;
        let s = &mut self.stats;
        if s.rtt_us == 0.0 {
            s.rtt_us = sample;
            s.rtt_var_us = sample / 2.0;
        } else {
            s.rtt_var_us += ((sample - s.rtt_us).abs() - s.rtt_var_us) * 0.25;
            s.rtt_us += (sample - s.rtt_us) * 0.125;
        }
        let ids = std::mem::take(&mut slot.reliable);
        self.rel_out.retain(|m| !ids.contains(&m.id));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::{LinkConditions, MemNet, Transport};

    #[test]
    fn sequence_wraparound() {
        assert!(seq_newer(1, 0));
        assert!(!seq_newer(0, 1));
        assert!(seq_newer(0, 65535));
        assert!(!seq_newer(65535, 0));
        assert!(!seq_newer(5, 5));
    }

    /// Two connections exchanging packets every `step_us` over a MemNet.
    fn exchange(
        c: LinkConditions,
        msgs: usize,
        duration_us: u64,
        step_us: u64,
        seed: u64,
    ) -> (Vec<Vec<u8>>, Connection, Connection) {
        let net = MemNet::new(seed);
        net.set_conditions(c);
        let (mut ea, mut eb) = (net.endpoint(1), net.endpoint(2));
        let (mut a, mut b) = (Connection::new(), Connection::new());
        for i in 0..msgs {
            a.send_reliable(format!("msg{i}").into_bytes());
        }
        let mut got = vec![];
        let mut t = 0;
        while t < duration_us {
            let pa = a.build_packet(t, b"ua");
            ea.send_to(eb.local_addr(), &pa).unwrap();
            let pb = b.build_packet(t, b"ub");
            eb.send_to(ea.local_addr(), &pb).unwrap();
            t += step_us;
            net.advance_to(t);
            while let Some((_, d)) = eb.recv() {
                if let Some(r) = b.on_packet(t, &d) {
                    got.extend(r.reliable);
                }
            }
            while let Some((_, d)) = ea.recv() {
                a.on_packet(t, &d);
            }
        }
        (got, a, b)
    }

    #[test]
    fn reliable_in_order_exactly_once_under_loss_and_reorder() {
        let c = LinkConditions {
            latency_us: 40_000,
            jitter_us: 20_000,
            loss: 0.2,
        };
        let (got, a, _) = exchange(c, 500, 5_000_000, 8_000, 11);
        let want: Vec<Vec<u8>> = (0..500).map(|i| format!("msg{i}").into_bytes()).collect();
        assert_eq!(got, want);
        assert_eq!(a.reliable_pending(), 0, "all reliable messages acked");
    }

    #[test]
    fn rtt_and_loss_estimates() {
        let c = LinkConditions {
            latency_us: 50_000,
            jitter_us: 0,
            loss: 0.1,
        };
        let (_, a, _) = exchange(c, 0, 4_000_000, 8_000, 5);
        let s = a.stats;
        // RTT = 2 × 50 ms, each leg rounded up to the 8 ms exchange step → 112 ms, plus smoothing.
        assert!((100_000.0..=116_000.0).contains(&s.rtt_us), "{s:?}");
        // Our packet or the ack carrying it may be lost; redundant ack bits recover most acks.
        assert!((0.05..0.15).contains(&s.loss()), "loss {}", s.loss());
    }

    #[test]
    fn duplicates_and_garbage_are_rejected() {
        let mut a = Connection::new();
        let mut b = Connection::new();
        a.send_reliable(b"once".to_vec());
        let p = a.build_packet(0, b"u");
        let first = b.on_packet(0, &p).unwrap();
        assert_eq!(first.reliable, vec![b"once".to_vec()]);
        assert_eq!(first.unreliable, b"u");
        assert!(b.on_packet(0, &p).is_none(), "duplicate dropped");
        assert!(b.on_packet(0, &[0x00, 1, 2]).is_none());
        assert!(b.on_packet(0, &[MAGIC, 1]).is_none());
        // A reordered older packet is delivered but flagged not newest.
        let p1 = a.build_packet(1, b"one");
        let p2 = a.build_packet(2, b"two");
        assert!(b.on_packet(2, &p2).unwrap().newest);
        let late = b.on_packet(3, &p1).unwrap();
        assert!(!late.newest);
        assert_eq!(late.unreliable, b"one");
    }

    #[test]
    fn unreliable_payload_is_capped_to_mtu() {
        let mut a = Connection::new();
        let big = vec![7u8; 5000];
        let p = a.build_packet(0, &big);
        assert!(p.len() <= MAX_PACKET);
    }
}
