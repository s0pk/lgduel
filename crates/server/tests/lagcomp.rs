//! Lag compensation (PLAN 4.8). Client A aims, every tick, exactly at where its
//! own interpolation shows client B (a "perfect aimer on his own screen").
//! With anti-lag the server tests the beam against B where A saw it, so A's
//! server-confirmed accuracy is ~100%. Without anti-lag the server uses B's
//! current position, which is (latency + interpolation delay) ahead, so a
//! strafing B is mostly missed. Rewinds beyond `max_rewind_ms` are clamped.

use lgduel_net::client::NetClient;
use lgduel_net::interp::{InterpSettings, Interpolator};
use lgduel_net::transport::{LinkConditions, MemNet, Transport};
use lgduel_server::{Server, ServerConfig};
use lgduel_sim::config::HitMode;
use lgduel_sim::math::{Angle, Vec3, look_angles};
use lgduel_sim::tick::FixedStepper;
use lgduel_sim::usercmd::{UserCmd, buttons};
use lgduel_sim::world::{World, builtin};

struct Outcome {
    accuracy: f32,
    shots: u32,
    mean_rewind_ms: f64,
    clamped: u64,
}

fn duel(hit_mode: HitMode, latency_us: u64, jitter_us: u64) -> Outcome {
    let net = MemNet::new(31);
    net.set_conditions(LinkConditions {
        latency_us,
        jitter_us,
        loss: 0.0,
    });
    let mut cfg = ServerConfig::default();
    cfg.game.rules.hit_mode = hit_mode;
    cfg.game.lg.knockback = 0.0;
    // Huge health so nobody dies: we measure pure hit-reg.
    cfg.game.rules.spawn_health = 100_000;
    cfg.game.rules.max_health = 100_000;
    let p = &cfg.game.physics;
    let world = World::from_toml_str(
        builtin::ARENA_FLAT,
        (
            Vec3::from_array(p.player_mins),
            Vec3::from_array(p.player_maxs),
        ),
    )
    .unwrap();
    let view_height = cfg.game.physics.view_height;
    let mut server = Server::new(net.endpoint(27960), cfg, world, 3);
    let addr = server.transport.local_addr();
    let mut a = NetClient::new(net.endpoint(5001), addr, "shooter", 1);
    let mut b = NetClient::new(net.endpoint(5002), addr, "strafer", 2);
    let mut stepper = FixedStepper::new(125);
    let mut interp = Interpolator::new(InterpSettings::default());
    let mut now = 0u64;
    let mut placed = false;
    let mut shots_at_start = None;
    for ms in 0..14_000u64 {
        now += 1_000;
        net.advance_to(now);
        server.poll(now);
        for _ in 0..stepper.advance(1_000).1 {
            server.tick(now);
        }
        a.update(now);
        b.update(now);
        // Once both are in, put them 300u apart facing each other.
        if !placed && server.client_count() == 2 {
            let z = server.game.players[0].pm.origin.z;
            server.game.players[0].pm.origin = Vec3::new(-150.0, 0.0, z);
            server.game.players[1].pm.origin = Vec3::new(150.0, 0.0, z);
            placed = true;
        }
        for s in a.take_snapshots() {
            interp.push(s.tick.0, &s.players);
        }
        interp.set_target_delay(a.recommended_interp_delay());
        // B: A/D strafes every 400 ms, never shoots.
        let right = if (ms / 400).is_multiple_of(2) {
            127
        } else {
            -127
        };
        for t in b.advance(now) {
            b.send_cmd(
                now,
                UserCmd {
                    tick: t,
                    right,
                    yaw: Angle::from_degrees(180.0),
                    ..UserCmd::default()
                },
            );
        }
        // A: aim at B as rendered right now; report that render time as the view.
        let (Some(aid), Some(bid)) = (a.player(), b.player()) else {
            continue;
        };
        let Some(clock) = a.snapshot_clock(now) else {
            continue;
        };
        let ticks = a.advance(now);
        let render_t = clock - interp.settings.delay_ticks;
        let seen_b = interp.sample(bid, clock);
        let me = a.snapshots.latest().and_then(|s| s.player(aid).cloned());
        for t in ticks {
            let mut cmd = UserCmd {
                tick: t,
                ..UserCmd::default()
            };
            if let (Some(seen), Some(me)) = (seen_b, &me)
                && ms > 3_000
            {
                let eye = me.pm.origin + Vec3::new(0.0, 0.0, view_height);
                let (pitch, yaw) = look_angles(seen.origin + Vec3::new(0.0, 0.0, 4.0) - eye);
                cmd = UserCmd {
                    pitch,
                    yaw,
                    buttons: buttons::FIRE,
                    ..cmd
                };
            }
            a.send_cmd_with_view(now, cmd, Some(render_t));
        }
        if ms == 3_000 {
            shots_at_start = server
                .game
                .player(aid)
                .map(|p| (p.stats.shots, p.stats.hits));
        }
    }
    let aid = a.player().unwrap();
    let st = server.game.player(aid).unwrap().stats;
    let (s0, h0) = shots_at_start.unwrap();
    let shots = st.shots - s0;
    let hits = st.hits - h0;
    let rs = server.rewind_stats;
    Outcome {
        accuracy: hits as f32 * 100.0 / shots.max(1) as f32,
        shots,
        mean_rewind_ms: if rs.shots_rewound == 0 {
            0.0
        } else {
            rs.sum_rewind_ticks / rs.shots_rewound as f64 * 8.0
        },
        clamped: rs.clamped,
    }
}

#[test]
fn anti_lag_lets_a_perfect_aimer_hit_and_no_anti_lag_does_not() {
    let on = duel(HitMode::AntiLag, 50_000, 0);
    let off = duel(HitMode::NoAntiLag, 50_000, 0);
    println!(
        "100 ms RTT: anti-lag {:.1}% ({} shots, mean rewind {:.0} ms) | no-anti-lag {:.1}% ({} shots)",
        on.accuracy, on.shots, on.mean_rewind_ms, off.accuracy, off.shots
    );
    assert!(on.shots > 150 && off.shots > 150);
    assert!(on.accuracy > 95.0, "anti-lag: {:.1}%", on.accuracy);
    assert!(
        off.accuracy < 75.0,
        "no-anti-lag should miss a strafer: {:.1}%",
        off.accuracy
    );
    assert!(
        on.mean_rewind_ms > 50.0 && on.mean_rewind_ms < 150.0,
        "{}",
        on.mean_rewind_ms
    );
    assert_eq!(on.clamped, 0);
}

#[test]
fn anti_lag_works_with_jitter() {
    let on = duel(HitMode::AntiLag, 50_000, 10_000);
    println!(
        "100±20 ms RTT: anti-lag {:.1}% ({} shots, mean rewind {:.0} ms)",
        on.accuracy, on.shots, on.mean_rewind_ms
    );
    assert!(on.accuracy > 90.0, "{:.1}%", on.accuracy);
}

#[test]
fn rewind_is_capped_at_max_rewind() {
    // 300 ms one way: the view is ~300+ ms old, beyond the 200 ms cap.
    let on = duel(HitMode::AntiLag, 300_000, 0);
    println!(
        "600 ms RTT: anti-lag {:.1}% ({} shots, mean rewind {:.0} ms, clamped {})",
        on.accuracy, on.shots, on.mean_rewind_ms, on.clamped
    );
    assert!(on.clamped > 0);
    assert!(on.mean_rewind_ms <= 200.0 + 1e-6);
    assert!(
        on.accuracy < 90.0,
        "capped rewind must not fully compensate 600 ms: {:.1}%",
        on.accuracy
    );
}
