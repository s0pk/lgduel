//! Determinism (PLAN 2.14): a 60 s vamp duel driven by a seeded input
//! generator is recorded, then the recorded input log is replayed on a fresh
//! game. Final state hashes must be bit-identical, and must equal the pinned
//! golden hash (catches cross-platform float drift: the same hash must come out
//! on Windows and Linux).

use lgduel_sim::config::GameConfig;
use lgduel_sim::game::{Game, GameEvent, PlayerId};
use lgduel_sim::math::{Angle, Vec3, look_angles};
use lgduel_sim::usercmd::{UserCmd, buttons};
use lgduel_sim::world::{World, builtin};

const TICKS: u32 = 125 * 60;

/// Pinned final state hash for the fixed seeds below over 60 s. If you change sim
/// behaviour on purpose, update this and note it in PROGRESS.md.
const GOLDEN_HASH: u64 = 0x9e74_a8ea_7918_ee78;

struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u32 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (self.0 >> 33) as u32
    }
    fn range(&mut self, lo: u32, hi: u32) -> u32 {
        lo + self.next() % (hi - lo)
    }
}

/// Bot-ish input: random A/D strafes with random durations, occasional jumps,
/// range keeping, aim at the opponent with per-player jitter, always firing.
struct Driver {
    rng: Lcg,
    jitter_units: u32,
    strafe: i8,
    hold: u32,
    jump: bool,
}

impl Driver {
    fn cmd(&mut self, g: &Game, me: PlayerId, foe: PlayerId) -> UserCmd {
        if self.hold == 0 {
            self.strafe = if self.rng.next().is_multiple_of(2) {
                127
            } else {
                -127
            };
            self.hold = self.rng.range(19, 75);
            self.jump = self.rng.next().is_multiple_of(10);
        }
        self.hold -= 1;
        let p = g.player(me).unwrap();
        let f = g.player(foe).unwrap();
        let eye = p.pm.eye(&g.cfg.physics);
        let target = f.pm.origin + Vec3::new(0.0, 0.0, 4.0);
        let (pitch, yaw) = look_angles(target - eye);
        // Keep mid range: close in when far, back off when too near.
        let dist = (f.pm.origin - p.pm.origin).length_xy();
        let forward = if dist > 450.0 {
            127
        } else if dist < 200.0 {
            -127
        } else {
            0
        };
        let j = self.jitter_units;
        let jy = self.rng.range(0, 2 * j + 1) as i64 - j as i64;
        let jp = self.rng.range(0, 2 * j + 1) as i64 - j as i64;
        UserCmd {
            tick: g.tick,
            pitch: Angle((pitch.0 as i64 + jp) as u32),
            yaw: Angle((yaw.0 as i64 + jy) as u32),
            forward,
            right: self.strafe,
            buttons: buttons::FIRE | if self.jump { buttons::JUMP } else { 0 },
        }
    }
}

fn new_game() -> (Game, PlayerId, PlayerId) {
    let cfg = GameConfig::default();
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

fn deg_units(deg: f64) -> u32 {
    Angle::from_degrees(deg).0
}

#[test]
fn recorded_inputs_replay_bit_identically() {
    // Run 1: generate + record.
    let (mut g, a, b) = new_game();
    let mut da = Driver {
        rng: Lcg(0x1_6D0E1),
        jitter_units: deg_units(1.5),
        strafe: 0,
        hold: 0,
        jump: false,
    };
    let mut db = Driver {
        rng: Lcg(0x2_6D0E1),
        jitter_units: deg_units(4.0),
        strafe: 0,
        hold: 0,
        jump: false,
    };
    let mut log: Vec<[(PlayerId, UserCmd); 2]> = Vec::with_capacity(TICKS as usize);
    let (mut damage_events, mut kills) = (0, 0);
    for _ in 0..TICKS {
        let cmds = [(a, da.cmd(&g, a, b)), (b, db.cmd(&g, b, a))];
        g.step(&cmds);
        log.push(cmds);
        for e in &g.events {
            match e {
                GameEvent::Damage { .. } => damage_events += 1,
                GameEvent::Kill { .. } => kills += 1,
                _ => {}
            }
        }
    }
    let h1 = g.state_hash();

    // Run 2: replay the log on a fresh game.
    let (mut g2, _, _) = new_game();
    for cmds in &log {
        g2.step(cmds);
    }
    let h2 = g2.state_hash();

    let sa = g.player(a).unwrap().stats;
    let sb = g.player(b).unwrap().stats;
    println!(
        "DETERMINISM hash {h1:#018x} | damage events {damage_events}, kills {kills} | \
         acc a {:.1}% b {:.1}% | fights a {}W/{}L",
        sa.accuracy_pct(),
        sb.accuracy_pct(),
        sa.fights_won,
        sa.fights_lost
    );
    assert_eq!(h1, h2, "replay diverged");
    assert!(damage_events > 100, "the duel should involve real combat");
    assert_eq!(
        h1, GOLDEN_HASH,
        "golden hash changed (platform drift or sim change)"
    );
}
