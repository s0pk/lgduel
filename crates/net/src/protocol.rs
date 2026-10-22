//! Game protocol messages.
//!
//! Connectionless (before a `Connection` exists), sent as raw datagrams that
//! start with `CONNECTIONLESS` instead of the packet-layer magic:
//!   client → `ConnectRequest{version, nonce}`
//!   server → `Challenge{nonce, cookie}`   (cookie = keyed hash of addr+nonce)
//!   client → `ConnectResponse{version, nonce, cookie, name}`
//!   server → `Accept{player id, tick rate, config, map name + content hash}`
//!            or `Reject{reason}`
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
        /// Map the server runs and its `World::content_hash` (the client must have the same map).
        map: String,
        map_hash: u64,
    },
    Reject {
        reason: String,
    },
    /// Server-browser query (padded to `INFO_REQUEST_SIZE`: never smaller than the answer).
    InfoRequest {
        nonce: u64,
    },
    InfoResponse {
        nonce: u64,
        info: ServerInfo,
    },
}

/// Request size: at least as large as any `InfoResponse` (no amplification).
pub const INFO_REQUEST_SIZE: usize = 128;
/// Max bytes of the server name / map name in `ServerInfo`.
pub const MAX_INFO_TEXT: usize = 32;

/// What the server browser shows about a server.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ServerInfo {
    pub protocol: u16,
    pub name: String,
    pub map: String,
    pub players: u8,
    pub max_players: u8,
    /// true = classic first-to-N, false = vamp.
    pub classic: bool,
    pub lifesteal_percent: u8,
    pub anti_lag: bool,
    pub tick_rate: u16,
}

fn truncate(s: &str, max: usize) -> &str {
    let mut end = s.len().min(max);
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
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
                map,
                map_hash,
            } => {
                w.u8(4);
                w.u8(*player);
                w.u32(*tick_rate);
                w.bytes(config_toml.as_bytes());
                w.bytes(map.as_bytes());
                w.u64(*map_hash);
            }
            Self::Reject { reason } => {
                w.u8(5);
                w.bytes(reason.as_bytes());
            }
            Self::InfoRequest { nonce } => {
                w.u8(6);
                w.u64(*nonce);
                let pad = INFO_REQUEST_SIZE.saturating_sub(w.buf.len());
                w.raw(&vec![0; pad]);
            }
            Self::InfoResponse { nonce, info } => {
                w.u8(7);
                w.u64(*nonce);
                w.u16(info.protocol);
                w.bytes(truncate(&info.name, MAX_INFO_TEXT).as_bytes());
                w.bytes(truncate(&info.map, MAX_INFO_TEXT).as_bytes());
                w.u8(info.players);
                w.u8(info.max_players);
                w.u8(info.classic as u8 | (info.anti_lag as u8) << 1);
                w.u8(info.lifesteal_percent);
                w.u16(info.tick_rate);
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
                map: text(&mut r)?,
                map_hash: r.u64()?,
            },
            5 => Self::Reject {
                reason: text(&mut r)?,
            },
            6 => Self::InfoRequest { nonce: r.u64()? },
            7 => {
                let nonce = r.u64()?;
                let protocol = r.u16()?;
                let name = text(&mut r)?;
                let map = text(&mut r)?;
                let players = r.u8()?;
                let max_players = r.u8()?;
                let flags = r.u8()?;
                let lifesteal_percent = r.u8()?;
                let tick_rate = r.u16()?;
                Self::InfoResponse {
                    nonce,
                    info: ServerInfo {
                        protocol,
                        name: truncate(&name, MAX_INFO_TEXT).to_string(),
                        map: truncate(&map, MAX_INFO_TEXT).to_string(),
                        players,
                        max_players,
                        classic: flags & 1 != 0,
                        lifesteal_percent,
                        anti_lag: flags & 2 != 0,
                        tick_rate,
                    },
                }
            }
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
    /// Per command (parallel to `cmds`): the fractional snapshot tick at which
    /// the player saw remote players when the command was made (anti-lag
    /// rewind target). Missing entries mean "unknown".
    pub views: Vec<Option<f64>>,
}

/// View times travel as ticks × 256 in a u32 (u32::MAX = none).
fn encode_view(v: Option<f64>) -> u32 {
    v.filter(|v| v.is_finite() && *v >= 0.0)
        .map_or(u32::MAX, |v| {
            ((v * 256.0).round() as u64).min(u32::MAX as u64 - 1) as u32
        })
}

fn decode_view(v: u32) -> Option<f64> {
    (v != u32::MAX).then_some(v as f64 / 256.0)
}

impl Inputs {
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new();
        w.u8(1); // message kind: inputs
        w.u32(self.ack_snapshot.map_or(u32::MAX, |t| t.0));
        w.u8(self.cmds.len().min(255) as u8);
        for (i, c) in self.cmds.iter().take(255).enumerate() {
            write_cmd(&mut w, c);
            w.u32(encode_view(self.views.get(i).copied().flatten()));
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
        let mut views = Vec::with_capacity(n as usize);
        for _ in 0..n {
            cmds.push(read_cmd(&mut r)?);
            views.push(decode_view(r.u32()?));
        }
        Some(Self {
            ack_snapshot: (ack != u32::MAX).then_some(Tick(ack)),
            cmds,
            views,
        })
    }
}

/// Server-side input queue for one client: accepts each tick's command once,
/// whichever (possibly redundant, reordered) packet brings it first.
#[derive(Debug, Clone, Default)]
pub struct InputBuffer {
    cmds: std::collections::BTreeMap<u32, (UserCmd, Option<f64>)>,
    /// Newest tick already consumed by the simulation.
    consumed: Option<u32>,
    pub received: u64,
    pub duplicates: u64,
    pub late: u64,
}

impl InputBuffer {
    pub fn push(&mut self, c: UserCmd) {
        self.push_with_view(c, None);
    }

    /// Like `push`, also recording the client's view time for anti-lag.
    pub fn push_with_view(&mut self, c: UserCmd, view: Option<f64>) {
        if self.consumed.is_some_and(|t| c.tick.0 <= t) {
            self.late += 1;
            return;
        }
        if self.cmds.insert(c.tick.0, (c.sanitized(), view)).is_some() {
            self.duplicates += 1;
        } else {
            self.received += 1;
        }
    }

    /// Command for `tick`, if it arrived. Older buffered commands are discarded.
    pub fn take(&mut self, tick: Tick) -> Option<UserCmd> {
        self.take_with_view(tick).map(|(c, _)| c)
    }

    /// Command for `tick` and its view time.
    pub fn take_with_view(&mut self, tick: Tick) -> Option<(UserCmd, Option<f64>)> {
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
    /// Server → client: where this client is (sent when it changes).
    Status(ArenaStatus),
    /// Client → server: watch instead of play (`on`), or back to playing.
    Spectate {
        on: bool,
    },
}

/// A client's place on a multi-arena server.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ArenaStatus {
    /// Arena the client plays in (None = queued or spectating).
    pub arena: Option<u8>,
    /// 1-based queue position (0 = not queued).
    pub queue_pos: u16,
    /// Name of the opponent in the arena (empty = none).
    pub opponent: String,
    /// The opponent is the server's practice bot.
    pub opponent_bot: bool,
    pub spectating: bool,
    /// Arena whose snapshots a queued/spectating client receives.
    pub watching: u8,
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
            Self::Status(s) => {
                w.u8(3);
                w.u8(s.arena.map_or(0, |a| a.saturating_add(1)));
                w.u16(s.queue_pos);
                w.bytes(truncate(&s.opponent, MAX_NAME).as_bytes());
                w.u8(s.opponent_bot as u8 | (s.spectating as u8) << 1);
                w.u8(s.watching);
            }
            Self::Spectate { on } => {
                w.u8(4);
                w.u8(*on as u8);
            }
        }
        w.finish()
    }

    pub fn decode(data: &[u8]) -> Option<Self> {
        let mut r = Reader::new(data);
        let kind = r.u8()?;
        let text = |r: &mut Reader| String::from_utf8(r.bytes()?.to_vec()).ok();
        Some(match kind {
            1 => Self::Disconnect {
                reason: text(&mut r)?,
            },
            2 => Self::ServerEvent {
                text: text(&mut r)?,
            },
            3 => {
                let arena = r.u8()?.checked_sub(1);
                let queue_pos = r.u16()?;
                let opponent = text(&mut r)?;
                let flags = r.u8()?;
                Self::Status(ArenaStatus {
                    arena,
                    queue_pos,
                    opponent,
                    opponent_bot: flags & 1 != 0,
                    spectating: flags & 2 != 0,
                    watching: r.u8()?,
                })
            }
            4 => Self::Spectate { on: r.u8()? != 0 },
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
                map: "arena_tiers".into(),
                map_hash: 0x1234_5678_9abc_def0,
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
    fn info_round_trip_and_no_amplification() {
        let info = ServerInfo {
            protocol: 2,
            name: "x".repeat(200),
            map: "arena_tiers".into(),
            players: 3,
            max_players: 16,
            classic: false,
            lifesteal_percent: 100,
            anti_lag: true,
            tick_rate: 125,
        };
        let resp = Connectionless::InfoResponse {
            nonce: 9,
            info: info.clone(),
        }
        .encode();
        let req = Connectionless::InfoRequest { nonce: 9 }.encode();
        assert_eq!(req.len(), INFO_REQUEST_SIZE);
        assert!(
            resp.len() <= req.len(),
            "response {} > request {}",
            resp.len(),
            req.len()
        );
        match Connectionless::decode(&resp) {
            Some(Connectionless::InfoResponse {
                nonce: 9,
                info: got,
            }) => {
                assert_eq!(got.name.len(), MAX_INFO_TEXT);
                assert_eq!(
                    ServerInfo {
                        name: info.name[..MAX_INFO_TEXT].to_string(),
                        ..info
                    },
                    got
                );
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(
            Connectionless::decode(&req),
            Some(Connectionless::InfoRequest { nonce: 9 })
        );
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
            views: (0..8)
                .map(|k| {
                    if k == 3 {
                        None
                    } else {
                        Some(95.0 + k as f64 * 0.25)
                    }
                })
                .collect(),
        };
        let d = Inputs::decode(&i.encode()).unwrap();
        assert_eq!(d, i);
        let none = Inputs {
            ack_snapshot: None,
            cmds: vec![],
            views: vec![],
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
        for m in [
            Control::Status(ArenaStatus::default()),
            Control::Status(ArenaStatus {
                arena: Some(7),
                queue_pos: 0,
                opponent: "practice bot".into(),
                opponent_bot: true,
                spectating: false,
                watching: 7,
            }),
            Control::Status(ArenaStatus {
                arena: None,
                queue_pos: 3,
                opponent: String::new(),
                opponent_bot: false,
                spectating: true,
                watching: 2,
            }),
            Control::Spectate { on: true },
            Control::Spectate { on: false },
        ] {
            assert_eq!(Control::decode(&m.encode()), Some(m));
        }
    }
}
