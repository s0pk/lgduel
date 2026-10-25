//! Headless match test: real server + two bot processes (+ netem proxy), then
//! assertions on the results. Used by `scripts/check.sh`.

use crate::netem::{Netem, Profile};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

pub struct MatchResult {
    pub profile: String,
    pub lines: Vec<String>,
    pub failures: Vec<String>,
}

fn exe(dir: &Path, name: &str) -> PathBuf {
    dir.join(format!("{name}{}", std::env::consts::EXE_SUFFIX))
}

fn free_udp_port() -> std::io::Result<u16> {
    Ok(std::net::UdpSocket::bind("127.0.0.1:0")?
        .local_addr()?
        .port())
}

/// Parse `key=value` pairs of a BOTRESULT line.
fn kv(line: &str) -> HashMap<String, String> {
    line.split_whitespace()
        .filter_map(|t| {
            t.split_once('=')
                .map(|(k, v)| (k.to_string(), v.to_string()))
        })
        .collect()
}

pub fn run_match(
    bin_dir: &Path,
    profile_name: &str,
    map: &str,
    secs: f64,
    out_dir: &Path,
) -> MatchResult {
    let mut r = MatchResult {
        profile: format!("{profile_name}@{map}"),
        lines: vec![],
        failures: vec![],
    };
    let fail = |r: &mut MatchResult, m: String| r.failures.push(m);
    let Some(profile) = Profile::named(profile_name) else {
        fail(&mut r, format!("unknown profile {profile_name}"));
        return r;
    };
    let _ = std::fs::create_dir_all(out_dir);
    let stats_path = out_dir.join(format!("match_{profile_name}_stats.toml"));
    let _ = std::fs::remove_file(&stats_path);
    let port = match free_udp_port() {
        Ok(p) => p,
        Err(e) => {
            fail(&mut r, format!("no free port: {e}"));
            return r;
        }
    };
    let server_addr: SocketAddr = ([127, 0, 0, 1], port).into();
    let server = Command::new(exe(bin_dir, "lgduel-server"))
        .args([
            "--bind",
            "127.0.0.1",
            "--port",
            &port.to_string(),
            "--duration-secs",
            &format!("{}", secs + 3.0),
            "--map",
            map,
        ])
        .arg("--stats-file")
        .arg(&stats_path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();
    let server = match server {
        Ok(s) => s,
        Err(e) => {
            fail(&mut r, format!("start server: {e}"));
            return r;
        }
    };
    std::thread::sleep(Duration::from_millis(300));
    let netem = match Netem::start(
        "127.0.0.1:0".parse().unwrap(),
        server_addr,
        profile,
        port as u64,
    ) {
        Ok(n) => n,
        Err(e) => {
            fail(&mut r, format!("start netem: {e}"));
            return r;
        }
    };
    let target = netem.listen_addr.to_string();
    let bots: Vec<_> = [("hard", "1"), ("medium", "2")]
        .iter()
        .map(|(skill, seed)| {
            Command::new(exe(bin_dir, "lgduel-bot"))
                .args([
                    "--server", &target, "--skill", skill, "--name", skill, "--seed", seed,
                ])
                .args(["--duration-secs", &format!("{secs}")])
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
        })
        .collect();
    let mut bot_results = Vec::new();
    for (i, b) in bots.into_iter().enumerate() {
        match b.and_then(|c| c.wait_with_output()) {
            Ok(out) => {
                let stdout = String::from_utf8_lossy(&out.stdout).to_string();
                let stderr = String::from_utf8_lossy(&out.stderr).to_string();
                if !out.status.success() {
                    fail(
                        &mut r,
                        format!("bot {i} exited with {}: {}", out.status, stderr.trim()),
                    );
                }
                if stderr.contains("panicked") {
                    fail(&mut r, format!("bot {i} panicked: {}", stderr.trim()));
                }
                if !stdout.contains("connected to") {
                    fail(&mut r, format!("bot {i} never connected"));
                }
                match stdout.lines().find(|l| l.starts_with("BOTRESULT")) {
                    Some(l) => {
                        r.lines.push(l.to_string());
                        bot_results.push(kv(l));
                    }
                    None => fail(&mut r, format!("bot {i} printed no BOTRESULT")),
                }
            }
            Err(e) => fail(&mut r, format!("bot {i}: {e}")),
        }
    }
    // Keep the proxy running until the server exits: the bots' Disconnect
    // messages are still in flight (delayed) when the bots quit.
    match server.wait_with_output() {
        Ok(out) => {
            let stderr = String::from_utf8_lossy(&out.stderr);
            if !out.status.success() || stderr.contains("panicked") {
                fail(
                    &mut r,
                    format!("server failed ({}): {}", out.status, stderr.trim()),
                );
            }
            let stdout = String::from_utf8_lossy(&out.stdout);
            if stdout.matches(" connected from ").count() != 2 {
                fail(&mut r, "server did not see both clients connect".into());
            }
        }
        Err(e) => fail(&mut r, format!("server: {e}")),
    }
    let counters = netem.stop();
    r.lines.push(format!(
        "NETEM {profile_name}: {:?}, forwarded {}, dropped {}",
        profile, counters.forwarded, counters.dropped
    ));

    // Authoritative checks from the server's stats file.
    let stats = std::fs::read_to_string(&stats_path);
    let doc: Option<toml::Table> = stats.as_ref().ok().and_then(|s| toml::from_str(s).ok());
    match doc {
        None => fail(
            &mut r,
            format!("stats file {} missing or unreadable", stats_path.display()),
        ),
        Some(doc) => {
            let pct = doc
                .get("lifesteal_percent")
                .and_then(|v| v.as_integer())
                .unwrap_or(-1);
            let players = doc
                .get("players")
                .and_then(|p| p.as_array())
                .cloned()
                .unwrap_or_default();
            if players.len() != 2 {
                fail(
                    &mut r,
                    format!("stats file lists {} players", players.len()),
                );
            }
            for p in &players {
                let g = |k: &str| p.get(k).and_then(|v| v.as_integer()).unwrap_or(-1);
                let name = p.get("name").and_then(|v| v.as_str()).unwrap_or("?");
                let (dealt, earned, healed) =
                    (g("damage_dealt"), g("lifesteal_earned"), g("healed"));
                r.lines.push(format!(
                    "STATS {profile_name} {name}: dealt {dealt} taken {} earned {earned} healed {healed} acc {:.1}% kills {} deaths {} missing_cmds {}",
                    g("damage_taken"),
                    p.get("accuracy_pct").and_then(|v| v.as_float()).unwrap_or(-1.0),
                    g("kills"),
                    g("deaths"),
                    g("missing_cmds")
                ));
                if dealt <= 0 {
                    fail(&mut r, format!("{name} dealt no damage"));
                }
                if earned != dealt * pct / 100 {
                    fail(
                        &mut r,
                        format!("{name}: lifesteal earned {earned} != {pct}% of {dealt}"),
                    );
                }
                if healed > earned || healed < 0 {
                    fail(
                        &mut r,
                        format!("{name}: healed {healed} not within earned {earned}"),
                    );
                }
            }
        }
    }
    // Desync: own-movement prediction must stay exact-ish.
    for b in &bot_results {
        let mean: f64 = b
            .get("pred_mean")
            .and_then(|v| v.parse().ok())
            .unwrap_or(f64::MAX);
        let checked: u64 = b
            .get("pred_checked")
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        if checked < 100 || mean >= 0.5 {
            fail(
                &mut r,
                format!(
                    "prediction: {} checked, mean error {mean} (threshold 0.5u)",
                    checked
                ),
            );
        }
        // Rubber-band detector: visible own-view snap-backs (> 1u, not caused by
        // being hit/killed) — allow at most 1 per 10 s of play.
        let bands: u64 = b
            .get("rubber_bands")
            .and_then(|v| v.parse().ok())
            .unwrap_or(u64::MAX);
        let allowed = ((secs - 5.0).max(1.0) / 10.0).ceil() as u64; // bots measure after a 5 s warm-up
        if bands > allowed {
            fail(
                &mut r,
                format!("{bands} rubber-band corrections (allowed {allowed})"),
            );
        }
    }
    r
}

/// Online smoke test of the real game client: server + one bot + `lgduel
/// --headless --connect`; the client must connect, see and render the bot,
/// and be told it plays in arena 1. With `spectate`: two bots duel and the
/// client watches them (must render them and be told it is spectating).
pub fn run_client_smoke(bin_dir: &Path, out_dir: &Path, secs: f64, spectate: bool) -> MatchResult {
    let mut r = MatchResult {
        profile: if spectate {
            "client-spectate"
        } else {
            "client-smoke"
        }
        .into(),
        lines: vec![],
        failures: vec![],
    };
    let _ = std::fs::create_dir_all(out_dir);
    let port = match free_udp_port() {
        Ok(p) => p,
        Err(e) => {
            r.failures.push(format!("no free port: {e}"));
            return r;
        }
    };
    let addr = format!("127.0.0.1:{port}");
    let server = Command::new(exe(bin_dir, "lgduel-server"))
        .args([
            "--bind",
            "127.0.0.1",
            "--port",
            &port.to_string(),
            "--duration-secs",
            &format!("{}", secs + 4.0),
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();
    let Ok(server) = server else {
        r.failures.push("start server failed".into());
        return r;
    };
    std::thread::sleep(Duration::from_millis(300));
    let bot_names: &[&str] = if spectate {
        &["smokebot", "smokebot2"]
    } else {
        &["smokebot"]
    };
    let bots: Vec<_> = bot_names
        .iter()
        .map(|name| {
            Command::new(exe(bin_dir, "lgduel-bot"))
                .args([
                    "--server",
                    &addr,
                    "--pattern",
                    "ad",
                    "--skill",
                    "medium",
                    "--name",
                    name,
                ])
                .args(["--duration-secs", &format!("{}", secs + 2.0)])
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
        })
        .collect();
    if spectate {
        // Let the two bots take arena 1 first.
        std::thread::sleep(Duration::from_millis(500));
    }
    let shot = out_dir.join(if spectate {
        "online_spectate.png"
    } else {
        "online.png"
    });
    let client = Command::new(exe(bin_dir, "lgduel"))
        .args([
            "--headless",
            "--no-settings",
            "--connect",
            &addr,
            "--name",
            "smokeclient",
            "--secs",
            &format!("{secs}"),
        ])
        .args(spectate.then_some("--spectate"))
        .args(["--size", "1280x720", "--screenshot"])
        .arg(&shot)
        .output();
    match client {
        Ok(out) => {
            let stdout = String::from_utf8_lossy(&out.stdout);
            for l in stdout.lines().filter(|l| {
                l.starts_with("ONLINE")
                    || l.starts_with("IMAGE")
                    || l.starts_with("SCREENSHOT")
                    || l.starts_with("NETGRAPH")
                    || l.starts_with("STATUS")
            }) {
                r.lines.push(l.to_string());
            }
            let want = if spectate {
                "STATUS SPECTATING ARENA 1"
            } else {
                "STATUS ARENA 1 - VS SMOKEBOT"
            };
            if !stdout.contains(want) {
                r.failures
                    .push(format!("client status line is not {want:?}"));
            }
            if !out.status.success() || !stdout.contains("ONLINE CHECK OK") {
                r.failures.push(format!(
                    "client failed ({}): {}",
                    out.status,
                    String::from_utf8_lossy(&out.stderr).trim()
                ));
            }
        }
        Err(e) => r.failures.push(format!("run client: {e}")),
    }
    for bot in bots {
        if let Ok(b) = bot.and_then(|b| b.wait_with_output())
            && !b.status.success()
        {
            r.failures.push(format!("bot exited with {}", b.status));
        }
    }
    match server.wait_with_output() {
        Ok(out) => {
            let s = String::from_utf8_lossy(&out.stdout);
            if !s.contains("smokeclient connected from") {
                r.failures
                    .push("server never saw the client connect".into());
            }
            if !out.status.success() || String::from_utf8_lossy(&out.stderr).contains("panicked") {
                r.failures.push("server failed".into());
            }
        }
        Err(e) => r.failures.push(format!("server: {e}")),
    }
    r
}

/// Join codes on real sockets: `lgduel-server relay` + a host registered with
/// it + a bot joining by code (direct path) + the real client joining by code
/// through the relay (`--force-relay`), then the client again by direct code
/// (the host's address, no rendezvous). Everything on 127.0.0.1, so no NAT:
/// the NAT behaviour itself is covered by server/tests/p2p.rs on MemNet.
pub fn run_p2p_smoke(bin_dir: &Path, out_dir: &Path, secs: f64) -> MatchResult {
    use std::io::{BufRead, BufReader};
    let mut r = MatchResult {
        profile: "p2p".into(),
        lines: vec![],
        failures: vec![],
    };
    let _ = std::fs::create_dir_all(out_dir);
    let (Ok(rport), Ok(hport)) = (free_udp_port(), free_udp_port()) else {
        r.failures.push("no free port".into());
        return r;
    };
    let rdv = format!("127.0.0.1:{rport}");
    let total = format!("{}", 2.0 * secs + 10.0);
    let relay = Command::new(exe(bin_dir, "lgduel-server"))
        .args(["relay", "--bind", "127.0.0.1", "--port", &rport.to_string()])
        .args(["--duration-secs", &total])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();
    let Ok(relay) = relay else {
        r.failures.push("start relay failed".into());
        return r;
    };
    std::thread::sleep(Duration::from_millis(300));
    let host = Command::new(exe(bin_dir, "lgduel-server"))
        .args([
            "--bind",
            "0.0.0.0",
            "--port",
            &hport.to_string(),
            "--rendezvous",
            &rdv,
        ])
        .args(["--duration-secs", &total])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();
    let Ok(mut host) = host else {
        r.failures.push("start host failed".into());
        return r;
    };
    // Read the host's output on a thread: the join code, then everything else.
    let (tx, rx) = std::sync::mpsc::channel::<String>();
    let out = host.stdout.take().unwrap();
    let reader = std::thread::spawn(move || {
        for line in BufReader::new(out).lines().map_while(Result::ok) {
            if tx.send(line).is_err() {
                break;
            }
        }
    });
    let mut host_lines = Vec::new();
    let code = loop {
        match rx.recv_timeout(Duration::from_secs(5)) {
            Ok(l) => {
                let code = l
                    .strip_prefix("JOIN CODE ")
                    .map(|c| c.split_whitespace().next().unwrap_or("").to_string());
                host_lines.push(l);
                if let Some(c) = code {
                    break Some(c);
                }
            }
            Err(_) => break None,
        }
    };
    let Some(code) = code else {
        r.failures
            .push(format!("host never got a join code: {host_lines:?}"));
        let _ = host.kill();
        return r;
    };
    r.lines.push(format!("P2P join code {code}"));
    let bot = Command::new(exe(bin_dir, "lgduel-bot"))
        .args(["--join", &code, "--rendezvous", &rdv, "--name", "codebot"])
        .args(["--pattern", "ad", "--skill", "medium"])
        .args(["--duration-secs", &format!("{}", 2.0 * secs + 6.0)])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();
    std::thread::sleep(Duration::from_millis(500));
    let client = Command::new(exe(bin_dir, "lgduel"))
        .args([
            "--headless",
            "--no-settings",
            "--join",
            &code,
            "--rendezvous",
            &rdv,
        ])
        .args([
            "--force-relay",
            "--name",
            "relayclient",
            "--secs",
            &format!("{secs}"),
        ])
        .args(["--size", "1280x720", "--screenshot"])
        .arg(out_dir.join("online_relay.png"))
        .output();
    match client {
        Ok(o) => {
            let s = String::from_utf8_lossy(&o.stdout);
            r.lines.extend(
                s.lines()
                    .filter(|l| l.starts_with("STATUS") || l.starts_with("ONLINE CHECK"))
                    .map(|l| format!("P2P client {l}")),
            );
            if !o.status.success() || !s.contains("ONLINE CHECK OK") {
                r.failures.push(format!(
                    "relayed client failed ({}): {}",
                    o.status,
                    String::from_utf8_lossy(&o.stderr).trim()
                ));
            }
        }
        Err(e) => r.failures.push(format!("run client: {e}")),
    }
    // Same host, joined by the direct code a UPnP-mapped host would show.
    let direct = lgduel_net::rendezvous::format_direct(std::net::SocketAddrV4::new(
        std::net::Ipv4Addr::LOCALHOST,
        hport,
    ));
    r.lines.push(format!("P2P direct code {direct}"));
    let client = Command::new(exe(bin_dir, "lgduel"))
        .args(["--headless", "--no-settings", "--join", &direct])
        .args(["--name", "directclient", "--secs", &format!("{secs}")])
        .args(["--size", "1280x720"])
        .output();
    match client {
        Ok(o) => {
            let s = String::from_utf8_lossy(&o.stdout);
            r.lines.extend(
                s.lines()
                    .filter(|l| l.starts_with("ONLINE CHECK"))
                    .map(|l| format!("P2P direct-code client {l}")),
            );
            if !o.status.success() || !s.contains("ONLINE CHECK OK") {
                r.failures.push(format!(
                    "client joining by direct code failed ({}): {}",
                    o.status,
                    String::from_utf8_lossy(&o.stderr).trim()
                ));
            }
        }
        Err(e) => r.failures.push(format!("run direct client: {e}")),
    }
    match bot.and_then(|b| b.wait_with_output()) {
        Ok(b) if b.status.success() => {}
        Ok(b) => r.failures.push(format!(
            "bot joining by code failed: {}",
            String::from_utf8_lossy(&b.stderr).trim()
        )),
        Err(e) => r.failures.push(format!("bot: {e}")),
    }
    let _ = host.wait();
    let _ = reader.join();
    host_lines.extend(rx.try_iter());
    let relay_out = relay
        .wait_with_output()
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .unwrap_or_default();
    // The bot came in directly (its real address), the client through the relay
    // (a virtual relay-session address on the host).
    let joined = |name: &str| {
        host_lines
            .iter()
            .find(|l| l.starts_with(&format!("{name} connected from")))
            .cloned()
    };
    match joined("codebot") {
        Some(l) if l.contains("127.0.0.1") => r.lines.push(format!("P2P direct: {l}")),
        other => r
            .failures
            .push(format!("bot did not join directly: {other:?}")),
    }
    match joined("relayclient") {
        Some(l) if l.contains("[fd00:") => r.lines.push(format!("P2P relayed: {l}")),
        other => r
            .failures
            .push(format!("client did not join through the relay: {other:?}")),
    }
    match joined("directclient") {
        Some(l) if l.contains("127.0.0.1") => r.lines.push(format!("P2P direct code: {l}")),
        other => r
            .failures
            .push(format!("client did not join by direct code: {other:?}")),
    }
    if !relay_out.contains("relay 127.0.0.1") {
        r.failures
            .push(format!("relay never opened a session: {relay_out}"));
    }
    r
}
