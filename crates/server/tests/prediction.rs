//! Prediction accuracy (PLAN 4.6 / 4.11): the client's predicted origin for
//! each command tick vs. the server's authoritative origin for the same tick.

use lgduel_net::client::NetClient;
use lgduel_net::predict::Predictor;
use lgduel_net::transport::{LinkConditions, MemNet, Transport};
use lgduel_server::{Server, ServerConfig};
use lgduel_sim::math::{Angle, Vec3};
use lgduel_sim::tick::FixedStepper;
use lgduel_sim::usercmd::{UserCmd, buttons};
use lgduel_sim::world::{World, builtin};

fn world(cfg: &ServerConfig) -> World {
    let p = &cfg.game.physics;
    World::from_toml_str(
        builtin::ARENA_FLAT,
        (
            Vec3::from_array(p.player_mins),
            Vec3::from_array(p.player_maxs),
        ),
    )
    .unwrap()
}

/// Busy input: A/D strafes, jumps, crouches, turning.
fn script(tick: u32) -> UserCmd {
    let phase = tick / 37;
    UserCmd {
        tick: lgduel_sim::tick::Tick(tick),
        pitch: Angle::from_degrees(((tick % 200) as f64 - 100.0) * 0.2),
        yaw: Angle::from_degrees(tick as f64 * 1.3),
        forward: if phase.is_multiple_of(3) { 127 } else { 0 },
        right: if phase.is_multiple_of(2) { 127 } else { -127 },
        buttons: if tick % 90 < 3 { buttons::JUMP } else { 0 }
            | if phase % 7 == 3 { buttons::CROUCH } else { 0 },
    }
}

/// Run a session and return (prediction stats, server missing-cmd count).
fn run(c: LinkConditions, secs: u64) -> (lgduel_net::predict::PredictionStats, u64) {
    let net = MemNet::new(5);
    net.set_conditions(c);
    let cfg = ServerConfig::default();
    let mut server = Server::new(net.endpoint(27960), cfg.clone(), world(&cfg), 9);
    let mut client = NetClient::new(net.endpoint(5000), server.transport.local_addr(), "p", 3);
    let mut stepper = FixedStepper::new(125);
    let mut predictor: Option<Predictor> = None;
    let mut now = 0u64;
    for _ in 0..secs * 1000 {
        now += 1_000;
        net.advance_to(now);
        server.poll(now);
        for _ in 0..stepper.advance(1_000).1 {
            server.tick(now);
        }
        client.update(now);
        if predictor.is_none()
            && let (Some(me), Some(gc)) = (client.player(), client.config.clone())
        {
            predictor = Some(Predictor::new(me, world(&cfg), gc));
        }
        let Some(pred) = predictor.as_mut() else {
            continue;
        };
        for snap in client.take_snapshots() {
            if client
                .snapshots
                .latest()
                .is_some_and(|l| l.tick == snap.tick)
            {
                pred.on_snapshot(&snap, &client.sent_cmds);
            }
        }
        for t in client.advance(now) {
            let cmd = script(t.0);
            client.send_cmd(now, cmd);
            pred.predict(&cmd);
        }
    }
    let missing = server.clients()[0].missing_cmds;
    (predictor.unwrap().stats, missing)
}

#[test]
fn prediction_is_exact_without_loss() {
    for (name, c) in [
        ("0 ms", LinkConditions::default()),
        (
            "50 ms",
            LinkConditions {
                latency_us: 50_000,
                ..Default::default()
            },
        ),
        (
            "100±20 ms",
            LinkConditions {
                latency_us: 100_000,
                jitter_us: 20_000,
                loss: 0.0,
            },
        ),
    ] {
        let (s, missing) = run(c, 20);
        println!(
            "{name}: checked {} mean {:.4} max {:.4} mispredicted {} | missing {missing}",
            s.checked,
            s.mean_error(),
            s.max_error,
            s.mispredicted
        );
        // Reordered (jittered) snapshots that aren't newest aren't reconciled/checked.
        assert!(s.checked > 1000, "{name}: {s:?}");
        assert_eq!(missing, 0, "{name}");
        assert_eq!(
            s.max_error, 0.0,
            "{name}: deterministic sim ⇒ bit-exact prediction"
        );
    }
}

#[test]
fn prediction_error_bounded_with_loss() {
    let (s, missing) = run(
        LinkConditions {
            latency_us: 50_000,
            jitter_us: 10_000,
            loss: 0.05,
        },
        20,
    );
    println!(
        "50±10 ms 5% loss: checked {} mean {:.4} max {:.4} mispredicted {} | missing {missing}",
        s.checked,
        s.mean_error(),
        s.max_error,
        s.mispredicted
    );
    assert!(s.checked > 1000);
    // Threshold from the plan: < 0.5 u mean. With 8× input redundancy no command
    // goes missing at 5% loss, so it is in fact exact.
    assert!(s.mean_error() < 0.5, "{s:?}");
    assert_eq!(missing, 0);
}
