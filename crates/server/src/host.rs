//! Running a server for real: argument parsing and the real-time loop, shared
//! by `lgduel-server` and `lgduel host`.

use crate::{Server, ServerConfig};
use lgduel_net::transport::{Transport, UdpTransport};
use lgduel_sim::math::Vec3;
use lgduel_sim::tick::FixedStepper;
use lgduel_sim::world::{World, builtin};
use std::net::{IpAddr, UdpSocket};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Default, PartialEq)]
pub struct HostArgs {
    pub config: Option<String>,
    pub port: Option<u16>,
    pub bind: Option<String>,
    pub duration: Option<f64>,
    pub stats_file: Option<String>,
    /// Built-in map name or map file path (overrides the config file).
    pub map: Option<String>,
}

pub const USAGE: &str = "[--config server.toml] [--port N] [--bind ADDR] [--map NAME|FILE] [--duration-secs N] [--stats-file out.toml]";

pub fn parse_args(args: impl IntoIterator<Item = String>) -> Result<HostArgs, String> {
    let mut a = HostArgs::default();
    let mut it = args.into_iter();
    while let Some(arg) = it.next() {
        let mut val = || it.next().ok_or(format!("{arg} needs a value"));
        match arg.as_str() {
            "--config" => a.config = Some(val()?),
            "--port" => a.port = Some(val()?.parse().map_err(|_| "--port needs a number")?),
            "--bind" => a.bind = Some(val()?),
            "--map" => a.map = Some(val()?),
            "--stats-file" => a.stats_file = Some(val()?),
            "--duration-secs" => {
                a.duration = Some(
                    val()?
                        .parse()
                        .map_err(|_| "--duration-secs needs a number")?,
                )
            }
            other => return Err(format!("unknown argument: {other} (usage: {USAGE})")),
        }
    }
    Ok(a)
}

pub fn load_world(map: &str, cfg: &ServerConfig) -> Result<World, String> {
    let p = &cfg.game.physics;
    let hull = (
        Vec3::from_array(p.player_mins),
        Vec3::from_array(p.player_maxs),
    );
    let text = match builtin::source(map) {
        Some(src) => src.to_string(),
        None => std::fs::read_to_string(map).map_err(|e| {
            let names: Vec<&str> = builtin::ALL.iter().map(|(n, _)| *n).collect();
            format!("map {map}: {e} (built-in maps: {})", names.join(", "))
        })?,
    };
    World::from_toml_str(&text, hull).map_err(|e| e.to_string())
}

/// This machine's address on the LAN (the interface used for outbound
/// traffic). Uses a connected UDP socket; no packet is sent.
pub fn lan_ip() -> Option<IpAddr> {
    let s = UdpSocket::bind("0.0.0.0:0").ok()?;
    s.connect("192.0.2.1:9").ok()?; // TEST-NET-1, never routed
    let ip = s.local_addr().ok()?.ip();
    (!ip.is_unspecified() && !ip.is_loopback()).then_some(ip)
}

/// Load config, bind, and run the server until `duration` (or forever).
pub fn run(args: &HostArgs) -> Result<(), String> {
    let cfg = config_from_args(args)?;
    let never = std::sync::atomic::AtomicBool::new(false);
    let admin = Admin {
        config_path: args.config.clone(),
        console: true,
    };
    run_config(
        cfg,
        args.duration,
        args.stats_file.as_deref(),
        &never,
        admin,
        |_| {},
    )
}

/// Administration options for `run_config`.
#[derive(Debug, Clone, Default)]
pub struct Admin {
    /// File `reload` re-reads.
    pub config_path: Option<String>,
    /// Read console commands from stdin (`help` lists them).
    pub console: bool,
}

/// Lines typed into the terminal, read on a background thread (EOF just ends it).
fn stdin_lines() -> std::sync::mpsc::Receiver<String> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for line in std::io::stdin().lines() {
            let Ok(line) = line else { break };
            if tx.send(line).is_err() {
                break;
            }
        }
    });
    rx
}

/// The server config described by command-line arguments.
pub fn config_from_args(args: &HostArgs) -> Result<ServerConfig, String> {
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
    if let Some(b) = &args.bind {
        cfg.server.bind = b.clone();
    }
    if let Some(m) = &args.map {
        cfg.server.map = m.clone();
    }
    Ok(cfg)
}

/// Run a server with `cfg` until `duration` elapses or `stop` is set.
/// `on_bound` is told the bound address (or why binding failed) right after
/// the socket is opened, so a caller on another thread can report errors.
pub fn run_config(
    cfg: ServerConfig,
    duration: Option<f64>,
    stats_file: Option<&str>,
    stop: &std::sync::atomic::AtomicBool,
    admin: Admin,
    on_bound: impl FnOnce(Result<std::net::SocketAddr, String>),
) -> Result<(), String> {
    let bound: Result<(World, UdpTransport), String> = (|| {
        let world = load_world(&cfg.server.map.clone(), &cfg)?;
        let addr: std::net::SocketAddr = format!("{}:{}", cfg.server.bind, cfg.server.port)
            .parse()
            .map_err(|e| format!("bad bind address: {e}"))?;
        let transport =
            UdpTransport::bind(addr).map_err(|e| format!("can't use port {}: {e}", addr.port()))?;
        Ok((world, transport))
    })();
    let (world, transport) = match bound {
        Ok(v) => {
            on_bound(Ok(v.1.local_addr()));
            v
        }
        Err(e) => {
            on_bound(Err(e.clone()));
            return Err(e);
        }
    };
    let secret = std::process::id() as u64
        ^ (std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos()) as u64);
    let rate = cfg.game.tick_rate_hz;
    let local = transport.local_addr();
    println!(
        "lgduel-server '{}' listening on {} | {} Hz | {:?} | lifesteal {}% | {:?} | map {}",
        cfg.server.name,
        local,
        rate,
        cfg.game.rules.mode,
        cfg.game.rules.lifesteal_percent,
        cfg.game.rules.hit_mode,
        world.name
    );
    if duration.is_none() {
        let port = local.port();
        println!("  players on this PC:      lgduel --connect 127.0.0.1:{port}");
        if let Some(ip) = lan_ip() {
            println!("  players on your network: lgduel --connect {ip}:{port}");
        }
        println!(
            "  over the internet: forward UDP port {port} on your router and share your public IP (see PLAY.md)"
        );
    }
    if !cfg.server.rcon_password.is_empty() {
        println!("  remote console on (rcon_password set): lgduel-server rcon ADDRESS COMMAND");
    }
    let mut server = Server::new(transport, cfg, world, secret);
    server.config_path = admin.config_path;
    let console = admin.console.then(stdin_lines);
    if console.is_some() {
        println!("  console: type help for commands");
    }
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
        if let Some(rx) = &console {
            while let Ok(line) = rx.try_recv() {
                let out = server.command(&line);
                if !out.is_empty() {
                    println!("{}", out.trim_end());
                }
            }
        }
        for line in &server.log[logged..] {
            println!("{line}");
        }
        logged = server.log.len();
        let stopping = stop.load(std::sync::atomic::Ordering::Relaxed);
        if stopping || duration.is_some_and(|d| (now - start).as_secs_f64() >= d) {
            println!(
                "shutting down after {:.1} s, {} ticks, {} clients",
                (now - start).as_secs_f64(),
                server.game().tick.0,
                server.client_count()
            );
            if let Some(path) = stats_file {
                std::fs::write(path, server.stats_toml()).map_err(|e| format!("{path}: {e}"))?;
                println!("stats written to {path}");
            }
            return Ok(());
        }
        // Sleep most of the way to the next tick, polling the socket at ≥ 1 kHz.
        let next = tick_dur.saturating_sub(Duration::from_micros(
            (stepper.alpha() as f64 * tick_dur.as_micros() as f64) as u64,
        ));
        std::thread::sleep(next.min(Duration::from_millis(1)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(s: &str) -> Vec<String> {
        s.split_whitespace().map(String::from).collect()
    }

    #[test]
    fn parses_host_args() {
        assert_eq!(parse_args(args("")).unwrap(), HostArgs::default());
        let a = parse_args(args(
            "--port 1234 --bind 127.0.0.1 --duration-secs 2.5 --config c.toml --stats-file s.toml --map arena_tiers",
        ))
        .unwrap();
        assert_eq!(a.map.as_deref(), Some("arena_tiers"));
        assert_eq!(a.port, Some(1234));
        assert_eq!(a.bind.as_deref(), Some("127.0.0.1"));
        assert_eq!(a.duration, Some(2.5));
        assert!(parse_args(args("--port x")).is_err());
        assert!(parse_args(args("--bogus")).is_err());
    }

    #[test]
    fn lan_ip_is_not_loopback() {
        if let Some(ip) = lan_ip() {
            assert!(!ip.is_loopback());
        }
    }
}
