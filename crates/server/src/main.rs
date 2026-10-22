//! `lgduel-server [--config server.toml] [--port N] [--bind ADDR] [--duration-secs N]`

use lgduel_net::transport::{Transport, UdpTransport};
use lgduel_server::{Server, ServerConfig};
use lgduel_sim::math::Vec3;
use lgduel_sim::tick::FixedStepper;
use lgduel_sim::world::{World, builtin};
use std::time::{Duration, Instant};

struct Args {
    config: Option<String>,
    port: Option<u16>,
    bind: Option<String>,
    duration: Option<f64>,
    stats_file: Option<String>,
}

fn parse_args() -> Result<Args, String> {
    let mut a = Args {
        config: None,
        port: None,
        bind: None,
        duration: None,
        stats_file: None,
    };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        let mut val = || it.next().ok_or(format!("{arg} needs a value"));
        match arg.as_str() {
            "--config" => a.config = Some(val()?),
            "--port" => a.port = Some(val()?.parse().map_err(|_| "--port needs a number")?),
            "--bind" => a.bind = Some(val()?),
            "--stats-file" => a.stats_file = Some(val()?),
            "--duration-secs" => {
                a.duration = Some(
                    val()?
                        .parse()
                        .map_err(|_| "--duration-secs needs a number")?,
                )
            }
            "--version" => {
                println!(
                    "lgduel-server {} (sim tick {} Hz, protocol v{})",
                    env!("CARGO_PKG_VERSION"),
                    lgduel_sim::DEFAULT_TICK_RATE_HZ,
                    lgduel_net::PROTOCOL_VERSION
                );
                std::process::exit(0);
            }
            other => return Err(format!("unknown argument: {other}")),
        }
    }
    Ok(a)
}

fn load_world(map: &str, cfg: &ServerConfig) -> Result<World, String> {
    let p = &cfg.game.physics;
    let hull = (
        Vec3::from_array(p.player_mins),
        Vec3::from_array(p.player_maxs),
    );
    let text = match map {
        "arena_flat" => builtin::ARENA_FLAT.to_string(),
        path => std::fs::read_to_string(path).map_err(|e| format!("map {path}: {e}"))?,
    };
    World::from_toml_str(&text, hull).map_err(|e| e.to_string())
}

fn run() -> Result<(), String> {
    let args = parse_args()?;
    let mut cfg = match &args.config {
        Some(path) => {
            let text = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
            ServerConfig::from_toml_str(&text).map_err(|e| format!("{path}: {e}"))?
        }
        None => ServerConfig::default(),
    };
    if let Some(p) = args.port {
        cfg.server.port = p;
    }
    if let Some(b) = args.bind {
        cfg.server.bind = b;
    }
    let world = load_world(&cfg.server.map.clone(), &cfg)?;
    let addr = format!("{}:{}", cfg.server.bind, cfg.server.port)
        .parse()
        .map_err(|e| format!("bad bind address: {e}"))?;
    let transport = UdpTransport::bind(addr).map_err(|e| format!("bind {addr}: {e}"))?;
    let secret = Instant::now().elapsed().as_nanos() as u64
        ^ std::process::id() as u64
        ^ (std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos()) as u64);
    let rate = cfg.game.tick_rate_hz;
    println!(
        "lgduel-server '{}' listening on {} | {} Hz | {:?} | lifesteal {}% | {:?} | map {}",
        cfg.server.name,
        transport.local_addr(),
        rate,
        cfg.game.rules.mode,
        cfg.game.rules.lifesteal_percent,
        cfg.game.rules.hit_mode,
        world.name
    );
    let mut server = Server::new(transport, cfg, world, secret);
    let start = Instant::now();
    let mut stepper = FixedStepper::new(rate);
    let mut last = start;
    let tick_dur = Duration::from_micros(1_000_000 / rate as u64);
    let mut logged = 0;
    loop {
        let now = Instant::now();
        let now_us = (now - start).as_micros() as u64;
        server.poll(now_us);
        let (_, n) = stepper.advance((now - last).as_micros() as u64);
        last = now;
        for _ in 0..n {
            server.tick(now_us);
        }
        for line in &server.log[logged..] {
            println!("{line}");
        }
        logged = server.log.len();
        if args
            .duration
            .is_some_and(|d| (now - start).as_secs_f64() >= d)
        {
            println!(
                "shutting down after {:.1} s, {} ticks, {} clients",
                (now - start).as_secs_f64(),
                server.game.tick.0,
                server.client_count()
            );
            if let Some(path) = &args.stats_file {
                std::fs::write(path, server.stats_toml()).map_err(|e| format!("{path}: {e}"))?;
                println!("stats written to {path}");
            };
            return Ok(());
        }
        // Sleep most of the way to the next tick, polling the socket at ≥ 1 kHz.
        let next = tick_dur.saturating_sub(Duration::from_micros(
            (stepper.alpha() as f64 * tick_dur.as_micros() as f64) as u64,
        ));
        std::thread::sleep(next.min(Duration::from_millis(1)));
    }
}

fn main() {
    if let Err(e) = run() {
        eprintln!("lgduel-server: {e}");
        std::process::exit(1);
    }
}
