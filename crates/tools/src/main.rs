//! `lgduel-tools` — developer tools.
//!
//!   lgduel-tools netem --listen ADDR --upstream ADDR [--profile clean|lossy|highping] [--secs N]
//!   lgduel-tools client-smoke [--secs 5] [--out artifacts]   (real client online, headless)
//!   lgduel-tools match [--secs 60] [--profiles clean,lossy,highping] [--maps arena_flat,...] [--out artifacts]
//!       Runs the headless match test (profiles in parallel); exit 1 on any failure.

mod matchrun;
mod netem;

use std::path::PathBuf;

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1).cloned())
}

fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("--version") => {
            println!(
                "lgduel-tools {} (sim tick {} Hz, protocol v{})",
                env!("CARGO_PKG_VERSION"),
                lgduel_sim::DEFAULT_TICK_RATE_HZ,
                lgduel_net::PROTOCOL_VERSION
            );
            Ok(())
        }
        Some("netem") => {
            let listen = arg(&args, "--listen")
                .ok_or("--listen required")?
                .parse()
                .map_err(|e| format!("{e}"))?;
            let upstream = arg(&args, "--upstream")
                .ok_or("--upstream required")?
                .parse()
                .map_err(|e| format!("{e}"))?;
            let profile = netem::Profile::named(&arg(&args, "--profile").unwrap_or("lossy".into()))
                .ok_or("bad --profile")?;
            let secs: f64 = arg(&args, "--secs")
                .map_or(Ok(3600.0), |s| s.parse())
                .map_err(|_| "bad --secs")?;
            let n = netem::Netem::start(listen, upstream, profile, 1).map_err(|e| e.to_string())?;
            println!("netem {:?} on {} → {upstream}", profile, n.listen_addr);
            std::thread::sleep(std::time::Duration::from_secs_f64(secs));
            println!("{:?}", n.stop());
            Ok(())
        }
        Some("p2p-smoke") => {
            let secs: f64 = arg(&args, "--secs")
                .map_or(Ok(5.0), |s| s.parse())
                .map_err(|_| "bad --secs")?;
            let out = PathBuf::from(arg(&args, "--out").unwrap_or("artifacts".into()));
            let bin_dir = std::env::current_exe()
                .map_err(|e| e.to_string())?
                .parent()
                .unwrap()
                .to_path_buf();
            let r = matchrun::run_p2p_smoke(&bin_dir, &out, secs);
            for l in &r.lines {
                println!("{l}");
            }
            if r.failures.is_empty() {
                println!("P2P SMOKE OK");
                Ok(())
            } else {
                for f in &r.failures {
                    println!("P2P SMOKE FAIL: {f}");
                }
                Err("p2p smoke failed".into())
            }
        }
        Some("client-smoke") => {
            let secs: f64 = arg(&args, "--secs")
                .map_or(Ok(5.0), |s| s.parse())
                .map_err(|_| "bad --secs")?;
            let out = PathBuf::from(arg(&args, "--out").unwrap_or("artifacts".into()));
            let bin_dir = std::env::current_exe()
                .map_err(|e| e.to_string())?
                .parent()
                .unwrap()
                .to_path_buf();
            // Playing and spectating runs in parallel (separate servers).
            let runs: Vec<_> = [false, true]
                .into_iter()
                .map(|spectate| {
                    let (bin_dir, out) = (bin_dir.clone(), out.clone());
                    std::thread::spawn(move || {
                        matchrun::run_client_smoke(&bin_dir, &out, secs, spectate)
                    })
                })
                .collect();
            let mut failed = false;
            for h in runs {
                let r = h.join().map_err(|_| "client smoke thread panicked")?;
                for l in &r.lines {
                    println!("{} {l}", r.profile);
                }
                if r.failures.is_empty() {
                    println!("CLIENT SMOKE {} OK", r.profile);
                } else {
                    failed = true;
                    for f in &r.failures {
                        println!("CLIENT SMOKE {} FAIL: {f}", r.profile);
                    }
                }
            }
            if failed {
                Err("client smoke failed".into())
            } else {
                Ok(())
            }
        }
        Some("match") => {
            let secs: f64 = arg(&args, "--secs")
                .map_or(Ok(60.0), |s| s.parse())
                .map_err(|_| "bad --secs")?;
            let profiles = arg(&args, "--profiles").unwrap_or("clean,lossy,highping".into());
            // Map per profile (by position; the last one repeats).
            let maps = arg(&args, "--maps").unwrap_or("arena_flat".into());
            let maps: Vec<String> = maps.split(',').map(String::from).collect();
            let out = PathBuf::from(arg(&args, "--out").unwrap_or("artifacts".into()));
            let bin_dir = std::env::current_exe()
                .map_err(|e| e.to_string())?
                .parent()
                .unwrap()
                .to_path_buf();
            let handles: Vec<_> = profiles
                .split(',')
                .enumerate()
                .map(|(i, p)| {
                    let map = maps[i.min(maps.len() - 1)].clone();
                    let (p, bin_dir, out) = (p.to_string(), bin_dir.clone(), out.clone());
                    std::thread::spawn(move || matchrun::run_match(&bin_dir, &p, &map, secs, &out))
                })
                .collect();
            let mut failed = false;
            for h in handles {
                let r = h.join().map_err(|_| "match thread panicked")?;
                for l in &r.lines {
                    println!("{l}");
                }
                if r.failures.is_empty() {
                    println!("MATCH {} OK", r.profile);
                } else {
                    failed = true;
                    for f in &r.failures {
                        println!("MATCH {} FAIL: {f}", r.profile);
                    }
                }
            }
            if failed {
                Err("headless match failed".into())
            } else {
                Ok(())
            }
        }
        _ => {
            println!("lgduel-tools: subcommands: netem, match (see source header); --version");
            Ok(())
        }
    }
}

fn main() {
    if let Err(e) = run() {
        eprintln!("lgduel-tools: {e}");
        std::process::exit(1);
    }
}
