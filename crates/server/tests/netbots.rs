//! Two network bots duel through the real server over the in-memory network.

use lgduel_bot::netbot::NetBot;
use lgduel_bot::{AimSkill, BotConfig, StrafePattern};
use lgduel_net::client::ClientState;
use lgduel_net::transport::{LinkConditions, MemNet, Transport};
use lgduel_server::{Server, ServerConfig};
use lgduel_sim::math::Vec3;
use lgduel_sim::tick::FixedStepper;
use lgduel_sim::world::{World, builtin};

#[test]
fn two_netbots_play_a_vamp_duel() {
    let net = MemNet::new(41);
    net.set_conditions(LinkConditions {
        latency_us: 25_000,
        jitter_us: 5_000,
        loss: 0.01,
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
    let mut server = Server::new(net.endpoint(27960), cfg, world.clone(), 5);
    let addr = server.transport.local_addr();
    let ad = StrafePattern::RandomAd {
        min_ms: 150,
        max_ms: 600,
    };
    let mk = |seed, aim| BotConfig {
        pattern: ad,
        aim,
        seed,
        ..BotConfig::default()
    };
    let mut a = NetBot::new(
        net.endpoint(5001),
        addr,
        "hard",
        mk(1, AimSkill::HARD),
        world.clone(),
    );
    let mut b = NetBot::new(
        net.endpoint(5002),
        addr,
        "easy",
        mk(2, AimSkill::EASY),
        world,
    );
    let mut stepper = FixedStepper::new(125);
    let mut now = 0;
    for _ in 0..30_000 {
        now += 1_000;
        net.advance_to(now);
        server.poll(now);
        for _ in 0..stepper.advance(1_000).1 {
            server.tick(now);
        }
        a.step(now);
        b.step(now);
    }
    assert!(
        matches!(a.state(), ClientState::Connected { .. })
            && matches!(b.state(), ClientState::Connected { .. })
    );
    let (sa, sb) = (a.stats().unwrap(), b.stats().unwrap());
    let (pa, pb) = (a.prediction(), b.prediction());
    println!("hard: {sa:?}\neasy: {sb:?}");
    println!(
        "prediction: a mean {:.4} max {:.4} ({} checked), b mean {:.4} max {:.4}",
        pa.mean_error(),
        pa.max_error,
        pa.checked,
        pb.mean_error(),
        pb.max_error
    );
    assert!(sa.damage_dealt > 0 && sb.damage_dealt > 0, "both fought");
    assert!(sa.accuracy_pct() > sb.accuracy_pct());
    // 100% lifesteal: earned exactly the damage dealt.
    assert_eq!(sa.lifesteal_earned, sa.damage_dealt);
    assert_eq!(sb.lifesteal_earned, sb.damage_dealt);
    assert!(sa.kills + sb.kills > 0, "someone died in 30 s");
    assert!(pa.mean_error() < 0.5 && pb.mean_error() < 0.5, "no desync");
}
