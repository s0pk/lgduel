//! NetClient ↔ Server over the in-memory network: the client's clock alignment
//! must get every command to the server just before its tick.

use lgduel_net::client::{ClientState, NetClient};
use lgduel_net::transport::{LinkConditions, MemEndpoint, MemNet, Transport};
use lgduel_server::{Server, ServerConfig};
use lgduel_sim::math::Vec3;
use lgduel_sim::tick::FixedStepper;
use lgduel_sim::usercmd::UserCmd;
use lgduel_sim::world::{World, builtin};

struct Rig {
    net: MemNet,
    server: Server<MemEndpoint>,
    client: NetClient<MemEndpoint>,
    stepper: FixedStepper,
    now: u64,
}

impl Rig {
    fn new(c: LinkConditions) -> Self {
        let net = MemNet::new(21);
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
        let server = Server::new(net.endpoint(27960), cfg, world, 77);
        let client = NetClient::new(net.endpoint(5000), server.transport.local_addr(), "sync", 1);
        Self {
            net,
            server,
            client,
            stepper: FixedStepper::new(125),
            now: 0,
        }
    }

    /// Advance 1 ms of virtual time.
    fn step_ms(&mut self) {
        self.now += 1_000;
        self.net.advance_to(self.now);
        self.server.poll(self.now);
        for _ in 0..self.stepper.advance(1_000).1 {
            self.server.tick(self.now);
        }
        self.client.update(self.now);
        for t in self.client.advance(self.now) {
            self.client.send_cmd(
                self.now,
                UserCmd {
                    tick: t,
                    forward: 127,
                    ..UserCmd::default()
                },
            );
        }
    }

    fn run_ms(&mut self, ms: u64) {
        for _ in 0..ms {
            self.step_ms();
        }
    }
}

#[test]
fn commands_arrive_just_in_time_and_adapt_to_latency_change() {
    let mut rig = Rig::new(LinkConditions {
        latency_us: 50_000,
        jitter_us: 10_000,
        loss: 0.0,
    });
    rig.run_ms(3_000);
    assert!(
        matches!(rig.client.state, ClientState::Connected { .. }),
        "{:?}",
        rig.client.state
    );
    let before = rig.server.clients()[0].missing_cmds;
    let mut max_buffered = 0;
    for _ in 0..10_000 {
        rig.step_ms();
        max_buffered = max_buffered.max(rig.server.clients()[0].buffered_cmds);
    }
    let info = rig.server.clients()[0];
    println!(
        "50±10 ms: missing in 10 s = {}, max server buffer {} ticks, rtt {:.1} ms, lead target {:.1} ticks",
        info.missing_cmds - before,
        max_buffered,
        info.rtt_ms,
        rig.client.target_lead_ticks()
    );
    assert_eq!(
        info.missing_cmds - before,
        0,
        "no command may arrive late in steady state"
    );
    assert!(
        max_buffered <= 8,
        "client must not run needlessly far ahead: {max_buffered}"
    );

    // Latency jumps to 150 ms: the client must re-align.
    rig.net.set_conditions(LinkConditions {
        latency_us: 150_000,
        jitter_us: 10_000,
        loss: 0.0,
    });
    rig.run_ms(3_000);
    let before = rig.server.clients()[0].missing_cmds;
    rig.run_ms(10_000);
    let info = rig.server.clients()[0];
    println!(
        "150±10 ms: missing in 10 s = {}, rtt {:.1} ms, lead target {:.1} ticks",
        info.missing_cmds - before,
        info.rtt_ms,
        rig.client.target_lead_ticks()
    );
    assert_eq!(info.missing_cmds - before, 0);
    assert!((290.0..330.0).contains(&info.rtt_ms), "{info:?}");
}

#[test]
fn client_ticks_track_server_rate() {
    let mut rig = Rig::new(LinkConditions {
        latency_us: 30_000,
        ..Default::default()
    });
    rig.run_ms(2_000);
    let (c0, _) = rig.client.client_tick();
    let s0 = rig.server.game.tick;
    rig.run_ms(20_000);
    let (c1, _) = rig.client.client_tick();
    let s1 = rig.server.game.tick;
    let (dc, ds) = (c1.0 - c0.0, s1.0 - s0.0);
    println!("20 s: client ticks {dc}, server ticks {ds}");
    assert_eq!(ds, 2500);
    assert!(dc.abs_diff(ds) <= 2, "client {dc} vs server {ds}");
    // The client stays ahead of the server by about the target lead.
    assert!(c1.0 > s1.0, "client must lead the server");
}

#[test]
fn rejected_when_full() {
    let mut rig = Rig::new(LinkConditions::default());
    rig.server.settings.max_clients = 0;
    rig.run_ms(500);
    assert!(
        matches!(rig.client.state, ClientState::Rejected(ref r) if r.contains("full")),
        "{:?}",
        rig.client.state
    );
    let _ = rig.server.transport.local_addr();
}
