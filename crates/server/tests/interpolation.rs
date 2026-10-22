//! Remote-player interpolation over a jittery link (PLAN 4.7): client A watches
//! client B walk; A's rendered view of B must be smooth and rarely starved.

use lgduel_net::client::NetClient;
use lgduel_net::interp::{InterpSettings, Interpolator};
use lgduel_net::transport::{LinkConditions, MemNet, Transport};
use lgduel_server::{Server, ServerConfig};
use lgduel_sim::math::{Angle, Vec3};
use lgduel_sim::tick::FixedStepper;
use lgduel_sim::usercmd::UserCmd;
use lgduel_sim::world::{World, builtin};

fn run(c: LinkConditions, delay_ticks: f64) -> (f64, f32, u64) {
    let net = MemNet::new(17);
    net.set_conditions(c);
    let cfg = ServerConfig::default();
    let p = &cfg.game.physics;
    let world = World::from_toml_str(
        builtin::ARENA_FLAT,
        (
            Vec3::from_array(p.player_mins),
            Vec3::from_array(p.player_maxs),
        ),
    )
    .unwrap();
    let mut server = Server::new(net.endpoint(27960), cfg, world, 1);
    let addr = server.transport.local_addr();
    let mut a = NetClient::new(net.endpoint(5001), addr, "a", 1);
    let mut b = NetClient::new(net.endpoint(5002), addr, "b", 2);
    let mut stepper = FixedStepper::new(125);
    let mut interp = Interpolator::new(InterpSettings {
        delay_ticks,
        ..Default::default()
    });
    let mut now = 0u64;
    let mut prev: Option<Vec3> = None;
    let (mut max_step, mut frames) = (0.0f32, 0u64);
    for ms in 0..12_000u64 {
        now += 1_000;
        net.advance_to(now);
        server.poll(now);
        for _ in 0..stepper.advance(1_000).1 {
            server.tick(now);
        }
        a.update(now);
        b.update(now);
        for t in a.advance(now) {
            a.send_cmd(
                now,
                UserCmd {
                    tick: t,
                    ..UserCmd::default()
                },
            );
        }
        // B walks back and forth along x: 4 s each way (stays well inside the arena).
        let fwd = if (ms / 4_000).is_multiple_of(2) {
            127
        } else {
            -127
        };
        for t in b.advance(now) {
            b.send_cmd(
                now,
                UserCmd {
                    tick: t,
                    forward: fwd,
                    yaw: Angle::ZERO,
                    ..UserCmd::default()
                },
            );
        }
        for s in a.take_snapshots() {
            interp.push(s.tick.0, &s.players);
        }
        let (Some(bid), Some(st)) = (b.player(), a.snapshot_clock(now)) else {
            continue;
        };
        if ms < 2_000 {
            continue; // warm-up
        }
        interp.set_target_delay(a.recommended_interp_delay());
        if let Some(v) = interp.sample(bid, st) {
            if let Some(p) = prev {
                let step = (v.origin - p).length();
                max_step = max_step.max(step);
            }
            prev = Some(v.origin);
            frames += 1;
        }
    }
    let starved = interp.stats.starved as f64 / interp.stats.samples.max(1) as f64;
    println!(
        "  final interp delay {:.2} ticks (recommended {:.2})",
        interp.settings.delay_ticks,
        a.recommended_interp_delay()
    );
    (starved, max_step, frames)
}

#[test]
fn smooth_remote_motion_under_jitter() {
    // (name, link, initial delay, max per-1ms-frame step, max starved fraction)
    // Walking = 0.32 u per 1 ms frame. Clean links: no visible catch-up at all (< ~2×).
    // Lossy + jittery: a lost newest snapshot during a jitter spike may cause a
    // single short hold; allow a catch-up of ≤ 5 ms of motion and < 0.1% starved.
    let cases = [
        (
            "50 ms",
            LinkConditions {
                latency_us: 50_000,
                ..Default::default()
            },
            3.0,
            0.7,
            0.001,
        ),
        (
            "50±10 ms",
            LinkConditions {
                latency_us: 50_000,
                jitter_us: 10_000,
                loss: 0.0,
            },
            3.0,
            0.7,
            0.001,
        ),
        (
            "100±20 ms 2% loss",
            LinkConditions {
                latency_us: 100_000,
                jitter_us: 20_000,
                loss: 0.02,
            },
            5.0,
            1.6,
            0.001,
        ),
    ];
    for (name, c, delay, max_allowed, starve_allowed) in cases {
        let (starved, max_step, frames) = run(c, delay);
        println!(
            "{name} (start delay {delay} ticks): starved {:.2}% of samples, max per-ms step {max_step:.3}u over {frames} frames",
            starved * 100.0
        );
        assert!(frames > 9_000, "{name}");
        assert!(
            max_step < max_allowed,
            "{name}: jerky motion, max step {max_step}"
        );
        assert!(
            starved < starve_allowed,
            "{name}: buffer ran dry {:.2}%",
            starved * 100.0
        );
    }
}
