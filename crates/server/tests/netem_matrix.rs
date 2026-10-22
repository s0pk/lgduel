//! Netcode under a matrix of link conditions (PLAN 5.2).
//!
//! Cells: one-way latency {0, 50, 100, 200 ms} × loss {0, 1, 2.5, 5 %}, jitter
//! ±20 ms (±0 for the 0 ms row so it stays a clean baseline). Per cell, 12 s of
//! virtual time over the deterministic in-memory network:
//! - client A moves with a busy script and predicts itself → prediction error
//!   (desync), server-side missing commands;
//! - client B walks back and forth; A interpolates B → starvation and largest
//!   per-1 ms-frame jump (rubber-banding).
//!
//! Thresholds (documented in STATE/DECISIONS.md D31) are asserted per cell.

use lgduel_net::client::NetClient;
use lgduel_net::interp::{InterpSettings, Interpolator};
use lgduel_net::predict::Predictor;
use lgduel_net::transport::{LinkConditions, MemNet, Transport};
use lgduel_server::{Server, ServerConfig};
use lgduel_sim::math::{Angle, Vec3};
use lgduel_sim::tick::{FixedStepper, Tick};
use lgduel_sim::usercmd::{UserCmd, buttons};
use lgduel_sim::world::{World, builtin};

#[derive(Debug, Clone, Copy)]
struct Cell {
    latency_ms: u64,
    jitter_ms: u64,
    loss: f64,
}

#[derive(Debug, Clone, Copy, Default)]
struct Metrics {
    rtt_ms: f64,
    pred_mean: f64,
    pred_max: f32,
    pred_checked: u64,
    max_correction: f32,
    rubber_bands: u64,
    missing_pct: f64,
    starved_pct: f64,
    max_step: f32,
    interp_delay: f64,
}

fn script(tick: u32) -> UserCmd {
    let phase = tick / 37;
    UserCmd {
        tick: Tick(tick),
        pitch: Angle::from_degrees(((tick % 200) as f64 - 100.0) * 0.2),
        yaw: Angle::from_degrees(tick as f64 * 1.3),
        forward: if phase.is_multiple_of(3) { 127 } else { 0 },
        right: if phase.is_multiple_of(2) { 127 } else { -127 },
        buttons: if tick % 90 < 3 { buttons::JUMP } else { 0 },
    }
}

fn run(cell: Cell) -> Metrics {
    let net = MemNet::new(1000 + cell.latency_ms * 7 + (cell.loss * 1000.0) as u64);
    net.set_conditions(LinkConditions {
        latency_us: cell.latency_ms * 1000,
        jitter_us: cell.jitter_ms * 1000,
        loss: cell.loss,
    });
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
    let mut server = Server::new(net.endpoint(27960), cfg, world.clone(), 7);
    let addr = server.transport.local_addr();
    let mut a = NetClient::new(net.endpoint(5001), addr, "a", 1);
    let mut b = NetClient::new(net.endpoint(5002), addr, "b", 2);
    let mut stepper = FixedStepper::new(125);
    let mut pred: Option<Predictor> = None;
    let mut interp = Interpolator::new(InterpSettings::default());
    let mut now = 0u64;
    let (mut prev, mut max_step) = (None::<Vec3>, 0.0f32);
    let mut placed = false;
    const WARMUP_MS: u64 = 3_000;
    const TOTAL_MS: u64 = 15_000;
    let mut missing_at_warmup = 0;
    let mut ticks_at_warmup = 0;
    for ms in 0..TOTAL_MS {
        now += 1_000;
        net.advance_to(now);
        server.poll(now);
        for _ in 0..stepper.advance(1_000).1 {
            server.tick(now);
        }
        a.update(now);
        b.update(now);
        if !placed && server.client_count() == 2 {
            // Keep both well inside the arena regardless of spawn.
            let z = server.arenas[0].game.players[0].pm.origin.z;
            server.arenas[0].game.players[0].pm.origin = Vec3::new(-300.0, -300.0, z);
            server.arenas[0].game.players[1].pm.origin = Vec3::new(0.0, 300.0, z);
            placed = true;
        }
        if pred.is_none()
            && let (Some(me), Some(gc)) = (a.player(), a.config.clone())
        {
            pred = Some(Predictor::new(me, world.clone(), gc));
        }
        let fwd = if (ms / 2_000).is_multiple_of(2) {
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
        let Some(pr) = pred.as_mut() else { continue };
        for s in a.take_snapshots() {
            interp.push(s.tick.0, &s.players);
            if a.snapshots.latest().is_some_and(|l| l.tick == s.tick) {
                pr.on_snapshot(&s, &a.sent_cmds);
            }
        }
        interp.set_target_delay(a.recommended_interp_delay());
        let clock = a.snapshot_clock(now);
        for t in a.advance(now) {
            let cmd = script(t.0);
            a.send_cmd_with_view(now, cmd, clock.map(|c| c - interp.settings.delay_ticks));
            pr.predict(&cmd);
        }
        if ms == WARMUP_MS {
            missing_at_warmup = server.clients().iter().map(|c| c.missing_cmds).sum::<u64>();
            ticks_at_warmup = server.game().tick.0;
            interp.stats = Default::default();
            pr.stats = Default::default();
        }
        // Sample every frame (as a real client does: the adaptive delay adapts
        // inside `sample`), but only measure after the warm-up.
        if let (Some(bid), Some(clock)) = (b.player(), clock)
            && let Some(v) = interp.sample(bid, clock)
            && ms > WARMUP_MS
        {
            if let Some(p) = prev {
                max_step = max_step.max((v.origin - p).length());
            }
            prev = Some(v.origin);
        }
    }
    let ps = pred.unwrap().stats;
    let missing = server.clients().iter().map(|c| c.missing_cmds).sum::<u64>() - missing_at_warmup;
    let ticks = (server.game().tick.0 - ticks_at_warmup) as u64 * 2; // two clients
    Metrics {
        rtt_ms: a.conn.stats.rtt_us / 1000.0,
        pred_mean: ps.mean_error(),
        pred_max: ps.max_error,
        pred_checked: ps.checked,
        max_correction: ps.max_correction,
        rubber_bands: ps.rubber_bands,
        missing_pct: missing as f64 * 100.0 / ticks as f64,
        starved_pct: interp.stats.starved as f64 * 100.0 / interp.stats.samples.max(1) as f64,
        max_step,
        interp_delay: interp.settings.delay_ticks,
    }
}

#[test]
fn netem_matrix() {
    let mut failures = vec![];
    println!(
        "| one-way | jitter | loss | RTT ms | pred mean u | pred max u | checked | own max correction u | rubber-bands | missing cmds % | interp delay | starved % | max step u/ms |"
    );
    println!("|---|---|---|---|---|---|---|---|---|---|---|---|---|");
    for latency_ms in [0, 50, 100, 200] {
        for loss in [0.0, 0.01, 0.025, 0.05] {
            let jitter_ms = if latency_ms == 0 { 0 } else { 20 };
            let cell = Cell {
                latency_ms,
                jitter_ms,
                loss,
            };
            let m = run(cell);
            println!(
                "| {latency_ms} ms | ±{jitter_ms} | {:.1}% | {:.0} | {:.4} | {:.2} | {} | {:.2} | {} | {:.3} | {:.2} | {:.3} | {:.3} |",
                loss * 100.0,
                m.rtt_ms,
                m.pred_mean,
                m.pred_max,
                m.pred_checked,
                m.max_correction,
                m.rubber_bands,
                m.missing_pct,
                m.interp_delay,
                m.starved_pct,
                m.max_step
            );
            // Thresholds (D31). Walking = 0.32 u per 1 ms frame.
            let name = format!("{latency_ms}ms ±{jitter_ms} {:.1}%", loss * 100.0);
            if m.pred_mean >= 0.5 {
                failures.push(format!("{name}: prediction mean {:.4} ≥ 0.5u", m.pred_mean));
            }
            if m.pred_checked < 500 {
                failures.push(format!("{name}: only {} prediction checks", m.pred_checked));
            }
            // Own-view rubber-banding: at most 2 visible snap-backs (> 1u) in 12 s.
            if m.rubber_bands > 2 {
                failures.push(format!(
                    "{name}: {} rubber-band corrections (max {:.2}u)",
                    m.rubber_bands, m.max_correction
                ));
            }
            if m.missing_pct > 0.5 {
                failures.push(format!(
                    "{name}: {:.3}% commands missing (> 0.5%)",
                    m.missing_pct
                ));
            }
            if m.starved_pct > 1.0 {
                failures.push(format!(
                    "{name}: interpolation starved {:.3}% (> 1%)",
                    m.starved_pct
                ));
            }
            let max_step_allowed = if loss == 0.0 && jitter_ms == 0 {
                0.7
            } else {
                2.56
            };
            if m.max_step > max_step_allowed {
                failures.push(format!(
                    "{name}: max per-frame jump {:.3}u (> {max_step_allowed}u)",
                    m.max_step
                ));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "netem matrix threshold failures:\n{}",
        failures.join("\n")
    );
}
