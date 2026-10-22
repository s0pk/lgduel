//! `lgduel-bot --server HOST:PORT [--pattern ad|still|mirror|anti|hsw|hswanti]
//!             [--skill easy|medium|hard|aimbot] [--name NAME] [--seed N] [--duration-secs N]`
//!
//! Plays as a network client. On exit prints a `BOTRESULT` line (stats,
//! prediction error, snapshots) for scripts.

use lgduel_bot::netbot::NetBot;
use lgduel_bot::{AimSkill, BotConfig, StrafePattern};
use lgduel_net::client::ClientState;
use lgduel_net::transport::UdpTransport;
use lgduel_sim::config::GameConfig;
use lgduel_sim::math::Vec3;
use lgduel_sim::world::{World, builtin};
use std::net::ToSocketAddrs;
use std::time::{Duration, Instant};

fn run() -> Result<(), String> {
    let mut server = None;
    let mut cfg = BotConfig::default();
    let mut name = "bot".to_string();
    let mut duration = 60.0;
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        let mut val = || it.next().ok_or(format!("{a} needs a value"));
        match a.as_str() {
            "--server" => server = Some(val()?),
            "--pattern" => cfg.pattern = StrafePattern::parse(&val()?).ok_or("bad --pattern")?,
            "--skill" => cfg.aim = AimSkill::preset(&val()?).ok_or("bad --skill")?,
            "--name" => name = val()?,
            "--seed" => cfg.seed = val()?.parse().map_err(|_| "--seed needs a number")?,
            "--duration-secs" => {
                duration = val()?
                    .parse()
                    .map_err(|_| "--duration-secs needs a number")?
            }
            "--version" => {
                println!(
                    "lgduel-bot {} (sim tick {} Hz, protocol v{})",
                    env!("CARGO_PKG_VERSION"),
                    lgduel_sim::DEFAULT_TICK_RATE_HZ,
                    lgduel_net::PROTOCOL_VERSION
                );
                return Ok(());
            }
            other => return Err(format!("unknown argument: {other}")),
        }
    }
    let Some(server) = server else {
        println!("lgduel-bot: nothing to do (use --server HOST:PORT; --version for version)");
        return Ok(());
    };
    let addr = server
        .to_socket_addrs()
        .map_err(|e| format!("{server}: {e}"))?
        .next()
        .ok_or(format!("{server}: no address"))?;
    let transport = UdpTransport::bind("0.0.0.0:0".parse().unwrap()).map_err(|e| e.to_string())?;
    let phys = GameConfig::default().physics;
    let world = World::from_toml_str(
        builtin::ARENA_FLAT,
        (
            Vec3::from_array(phys.player_mins),
            Vec3::from_array(phys.player_maxs),
        ),
    )
    .map_err(|e| e.to_string())?;
    let mut bot = NetBot::new(transport, addr, &name, cfg, world);
    let start = Instant::now();
    let mut announced = false;
    // Measurements (prediction, rubber-bands, late commands) start after a
    // warm-up: connection, clock convergence, and freshly started processes
    // (e.g. antivirus scanning new binaries) are not steady-state netcode.
    const WARMUP_SECS: f64 = 5.0;
    let mut connected_at: Option<f64> = None;
    let mut measuring = false;
    loop {
        let now = start.elapsed();
        let now_us = now.as_micros() as u64;
        bot.step(now_us);
        match bot.state() {
            ClientState::Connected { player } if !announced => {
                println!("{name}: connected to {addr} as player {}", player.0);
                announced = true;
                connected_at = Some(now.as_secs_f64());
            }
            ClientState::Rejected(r) => return Err(format!("rejected: {r}")),
            ClientState::Disconnected(r) => return Err(format!("disconnected: {r}")),
            ClientState::Connecting if now.as_secs_f64() > 5.0 => {
                return Err("could not connect within 5 s".into());
            }
            _ => {}
        }
        if !measuring && connected_at.is_some_and(|t| now.as_secs_f64() - t >= WARMUP_SECS) {
            bot.reset_measurements();
            measuring = true;
        }
        if now.as_secs_f64() >= duration {
            break;
        }
        std::thread::sleep(Duration::from_micros(500));
    }
    let s = bot.stats().unwrap_or_default();
    let p = bot.prediction();
    println!(
        "BOTRESULT name={name} shots={} hits={} acc={:.1} dealt={} taken={} healed={} lifesteal_earned={} kills={} deaths={} \
         wins={} losses={} snapshots={} pred_checked={} pred_mean={:.4} pred_max={:.4} max_correction={:.3} rubber_bands={} late_cmds={} extra_lead={:.2} rtt_ms={:.1} loss={:.3}",
        s.shots,
        s.hits,
        s.accuracy_pct(),
        s.damage_dealt,
        s.damage_taken,
        s.healed,
        s.lifesteal_earned,
        s.kills,
        s.deaths,
        s.fights_won,
        s.fights_lost,
        bot.snapshots,
        p.checked,
        p.mean_error(),
        p.max_error,
        p.max_correction,
        p.rubber_bands,
        bot.client.late_cmds,
        bot.client.extra_lead_ticks,
        bot.client.conn.stats.rtt_us / 1000.0,
        bot.client.conn.stats.loss()
    );
    bot.disconnect(start.elapsed().as_micros() as u64);
    Ok(())
}

fn main() {
    if let Err(e) = run() {
        eprintln!("lgduel-bot: {e}");
        std::process::exit(1);
    }
}
