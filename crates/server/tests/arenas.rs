//! Multi-arena server (M4.1): pairs are isolated in their own arenas, extra
//! players queue, and lone players are paired up.

use lgduel_bot::netbot::NetBot;
use lgduel_bot::{AimSkill, BotConfig, StrafePattern};
use lgduel_net::client::ClientState;
use lgduel_net::transport::{MemEndpoint, MemNet, Transport};
use lgduel_server::{Server, ServerConfig};
use lgduel_sim::math::Vec3;
use lgduel_sim::tick::FixedStepper;
use lgduel_sim::world::builtin;

struct Rig {
    net: MemNet,
    server: Server<MemEndpoint>,
    bots: Vec<Option<NetBot<MemEndpoint>>>,
    stepper: FixedStepper,
    now: u64,
}

impl Rig {
    fn new(max_arenas: u8, bots: usize) -> Rig {
        let net = MemNet::new(11);
        let mut cfg = ServerConfig::default();
        cfg.server.max_arenas = max_arenas;
        let p = &cfg.game.physics;
        let hull = (
            Vec3::from_array(p.player_mins),
            Vec3::from_array(p.player_maxs),
        );
        let world = builtin::load("arena_flat", hull).unwrap();
        let server = Server::new(net.endpoint(27960), cfg, world.clone(), 3);
        let addr = server.transport.local_addr();
        let bots = (0..bots)
            .map(|i| {
                let cfg = BotConfig {
                    pattern: StrafePattern::RandomAd {
                        min_ms: 150,
                        max_ms: 600,
                    },
                    aim: AimSkill::HARD,
                    seed: i as u64 + 1,
                    ..BotConfig::default()
                };
                Some(NetBot::new(
                    net.endpoint(6000 + i as u16),
                    addr,
                    &format!("bot{i}"),
                    cfg,
                    world.clone(),
                ))
            })
            .collect();
        Rig {
            net,
            server,
            bots,
            stepper: FixedStepper::new(125),
            now: 0,
        }
    }

    fn run_ms(&mut self, ms: u64) {
        for _ in 0..ms {
            self.now += 1_000;
            self.net.advance_to(self.now);
            self.server.poll(self.now);
            for _ in 0..self.stepper.advance(1_000).1 {
                self.server.tick(self.now);
            }
            for b in self.bots.iter_mut().flatten() {
                b.step(self.now);
            }
        }
    }

    fn leave(&mut self, i: usize) {
        let mut b = self.bots[i].take().unwrap();
        b.client.disconnect(self.now);
    }

    /// Arena of each connected bot, by bot index.
    fn arena_of(&self, i: usize) -> Option<usize> {
        let id = self.bots[i].as_ref()?.client.player()?;
        self.server
            .clients()
            .into_iter()
            .find(|c| c.player == id)?
            .arena
    }
}

#[test]
fn six_players_three_isolated_arenas() {
    let mut rig = Rig::new(4, 6);
    rig.run_ms(20_000);
    let arenas: Vec<Option<usize>> = (0..6).map(|i| rig.arena_of(i)).collect();
    println!("arenas {arenas:?}");
    // Joined in order: 0+1 → arena 0, 2+3 → arena 1, 4+5 → arena 2.
    assert_eq!(
        arenas,
        vec![Some(0), Some(0), Some(1), Some(1), Some(2), Some(2)]
    );
    for (a, arena) in rig.server.arenas.iter().enumerate() {
        let want = if a < 3 { 2 } else { 0 };
        assert_eq!(arena.game.players.len(), want, "arena {a}");
    }
    // All arenas tick together.
    let t = rig.server.game().tick;
    assert!(rig.server.arenas.iter().all(|a| a.game.tick == t));
    for pair in [[0, 1], [2, 3], [4, 5]] {
        let [x, y] = pair.map(|i| rig.bots[i].as_ref().unwrap());
        let (sx, sy) = (x.stats().unwrap(), y.stats().unwrap());
        println!("pair {pair:?}: {sx:?}\n             {sy:?}");
        // Damage only flows inside the pair.
        assert!(
            sx.damage_dealt > 0 && sy.damage_dealt > 0,
            "{pair:?} fought"
        );
        assert_eq!(sx.damage_dealt, sy.damage_taken, "{pair:?}");
        assert_eq!(sy.damage_dealt, sx.damage_taken, "{pair:?}");
        // Each bot only ever sees its own arena.
        for b in [x, y] {
            let snap = b.client.snapshots.latest().expect("snapshot");
            assert_eq!(snap.players.len(), 2);
            let ids: Vec<_> = snap.players.iter().map(|p| p.id).collect();
            assert!(
                ids.contains(&x.client.player().unwrap())
                    && ids.contains(&y.client.player().unwrap())
            );
        }
    }
}

#[test]
fn extra_player_queues_then_takes_a_free_place() {
    let mut rig = Rig::new(2, 5);
    rig.run_ms(3_000);
    assert!(
        rig.bots
            .iter()
            .flatten()
            .all(|b| matches!(b.state(), ClientState::Connected { .. }))
    );
    let arenas: Vec<_> = (0..5).map(|i| rig.arena_of(i)).collect();
    assert_eq!(
        arenas,
        vec![Some(0), Some(0), Some(1), Some(1), None],
        "bot 4 queued"
    );
    // The queued player still gets snapshots (it watches arena 0) and stays connected.
    rig.run_ms(3_000);
    assert!(matches!(
        rig.bots[4].as_ref().unwrap().state(),
        ClientState::Connected { .. }
    ));
    // Someone in arena 0 leaves: the queued player takes the place.
    rig.leave(1);
    rig.run_ms(2_000);
    assert_eq!(rig.arena_of(4), Some(0));
    assert_eq!(rig.arena_of(0), Some(0));
    rig.run_ms(10_000);
    let s4 = rig.bots[4].as_ref().unwrap().stats().unwrap();
    assert!(
        s4.damage_dealt > 0,
        "the queued player got to fight: {s4:?}"
    );
}

#[test]
fn lone_players_are_paired_up() {
    let mut rig = Rig::new(3, 4);
    rig.run_ms(3_000);
    let arenas: Vec<_> = (0..4).map(|i| rig.arena_of(i)).collect();
    assert_eq!(arenas, vec![Some(0), Some(0), Some(1), Some(1)]);
    // One player leaves each arena: the two who are left alone get paired.
    rig.leave(1);
    rig.leave(2);
    rig.run_ms(2_000);
    assert_eq!(rig.arena_of(0), Some(0));
    assert_eq!(rig.arena_of(3), Some(0), "moved into arena 0");
    assert_eq!(rig.server.arenas[1].game.players.len(), 0);
    let before = rig.bots[3].as_ref().unwrap().stats().unwrap();
    rig.run_ms(10_000);
    let after = rig.bots[3].as_ref().unwrap().stats().unwrap();
    assert!(
        after.damage_dealt > before.damage_dealt,
        "fights in its new arena"
    );
    // The server's stats for a moved player include its old arena.
    let stats = rig.server.stats_toml();
    assert!(stats.contains("name = \"bot3\""), "{stats}");
}
