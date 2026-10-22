//! Golden movement tests (PLAN 2.9) and strafe-aim relative speed ratios (PLAN 2.10).
//!
//! Golden values were measured from this implementation at 125 Hz with default
//! config and pinned; any physics change that moves them must be deliberate
//! (update the numbers and note it in STATE/DECISIONS.md).

use lgduel_sim::config::PhysicsConfig;
use lgduel_sim::math::{Angle, Vec3};
use lgduel_sim::pmove::{PlayerMove, pmove};
use lgduel_sim::tick::Tick;
use lgduel_sim::trace::SURFACE_EPSILON;
use lgduel_sim::usercmd::{UserCmd, buttons};
use lgduel_sim::world::{Aabb, World};

const DT: f32 = 0.008;
const REST_Z: f32 = 24.0 + SURFACE_EPSILON;

fn flat_world() -> World {
    World {
        convex: Vec::new(),
        name: "golden".into(),
        brushes: vec![Aabb::new(
            Vec3::new(-100_000.0, -100_000.0, -32.0),
            Vec3::new(100_000.0, 100_000.0, 0.0),
        )],
        colors: vec![[0.5; 3]],
        spawns: vec![],
    }
}

fn cmd(yaw_deg: f64, forward: i8, right: i8, buttons: u8) -> UserCmd {
    UserCmd {
        tick: Tick(0),
        pitch: Angle::ZERO,
        yaw: Angle::from_degrees(yaw_deg),
        forward,
        right,
        buttons,
    }
}

fn step(pm: &mut PlayerMove, w: &World, c: UserCmd) {
    pmove(pm, &c, w, &PhysicsConfig::default(), DT);
}

fn yaw_of(v: Vec3) -> f64 {
    (v.y as f64).atan2(v.x as f64).to_degrees()
}

/// Yaw that puts the forward+right wish direction (yaw - 45°) at the angle that
/// maximises speed gain for the given accel: cos θ = (wish - accel·dt·wish) / speed.
fn optimal_strafe_yaw(v: Vec3, accel: f32) -> f64 {
    let speed = v.length_xy() as f64;
    let wish = 320.0;
    let add = accel as f64 * DT as f64 * wish;
    let c = ((wish - add) / speed).clamp(-1.0, 1.0);
    // Wish dir rotated clockwise (to the right) of velocity by θ; yaw = wish + 45°.
    yaw_of(v) - c.acos().to_degrees() + 45.0
}

#[test]
fn standing_jump_apex_and_airtime() {
    let w = flat_world();
    let mut pm = PlayerMove::at(Vec3::new(0.0, 0.0, REST_Z));
    step(&mut pm, &w, cmd(0.0, 0, 0, buttons::JUMP));
    let mut apex = pm.origin.z;
    let mut ticks = 1;
    while !pm.on_ground() {
        step(&mut pm, &w, cmd(0.0, 0, 0, 0));
        apex = apex.max(pm.origin.z);
        ticks += 1;
        assert!(ticks < 1000);
    }
    let height = apex - REST_Z;
    println!("GOLDEN jump: apex {height} airtime {ticks} ticks");
    // Ideal parabola for 275 u/s at 800 u/s² is 47.27u / 0.6875 s (86 ticks);
    // velocity snapping (Q3-style) makes it a little higher and longer.
    assert!((47.0..52.0).contains(&height), "apex {height}");
    assert_eq!(height, GOLDEN_JUMP_APEX);
    assert_eq!(ticks, GOLDEN_JUMP_TICKS);
}

#[test]
fn ground_accel_time_to_max_speed() {
    let w = flat_world();
    let mut pm = PlayerMove::at(Vec3::new(0.0, 0.0, REST_Z));
    let mut ticks = 0;
    while pm.velocity.length() < 320.0 {
        step(&mut pm, &w, cmd(0.0, 127, 0, 0));
        ticks += 1;
        assert!(ticks < 1000);
    }
    println!("GOLDEN accel: {ticks} ticks to 320");
    assert_eq!(ticks, GOLDEN_ACCEL_TICKS);
}

#[test]
fn strafe_jumping_gains_speed() {
    let w = flat_world();
    let mut pm = PlayerMove::at(Vec3::new(0.0, 0.0, REST_Z));
    // Run up to 320 straight along +x.
    for _ in 0..125 {
        step(&mut pm, &w, cmd(0.0, 127, 0, 0));
    }
    let air_accel = PhysicsConfig::default().air_accelerate;
    let mut hop_speeds = vec![];
    let mut was_air = false;
    for _ in 0..(125 * 6) {
        let yaw = optimal_strafe_yaw(pm.velocity, air_accel);
        step(&mut pm, &w, cmd(yaw, 127, 127, buttons::JUMP));
        if pm.on_ground() && was_air {
            hop_speeds.push(pm.velocity.length_xy());
        }
        was_air = !pm.on_ground();
    }
    let final_speed = pm.velocity.length_xy();
    println!("GOLDEN strafe-jump: hops {hop_speeds:?} final {final_speed}");
    assert!(
        final_speed > 400.0,
        "6 s of strafe jumping should exceed 400 u/s"
    );
    assert!(
        hop_speeds.windows(2).all(|p| p[1] >= p[0]),
        "speed must not drop between hops"
    );
    assert_eq!(final_speed, GOLDEN_STRAFE_SPEED);
}

#[test]
fn circle_jump_beats_run_speed() {
    let w = flat_world();
    let mut pm = PlayerMove::at(Vec3::new(0.0, 0.0, REST_Z));
    for _ in 0..125 {
        step(&mut pm, &w, cmd(0.0, 127, 0, 0));
    }
    assert_eq!(pm.velocity.length_xy(), 320.0);
    let ground_accel = PhysicsConfig::default().accelerate;
    // ~100 ms of turning on the ground at the optimal ground-strafe angle, then jump.
    for _ in 0..12 {
        let yaw = optimal_strafe_yaw(pm.velocity, ground_accel);
        step(&mut pm, &w, cmd(yaw, 127, 127, 0));
    }
    let yaw = optimal_strafe_yaw(pm.velocity, ground_accel);
    step(&mut pm, &w, cmd(yaw, 127, 127, buttons::JUMP));
    assert!(!pm.on_ground());
    let takeoff = pm.velocity.length_xy();
    println!("GOLDEN circle-jump: takeoff {takeoff}");
    assert!(takeoff > 340.0, "circle jump takeoff {takeoff}");
    assert_eq!(takeoff, GOLDEN_CIRCLE_SPEED);
}

/// Steady-state ground velocity for a key combination while facing `yaw_deg`.
fn steady_velocity(yaw_deg: f64, forward: i8, right: i8) -> Vec3 {
    let w = flat_world();
    let mut pm = PlayerMove::at(Vec3::new(0.0, 0.0, REST_Z));
    for _ in 0..250 {
        step(&mut pm, &w, cmd(yaw_deg, forward, right, 0));
    }
    pm.velocity
}

/// Strafe-aim forms (x76.gg strafe-aim guide): relative lateral speed between
/// two duelists, in units of one player's speed v. Shooter at the origin faces
/// +x toward the target; the target faces -x back at the shooter. "Lateral" is
/// the y component (perpendicular to the line of sight).
#[test]
fn strafe_aim_relative_speed_ratios() {
    let v = 320.0f32;
    // Target strafes pure sideways to its right (= +y, since it faces -x).
    let target = steady_velocity(180.0, 0, 127);
    assert!(
        (target.y - v).abs() <= 1.0 && target.x.abs() <= 1.0,
        "{target:?}"
    );

    // Shooter options (faces +x, so its right is -y, left is +y).
    let mirror = steady_velocity(0.0, 0, -127); // left: same world direction
    let anti = steady_velocity(0.0, 0, 127); // right: opposite
    let hsw_mirror = steady_velocity(0.0, 127, -127); // forward-left diagonal
    let hsw_anti = steady_velocity(0.0, 127, 127); // forward-right diagonal

    let rel = |shooter: Vec3| ((target.y - shooter.y) / v).abs();
    let cases = [
        ("mirroring", rel(mirror), 0.0),
        ("anti-mirroring", rel(anti), 2.0),
        (
            "HSW mirroring",
            rel(hsw_mirror),
            1.0 - std::f32::consts::FRAC_1_SQRT_2,
        ),
        (
            "HSW anti-mirroring",
            rel(hsw_anti),
            1.0 + std::f32::consts::FRAC_1_SQRT_2,
        ),
    ];
    for (name, got, want) in cases {
        println!("RATIO {name}: {got:.4}v (want {want:.4}v)");
        assert!((got - want).abs() < 0.01, "{name}: {got} vs {want}");
    }
    // Wiki's rounded figures.
    assert_eq!(format!("{:.2}", cases[2].1), "0.29");
    assert_eq!(format!("{:.2}", cases[3].1), "1.71");
}

// Pinned golden values (measured, see PROGRESS.md iteration 9).
const GOLDEN_JUMP_APEX: f32 = 50.342407;
const GOLDEN_JUMP_TICKS: u32 = 92;
const GOLDEN_ACCEL_TICKS: u32 = 19;
const GOLDEN_STRAFE_SPEED: f32 = 1145.7408;
const GOLDEN_CIRCLE_SPEED: f32 = 378.34375;
