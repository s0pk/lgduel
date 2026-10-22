//! Server snapshots: complete game state at a tick plus the events of the
//! ticks since the previous snapshot. Floats travel as exact bit patterns so
//! the client's copy of its own state is bit-identical to the server's, which
//! prediction relies on.

use crate::wire::{Reader, Writer};
use lgduel_sim::game::{Game, GameEvent, Player, PlayerId, PlayerStats};
use lgduel_sim::math::{Angle, Vec3};
use lgduel_sim::pmove::PlayerMove;
use lgduel_sim::tick::Tick;

#[derive(Debug, Clone, PartialEq)]
pub struct Snapshot {
    /// The state is the game *before* simulating `tick` (i.e. `game.tick`).
    pub tick: Tick,
    pub fight_start: Option<Tick>,
    pub players: Vec<Player>,
    pub events: Vec<GameEvent>,
    /// Newest client command tick the server had consumed for the receiving
    /// client (prediction replays commands after this).
    pub last_cmd: Option<Tick>,
}

impl Snapshot {
    pub fn capture(game: &Game, events: Vec<GameEvent>, last_cmd: Option<Tick>) -> Self {
        Self {
            tick: game.tick,
            fight_start: game.fight_start,
            players: game.players.clone(),
            events,
            last_cmd,
        }
    }

    /// Overwrite the game's dynamic state with this snapshot.
    pub fn apply(&self, game: &mut Game) {
        game.tick = self.tick;
        game.fight_start = self.fight_start;
        game.players = self.players.clone();
    }

    pub fn player(&self, id: PlayerId) -> Option<&Player> {
        self.players.iter().find(|p| p.id == id)
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new();
        w.u8(2); // message kind: snapshot
        w.u32(self.tick.0);
        opt_tick(&mut w, self.fight_start);
        opt_tick(&mut w, self.last_cmd);
        w.u8(self.players.len() as u8);
        for p in &self.players {
            write_player(&mut w, p);
        }
        w.var(self.events.len() as u64);
        for e in &self.events {
            write_event(&mut w, e);
        }
        w.finish()
    }

    pub fn decode(data: &[u8]) -> Option<Self> {
        let mut r = Reader::new(data);
        if r.u8()? != 2 {
            return None;
        }
        let tick = Tick(r.u32()?);
        let fight_start = read_opt_tick(&mut r)?;
        let last_cmd = read_opt_tick(&mut r)?;
        let n = r.u8()?;
        let mut players = Vec::with_capacity(n as usize);
        for _ in 0..n {
            players.push(read_player(&mut r)?);
        }
        let ne = r.var()?.min(4096);
        let mut events = Vec::with_capacity(ne as usize);
        for _ in 0..ne {
            events.push(read_event(&mut r)?);
        }
        Some(Self {
            tick,
            fight_start,
            players,
            events,
            last_cmd,
        })
    }
}

fn opt_tick(w: &mut Writer, t: Option<Tick>) {
    w.u32(t.map_or(u32::MAX, |t| t.0));
}

fn read_opt_tick(r: &mut Reader) -> Option<Option<Tick>> {
    let v = r.u32()?;
    Some((v != u32::MAX).then_some(Tick(v)))
}

fn vec3(w: &mut Writer, v: Vec3) {
    w.f32(v.x);
    w.f32(v.y);
    w.f32(v.z);
}

fn read_vec3(r: &mut Reader) -> Option<Vec3> {
    Some(Vec3::new(r.f32()?, r.f32()?, r.f32()?))
}

pub(crate) fn write_player(w: &mut Writer, p: &Player) {
    w.u8(p.id.0);
    vec3(w, p.pm.origin);
    vec3(w, p.pm.velocity);
    match p.pm.ground_normal {
        Some(n) => {
            w.u8(1);
            vec3(w, n);
        }
        None => w.u8(0),
    }
    w.u8(p.pm.ducked as u8 | (p.pm.jump_held as u8) << 1 | (p.alive as u8) << 2);
    w.u32(p.pitch.0);
    w.u32(p.yaw.0);
    w.i32(p.health);
    w.i32(p.armor);
    opt_tick(w, p.respawn_tick);
    w.i32(p.weapon_time_us);
    w.u32(p.lifesteal_rem);
    w.u8(p.buttons);
    w.u32(p.frags);
    let s = &p.stats;
    for v in [
        s.shots,
        s.hits,
        s.damage_dealt,
        s.damage_taken,
        s.healed,
        s.lifesteal_earned,
        s.kills,
        s.deaths,
        s.fights_won,
        s.fights_lost,
        s.fight_ticks,
    ] {
        w.var(v as u64);
    }
}

pub(crate) fn read_player(r: &mut Reader) -> Option<Player> {
    let id = PlayerId(r.u8()?);
    let origin = read_vec3(r)?;
    let velocity = read_vec3(r)?;
    let ground_normal = match r.u8()? {
        0 => None,
        1 => Some(read_vec3(r)?),
        _ => return None,
    };
    let flags = r.u8()?;
    let pitch = Angle(r.u32()?);
    let yaw = Angle(r.u32()?);
    let health = r.i32()?;
    let armor = r.i32()?;
    let respawn_tick = read_opt_tick(r)?;
    let weapon_time_us = r.i32()?;
    let lifesteal_rem = r.u32()?;
    let buttons = r.u8()?;
    let frags = r.u32()?;
    let mut v = [0u32; 11];
    for x in &mut v {
        *x = u32::try_from(r.var()?).ok()?;
    }
    Some(Player {
        id,
        pm: PlayerMove {
            origin,
            velocity,
            ground_normal,
            ducked: flags & 1 != 0,
            jump_held: flags & 2 != 0,
        },
        pitch,
        yaw,
        health,
        armor,
        alive: flags & 4 != 0,
        respawn_tick,
        weapon_time_us,
        lifesteal_rem,
        buttons,
        frags,
        stats: PlayerStats {
            shots: v[0],
            hits: v[1],
            damage_dealt: v[2],
            damage_taken: v[3],
            healed: v[4],
            lifesteal_earned: v[5],
            kills: v[6],
            deaths: v[7],
            fights_won: v[8],
            fights_lost: v[9],
            fight_ticks: v[10],
        },
    })
}

fn write_event(w: &mut Writer, e: &GameEvent) {
    match e {
        GameEvent::Fire {
            shooter,
            start,
            end,
            target,
        } => {
            w.u8(1);
            w.u8(shooter.0);
            vec3(w, *start);
            vec3(w, *end);
            w.u8(target.map_or(0xFF, |t| t.0));
        }
        GameEvent::Damage {
            attacker,
            target,
            amount,
            healed,
        } => {
            w.u8(2);
            w.u8(attacker.0);
            w.u8(target.0);
            w.var(*amount as u64);
            w.var(*healed as u64);
        }
        GameEvent::Kill { killer, victim } => {
            w.u8(3);
            w.u8(killer.0);
            w.u8(victim.0);
        }
        GameEvent::Respawn { player, origin } => {
            w.u8(4);
            w.u8(player.0);
            vec3(w, *origin);
        }
        GameEvent::FightEnd {
            winner,
            loser,
            ticks,
        } => {
            w.u8(5);
            w.u8(winner.0);
            w.u8(loser.0);
            w.var(*ticks as u64);
        }
        GameEvent::MatchEnd { winner } => {
            w.u8(6);
            w.u8(winner.0);
        }
    }
}

fn read_event(r: &mut Reader) -> Option<GameEvent> {
    let pid = |r: &mut Reader| Some(PlayerId(r.u8()?));
    let small = |r: &mut Reader| u32::try_from(r.var()?).ok();
    Some(match r.u8()? {
        1 => GameEvent::Fire {
            shooter: pid(r)?,
            start: read_vec3(r)?,
            end: read_vec3(r)?,
            target: match r.u8()? {
                0xFF => None,
                t => Some(PlayerId(t)),
            },
        },
        2 => GameEvent::Damage {
            attacker: pid(r)?,
            target: pid(r)?,
            amount: small(r)?,
            healed: small(r)?,
        },
        3 => GameEvent::Kill {
            killer: pid(r)?,
            victim: pid(r)?,
        },
        4 => GameEvent::Respawn {
            player: pid(r)?,
            origin: read_vec3(r)?,
        },
        5 => GameEvent::FightEnd {
            winner: pid(r)?,
            loser: pid(r)?,
            ticks: small(r)?,
        },
        6 => GameEvent::MatchEnd { winner: pid(r)? },
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use lgduel_sim::config::GameConfig;
    use lgduel_sim::math::look_angles;
    use lgduel_sim::usercmd::{UserCmd, buttons};
    use lgduel_sim::world::{World, builtin};

    fn game() -> Game {
        let cfg = GameConfig::default();
        let p = &cfg.physics;
        let hull = (
            Vec3::from_array(p.player_mins),
            Vec3::from_array(p.player_maxs),
        );
        let mut g = Game::new(
            cfg,
            World::from_toml_str(builtin::ARENA_FLAT, hull).unwrap(),
        );
        g.add_player();
        g.add_player();
        g
    }

    /// Play a messy fight so every field is non-trivial.
    fn played() -> (Game, Vec<GameEvent>) {
        let mut g = game();
        let (a, b) = (g.players[0].id, g.players[1].id);
        let z = g.players[0].pm.origin.z;
        g.players[0].pm.origin = Vec3::new(-150.0, 0.0, z);
        g.players[1].pm.origin = Vec3::new(150.0, 0.0, z);
        let mut events = vec![];
        for t in 0..900u32 {
            let pa = g.player(a).unwrap().pm.eye(&g.cfg.physics);
            let pb = g.player(b).unwrap().pm.origin;
            let (pitch, yaw) = look_angles(pb - pa);
            let ca = UserCmd {
                tick: g.tick,
                pitch,
                yaw,
                right: if (t / 40) % 2 == 0 { 127 } else { -127 },
                buttons: buttons::FIRE | if t % 97 == 0 { buttons::JUMP } else { 0 },
                ..UserCmd::default()
            };
            let cb = UserCmd {
                tick: g.tick,
                forward: 127,
                buttons: buttons::CROUCH,
                ..UserCmd::default()
            };
            g.step(&[(a, ca), (b, cb)]);
            events.extend(g.events.clone());
        }
        (g, events)
    }

    #[test]
    fn snapshot_round_trip_reproduces_state_hash() {
        let (g, events) = played();
        assert!(
            events.iter().any(|e| matches!(e, GameEvent::Kill { .. })),
            "fight had a kill"
        );
        let snap = Snapshot::capture(&g, events.clone(), Some(Tick(899)));
        let bytes = snap.encode();
        let back = Snapshot::decode(&bytes).unwrap();
        assert_eq!(back, snap);
        let mut fresh = game();
        back.apply(&mut fresh);
        assert_eq!(fresh.state_hash(), g.state_hash());
        // And the client can keep simulating identically from it.
        let mut g2 = g.clone();
        let c = UserCmd {
            tick: g.tick,
            forward: 127,
            ..UserCmd::default()
        };
        g2.step(&[(g.players[0].id, c)]);
        fresh.step(&[(g.players[0].id, c)]);
        assert_eq!(fresh.state_hash(), g2.state_hash());
    }

    #[test]
    fn snapshot_size_is_small() {
        let (g, _) = played();
        let s = Snapshot::capture(&g, vec![], None).encode();
        println!("full 2-player snapshot: {} bytes", s.len());
        assert!(s.len() < 200, "{}", s.len());
    }

    #[test]
    fn truncated_snapshot_rejected() {
        let (g, ev) = played();
        let bytes = Snapshot::capture(&g, ev, None).encode();
        for cut in [1, 5, bytes.len() / 2, bytes.len() - 1] {
            assert!(Snapshot::decode(&bytes[..cut]).is_none(), "cut {cut}");
        }
    }
}
