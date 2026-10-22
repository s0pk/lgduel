//! Server-browser info query (M2.1) over the in-memory network.

use lgduel_net::browser::Pinger;
use lgduel_net::transport::{LinkConditions, MemNet, Transport};
use lgduel_server::{Server, ServerConfig};
use lgduel_sim::math::Vec3;
use lgduel_sim::world::{World, builtin};

#[test]
fn pinger_gets_info_and_rtt() {
    let net = MemNet::new(3);
    net.set_conditions(LinkConditions {
        latency_us: 40_000,
        ..Default::default()
    });
    let mut cfg = ServerConfig::default();
    cfg.server.name = "EU vamp #1".into();
    cfg.game.rules.lifesteal_percent = 50;
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
    let nobody = net.endpoint(9999).local_addr();
    let mut pinger = Pinger::new(net.endpoint(5000), 7);
    pinger.query(addr, 0);
    pinger.query(nobody, 0);
    let mut now = 0;
    let mut got = vec![];
    while now < 300_000 {
        now += 1_000;
        net.advance_to(now);
        server.poll(now);
        got.extend(pinger.poll(now, 1_000_000));
    }
    assert_eq!(got, vec![addr]);
    let r = &pinger.results[&addr];
    assert_eq!(r.info.name, "EU vamp #1");
    assert_eq!(r.info.map, "arena_flat");
    assert_eq!((r.info.players, r.info.max_players), (0, 16));
    assert_eq!(r.info.lifesteal_percent, 50);
    assert!(r.info.anti_lag && !r.info.classic);
    assert_eq!(r.info.tick_rate, 125);
    assert_eq!(r.info.protocol, lgduel_net::PROTOCOL_VERSION);
    assert!((80.0..=82.0).contains(&r.rtt_ms), "rtt {}", r.rtt_ms);
    assert!(pinger.waiting(nobody));
    pinger.poll(2_000_000, 1_000_000);
    assert!(!pinger.waiting(nobody), "timed-out query forgotten");
}
