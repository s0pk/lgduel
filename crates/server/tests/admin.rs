//! Server administration (PLAN 6.5): console commands, live config changes,
//! reload from file, remote console with password and lockout.

use lgduel_bot::BotConfig;
use lgduel_bot::netbot::NetBot;
use lgduel_net::client::ClientState;
use lgduel_net::protocol::Connectionless;
use lgduel_net::transport::{MemEndpoint, MemNet, Transport};
use lgduel_server::{Server, ServerConfig};
use lgduel_sim::math::Vec3;
use lgduel_sim::tick::FixedStepper;
use lgduel_sim::world::builtin;

struct Rig {
    net: MemNet,
    server: Server<MemEndpoint>,
    bots: Vec<NetBot<MemEndpoint>>,
    stepper: FixedStepper,
    now: u64,
}

impl Rig {
    fn new(cfg: ServerConfig, bots: usize) -> Rig {
        let net = MemNet::new(3);
        let p = &cfg.game.physics;
        let hull = (
            Vec3::from_array(p.player_mins),
            Vec3::from_array(p.player_maxs),
        );
        let world = builtin::load("arena_flat", hull).unwrap();
        let server = Server::new(net.endpoint(27960), cfg, world.clone(), 8);
        let addr = server.transport.local_addr();
        let bots = (0..bots)
            .map(|i| {
                let cfg = BotConfig {
                    seed: i as u64 + 1,
                    ..BotConfig::default()
                };
                NetBot::new(
                    net.endpoint(8000 + i as u16),
                    addr,
                    &format!("bot{i}"),
                    cfg,
                    world.clone(),
                )
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
            for b in &mut self.bots {
                b.step(self.now);
            }
        }
    }
}

#[test]
fn console_commands_and_live_settings() {
    let mut rig = Rig::new(ServerConfig::default(), 2);
    rig.run_ms(2_000);
    let s = rig.server.command("status");
    assert!(
        s.contains("bot0") && s.contains("bot1") && s.contains("arena 1"),
        "{s}"
    );
    assert!(rig.server.command("help").contains("kick"));
    assert!(rig.server.command("frobnicate").contains("unknown command"));

    // set: applies to every arena at once, validated like the config file.
    assert_eq!(
        rig.server.command("get rules.lifesteal_percent"),
        "rules.lifesteal_percent = 100"
    );
    assert_eq!(
        rig.server.command("set rules.lifesteal_percent 50"),
        "rules.lifesteal_percent = 50"
    );
    assert!(
        rig.server
            .arenas
            .iter()
            .all(|a| a.game.cfg.rules.lifesteal_percent == 50)
    );
    assert_eq!(rig.server.info().lifesteal_percent, 50);
    rig.server.command("set rules.mode classic");
    assert_eq!(
        rig.server.game().cfg.rules.mode,
        lgduel_sim::config::DuelMode::Classic
    );
    rig.server.command("set lg.range 700");
    assert_eq!(rig.server.game().cfg.lg.range, 700.0);
    rig.server.command("set server.name \"Test server\"");
    assert_eq!(rig.server.info().name, "Test server");
    // Refused: unknown keys, bad values, and what needs a restart.
    for (cmd, why) in [
        ("set rules.lifestael 5", "no setting"),
        ("set rules.lifesteal_percent banana", "not changed"),
        ("set physics.gravity 400", "restart"),
        ("set tick_rate_hz.x 60", "no setting"),
        ("set server.max_arenas 2", "restart"),
        ("set server.port 1", "restart"),
    ] {
        let out = rig.server.command(cmd);
        assert!(out.contains(why), "{cmd}: {out}");
    }
    assert_eq!(rig.server.game().cfg.physics.gravity, 800.0);

    // kick: the player is told why and removed.
    let out = rig.server.command("kick bot1 too good");
    assert_eq!(out, "kicked bot1");
    rig.run_ms(500);
    assert_eq!(rig.server.client_count(), 1);
    assert_eq!(
        rig.bots[1].state(),
        &ClientState::Disconnected("kicked: too good".into())
    );
    assert!(rig.server.command("kick nobody").contains("no player"));
    // say reaches the players.
    rig.server.command("say gg");
    rig.run_ms(300);
    assert!(
        rig.bots[0]
            .client
            .messages
            .iter()
            .any(|m| m == "SERVER: GG")
    );
}

#[test]
fn reload_reads_the_config_file() {
    let dir = std::env::temp_dir().join(format!("lgduel-admin-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("server.toml");
    std::fs::write(&path, "[rules]\nlifesteal_percent = 30\n").unwrap();
    let mut rig = Rig::new(ServerConfig::default(), 0);
    assert!(rig.server.command("reload").contains("no config file"));
    rig.server.config_path = Some(path.display().to_string());
    assert!(rig.server.command("reload").starts_with("reloaded"));
    assert_eq!(rig.server.game().cfg.rules.lifesteal_percent, 30);
    std::fs::write(&path, "[physics]\ngravity = 1.0\n").unwrap();
    assert!(rig.server.command("reload").contains("restart"));
    std::fs::write(&path, "[rules]\nnot_a_key = 1\n").unwrap();
    assert!(rig.server.command("reload").contains("not reloaded"));
    assert_eq!(rig.server.game().cfg.rules.lifesteal_percent, 30);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn rcon_needs_the_password_and_locks_out_guessers() {
    let mut cfg = ServerConfig::default();
    cfg.server.rcon_password = "hunter2".into();
    let mut rig = Rig::new(cfg, 1);
    rig.run_ms(1_000);
    let addr = rig.server.transport.local_addr();
    let mut admin = rig.net.endpoint(9000);
    let mut ask = |rig: &mut Rig, password: &str, command: &str| -> Option<String> {
        let req = Connectionless::Rcon {
            nonce: 42,
            password: password.into(),
            command: command.into(),
        };
        admin.send_to(addr, &req.encode()).unwrap();
        rig.run_ms(100);
        let mut reply = None;
        while let Some((_, data)) = admin.recv() {
            if let Some(Connectionless::RconReply { nonce: 42, text }) =
                Connectionless::decode(&data)
            {
                reply = Some(text);
            }
        }
        reply
    };
    let s = ask(&mut rig, "hunter2", "status").expect("reply");
    assert!(s.contains("bot0"), "{s}");
    assert_eq!(
        ask(&mut rig, "hunter2", "set rules.lifesteal_percent 70").as_deref(),
        Some("rules.lifesteal_percent = 70")
    );
    assert_eq!(rig.server.game().cfg.rules.lifesteal_percent, 70);
    // Wrong password: silence (never more bytes out than in); 5 tries lock the address out.
    for _ in 0..5 {
        assert_eq!(ask(&mut rig, "guess", "status"), None);
    }
    assert_eq!(ask(&mut rig, "hunter2", "status"), None, "locked out");
    // A minute later it works again.
    rig.run_ms(61_000);
    assert!(ask(&mut rig, "hunter2", "status").is_some());
    // No password configured: RCON is off.
    let mut off = Rig::new(ServerConfig::default(), 0);
    let addr = off.server.transport.local_addr();
    let mut e = off.net.endpoint(9001);
    e.send_to(
        addr,
        &Connectionless::Rcon {
            nonce: 1,
            password: String::new(),
            command: "status".into(),
        }
        .encode(),
    )
    .unwrap();
    off.run_ms(100);
    assert!(e.recv().is_none());
}
