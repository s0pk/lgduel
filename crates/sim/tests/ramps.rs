//! Movement on sloped convex brushes (M3.1).

use lgduel_sim::brush::{ConvexBrush, Rise};
use lgduel_sim::config::PhysicsConfig;
use lgduel_sim::math::{Angle, Vec3};
use lgduel_sim::pmove::{PlayerMove, pmove};
use lgduel_sim::tick::Tick;
use lgduel_sim::trace::SURFACE_EPSILON;
use lgduel_sim::usercmd::{UserCmd, buttons};
use lgduel_sim::world::{Aabb, World};

const DT: f32 = 0.008;
const REST_Z: f32 = 24.0 + SURFACE_EPSILON;

/// Floor, a ramp rising +x from x=0 to x=`run` with angle `deg`, and a long
/// platform at the ramp's top height from x=run to x=1500.
fn ramp_world(deg: f32, run: f32) -> (World, f32) {
    let top = run * deg.to_radians().tan();
    let floor = Aabb::new(
        Vec3::new(-2000.0, -2000.0, -32.0),
        Vec3::new(2000.0, 2000.0, 0.0),
    );
    let platform = Aabb::new(Vec3::new(run, -200.0, 0.0), Vec3::new(1500.0, 200.0, top));
    let ramp = ConvexBrush::ramp(
        Vec3::new(0.0, -200.0, 0.0),
        Vec3::new(run, 200.0, top),
        Rise::PosX,
        [1.0; 3],
    )
    .unwrap();
    let w = World {
        name: "ramps".into(),
        brushes: vec![floor, platform],
        colors: vec![[0.5; 3]; 2],
        convex: vec![ramp],
        spawns: vec![],
    };
    (w, top)
}

fn cmd(forward: i8, yaw: f64, buttons: u8) -> UserCmd {
    UserCmd {
        tick: Tick(0),
        pitch: Angle::ZERO,
        yaw: Angle::from_degrees(yaw),
        forward,
        right: 0,
        buttons,
    }
}

fn step(pm: &mut PlayerMove, w: &World, c: UserCmd) {
    pmove(pm, &c, w, &PhysicsConfig::default(), DT);
}

#[test]
fn box_lands_on_slope_at_the_right_height() {
    let (w, _) = ramp_world(30.0, 160.0);
    let mins = Vec3::new(-15.0, -15.0, -24.0);
    let maxs = Vec3::new(15.0, 15.0, 32.0);
    let x = 80.0;
    let t = w.trace(
        Vec3::new(x, 0.0, 300.0),
        Vec3::new(x, 0.0, -100.0),
        mins,
        maxs,
    );
    assert!(t.hit() && !t.start_solid);
    let tan = 30f32.to_radians().tan();
    // The box touches the slope with its higher-side bottom corner (x + 15, z - 24).
    let expected = (x + 15.0) * tan + 24.0;
    assert!(
        (t.end.z - expected).abs() < 0.3,
        "z {} vs {expected}",
        t.end.z
    );
    assert!(
        (t.normal.z - 30f32.to_radians().cos()).abs() < 1e-4,
        "{:?}",
        t.normal
    );
    assert!(t.normal.x < 0.0);
}

#[test]
fn walks_up_a_30_degree_ramp_onto_the_platform() {
    let (w, top) = ramp_world(30.0, 160.0);
    let mut pm = PlayerMove::at(Vec3::new(-200.0, 0.0, REST_Z));
    let mut on_ramp_walking = 0;
    for _ in 0..375 {
        step(&mut pm, &w, cmd(127, 0.0, 0));
        if pm.origin.x > 20.0 && pm.origin.x < 140.0 && pm.walking() {
            on_ramp_walking += 1;
        }
    }
    assert!(
        pm.origin.x > 260.0,
        "reached the platform: x {}",
        pm.origin.x
    );
    assert!(
        (pm.origin.z - (top + REST_Z)).abs() < 0.5,
        "on top: z {} (top {})",
        pm.origin.z,
        top + REST_Z
    );
    assert!(pm.walking());
    assert!(
        on_ramp_walking > 20,
        "walked (not hopped) up the ramp: {on_ramp_walking} ticks"
    );
}

#[test]
fn walks_back_down_without_sticking() {
    let (w, top) = ramp_world(30.0, 160.0);
    let mut pm = PlayerMove::at(Vec3::new(300.0, 0.0, top + REST_Z));
    for _ in 0..375 {
        step(&mut pm, &w, cmd(127, 180.0, 0));
    }
    assert!(pm.origin.x < -150.0, "came down: x {}", pm.origin.x);
    assert!(
        (pm.origin.z - REST_Z).abs() < 0.5,
        "on the floor: z {}",
        pm.origin.z
    );
}

#[test]
fn cannot_walk_up_a_55_degree_slope() {
    let (w, top) = ramp_world(55.0, 100.0);
    let mut pm = PlayerMove::at(Vec3::new(-150.0, 0.0, REST_Z));
    let mut max_z: f32 = 0.0;
    for _ in 0..500 {
        step(&mut pm, &w, cmd(127, 0.0, 0));
        max_z = max_z.max(pm.origin.z);
    }
    assert!(max_z < top * 0.5, "climbed {max_z} of {top}");
}

#[test]
fn standing_at_the_seam_does_not_sink() {
    let (w, _) = ramp_world(30.0, 160.0);
    for x in [-14.0, -1.0, 0.0, 1.0, 14.0, 30.0] {
        let mut pm = PlayerMove::at(Vec3::new(x, 0.0, 120.0));
        for _ in 0..250 {
            step(&mut pm, &w, cmd(0, 0.0, 0));
        }
        assert!(pm.walking(), "x {x}");
        assert!(
            pm.origin.z >= REST_Z - 0.01,
            "x {x}: sank to {}",
            pm.origin.z
        );
    }
}

#[test]
fn jump_off_the_top() {
    let (w, top) = ramp_world(30.0, 160.0);
    let mut pm = PlayerMove::at(Vec3::new(-200.0, 0.0, REST_Z));
    let mut max_z: f32 = 0.0;
    let mut jumped = false;
    for _ in 0..500 {
        // Jump once, right at the top edge of the ramp.
        let jump = if !jumped && pm.walking() && pm.origin.x > 145.0 {
            jumped = true;
            buttons::JUMP
        } else {
            0
        };
        step(&mut pm, &w, cmd(127, 0.0, jump));
        max_z = max_z.max(pm.origin.z);
    }
    assert!(jumped);
    assert!(
        max_z > top + REST_Z + 30.0,
        "jumped above the platform: {max_z}"
    );
    assert!(pm.walking());
}
