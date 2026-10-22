//! Movement on the built-in multi-level maps (M3.4): spawns, ramps, stairs and
//! drops on arena_pillars / arena_tiers, with golden tick counts and speeds.
//!
//! Golden values were measured from this implementation at 125 Hz with default
//! config; a physics or map change that moves them must be deliberate.

use lgduel_sim::config::PhysicsConfig;
use lgduel_sim::math::{Angle, Vec3};
use lgduel_sim::pmove::{PlayerMove, pmove};
use lgduel_sim::tick::Tick;
use lgduel_sim::trace::SURFACE_EPSILON;
use lgduel_sim::usercmd::UserCmd;
use lgduel_sim::world::{World, builtin};

const DT: f32 = 0.008;
const REST_Z: f32 = 24.0 + SURFACE_EPSILON;

fn map(name: &str) -> World {
    let p = PhysicsConfig::default();
    builtin::load(
        name,
        (
            Vec3::from_array(p.player_mins),
            Vec3::from_array(p.player_maxs),
        ),
    )
    .unwrap()
}

fn walk(yaw: f64) -> UserCmd {
    UserCmd {
        tick: Tick(0),
        pitch: Angle::ZERO,
        yaw: Angle::from_degrees(yaw),
        forward: 127,
        right: 0,
        buttons: 0,
    }
}

fn step(pm: &mut PlayerMove, w: &World, c: UserCmd) {
    pmove(pm, &c, w, &PhysicsConfig::default(), DT);
}

#[test]
fn every_spawn_on_every_builtin_map_is_standable() {
    for (name, _) in builtin::ALL {
        let w = map(name);
        for (i, s) in w.spawns.iter().enumerate() {
            let mut pm = PlayerMove::at(s.origin);
            let idle = UserCmd {
                forward: 0,
                ..walk(0.0)
            };
            for _ in 0..125 {
                step(&mut pm, &w, idle);
            }
            assert!(pm.walking(), "{name} spawn {i} not on ground");
            assert!(
                (pm.origin.z - s.origin.z).abs() < 1.0,
                "{name} spawn {i}: settled at z {} (spawn z {})",
                pm.origin.z,
                s.origin.z
            );
            assert!(
                (pm.origin.x - s.origin.x).abs() < 0.01 && (pm.origin.y - s.origin.y).abs() < 0.01,
                "{name} spawn {i} slid"
            );
        }
    }
}

/// Ground → ramp → mid deck (96) → south → ramp → high deck (192) on arena_tiers.
#[test]
fn tiers_ramps_ground_to_high_deck() {
    let w = map("arena_tiers");
    let mut pm = PlayerMove::at(Vec3::new(-200.0, 0.0, REST_Z));
    let (mut mid_tick, mut high_tick) = (None, None);
    let mut min_ramp_speed = f32::MAX;
    let mut max_ramp_speed: f32 = 0.0;
    for t in 0..1000u32 {
        // Legs: east up the first ramp, south along the mid deck, east up the second ramp.
        let yaw = if pm.origin.z < 96.0 + 20.0 || (pm.origin.x < 400.0 && pm.origin.y > -700.0) {
            0.0
        } else if pm.origin.y > -704.0 && pm.origin.z < 150.0 {
            270.0
        } else {
            0.0
        };
        step(&mut pm, &w, walk(yaw));
        // Steady state on the first ramp (x 0..256), after the run-up.
        if (64.0..192.0).contains(&pm.origin.x) && pm.origin.y.abs() < 100.0 && pm.walking() {
            let s = pm.velocity.length_xy();
            min_ramp_speed = min_ramp_speed.min(s);
            max_ramp_speed = max_ramp_speed.max(s);
        }
        if mid_tick.is_none() && pm.walking() && (pm.origin.z - (96.0 + REST_Z)).abs() < 0.01 {
            mid_tick = Some(t);
        }
        if high_tick.is_none() && pm.walking() && (pm.origin.z - (192.0 + REST_Z)).abs() < 0.01 {
            high_tick = Some(t);
        }
        if high_tick.is_some_and(|h| t > h + 50) {
            break;
        }
    }
    println!(
        "GOLDEN tiers: mid deck at tick {mid_tick:?}, high deck at {high_tick:?}, ramp speed {min_ramp_speed}..{max_ramp_speed}, end {:?}",
        pm.origin
    );
    let (mid, high) = (
        mid_tick.expect("reached mid deck"),
        high_tick.expect("reached high deck"),
    );
    assert!(
        pm.walking() && pm.origin.x > 704.0 && pm.origin.y < -448.0,
        "on the high deck"
    );
    // Walking up a ~20.6° ramp keeps most of the run speed (velocity is clipped to the slope).
    assert!(
        min_ramp_speed > 280.0 && max_ramp_speed <= 320.01,
        "{min_ramp_speed}..{max_ramp_speed}"
    );
    assert_eq!((mid, high), GOLDEN_TIERS_TICKS);
}

/// Six 16u steps (step height 18) onto the mid deck without losing speed.
#[test]
fn tiers_stairs_keep_run_speed() {
    let w = map("arena_tiers");
    let mut pm = PlayerMove::at(Vec3::new(-100.0, 656.0, REST_Z));
    let mut min_speed_on_stairs = f32::MAX;
    let mut top_tick = None;
    for t in 0..250u32 {
        step(&mut pm, &w, walk(0.0));
        if (80.0..240.0).contains(&pm.origin.x) {
            min_speed_on_stairs = min_speed_on_stairs.min(pm.velocity.length_xy());
        }
        if top_tick.is_none() && pm.walking() && (pm.origin.z - (96.0 + REST_Z)).abs() < 0.01 {
            top_tick = Some(t);
        }
    }
    println!(
        "GOLDEN stairs: top at tick {top_tick:?}, min speed {min_speed_on_stairs}, end {:?}",
        pm.origin
    );
    assert!(
        pm.walking() && (pm.origin.z - (96.0 + REST_Z)).abs() < 0.01,
        "on the mid deck"
    );
    assert!(
        min_speed_on_stairs > 300.0,
        "stairs slowed to {min_speed_on_stairs}"
    );
    assert_eq!(top_tick, Some(GOLDEN_STAIRS_TICK));
}

/// Running off the high deck's west edge drops onto the mid deck and keeps going.
#[test]
fn tiers_drop_off_the_high_deck() {
    let w = map("arena_tiers");
    let mut pm = PlayerMove::at(Vec3::new(900.0, -900.0, 192.0 + REST_Z));
    let mut left_ground = false;
    for _ in 0..200 {
        step(&mut pm, &w, walk(180.0));
        left_ground |= !pm.walking();
    }
    assert!(left_ground, "walked off the edge");
    assert!(pm.walking());
    assert!(
        (pm.origin.z - (96.0 + REST_Z)).abs() < 0.01,
        "landed on the mid deck: z {}",
        pm.origin.z
    );
    assert!(pm.origin.x < 500.0, "kept running: x {}", pm.origin.x);
}

/// arena_pillars: up the west ramp onto the centre platform, across, down the east ramp.
#[test]
fn pillars_over_the_centre_platform() {
    let w = map("arena_pillars");
    let mut pm = PlayerMove::at(Vec3::new(-600.0, 0.0, REST_Z));
    let mut max_z: f32 = 0.0;
    let mut on_top_ticks = 0;
    for _ in 0..500 {
        step(&mut pm, &w, walk(0.0));
        max_z = max_z.max(pm.origin.z);
        if pm.walking() && (pm.origin.z - (64.0 + REST_Z)).abs() < 0.01 {
            on_top_ticks += 1;
        }
        if pm.origin.x > 600.0 {
            break;
        }
    }
    assert!(pm.origin.x > 600.0, "crossed: x {}", pm.origin.x);
    assert!(
        (pm.origin.z - REST_Z).abs() < 0.01,
        "back on the floor: z {}",
        pm.origin.z
    );
    // 384u of platform at ~320 u/s ≈ 150 ticks walking on top (not launched over it).
    assert!(
        on_top_ticks > 100,
        "walked on the platform for {on_top_ticks} ticks"
    );
    // Topping the ramp throws you up a little (Q3 "kickoff": upward velocity
    // into a flat ground plane leaves the ground), ~145 u/s up → ~14u hop.
    println!("GOLDEN pillars: ramp-top hop {}", max_z - (64.0 + REST_Z));
    assert!(
        max_z > 64.0 + REST_Z + 5.0 && max_z < 64.0 + REST_Z + 20.0,
        "small hop at the ramp top: max z {max_z}"
    );
}

const GOLDEN_TIERS_TICKS: (u32, u32) = (224, 677);
const GOLDEN_STAIRS_TICK: u32 = 128;
