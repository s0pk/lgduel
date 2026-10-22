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
    /// Events that matter if lost (damage, kills, respawns, fight/match ends):
    /// the server repeats them in every snapshot until one carrying them is
    /// acked; `event_seq` numbers the first, so the client can drop repeats.
    pub reliable_events: Vec<GameEvent>,
    pub event_seq: u32,
}

impl Snapshot {
    pub fn capture(game: &Game, events: Vec<GameEvent>, last_cmd: Option<Tick>) -> Self {
        Self {
            tick: game.tick,
            fight_start: game.fight_start,
            players: game.players.clone(),
            events,
            last_cmd,
            reliable_events: Vec::new(),
            event_seq: 0,
        }
    }

    /// Fire events (beam visuals) are only worth showing when fresh; every
    /// other event is sent reliably.
    pub fn is_reliable_event(e: &GameEvent) -> bool {
        !matches!(e, GameEvent::Fire { .. })
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
        write_events(&mut w, self);
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
        let (events, reliable_events, event_seq) = read_events(&mut r)?;
        Some(Self {
            tick,
            fight_start,
            players,
            events,
            last_cmd,
            reliable_events,
            event_seq,
        })
    }
}

fn write_events(w: &mut Writer, s: &Snapshot) {
    w.var(s.events.len() as u64);
    for e in &s.events {
        write_event(w, e);
    }
    // Reliable events: count, then (only if any) the first sequence number.
    w.var(s.reliable_events.len() as u64);
    if !s.reliable_events.is_empty() {
        w.var(s.event_seq as u64);
    }
    for e in &s.reliable_events {
        write_event(w, e);
    }
}

#[allow(clippy::type_complexity)]
fn read_events(r: &mut Reader) -> Option<(Vec<GameEvent>, Vec<GameEvent>, u32)> {
    let mut list = || -> Option<Vec<GameEvent>> {
        let n = r.var()?.min(4096);
        let mut v = Vec::with_capacity(n as usize);
        for _ in 0..n {
            v.push(read_event(r)?);
        }
        Some(v)
    };
    let events = list()?;
    let n = r.var()?.min(4096);
    let seq = if n > 0 {
        u32::try_from(r.var()?).ok()?
    } else {
        0
    };
    let mut reliable = Vec::with_capacity(n as usize);
    for _ in 0..n {
        reliable.push(read_event(r)?);
    }
    Some((events, reliable, seq))
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

// ---------------------------------------------------------------------------
// Delta compression against a baseline the client has acknowledged.
// ---------------------------------------------------------------------------

const F_ORIGIN: u16 = 1 << 0;
const F_VELOCITY: u16 = 1 << 1;
const F_GROUND: u16 = 1 << 2;
const F_FLAGS: u16 = 1 << 3;
const F_PITCH: u16 = 1 << 4;
const F_YAW: u16 = 1 << 5;
const F_HEALTH: u16 = 1 << 6;
const F_ARMOR: u16 = 1 << 7;
const F_RESPAWN: u16 = 1 << 8;
const F_WEAPON: u16 = 1 << 9;
const F_LIFESTEAL: u16 = 1 << 10;
const F_BUTTONS: u16 = 1 << 11;
const F_FRAGS: u16 = 1 << 12;
const F_STATS: u16 = 1 << 13;
/// Player not in the baseline: full record follows.
const F_FULL: u16 = 1 << 15;

fn bits_eq(a: Vec3, b: Vec3) -> bool {
    a.x.to_bits() == b.x.to_bits()
        && a.y.to_bits() == b.y.to_bits()
        && a.z.to_bits() == b.z.to_bits()
}

fn player_flags(p: &Player) -> u8 {
    p.pm.ducked as u8 | (p.pm.jump_held as u8) << 1 | (p.alive as u8) << 2
}

fn ground_eq(a: Option<Vec3>, b: Option<Vec3>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(x), Some(y)) => bits_eq(x, y),
        _ => false,
    }
}

fn changed(b: &Player, p: &Player) -> u16 {
    let checks = [
        (F_ORIGIN, !bits_eq(b.pm.origin, p.pm.origin)),
        (F_VELOCITY, !bits_eq(b.pm.velocity, p.pm.velocity)),
        (F_GROUND, !ground_eq(b.pm.ground_normal, p.pm.ground_normal)),
        (F_FLAGS, player_flags(b) != player_flags(p)),
        (F_PITCH, b.pitch != p.pitch),
        (F_YAW, b.yaw != p.yaw),
        (F_HEALTH, b.health != p.health),
        (F_ARMOR, b.armor != p.armor),
        (F_RESPAWN, b.respawn_tick != p.respawn_tick),
        (F_WEAPON, b.weapon_time_us != p.weapon_time_us),
        (F_LIFESTEAL, b.lifesteal_rem != p.lifesteal_rem),
        (F_BUTTONS, b.buttons != p.buttons),
        (F_FRAGS, b.frags != p.frags),
        (F_STATS, b.stats != p.stats),
    ];
    checks
        .iter()
        .filter(|(_, d)| *d)
        .fold(0, |m, (bit, _)| m | bit)
}

fn write_stats(w: &mut Writer, s: &PlayerStats) {
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

fn read_stats(r: &mut Reader) -> Option<PlayerStats> {
    let mut v = [0u32; 11];
    for x in &mut v {
        *x = u32::try_from(r.var()?).ok()?;
    }
    Some(PlayerStats {
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
    })
}

fn write_ground(w: &mut Writer, g: Option<Vec3>) {
    match g {
        Some(n) => {
            w.u8(1);
            vec3(w, n);
        }
        None => w.u8(0),
    }
}

fn read_ground(r: &mut Reader) -> Option<Option<Vec3>> {
    match r.u8()? {
        0 => Some(None),
        1 => Some(Some(read_vec3(r)?)),
        _ => None,
    }
}

fn write_player_delta(w: &mut Writer, b: &Player, p: &Player) {
    let m = changed(b, p);
    w.u8(p.id.0);
    w.u16(m);
    if m & F_ORIGIN != 0 {
        vec3(w, p.pm.origin);
    }
    if m & F_VELOCITY != 0 {
        vec3(w, p.pm.velocity);
    }
    if m & F_GROUND != 0 {
        write_ground(w, p.pm.ground_normal);
    }
    if m & F_FLAGS != 0 {
        w.u8(player_flags(p));
    }
    if m & F_PITCH != 0 {
        w.u32(p.pitch.0);
    }
    if m & F_YAW != 0 {
        w.u32(p.yaw.0);
    }
    if m & F_HEALTH != 0 {
        w.svar(p.health as i64);
    }
    if m & F_ARMOR != 0 {
        w.svar(p.armor as i64);
    }
    if m & F_RESPAWN != 0 {
        opt_tick(w, p.respawn_tick);
    }
    if m & F_WEAPON != 0 {
        w.svar(p.weapon_time_us as i64);
    }
    if m & F_LIFESTEAL != 0 {
        w.var(p.lifesteal_rem as u64);
    }
    if m & F_BUTTONS != 0 {
        w.u8(p.buttons);
    }
    if m & F_FRAGS != 0 {
        w.var(p.frags as u64);
    }
    if m & F_STATS != 0 {
        write_stats(w, &p.stats);
    }
}

fn read_player_fields(r: &mut Reader, m: u16, b: &Player) -> Option<Player> {
    let mut p = b.clone();
    if m & F_ORIGIN != 0 {
        p.pm.origin = read_vec3(r)?;
    }
    if m & F_VELOCITY != 0 {
        p.pm.velocity = read_vec3(r)?;
    }
    if m & F_GROUND != 0 {
        p.pm.ground_normal = read_ground(r)?;
    }
    if m & F_FLAGS != 0 {
        let f = r.u8()?;
        p.pm.ducked = f & 1 != 0;
        p.pm.jump_held = f & 2 != 0;
        p.alive = f & 4 != 0;
    }
    if m & F_PITCH != 0 {
        p.pitch = Angle(r.u32()?);
    }
    if m & F_YAW != 0 {
        p.yaw = Angle(r.u32()?);
    }
    if m & F_HEALTH != 0 {
        p.health = i32::try_from(r.svar()?).ok()?;
    }
    if m & F_ARMOR != 0 {
        p.armor = i32::try_from(r.svar()?).ok()?;
    }
    if m & F_RESPAWN != 0 {
        p.respawn_tick = read_opt_tick(r)?;
    }
    if m & F_WEAPON != 0 {
        p.weapon_time_us = i32::try_from(r.svar()?).ok()?;
    }
    if m & F_LIFESTEAL != 0 {
        p.lifesteal_rem = u32::try_from(r.var()?).ok()?;
    }
    if m & F_BUTTONS != 0 {
        p.buttons = r.u8()?;
    }
    if m & F_FRAGS != 0 {
        p.frags = u32::try_from(r.var()?).ok()?;
    }
    if m & F_STATS != 0 {
        p.stats = read_stats(r)?;
    }
    Some(p)
}

impl Snapshot {
    /// Encode against `base` (a snapshot the receiver is known to have), or
    /// self-contained if `base` is None. Events are always sent in full.
    pub fn encode_delta(&self, base: Option<&Snapshot>) -> Vec<u8> {
        let mut w = Writer::new();
        w.u8(3); // message kind: delta snapshot
        w.u32(self.tick.0);
        opt_tick(&mut w, base.map(|b| b.tick));
        opt_tick(&mut w, self.fight_start);
        opt_tick(&mut w, self.last_cmd);
        w.u8(self.players.len() as u8);
        for p in &self.players {
            match base.and_then(|b| b.player(p.id)) {
                Some(bp) => write_player_delta(&mut w, bp, p),
                None => {
                    w.u8(p.id.0);
                    w.u16(F_FULL);
                    write_player(&mut w, p);
                }
            }
        }
        write_events(&mut w, self);
        w.finish()
    }

    /// Baseline tick named by a delta snapshot (`Some(None)` = self-contained);
    /// `None` if the bytes aren't a delta snapshot.
    pub fn delta_base(data: &[u8]) -> Option<Option<Tick>> {
        let mut r = Reader::new(data);
        if r.u8()? != 3 {
            return None;
        }
        r.u32()?;
        read_opt_tick(&mut r)
    }

    /// Decode a delta snapshot; `base` must be the snapshot named by `delta_base`.
    pub fn decode_delta(data: &[u8], base: Option<&Snapshot>) -> Option<Snapshot> {
        let mut r = Reader::new(data);
        if r.u8()? != 3 {
            return None;
        }
        let tick = Tick(r.u32()?);
        if read_opt_tick(&mut r)? != base.map(|b| b.tick) {
            return None; // wrong or missing baseline
        }
        let fight_start = read_opt_tick(&mut r)?;
        let last_cmd = read_opt_tick(&mut r)?;
        let n = r.u8()?;
        let mut players = Vec::with_capacity(n as usize);
        for _ in 0..n {
            let id = PlayerId(r.u8()?);
            let m = r.u16()?;
            let p = if m == F_FULL {
                read_player(&mut r)?
            } else {
                read_player_fields(&mut r, m, base.and_then(|b| b.player(id))?)?
            };
            if p.id != id {
                return None;
            }
            players.push(p);
        }
        let (events, reliable_events, event_seq) = read_events(&mut r)?;
        Some(Self {
            tick,
            fight_start,
            players,
            events,
            last_cmd,
            reliable_events,
            event_seq,
        })
    }
}

/// Ring of recently sent (server) or received (client) snapshots, by tick.
#[derive(Debug, Clone, Default)]
pub struct SnapshotHistory {
    items: std::collections::VecDeque<Snapshot>,
    cap: usize,
}

impl SnapshotHistory {
    pub fn new(cap: usize) -> Self {
        Self {
            items: Default::default(),
            cap,
        }
    }

    pub fn push(&mut self, s: Snapshot) {
        self.items.retain(|x| x.tick != s.tick);
        self.items.push_back(s);
        while self.items.len() > self.cap {
            self.items.pop_front();
        }
    }

    pub fn get(&self, tick: Tick) -> Option<&Snapshot> {
        self.items.iter().find(|s| s.tick == tick)
    }

    pub fn latest(&self) -> Option<&Snapshot> {
        self.items.iter().max_by_key(|s| s.tick)
    }
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
    fn delta_chain_reproduces_every_snapshot_and_is_smaller() {
        let mut g = game();
        let (a, b) = (g.players[0].id, g.players[1].id);
        let z = g.players[0].pm.origin.z;
        g.players[0].pm.origin = Vec3::new(-150.0, 0.0, z);
        g.players[1].pm.origin = Vec3::new(150.0, 0.0, z);
        // The client acks every snapshot it gets; every 5th is lost in transit,
        // so the server keeps deltaing from an older baseline.
        let mut server_hist = SnapshotHistory::new(64);
        let mut client_hist = SnapshotHistory::new(64);
        let mut acked: Option<Tick> = None;
        let (mut full_bytes, mut delta_bytes) = (0usize, 0usize);
        for t in 0..1500u32 {
            let pa = g.player(a).unwrap().pm.eye(&g.cfg.physics);
            let (pitch, yaw) = look_angles(g.player(b).unwrap().pm.origin - pa);
            let ca = UserCmd {
                tick: g.tick,
                pitch,
                yaw,
                right: if (t / 40).is_multiple_of(2) {
                    127
                } else {
                    -127
                },
                buttons: buttons::FIRE,
                ..UserCmd::default()
            };
            g.step(&[(a, ca)]);
            let snap = Snapshot::capture(&g, g.events.clone(), Some(Tick(t)));
            let base = acked.and_then(|t| server_hist.get(t));
            let bytes = snap.encode_delta(base);
            full_bytes += snap.encode().len();
            delta_bytes += bytes.len();
            server_hist.push(snap.clone());
            if t % 5 == 4 {
                continue; // lost in transit
            }
            let base_tick = Snapshot::delta_base(&bytes).unwrap();
            let cbase = base_tick.and_then(|bt| client_hist.get(bt));
            let got = Snapshot::decode_delta(&bytes, cbase).expect("client has the baseline");
            assert_eq!(got, snap, "tick {t}");
            client_hist.push(got);
            acked = Some(snap.tick);
        }
        println!(
            "1500 snapshots: full {full_bytes} B, delta {delta_bytes} B ({:.0}%)",
            100.0 * delta_bytes as f64 / full_bytes as f64
        );
        assert!(delta_bytes * 10 < full_bytes * 7, "delta must save >30%");
    }

    #[test]
    fn delta_rejects_wrong_baseline() {
        let (g, _) = played();
        let base = Snapshot::capture(&g, vec![], None);
        let mut g2 = g.clone();
        g2.step(&[]);
        let s = Snapshot::capture(&g2, vec![], None);
        let bytes = s.encode_delta(Some(&base));
        assert!(Snapshot::decode_delta(&bytes, None).is_none());
        let mut other = base.clone();
        other.tick = Tick(base.tick.0 + 100);
        assert!(Snapshot::decode_delta(&bytes, Some(&other)).is_none());
        assert_eq!(Snapshot::decode_delta(&bytes, Some(&base)), Some(s));
    }

    #[test]
    fn unchanged_state_costs_almost_nothing() {
        let (g, _) = played();
        let base = Snapshot::capture(&g, vec![], None);
        let mut s = base.clone();
        s.tick = Tick(base.tick.0 + 1);
        let d = s.encode_delta(Some(&base));
        println!("unchanged-state delta: {} bytes", d.len());
        // kind + tick + base + fight + last_cmd + count + 2 × (id + mask) + events
        // + reliable events.
        assert_eq!(d.len(), 1 + 4 * 4 + 1 + 2 * 3 + 1 + 1);
        assert_eq!(Snapshot::decode_delta(&d, Some(&base)), Some(s));
    }

    #[test]
    fn reliable_events_round_trip() {
        let (g, ev) = played();
        let mut s = Snapshot::capture(&g, vec![], None);
        s.reliable_events = ev
            .iter()
            .filter(|e| Snapshot::is_reliable_event(e))
            .cloned()
            .collect();
        assert!(!s.reliable_events.is_empty());
        s.event_seq = 70_000;
        assert_eq!(Snapshot::decode(&s.encode()), Some(s.clone()));
        assert_eq!(Snapshot::decode_delta(&s.encode_delta(None), None), Some(s));
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
