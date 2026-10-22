//! Demo files (`.lgdemo`): a recorded practice session or online game.
//!
//! Local (practice) demos store the initial game state and every tick's
//! commands of every player: playback re-simulates them, so it reproduces the
//! session exactly (the final state hash is stored and checked).
//! Net demos store the snapshots as received plus the own commands sent;
//! playback shows them from the recording player's view.
//!
//! Layout: "LGDM", version u16, header, then length-prefixed frames.

use crate::protocol::{read_cmd, write_cmd};
use crate::wire::{Reader, Writer};
use lgduel_sim::usercmd::UserCmd;

const MAGIC: &[u8; 4] = b"LGDM";
pub const DEMO_VERSION: u16 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DemoKind {
    Local,
    Net,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DemoHeader {
    pub kind: DemoKind,
    pub map: String,
    pub map_hash: u64,
    /// Gameplay config (TOML) the session ran with.
    pub config_toml: String,
    /// The recording player.
    pub me: u8,
    /// Local: the game state at the start (a full snapshot encoding).
    pub initial: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum DemoFrame {
    /// Local: one simulated tick with every player's command.
    Tick { tick: u32, cmds: Vec<(u8, UserCmd)> },
    /// Net: a snapshot (full encoding) and when it arrived (µs since the start).
    Snapshot { at_us: u64, data: Vec<u8> },
    /// Net: a command the player sent.
    Cmd { at_us: u64, cmd: UserCmd },
    /// End of recording: last tick and the game's state hash after it (local).
    End { tick: u32, hash: u64 },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Demo {
    pub header: DemoHeader,
    pub frames: Vec<DemoFrame>,
}

fn write_frame(w: &mut Writer, f: &DemoFrame) {
    match f {
        DemoFrame::Tick { tick, cmds } => {
            w.u8(1);
            w.u32(*tick);
            w.u8(cmds.len() as u8);
            for (id, c) in cmds {
                w.u8(*id);
                write_cmd(w, c);
            }
        }
        DemoFrame::Snapshot { at_us, data } => {
            w.u8(2);
            w.u64(*at_us);
            w.bytes(data);
        }
        DemoFrame::Cmd { at_us, cmd } => {
            w.u8(3);
            w.u64(*at_us);
            write_cmd(w, cmd);
        }
        DemoFrame::End { tick, hash } => {
            w.u8(4);
            w.u32(*tick);
            w.u64(*hash);
        }
    }
}

fn read_frame(r: &mut Reader) -> Option<DemoFrame> {
    Some(match r.u8()? {
        1 => {
            let tick = r.u32()?;
            let n = r.u8()?;
            let mut cmds = Vec::with_capacity(n as usize);
            for _ in 0..n {
                cmds.push((r.u8()?, read_cmd(r)?));
            }
            DemoFrame::Tick { tick, cmds }
        }
        2 => DemoFrame::Snapshot {
            at_us: r.u64()?,
            data: r.bytes()?.to_vec(),
        },
        3 => DemoFrame::Cmd {
            at_us: r.u64()?,
            cmd: read_cmd(r)?,
        },
        4 => DemoFrame::End {
            tick: r.u32()?,
            hash: r.u64()?,
        },
        _ => return None,
    })
}

impl Demo {
    pub fn new(header: DemoHeader) -> Self {
        Self {
            header,
            frames: Vec::new(),
        }
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new();
        w.raw(MAGIC);
        w.u16(DEMO_VERSION);
        let h = &self.header;
        w.u8(match h.kind {
            DemoKind::Local => 1,
            DemoKind::Net => 2,
        });
        w.bytes(h.map.as_bytes());
        w.u64(h.map_hash);
        w.bytes(h.config_toml.as_bytes());
        w.u8(h.me);
        w.bytes(&h.initial);
        for f in &self.frames {
            write_frame(&mut w, f);
        }
        w.finish()
    }

    pub fn decode(data: &[u8]) -> Result<Demo, String> {
        let mut r = Reader::new(data);
        let bad = || "not an LGDuel demo (or a damaged one)".to_string();
        let magic = [r.u8(), r.u8(), r.u8(), r.u8()];
        if magic.map(|b| b.unwrap_or(0)) != *MAGIC {
            return Err(bad());
        }
        let version = r.u16().ok_or_else(bad)?;
        if version != DEMO_VERSION {
            return Err(format!(
                "demo version {version}, this LGDuel plays {DEMO_VERSION}"
            ));
        }
        let text = |r: &mut Reader| String::from_utf8(r.bytes()?.to_vec()).ok();
        let kind = match r.u8().ok_or_else(bad)? {
            1 => DemoKind::Local,
            2 => DemoKind::Net,
            _ => return Err(bad()),
        };
        let header = DemoHeader {
            kind,
            map: text(&mut r).ok_or_else(bad)?,
            map_hash: r.u64().ok_or_else(bad)?,
            config_toml: text(&mut r).ok_or_else(bad)?,
            me: r.u8().ok_or_else(bad)?,
            initial: r.bytes().ok_or_else(bad)?.to_vec(),
        };
        let mut frames = Vec::new();
        while r.remaining() > 0 {
            frames.push(read_frame(&mut r).ok_or_else(bad)?);
        }
        Ok(Demo { header, frames })
    }

    /// The stored end marker (tick, state hash), if the recording was finished.
    pub fn end(&self) -> Option<(u32, u64)> {
        self.frames.iter().rev().find_map(|f| match f {
            DemoFrame::End { tick, hash } => Some((*tick, *hash)),
            _ => None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lgduel_sim::math::Angle;
    use lgduel_sim::tick::Tick;

    #[test]
    fn round_trip_and_rejects_garbage() {
        let cmd = UserCmd {
            tick: Tick(7),
            pitch: Angle(123),
            yaw: Angle(u32::MAX),
            forward: 127,
            right: -127,
            buttons: 5,
        };
        let mut d = Demo::new(DemoHeader {
            kind: DemoKind::Local,
            map: "arena_tiers".into(),
            map_hash: 42,
            config_toml: "tick_rate_hz = 125\n".into(),
            me: 0,
            initial: vec![1, 2, 3],
        });
        d.frames = vec![
            DemoFrame::Tick {
                tick: 7,
                cmds: vec![(0, cmd), (1, UserCmd::default())],
            },
            DemoFrame::Snapshot {
                at_us: 9,
                data: vec![9, 9],
            },
            DemoFrame::Cmd { at_us: 10, cmd },
            DemoFrame::End { tick: 8, hash: 77 },
        ];
        let bytes = d.encode();
        assert_eq!(Demo::decode(&bytes), Ok(d.clone()));
        assert_eq!(d.end(), Some((8, 77)));
        assert!(Demo::decode(b"nope").is_err());
        assert!(
            Demo::decode(&bytes[..bytes.len() - 3]).is_err(),
            "truncated"
        );
        let mut v2 = bytes.clone();
        v2[4] = 99;
        assert!(Demo::decode(&v2).unwrap_err().contains("version"));
    }
}
