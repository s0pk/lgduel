//! Game protocol messages.
//!
//! Connectionless (before a `Connection` exists), sent as raw datagrams that
//! start with `CONNECTIONLESS` instead of the packet-layer magic:
//!   client → `ConnectRequest{version, nonce}`
//!   server → `Challenge{nonce, cookie}`   (cookie = keyed hash of addr+nonce)
//!   client → `ConnectResponse{version, nonce, cookie, name}`
//!   server → `Accept{player id, tick rate, config}` or `Reject{reason}`
//! The cookie round trip means a spoofed source address can't claim a slot.
//!
//! Connected (inside `Connection` payloads):
//!   client → server unreliable: `Inputs` — the last N usercmds (redundant),
//!            plus the newest snapshot tick the client has (for delta baselines).
//!   server → client unreliable: `Snapshot` (see `snapshot.rs`).
//!   both reliable: `Control` messages (disconnect, chat, server events).

use crate::wire::{Reader, Writer};
use lgduel_sim::math::Angle;
use lgduel_sim::tick::Tick;
use lgduel_sim::usercmd::UserCmd;

pub const CONNECTIONLESS: u8 = 0xFF;
/// How many recent commands each input packet repeats.
pub const INPUT_REDUNDANCY: usize = 8;
pub const MAX_NAME: usize = 24;

#[derive(Debug, Clone, PartialEq)]
pub enum Connectionless {
    ConnectRequest {
        version: u16,
        nonce: u64,
    },
    Challenge {
        nonce: u64,
        cookie: u64,
    },
    ConnectResponse {
        version: u16,
        nonce: u64,
        cookie: u64,
        name: String,
    },
    Accept {
        player: u8,
        tick_rate: u32,
        config_toml: String,
    },
    Reject {
        reason: String,
    },
}

impl Connectionless {
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new();
        w.u8(CONNECTIONLESS);
        match self {
            Self::ConnectRequest { version, nonce } => {
                w.u8(1);
                w.u16(*version);
                w.u64(*nonce);
                // Pad so a request is as large as the challenge reply: no amplification.
                w.raw(&[0; 32]);
            }
            Self::Challenge { nonce, cookie } => {
                w.u8(2);
                w.u64(*nonce);
                w.u64(*cookie);
            }
            Self::ConnectResponse {
                version,
                nonce,
                cookie,
                name,
            } => {
                w.u8(3);
                w.u16(*version);
                w.u64(*nonce);
                w.u64(*cookie);
                w.bytes(name.as_bytes());
            }
            Self::Accept {
                player,
                tick_rate,
                config_toml,
            } => {
                w.u8(4);
                w.u8(*player);
                w.u32(*tick_rate);
                w.bytes(config_toml.as_bytes());
            }
            Self::Reject { reason } => {
                w.u8(5);
                w.bytes(reason.as_bytes());
            }
        }
        w.finish()
    }

    pub fn decode(data: &[u8]) -> Option<Self> {
        let mut r = Reader::new(data);
        if r.u8()? != CONNECTIONLESS {
            return None;
        }
        let text = |r: &mut Reader| String::from_utf8(r.bytes()?.to_vec()).ok();
        Some(match r.u8()? {
            1 => Self::ConnectRequest {
                version: r.u16()?,
                nonce: r.u64()?,
            },
            2 => Self::Challenge {
                nonce: r.u64()?,
                cookie: r.u64()?,
            },
            3 => {
                let version = r.u16()?;
                let nonce = r.u64()?;
                let cookie = r.u64()?;
                let mut name = text(&mut r)?;
                name.truncate(MAX_NAME);
                Self::ConnectResponse {
                    version,
                    nonce,
                    cookie,
                    name,
                }
            }
            4 => Self::Accept {
                player: r.u8()?,
                tick_rate: r.u32()?,
                config_toml: text(&mut r)?,
            },
            5 => Self::Reject {
                reason: text(&mut r)?,
            },
            _ => return None,
        })
    }
}

/// Keyed 64-bit hash for challenge cookies (SipHash-like mixing; not crypto,
/// just unguessable without the server secret).
pub fn cookie(secret: u64, addr: &std::net::SocketAddr, nonce: u64) -> u64 {
    let mut h = secret ^ 0x9E37_79B9_7F4A_7C15;
    let mut mix = |v: u64| {
        h ^= v;
        h = h.wrapping_mul(0xBF58_476D_1CE4_E5B9);
        h ^= h >> 31;
        h = h.wrapping_mul(0x94D0_49BB_1331_11EB);
        h ^= h >> 29;
    };
    match addr.ip() {
        std::net::IpAddr::V4(v) => mix(u32::from(v) as u64),
        std::net::IpAddr::V6(v) => {
            let b = u128::from(v);
            mix(b as u64);
            mix((b >> 64) as u64);
        }
    }
    mix(addr.port() as u64);
    mix(nonce);
    h
}

pub fn write_cmd(w: &mut Writer, c: &UserCmd) {
    w.u32(c.tick.0);
    w.u32(c.pitch.0);
    w.u32(c.yaw.0);
    w.u8(c.forward as u8);
    w.u8(c.right as u8);
    w.u8(c.buttons);
}

pub fn read_cmd(r: &mut Reader) -> Option<UserCmd> {
    Some(UserCmd {
        tick: Tick(r.u32()?),
        pitch: Angle(r.u32()?),
        yaw: Angle(r.u32()?),
        forward: r.u8()? as i8,
        right: r.u8()? as i8,
        buttons: r.u8()?,
    })
}

/// Client → server input bundle: newest command last, older ones repeated so a
/// lost packet costs nothing as long as one of the next N arrives.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Inputs {
    /// Newest snapshot tick the client has received (delta baseline / ack).
    pub ack_snapshot: Option<Tick>,
    pub cmds: Vec<UserCmd>,
}

impl Inputs {
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new();
        w.u8(1); // message kind: inputs
        w.u32(self.ack_snapshot.map_or(u32::MAX, |t| t.0));
        w.u8(self.cmds.len().min(255) as u8);
        for c in self.cmds.iter().take(255) {
            write_cmd(&mut w, c);
        }
        w.finish()
    }

    pub fn decode(data: &[u8]) -> Option<Self> {
        let mut r = Reader::new(data);
        if r.u8()? != 1 {
            return None;
        }
        let ack = r.u32()?;
        let n = r.u8()?;
        let mut cmds = Vec::with_capacity(n as usize);
        for _ in 0..n {
            cmds.push(read_cmd(&mut r)?);
        }
        Some(Self {
            ack_snapshot: (ack != u32::MAX).then_some(Tick(ack)),
            cmds,
        })
    }
}

/// Server-side input queue for one client: accepts each tick's command once,
/// whichever (possibly redundant, reordered) packet brings it first.
#[derive(Debug, Clone, Default)]
pub struct InputBuffer {
    cmds: std::collections::BTreeMap<u32, UserCmd>,
    /// Newest tick already consumed by the simulation.
    consumed: Option<u32>,
    pub received: u64,
    pub duplicates: u64,
    pub late: u64,
}

impl InputBuffer {
    pub fn push(&mut self, c: UserCmd) {
        if self.consumed.is_some_and(|t| c.tick.0 <= t) {
            self.late += 1;
            return;
        }
        if self.cmds.insert(c.tick.0, c.sanitized()).is_some() {
            self.duplicates += 1;
        } else {
            self.received += 1;
        }
    }

    /// Command for `tick`, if it arrived. Older buffered commands are discarded.
    pub fn take(&mut self, tick: Tick) -> Option<UserCmd> {
        self.consumed = Some(tick.0);
        let c = self.cmds.remove(&tick.0);
        self.cmds.retain(|t, _| *t > tick.0);
        c
    }

    pub fn buffered(&self) -> usize {
        self.cmds.len()
    }

    /// Ticks buffered ahead of the simulation (for client time sync feedback).
    pub fn lead(&self, next_tick: Tick) -> i64 {
        self.cmds
            .keys()
            .next_back()
            .map_or(-1, |t| *t as i64 - next_tick.0 as i64)
    }
}

/// Reliable control messages, both directions.
#[derive(Debug, Clone, PartialEq)]
pub enum Control {
    Disconnect {
        reason: String,
    },
    /// Server → client: a player joined/left, scores, etc. (free text for now).
    ServerEvent {
        text: String,
    },
}

impl Control {
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new();
        match self {
            Self::Disconnect { reason } => {
                w.u8(1);
                w.bytes(reason.as_bytes());
            }
            Self::ServerEvent { text } => {
                w.u8(2);
                w.bytes(text.as_bytes());
            }
        }
        w.finish()
    }

    pub fn decode(data: &[u8]) -> Option<Self> {
        let mut r = Reader::new(data);
        let kind = r.u8()?;
        let s = String::from_utf8(r.bytes()?.to_vec()).ok()?;
        Some(match kind {
            1 => Self::Disconnect { reason: s },
            2 => Self::ServerEvent { text: s },
            _ => return None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lgduel_sim::usercmd::buttons;

    fn cmd(t: u32) -> UserCmd {
        UserCmd {
            tick: Tick(t),
            pitch: Angle(0xFFFF_0000),
            yaw: Angle(123_456_789),
            forward: -127,
            right: 127,
            buttons: buttons::FIRE | buttons::JUMP,
        }
    }

    #[test]
    fn connectionless_round_trip() {
        for m in [
            Connectionless::ConnectRequest {
                version: 1,
                nonce: 99,
            },
            Connectionless::Challenge {
                nonce: 99,
                cookie: 0xABCD,
            },
            Connectionless::ConnectResponse {
                version: 1,
                nonce: 99,
                cookie: 0xABCD,
                name: "player".into(),
            },
            Connectionless::Accept {
                player: 3,
                tick_rate: 125,
                config_toml: "tick_rate_hz = 125\n".into(),
            },
            Connectionless::Reject {
                reason: "server full".into(),
            },
        ] {
            assert_eq!(Connectionless::decode(&m.encode()), Some(m));
        }
        assert_eq!(Connectionless::decode(&[0xFF, 99]), None);
        assert_eq!(Connectionless::decode(&[0x4C, 1]), None);
    }

    #[test]
    fn request_not_smaller_than_challenge() {
        let req = Connectionless::ConnectRequest {
            version: 1,
            nonce: 1,
        }
        .encode();
        let ch = Connectionless::Challenge {
            nonce: 1,
            cookie: 2,
        }
        .encode();
        assert!(
            req.len() >= ch.len(),
            "no amplification: {} < {}",
            req.len(),
            ch.len()
        );
    }

    #[test]
    fn long_names_truncated() {
        let m = Connectionless::ConnectResponse {
            version: 1,
            nonce: 1,
            cookie: 1,
            name: "x".repeat(100),
        };
        match Connectionless::decode(&m.encode()) {
            Some(Connectionless::ConnectResponse { name, .. }) => assert_eq!(name.len(), MAX_NAME),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn cookie_depends_on_secret_addr_nonce() {
        let a: std::net::SocketAddr = "1.2.3.4:5000".parse().unwrap();
        let b: std::net::SocketAddr = "1.2.3.4:5001".parse().unwrap();
        let c = cookie(7, &a, 1);
        assert_eq!(c, cookie(7, &a, 1));
        assert_ne!(c, cookie(8, &a, 1));
        assert_ne!(c, cookie(7, &b, 1));
        assert_ne!(c, cookie(7, &a, 2));
    }

    #[test]
    fn inputs_round_trip_exact_angles() {
        let i = Inputs {
            ack_snapshot: Some(Tick(77)),
            cmds: (100..108).map(cmd).collect(),
        };
        let d = Inputs::decode(&i.encode()).unwrap();
        assert_eq!(d, i);
        let none = Inputs {
            ack_snapshot: None,
            cmds: vec![],
        };
        assert_eq!(Inputs::decode(&none.encode()), Some(none));
        assert_eq!(Inputs::decode(&[1, 0, 0, 0]), None);
    }

    #[test]
    fn input_buffer_dedups_and_survives_loss() {
        let mut b = InputBuffer::default();
        // Packets carry ticks [1..=8], [2..=9], [3..=10]; the middle one is lost.
        for t in 1..=8 {
            b.push(cmd(t));
        }
        for t in 3..=10 {
            b.push(cmd(t));
        }
        assert_eq!(b.received, 10);
        assert_eq!(b.duplicates, 6);
        for t in 1..=10 {
            assert_eq!(b.take(Tick(t)).map(|c| c.tick), Some(Tick(t)));
        }
        assert!(b.take(Tick(11)).is_none());
        b.push(cmd(5));
        assert_eq!(b.late, 1);
    }

    #[test]
    fn input_buffer_sanitizes() {
        let mut b = InputBuffer::default();
        let mut c = cmd(1);
        c.buttons = 0xFF;
        c.forward = i8::MIN;
        b.push(c);
        let got = b.take(Tick(1)).unwrap();
        assert_eq!(got.buttons, 0b111);
        assert_eq!(got.forward, -127);
    }

    #[test]
    fn control_round_trip() {
        for m in [
            Control::Disconnect {
                reason: "bye".into(),
            },
            Control::ServerEvent {
                text: "joined".into(),
            },
        ] {
            assert_eq!(Control::decode(&m.encode()), Some(m));
        }
    }
}
