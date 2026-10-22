//! Game state: players, Lightning Gun combat, damage, lifesteal, respawn.
//! Advanced one fixed tick at a time from per-player `UserCmd`s.

use crate::config::GameConfig;
use crate::math::{Angle, StateHasher, Vec3, angle_vectors};
use crate::pmove::{PlayerMove, pmove};
use crate::tick::Tick;
use crate::trace::trace_boxes;
use crate::usercmd::{UserCmd, buttons};
use crate::world::{Aabb, World};
use serde::{Deserialize, Serialize};

/// Velocity per point of damage at knockback scale 1.0 (Quake: g_knockback 1000 / mass 200).
pub const KNOCKBACK_PER_DAMAGE: f32 = 5.0;
/// Fraction of damage armor absorbs, in percent (Quake: 0.66).
pub const ARMOR_PROTECTION_PCT: u32 = 66;

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
)]
pub struct PlayerId(pub u8);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct PlayerStats {
    /// LG cells fired (one per refire interval while firing).
    pub shots: u32,
    pub hits: u32,
    pub damage_dealt: u32,
    pub damage_taken: u32,
    /// Health actually gained from lifesteal (after the max-health cap).
    pub healed: u32,
    /// Lifesteal earned before the cap (= floor(dealt * pct / 100) over the life of the stats).
    pub lifesteal_earned: u32,
    pub kills: u32,
    pub deaths: u32,
}

impl PlayerStats {
    /// LG accuracy in percent (0 if nothing fired).
    pub fn accuracy_pct(&self) -> f32 {
        if self.shots == 0 {
            0.0
        } else {
            self.hits as f32 * 100.0 / self.shots as f32
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Player {
    pub id: PlayerId,
    pub pm: PlayerMove,
    pub pitch: Angle,
    pub yaw: Angle,
    pub health: i32,
    pub armor: i32,
    pub alive: bool,
    /// Tick at which a dead player respawns.
    pub respawn_tick: Option<Tick>,
    /// Remaining refire time, µs; can go negative to carry over fractional ticks.
    pub weapon_time_us: i32,
    /// Lifesteal remainder in hundredths of a health point.
    pub lifesteal_rem: u32,
    /// Last buttons (for input display / demos).
    pub buttons: u8,
    pub stats: PlayerStats,
}

/// Something that happened during a tick (for clients, stats, demos).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum GameEvent {
    /// One LG cell: beam from `start` to `end`, hitting `target` if any.
    Fire {
        shooter: PlayerId,
        start: Vec3,
        end: Vec3,
        target: Option<PlayerId>,
    },
    Damage {
        attacker: PlayerId,
        target: PlayerId,
        /// Health + armor actually removed.
        amount: u32,
        /// Health the attacker gained from lifesteal (after cap).
        healed: u32,
    },
    Kill {
        killer: PlayerId,
        victim: PlayerId,
    },
    Respawn {
        player: PlayerId,
        origin: Vec3,
    },
}

/// Result of a hitscan.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Hitscan {
    pub end: Vec3,
    pub target: Option<PlayerId>,
}

/// Cast a ray against the world and a set of target boxes; nearest hit wins.
/// Takes boxes explicitly so lag compensation can pass rewound positions.
pub fn hitscan(
    world: &World,
    start: Vec3,
    dir: Vec3,
    range: f32,
    targets: &[(PlayerId, Aabb)],
) -> Hitscan {
    let end = start.mad(range, dir);
    let wt = world.trace_ray(start, end);
    let mut best = wt.fraction;
    let mut target = None;
    for (id, b) in targets {
        let t = trace_boxes(std::slice::from_ref(b), start, end, Vec3::ZERO, Vec3::ZERO);
        let f = if t.start_solid { 0.0 } else { t.fraction };
        if f < best || (t.start_solid && target.is_none()) {
            best = f;
            target = Some(*id);
        }
    }
    Hitscan {
        end: start + (end - start) * best,
        target,
    }
}

#[derive(Debug, Clone)]
pub struct Game {
    pub cfg: GameConfig,
    pub world: World,
    /// Next tick to simulate.
    pub tick: Tick,
    pub players: Vec<Player>,
    /// Events from the most recent `step`.
    pub events: Vec<GameEvent>,
}

impl Game {
    pub fn new(cfg: GameConfig, world: World) -> Self {
        Self {
            cfg,
            world,
            tick: Tick(0),
            players: Vec::new(),
            events: Vec::new(),
        }
    }

    pub fn dt(&self) -> f32 {
        1.0 / self.cfg.tick_rate_hz as f32
    }

    fn tick_us(&self) -> i32 {
        (1_000_000 / self.cfg.tick_rate_hz) as i32
    }

    /// Add a player at the best spawn; returns its id.
    pub fn add_player(&mut self) -> PlayerId {
        let id = PlayerId(
            (0..=u8::MAX)
                .find(|i| self.players.iter().all(|p| p.id.0 != *i))
                .expect("player slots exhausted"),
        );
        let mut p = Player {
            id,
            pm: PlayerMove::default(),
            pitch: Angle::ZERO,
            yaw: Angle::ZERO,
            health: 0,
            armor: 0,
            alive: false,
            respawn_tick: None,
            weapon_time_us: 0,
            lifesteal_rem: 0,
            buttons: 0,
            stats: PlayerStats::default(),
        };
        self.spawn(&mut p);
        self.players.push(p);
        id
    }

    pub fn remove_player(&mut self, id: PlayerId) {
        self.players.retain(|p| p.id != id);
    }

    pub fn player(&self, id: PlayerId) -> Option<&Player> {
        self.players.iter().find(|p| p.id == id)
    }

    pub fn player_mut(&mut self, id: PlayerId) -> Option<&mut Player> {
        self.players.iter_mut().find(|p| p.id == id)
    }

    /// Spawn point farthest from every other living player (ties → first listed).
    fn pick_spawn(&self, exclude: PlayerId) -> crate::world::Spawn {
        let others: Vec<Vec3> = self
            .players
            .iter()
            .filter(|p| p.alive && p.id != exclude)
            .map(|p| p.pm.origin)
            .collect();
        let mut best = self.world.spawns[0];
        let mut best_d = f32::NEG_INFINITY;
        for s in &self.world.spawns {
            let d = others
                .iter()
                .map(|o| (s.origin - *o).length())
                .fold(f32::INFINITY, f32::min);
            if d > best_d {
                best_d = d;
                best = *s;
            }
        }
        best
    }

    fn spawn(&mut self, p: &mut Player) {
        let s = self.pick_spawn(p.id);
        p.pm = PlayerMove::at(s.origin);
        p.pitch = Angle::ZERO;
        p.yaw = s.yaw;
        p.health = self.cfg.rules.spawn_health as i32;
        p.armor = self.cfg.rules.spawn_armor as i32;
        p.alive = true;
        p.respawn_tick = None;
        p.weapon_time_us = 0;
        self.events.push(GameEvent::Respawn {
            player: p.id,
            origin: s.origin,
        });
    }

    /// Hit box of a living player.
    pub fn hitbox(&self, p: &Player) -> Aabb {
        let (mins, maxs) = p.pm.hull(&self.cfg.physics);
        Aabb::at(p.pm.origin, mins, maxs)
    }

    /// Advance one tick. `cmds` holds at most one command per player; players
    /// without one repeat neutral input. Players are processed in list order.
    pub fn step(&mut self, cmds: &[(PlayerId, UserCmd)]) {
        self.events.clear();
        let tick = self.tick;
        let dt = self.dt();
        let tick_us = self.tick_us();

        // Respawns due this tick.
        for i in 0..self.players.len() {
            let due =
                !self.players[i].alive && self.players[i].respawn_tick.is_some_and(|t| t <= tick);
            if due {
                let mut p = self.players[i].clone();
                self.spawn(&mut p);
                self.players[i] = p;
            }
        }

        for i in 0..self.players.len() {
            let id = self.players[i].id;
            let cmd = cmds
                .iter()
                .find(|(pid, _)| *pid == id)
                .map(|(_, c)| c.sanitized())
                .unwrap_or_else(|| UserCmd {
                    tick,
                    pitch: self.players[i].pitch,
                    yaw: self.players[i].yaw,
                    ..UserCmd::default()
                });
            let p = &mut self.players[i];
            p.pitch = cmd.pitch;
            p.yaw = cmd.yaw;
            p.buttons = cmd.buttons;
            if !p.alive {
                continue;
            }
            pmove(&mut p.pm, &cmd, &self.world, &self.cfg.physics, dt);
            self.weapon(i, &cmd, tick_us);
        }
        self.tick = tick.next();
    }

    fn weapon(&mut self, i: usize, cmd: &UserCmd, tick_us: i32) {
        let p = &mut self.players[i];
        if p.weapon_time_us > 0 {
            p.weapon_time_us -= tick_us;
        }
        if !cmd.pressed(buttons::FIRE) {
            p.weapon_time_us = p.weapon_time_us.max(0);
            return;
        }
        if p.weapon_time_us > 0 {
            return;
        }
        p.weapon_time_us += self.cfg.lg.refire_ms as i32 * 1000;
        self.fire(i);
    }

    fn fire(&mut self, i: usize) {
        let shooter = &self.players[i];
        let sid = shooter.id;
        let start = shooter.pm.eye(&self.cfg.physics);
        let (dir, _, _) = angle_vectors(shooter.pitch, shooter.yaw);
        let targets: Vec<(PlayerId, Aabb)> = self
            .players
            .iter()
            .filter(|p| p.alive && p.id != sid)
            .map(|p| (p.id, self.hitbox(p)))
            .collect();
        let hs = hitscan(&self.world, start, dir, self.cfg.lg.range, &targets);
        self.players[i].stats.shots += 1;
        self.events.push(GameEvent::Fire {
            shooter: sid,
            start,
            end: hs.end,
            target: hs.target,
        });
        if let Some(tid) = hs.target {
            self.players[i].stats.hits += 1;
            self.damage(sid, tid, self.cfg.lg.damage, dir);
        }
    }

    /// Apply damage from `attacker` to `target` along `dir`.
    pub fn damage(&mut self, attacker: PlayerId, target: PlayerId, damage: u32, dir: Vec3) {
        let ti = self
            .players
            .iter()
            .position(|p| p.id == target)
            .expect("target exists");
        let ai = self
            .players
            .iter()
            .position(|p| p.id == attacker)
            .expect("attacker exists");
        let knock = damage as f32 * KNOCKBACK_PER_DAMAGE * self.cfg.lg.knockback;
        let t = &mut self.players[ti];
        if !t.alive {
            return;
        }
        t.pm.velocity = t.pm.velocity.mad(knock, dir);
        // Armor absorbs its share first.
        let mut save = (damage * ARMOR_PROTECTION_PCT).div_ceil(100) as i32;
        save = save.min(t.armor).max(0);
        t.armor -= save;
        let take = damage as i32 - save;
        let health_removed = take.min(t.health.max(0));
        t.health -= take;
        let dealt = (health_removed + save) as u32;
        t.stats.damage_taken += dealt;
        let died = t.health <= 0;

        // Lifesteal: exact integer percentage with carried remainder.
        let max_health = self.cfg.rules.max_health as i32;
        let pct = self.cfg.rules.lifesteal_percent;
        let a = &mut self.players[ai];
        a.stats.damage_dealt += dealt;
        let total = dealt * pct + a.lifesteal_rem;
        let earned = total / 100;
        a.lifesteal_rem = total % 100;
        a.stats.lifesteal_earned += earned;
        let mut healed = 0;
        if a.alive {
            let before = a.health;
            a.health = (a.health + earned as i32).min(max_health.max(a.health));
            healed = (a.health - before) as u32;
            a.stats.healed += healed;
        }
        self.events.push(GameEvent::Damage {
            attacker,
            target,
            amount: dealt,
            healed,
        });
        if died {
            self.kill(attacker, target);
        }
    }

    fn kill(&mut self, killer: PlayerId, victim: PlayerId) {
        let respawn = Tick(
            self.tick.0 + (self.cfg.rules.respawn_delay_ms * self.cfg.tick_rate_hz).div_ceil(1000),
        );
        if let Some(v) = self.player_mut(victim) {
            v.alive = false;
            v.respawn_tick = Some(respawn);
            v.stats.deaths += 1;
            v.pm.velocity = Vec3::ZERO;
        }
        if killer != victim
            && let Some(k) = self.player_mut(killer)
        {
            k.stats.kills += 1;
        }
        self.events.push(GameEvent::Kill { killer, victim });
    }

    /// Hash of all simulation state (determinism checks, desync detection).
    pub fn state_hash(&self) -> u64 {
        let mut h = StateHasher::new();
        h.u32(self.tick.0);
        for p in &self.players {
            h.u32(p.id.0 as u32);
            h.vec3(p.pm.origin);
            h.vec3(p.pm.velocity);
            match p.pm.ground_normal {
                Some(n) => {
                    h.u32(1);
                    h.vec3(n);
                }
                None => h.u32(0),
            }
            h.u32(p.pm.ducked as u32 | (p.pm.jump_held as u32) << 1 | (p.alive as u32) << 2);
            h.u32(p.pitch.0);
            h.u32(p.yaw.0);
            h.u32(p.health as u32);
            h.u32(p.armor as u32);
            h.u32(p.respawn_tick.map_or(u32::MAX, |t| t.0));
            h.u32(p.weapon_time_us as u32);
            h.u32(p.lifesteal_rem);
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
            ] {
                h.u32(v);
            }
        }
        h.finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::builtin;

    fn game_with(cfg: GameConfig) -> (Game, PlayerId, PlayerId) {
        let p = &cfg.physics;
        let hull = (
            Vec3::from_array(p.player_mins),
            Vec3::from_array(p.player_maxs),
        );
        let world = World::from_toml_str(builtin::ARENA_FLAT, hull).unwrap();
        let mut g = Game::new(cfg, world);
        let a = g.add_player();
        let b = g.add_player();
        (g, a, b)
    }

    /// Place a at x=-200, b at x=+200 facing each other, both on the floor.
    fn face_off(g: &mut Game, a: PlayerId, b: PlayerId, dist: f32) {
        let z = 24.0 + crate::trace::SURFACE_EPSILON;
        g.player_mut(a).unwrap().pm = PlayerMove::at(Vec3::new(-dist / 2.0, 0.0, z));
        g.player_mut(b).unwrap().pm = PlayerMove::at(Vec3::new(dist / 2.0, 0.0, z));
    }

    fn aim(g: &Game, from: PlayerId, at: PlayerId, fire: bool) -> UserCmd {
        let f = g.player(from).unwrap();
        let t = g.player(at).unwrap();
        let d = t.pm.origin - f.pm.eye(&g.cfg.physics);
        let yaw = (d.y as f64).atan2(d.x as f64).to_degrees();
        let pitch = -(d.z as f64).atan2(d.length_xy() as f64).to_degrees();
        UserCmd {
            tick: g.tick,
            pitch: Angle::from_degrees(pitch),
            yaw: Angle::from_degrees(yaw),
            buttons: if fire { buttons::FIRE } else { 0 },
            ..UserCmd::default()
        }
    }

    fn fire_ticks(g: &Game) -> usize {
        g.events
            .iter()
            .filter(|e| matches!(e, GameEvent::Fire { .. }))
            .count()
    }

    #[test]
    fn refire_averages_50ms() {
        let (mut g, a, b) = game_with(GameConfig::default());
        face_off(&mut g, a, b, 2000.0); // out of range: just count cells
        let mut fired_at = vec![];
        for t in 0..126 {
            let c = aim(&g, a, b, true);
            g.step(&[(a, c)]);
            if fire_ticks(&g) > 0 {
                fired_at.push(t);
            }
        }
        // 1.008 s of holding fire at 50 ms → 21 cells (t = 0 and every 50 ms after).
        assert_eq!(fired_at.len(), 21, "{fired_at:?}");
        let gaps: Vec<i32> = fired_at.windows(2).map(|w| w[1] - w[0]).collect();
        assert!(gaps.iter().all(|g| *g == 6 || *g == 7), "{gaps:?}");
    }

    #[test]
    fn beam_hits_in_range_and_misses_beyond() {
        let (mut g, a, b) = game_with(GameConfig::default());
        face_off(&mut g, a, b, 700.0);
        let c = aim(&g, a, b, true);
        g.step(&[(a, c)]);
        assert_eq!(g.player(b).unwrap().health, 93);
        assert!(
            g.events
                .iter()
                .any(|e| matches!(e, GameEvent::Fire { target: Some(t), .. } if *t == b))
        );

        let (mut g, a, b) = game_with(GameConfig::default());
        face_off(&mut g, a, b, 900.0); // 900 - 15 > 768
        let c = aim(&g, a, b, true);
        g.step(&[(a, c)]);
        assert_eq!(g.player(b).unwrap().health, 100);
    }

    #[test]
    fn wall_blocks_beam() {
        let (mut g, a, b) = game_with(GameConfig::default());
        face_off(&mut g, a, b, 400.0);
        g.world.brushes.push(Aabb::new(
            Vec3::new(-10.0, -100.0, 0.0),
            Vec3::new(10.0, 100.0, 200.0),
        ));
        let c = aim(&g, a, b, true);
        g.step(&[(a, c)]);
        assert_eq!(g.player(b).unwrap().health, 100);
    }

    #[test]
    fn vamp_heals_exactly_and_caps() {
        let (mut g, a, b) = game_with(GameConfig::default());
        face_off(&mut g, a, b, 300.0);
        g.player_mut(a).unwrap().health = 50;
        let mut events = vec![];
        for _ in 0..20 {
            let c = aim(&g, a, b, true);
            g.step(&[(a, c)]);
            events.extend(g.events.clone());
        }
        let pa = g.player(a).unwrap();
        let pb = g.player(b).unwrap();
        let dealt = pa.stats.damage_dealt;
        assert!(dealt > 0);
        assert_eq!(pb.stats.damage_taken, dealt);
        assert_eq!(pa.stats.lifesteal_earned, dealt); // 100%
        // Healed up to the cap, never above.
        assert_eq!(pa.health, (50 + dealt as i32).min(100));
        assert_eq!(pa.stats.healed as i32, pa.health - 50);
    }

    #[test]
    fn partial_lifesteal_is_exact_over_many_hits() {
        let mut cfg = GameConfig::default();
        cfg.rules.lifesteal_percent = 33;
        cfg.lg.knockback = 0.0;
        let (mut g, a, b) = game_with(cfg);
        face_off(&mut g, a, b, 300.0);
        g.player_mut(a).unwrap().health = 1;
        g.player_mut(b).unwrap().health = 10_000;
        for _ in 0..500 {
            let c = aim(&g, a, b, true);
            g.step(&[(a, c)]);
        }
        let s = g.player(a).unwrap().stats;
        assert!(s.hits > 50);
        assert_eq!(s.lifesteal_earned, s.damage_dealt * 33 / 100);
    }

    #[test]
    fn no_lifesteal_in_zero_pct() {
        let mut cfg = GameConfig::default();
        cfg.rules.lifesteal_percent = 0;
        let (mut g, a, b) = game_with(cfg);
        face_off(&mut g, a, b, 300.0);
        g.player_mut(a).unwrap().health = 50;
        for _ in 0..10 {
            let c = aim(&g, a, b, true);
            g.step(&[(a, c)]);
        }
        assert_eq!(g.player(a).unwrap().health, 50);
        assert!(g.player(b).unwrap().health < 100);
    }

    #[test]
    fn kill_and_respawn() {
        let mut cfg = GameConfig::default();
        cfg.lg.knockback = 0.0;
        let (mut g, a, b) = game_with(cfg);
        face_off(&mut g, a, b, 300.0);
        g.player_mut(b).unwrap().health = 10;
        let mut killed_at = None;
        for t in 0..20 {
            let c = aim(&g, a, b, true);
            g.step(&[(a, c)]);
            if g.events.iter().any(|e| matches!(e, GameEvent::Kill { .. })) {
                killed_at = Some(t);
                break;
            }
        }
        assert!(killed_at.is_some());
        let pb = g.player(b).unwrap();
        assert!(!pb.alive && pb.stats.deaths == 1);
        assert_eq!(g.player(a).unwrap().stats.kills, 1);
        // Dead players can't be hit; respawn after 1000 ms = 125 ticks.
        let mut respawned = None;
        for t in 0..200 {
            g.step(&[]);
            if g.events
                .iter()
                .any(|e| matches!(e, GameEvent::Respawn { player, .. } if *player == b))
            {
                respawned = Some(t);
                break;
            }
        }
        assert_eq!(respawned, Some(124));
        let pb = g.player(b).unwrap();
        assert!(pb.alive && pb.health == 100);
    }

    #[test]
    fn knockback_pushes_target_along_beam() {
        let (mut g, a, b) = game_with(GameConfig::default());
        face_off(&mut g, a, b, 300.0);
        let c = aim(&g, a, b, true);
        g.step(&[(a, c)]);
        assert!(g.player(b).unwrap().pm.velocity.x > 0.0);
    }

    #[test]
    fn armor_absorbs_two_thirds() {
        let mut cfg = GameConfig::default();
        cfg.rules.spawn_armor = 100;
        let (mut g, a, b) = game_with(cfg);
        g.damage(a, b, 30, Vec3::new(1.0, 0.0, 0.0));
        let pb = g.player(b).unwrap();
        assert_eq!((pb.health, pb.armor), (90, 80));
    }

    #[test]
    fn hitscan_prefers_nearest() {
        let w = World {
            name: "t".into(),
            brushes: vec![Aabb::new(
                Vec3::new(500.0, -50.0, -50.0),
                Vec3::new(510.0, 50.0, 50.0),
            )],
            colors: vec![[0.5; 3]],
            spawns: vec![],
        };
        let near = (
            PlayerId(1),
            Aabb::new(Vec3::new(100.0, -15.0, -15.0), Vec3::new(130.0, 15.0, 15.0)),
        );
        let far = (
            PlayerId(2),
            Aabb::new(Vec3::new(200.0, -15.0, -15.0), Vec3::new(230.0, 15.0, 15.0)),
        );
        let behind_wall = (
            PlayerId(3),
            Aabb::new(Vec3::new(600.0, -15.0, -15.0), Vec3::new(630.0, 15.0, 15.0)),
        );
        let x = Vec3::new(1.0, 0.0, 0.0);
        assert_eq!(
            hitscan(&w, Vec3::ZERO, x, 768.0, &[far, near]).target,
            Some(PlayerId(1))
        );
        assert_eq!(
            hitscan(&w, Vec3::ZERO, x, 768.0, &[behind_wall]).target,
            None
        );
        assert_eq!(hitscan(&w, Vec3::ZERO, -x, 768.0, &[near]).target, None);
    }
}
