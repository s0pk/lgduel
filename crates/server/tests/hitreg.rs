//! Hit-reg agreement (PLAN 5.3): the shooter's on-screen verdict per LG cell
//! vs. the server's, with anti-lag on and off, across latencies.

use lgduel_net::client::NetClient;
use lgduel_net::hitreg::{HitReg, HitRegStats};
use lgduel_net::interp::{InterpSettings, Interpolator};
use lgduel_net::predict::Predictor;
use lgduel_net::transport::{LinkConditions, MemNet, Transport};
use lgduel_server::{Server, ServerConfig};
use lgduel_sim::config::HitMode;
use lgduel_sim::game::{PlayerId, hitscan};
use lgduel_sim::math::{Angle, Vec3, angle_vectors, look_angles};
use lgduel_sim::tick::FixedStepper;
use lgduel_sim::usercmd::{UserCmd, buttons};
use lgduel_sim::world::{Aabb, World, builtin};

fn run(
    hit_mode: HitMode,
    one_way_ms: u64,
    jitter_ms: u64,
    loss: f64,
    aim_noise_deg: f64,
    max_rewind_ms: u32,
) -> HitRegStats {
    let net = MemNet::new(77 + one_way_ms);
    net.set_conditions(LinkConditions {
        latency_us: one_way_ms * 1000,
        jitter_us: jitter_ms * 1000,
        loss,
    });
    let mut cfg = ServerConfig::default();
    cfg.game.rules.hit_mode = hit_mode;
    cfg.game.rules.max_rewind_ms = max_rewind_ms;
    cfg.game.lg.knockback = 0.0;
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
    let mut server = Server::new(net.endpoint(27960), cfg.clone(), world.clone(), 3);
    let addr = server.transport.local_addr();
    let mut a = NetClient::new(net.endpoint(5001), addr, "shooter", 1);
    let mut b = NetClient::new(net.endpoint(5002), addr, "strafer", 2);
    let mut stepper = FixedStepper::new(125);
    let mut interp = Interpolator::new(InterpSettings::default());
    let mut pred: Option<Predictor> = None;
    let mut hitreg = HitReg::default();
    let mut placed = false;
    let mut now = 0u64;
    let mut noise_seed = 12345u64;
    let mut noise = move || {
        noise_seed ^= noise_seed << 13;
        noise_seed ^= noise_seed >> 7;
        noise_seed ^= noise_seed << 17;
        ((noise_seed >> 11) as f64 / (1u64 << 53) as f64 * 2.0 - 1.0) * aim_noise_deg
    };
    for ms in 0..15_000u64 {
        now += 1_000;
        net.advance_to(now);
        server.poll(now);
        for _ in 0..stepper.advance(1_000).1 {
            server.tick(now);
        }
        a.update(now);
        b.update(now);
        if !placed && server.client_count() == 2 {
            let z = server.game.players[0].pm.origin.z;
            server.game.players[0].pm.origin = Vec3::new(-150.0, 0.0, z);
            server.game.players[1].pm.origin = Vec3::new(150.0, 0.0, z);
            placed = true;
        }
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
        let (Some(aid), Some(bid)) = (a.player(), b.player()) else {
            continue;
        };
        let pr = pred.get_or_insert_with(|| Predictor::new(aid, world.clone(), cfg.game.clone()));
        for s in a.take_snapshots() {
            interp.push(s.tick.0, &s.players);
            hitreg.on_snapshot(aid, &s);
            if a.snapshots.latest().is_some_and(|l| l.tick == s.tick) {
                pr.on_snapshot(&s, &a.sent_cmds);
            }
        }
        interp.set_target_delay(a.recommended_interp_delay());
        let Some(clock) = a.snapshot_clock(now) else {
            continue;
        };
        let seen = interp.sample(bid, clock);
        let view = clock - interp.settings.delay_ticks;
        for t in a.advance(now) {
            let mut cmd = UserCmd {
                tick: t,
                ..UserCmd::default()
            };
            let firing = ms > 3_000 && seen.is_some_and(|s| s.alive);
            if let Some(s) = seen
                && firing
            {
                let (pitch, yaw) = look_angles(s.origin + Vec3::new(0.0, 0.0, 4.0) - pr.eye());
                let pitch = Angle::from_degrees(pitch.signed_degrees() + noise() * 0.5);
                let yaw = Angle::from_degrees(yaw.degrees() + noise());
                cmd = UserCmd {
                    pitch,
                    yaw,
                    buttons: buttons::FIRE,
                    ..cmd
                };
            }
            a.send_cmd_with_view(now, cmd, Some(view));
            pr.predict(&cmd);
            if firing && let Some(s) = seen {
                // What the shooter's screen shows for this cell.
                let mins = Vec3::from_array(cfg.game.physics.player_mins);
                let maxs = Vec3::from_array(cfg.game.physics.player_maxs);
                let target = [(bid, Aabb::at(s.origin, mins, maxs))];
                let (f, _, _) = angle_vectors(cmd.pitch, cmd.yaw);
                let hs = hitscan(
                    &world,
                    pr.state.eye(&cfg.game.physics),
                    f,
                    cfg.game.lg.range,
                    &target,
                );
                hitreg.record(t.0, hs.target == Some(PlayerId(bid.0)));
            }
        }
    }
    hitreg.stats
}

#[test]
fn hitreg_agreement_anti_lag_vs_no_anti_lag() {
    // Total rewind a command needs ≈ RTT + input lead margin + interpolation delay;
    // with the default 200 ms cap that covers RTT up to ~140 ms. The 150±40 ms case
    // is run with the default cap (expected to fall short) and with a 300 ms cap.
    let mut table = vec![];
    for (name, ow, jit, loss, cap) in [
        ("50 ms", 25, 0, 0.0, 200),
        ("100 ms", 50, 0, 0.0, 200),
        ("150±40 ms 2%, cap 200", 75, 20, 0.02, 200),
        ("150±40 ms 2%, cap 300", 75, 20, 0.02, 300),
    ] {
        // Noisy aim (±4° yaw ≈ ±21u at 300u vs a 15u half-width) so the shooter's
        // screen shows both hits and misses.
        let on = run(HitMode::AntiLag, ow, jit, loss, 4.0, cap);
        let off = run(HitMode::NoAntiLag, ow, jit, loss, 4.0, cap);
        println!(
            "RTT {name}: anti-lag agreement {:.1}% ({} shots, false hit {}, false miss {}) | no-anti-lag {:.1}% ({} shots, false hit {}, false miss {})",
            on.agreement_pct(),
            on.shots(),
            on.false_hit,
            on.false_miss,
            off.agreement_pct(),
            off.shots(),
            off.false_hit,
            off.false_miss
        );
        table.push((name, cap, ow, on, off));
    }
    for (name, cap, ow, on, off) in &table {
        // Lossy links compare fewer shots: events lost with dropped snapshots (KI-10).
        assert!(on.shots() > 100, "{name}: {on:?}");
        assert!(
            off.agreement_pct() < on.agreement_pct(),
            "{name}: no-anti-lag should agree less"
        );
        let within_cap = 2 * ow + 2 * 20 + 60 <= *cap as u64;
        if within_cap {
            assert!(
                on.agreement_pct() >= 97.0,
                "{name}: anti-lag agreement {:.1}%",
                on.agreement_pct()
            );
        }
    }
    let capped = &table[2].3;
    let uncapped = &table[3].3;
    assert!(
        uncapped.agreement_pct() > capped.agreement_pct(),
        "raising the cap must restore agreement"
    );
}
