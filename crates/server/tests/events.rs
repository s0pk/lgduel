//! Reliable game events (M4.3 / KI-10): damage, kills, respawns and fight ends
//! reach the client exactly once even with heavy packet loss.

use lgduel_bot::netbot::NetBot;
use lgduel_bot::{AimSkill, BotConfig};
use lgduel_net::transport::{LinkConditions, MemNet, Transport};
use lgduel_server::{Server, ServerConfig};
use lgduel_sim::math::Vec3;
use lgduel_sim::tick::FixedStepper;
use lgduel_sim::world::builtin;

#[test]
fn reliable_events_survive_25_percent_loss() {
    let net = MemNet::new(5);
    net.set_conditions(LinkConditions {
        latency_us: 40_000,
        jitter_us: 10_000,
        loss: 0.25,
    });
    let mut cfg = ServerConfig::default();
    // Short fights: plenty of kills/respawns/fight ends, not just damage.
    cfg.game.rules.spawn_health = 50;
    cfg.game.rules.max_health = 50;
    let p = &cfg.game.physics;
    let hull = (
        Vec3::from_array(p.player_mins),
        Vec3::from_array(p.player_maxs),
    );
    let world = builtin::load("arena_flat", hull).unwrap();
    let mut server = Server::new(net.endpoint(27960), cfg, world.clone(), 1);
    let addr = server.transport.local_addr();
    let mk = |seed| BotConfig {
        aim: AimSkill::HARD,
        seed,
        ..BotConfig::default()
    };
    let mut bots = [
        NetBot::new(net.endpoint(7001), addr, "a", mk(1), world.clone()),
        NetBot::new(net.endpoint(7002), addr, "b", mk(2), world),
    ];
    let mut stepper = FixedStepper::new(125);
    let mut now = 0;
    for _ in 0..30_000 {
        now += 1_000;
        net.advance_to(now);
        server.poll(now);
        for _ in 0..stepper.advance(1_000).1 {
            server.tick(now);
        }
        for b in &mut bots {
            b.step(now);
        }
    }
    let infos = server.clients();
    for (b, info) in bots.iter().zip(&infos) {
        let c = &b.client;
        println!(
            "player {:?}: server numbered {} reliable events, client got {} (gaps {}, dropped {}), loss {:.1}%",
            info.player,
            info.events_sequenced,
            c.reliable_events_delivered,
            c.event_gaps,
            info.events_dropped,
            c.conn.stats.loss() * 100.0
        );
        assert!(
            info.events_sequenced > 100,
            "enough events to mean something"
        );
        assert_eq!(info.events_dropped, 0);
        assert_eq!(c.event_gaps, 0, "nothing skipped");
        // Exactly once: never more than were sent; only the last few can still be in
        // flight. (Unreliable delivery would lose ~loss% of them: ~40 here.)
        assert!(c.reliable_events_delivered <= info.events_sequenced);
        assert!(
            info.events_sequenced - c.reliable_events_delivered < 10,
            "missing {} of {}",
            info.events_sequenced - c.reliable_events_delivered,
            info.events_sequenced
        );
    }
    let kills: u32 = bots.iter().map(|b| b.stats().unwrap().kills).sum();
    assert!(kills >= 4, "fights ended with frags: {kills}");
}
