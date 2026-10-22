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

pub fn run_match(bin_dir: &Path, profile_name: &str, secs: f64, out_dir: &Path) -> MatchResult {
    let mut r = MatchResult {
        profile: profile_name.to_string(),
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
    let counters = netem.stop();
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
                    "STATS {profile_name} {name}: dealt {dealt} taken {} earned {earned} healed {healed} acc {:.1}% kills {} deaths {}",
                    g("damage_taken"),
                    p.get("accuracy_pct").and_then(|v| v.as_float()).unwrap_or(-1.0),
                    g("kills"),
                    g("deaths")
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
        let allowed = (secs / 10.0).ceil() as u64;
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
/// --headless --connect`; the client must connect, see and render the bot.
pub fn run_client_smoke(bin_dir: &Path, out_dir: &Path, secs: f64) -> MatchResult {
    let mut r = MatchResult {
        profile: "client-smoke".into(),
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
    let bot = Command::new(exe(bin_dir, "lgduel-bot"))
        .args([
            "--server",
            &addr,
            "--pattern",
            "ad",
            "--skill",
            "medium",
            "--name",
            "smokebot",
        ])
        .args(["--duration-secs", &format!("{}", secs + 2.0)])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();
    let shot = out_dir.join("online.png");
    let client = Command::new(exe(bin_dir, "lgduel"))
        .args([
            "--headless",
            "--connect",
            &addr,
            "--name",
            "smokeclient",
            "--secs",
            &format!("{secs}"),
        ])
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
            }) {
                r.lines.push(l.to_string());
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
    if let Ok(b) = bot.and_then(|b| b.wait_with_output())
        && !b.status.success()
    {
        r.failures.push(format!("bot exited with {}", b.status));
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
