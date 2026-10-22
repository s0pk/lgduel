//! Map agreement: the server announces its map (name + content hash) in the
//! accept; clients switch to that built-in map, or refuse a map they don't have.

use lgduel_bot::netbot::NetBot;
use lgduel_bot::{AimSkill, BotConfig, StrafePattern};
use lgduel_net::client::ClientState;
use lgduel_net::transport::{MemNet, Transport};
use lgduel_server::{Server, ServerConfig};
use lgduel_sim::math::Vec3;
use lgduel_sim::tick::FixedStepper;
use lgduel_sim::world::{World, builtin};

fn hull(cfg: &ServerConfig) -> (Vec3, Vec3) {
    let p = &cfg.game.physics;
    (
        Vec3::from_array(p.player_mins),
        Vec3::from_array(p.player_maxs),
    )
}

/// Run a server on `server_world` with two bots that start out holding arena_flat.
fn run(
    server_world: World,
    secs: u64,
) -> (
    NetBot<lgduel_net::transport::MemEndpoint>,
    NetBot<lgduel_net::transport::MemEndpoint>,
) {
    let net = MemNet::new(7);
    let cfg = ServerConfig::default();
    let flat = builtin::load("arena_flat", hull(&cfg)).unwrap();
    let mut server = Server::new(net.endpoint(27960), cfg, server_world, 9);
    let addr = server.transport.local_addr();
    let mk = |seed| BotConfig {
        pattern: StrafePattern::RandomAd {
            min_ms: 150,
            max_ms: 600,
        },
        aim: AimSkill::HARD,
        seed,
        ..BotConfig::default()
    };
    let mut a = NetBot::new(net.endpoint(5001), addr, "a", mk(1), flat.clone());
    let mut b = NetBot::new(net.endpoint(5002), addr, "b", mk(2), flat);
    let mut stepper = FixedStepper::new(125);
    let mut now = 0;
    for _ in 0..secs * 1000 {
        now += 1_000;
        net.advance_to(now);
        server.poll(now);
        for _ in 0..stepper.advance(1_000).1 {
            server.tick(now);
        }
        a.step(now);
        b.step(now);
    }
    (a, b)
}

#[test]
fn clients_switch_to_the_servers_builtin_map_and_fight_there() {
    let cfg = ServerConfig::default();
    for name in ["arena_pillars", "arena_tiers"] {
        let world = builtin::load(name, hull(&cfg)).unwrap();
        let hash = world.content_hash();
        let (a, b) = run(world, 20);
        for bot in [&a, &b] {
            assert!(
                matches!(bot.state(), ClientState::Connected { .. }),
                "{name}: {:?}",
                bot.state()
            );
            assert_eq!(bot.map_error, None);
            assert_eq!(bot.client.map, Some((name.to_string(), hash)));
            assert_eq!(bot.world().name, name);
        }
        let (sa, sb) = (a.stats().unwrap(), b.stats().unwrap());
        println!("{name}: a {sa:?}\n{name}: b {sb:?}");
        assert!(
            sa.damage_dealt + sb.damage_dealt > 0,
            "{name}: the bots found and hit each other"
        );
        // Prediction on the switched map stays exact (same collision as the server).
        assert!(a.prediction().mean_error() < 0.01, "{name}");
    }
}

#[test]
fn client_refuses_a_map_it_does_not_have() {
    let cfg = ServerConfig::default();
    // Same name as a built-in map but different geometry (e.g. an edited copy).
    let mut world = builtin::load("arena_tiers", hull(&cfg)).unwrap();
    world.brushes[0].max.z -= 8.0;
    world.spawns.iter_mut().for_each(|s| s.origin.z -= 8.0);
    let (a, _) = run(world.clone(), 3);
    let e = a.map_error.clone().expect("refused");
    assert!(e.contains("differs"), "{e}");
    assert!(
        matches!(a.state(), ClientState::Disconnected(_)),
        "{:?}",
        a.state()
    );

    world.name = "my_custom_map".into();
    let (a, _) = run(world, 3);
    let e = a.map_error.clone().expect("refused");
    assert!(e.contains("doesn't have"), "{e}");
}
