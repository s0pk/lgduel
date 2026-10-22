//! Join codes (PLAN 7.4/7.5): host and client behind simulated NATs find each
//! other through the rendezvous service; punched direct path when the NATs
//! allow it, relay when they don't.

use lgduel_bot::BotConfig;
use lgduel_bot::netbot::NetBot;
use lgduel_net::client::ClientState;
use lgduel_net::rendezvous::{JOIN_ADDR, JoinState, RDV_PORT, RelayTransport, format_code};
use lgduel_net::transport::{LinkConditions, MemEndpoint, MemNet, NatKind, Transport};
use lgduel_server::relay::Rendezvous;
use lgduel_server::{Server, ServerConfig};
use lgduel_sim::math::Vec3;
use lgduel_sim::tick::FixedStepper;
use lgduel_sim::world::builtin;

struct Rig {
    net: MemNet,
    rdv: Rendezvous<MemEndpoint>,
    host: Server<RelayTransport<MemEndpoint>>,
    bot: NetBot<RelayTransport<MemEndpoint>>,
    stepper: FixedStepper,
    now: u64,
}

impl Rig {
    /// Host behind NAT `host_nat`, the joining bot behind `client_nat`; the
    /// bot only starts once the host has its code.
    fn new(host_nat: NatKind, client_nat: NatKind, code_override: Option<u32>) -> Rig {
        let net = MemNet::new(17);
        net.set_conditions(LinkConditions {
            latency_us: 20_000,
            jitter_us: 2_000,
            loss: 0.0,
        });
        let rdv = Rendezvous::new(net.endpoint(RDV_PORT), 5);
        let rdv_addr = rdv.transport.local_addr();
        let cfg = ServerConfig::default();
        let p = &cfg.game.physics;
        let hull = (
            Vec3::from_array(p.player_mins),
            Vec3::from_array(p.player_maxs),
        );
        let world = builtin::load("arena_flat", hull).unwrap();
        let host_t = RelayTransport::host(
            net.endpoint_behind_nat(27960, [203, 0, 113, 10], host_nat),
            rdv_addr,
            11,
        );
        let mut host = Server::new(host_t, cfg, world.clone(), 4);
        let mut rdv = rdv;
        let mut now = 0;
        // Register and learn the code.
        while host.transport.code().is_none() {
            now += 1_000;
            net.advance_to(now);
            host.transport.tick(now);
            rdv.poll(now);
            host.poll(now);
            assert!(now < 2_000_000, "host never registered");
        }
        let code = code_override.unwrap_or(host.transport.code().unwrap());
        let client_t = RelayTransport::join(
            net.endpoint_behind_nat(5000, [198, 51, 100, 20], client_nat),
            rdv_addr,
            code,
            12,
        );
        let bot = NetBot::new(client_t, JOIN_ADDR, "joiner", BotConfig::default(), world);
        Rig {
            net,
            rdv,
            host,
            bot,
            stepper: FixedStepper::new(125),
            now,
        }
    }

    fn run_ms(&mut self, ms: u64) {
        for _ in 0..ms {
            self.now += 1_000;
            let now = self.now;
            self.net.advance_to(now);
            self.rdv.poll(now);
            self.host.transport.tick(now);
            self.host.poll(now);
            for _ in 0..self.stepper.advance(1_000).1 {
                self.host.tick(now);
            }
            self.bot.client.transport.tick(now);
            self.bot.step(now);
        }
    }
}

#[test]
fn cone_nats_connect_directly_by_join_code() {
    let mut rig = Rig::new(NatKind::Cone, NatKind::Cone, None);
    let code = format_code(rig.host.transport.code().unwrap());
    assert!(code.starts_with("LG-") && code.len() == 7, "{code}");
    rig.run_ms(8_000);
    assert!(
        matches!(rig.bot.state(), ClientState::Connected { .. }),
        "{:?} / {:?}",
        rig.bot.state(),
        rig.bot.client.transport.join_state()
    );
    assert!(
        matches!(
            rig.bot.client.transport.join_state(),
            Some(JoinState::Direct { heard: true, .. })
        ),
        "{:?}",
        rig.bot.client.transport.join_state()
    );
    assert_eq!(rig.rdv.sessions(), 0, "no relay needed");
    assert_eq!(rig.host.client_count(), 1);
    // The joiner plays (practice bot after 2 s alone).
    let s = rig.bot.stats().unwrap();
    assert!(s.damage_dealt + s.damage_taken > 0, "{s:?}");
    println!(
        "direct: NAT dropped {} (before the punches met)",
        rig.net.nat_dropped()
    );
}

#[test]
fn symmetric_nats_fall_back_to_the_relay() {
    let mut rig = Rig::new(NatKind::Symmetric, NatKind::Symmetric, None);
    rig.run_ms(12_000);
    assert!(
        matches!(rig.bot.state(), ClientState::Connected { .. }),
        "{:?} / {:?}",
        rig.bot.state(),
        rig.bot.client.transport.join_state()
    );
    assert!(
        matches!(
            rig.bot.client.transport.join_state(),
            Some(JoinState::Relayed { .. })
        ),
        "{:?}",
        rig.bot.client.transport.join_state()
    );
    assert_eq!(rig.rdv.sessions(), 1);
    assert!(
        rig.net.nat_dropped() > 0,
        "the direct attempt really was blocked"
    );
    let (inb, out) = rig.host.transport.relayed;
    assert!(
        inb > 500 && out > 500,
        "game traffic went through the relay: {inb}/{out}"
    );
    let s = rig.bot.stats().unwrap();
    assert!(s.damage_dealt + s.damage_taken > 0, "{s:?}");
    println!("relay stats {:?}", rig.rdv.stats);
}

#[test]
fn unknown_code_fails_cleanly() {
    let mut rig = Rig::new(NatKind::Cone, NatKind::Cone, Some(0));
    // Code 0 ("LG-2222") is free unless the rendezvous picked it for the host.
    if rig.host.transport.code() == Some(0) {
        return;
    }
    rig.run_ms(3_000);
    assert_eq!(
        rig.bot.client.transport.join_state(),
        Some(&JoinState::Failed("no game with that join code".into()))
    );
    assert_eq!(rig.host.client_count(), 0);
}
