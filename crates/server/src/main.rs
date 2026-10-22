//! `lgduel-server [--config server.toml] [--port N] [--bind ADDR] [--duration-secs N] [--stats-file F]`
//! (same as `lgduel host …`)

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("--version") {
        println!(
            "lgduel-server {} (sim tick {} Hz, protocol v{})",
            env!("CARGO_PKG_VERSION"),
            lgduel_sim::DEFAULT_TICK_RATE_HZ,
            lgduel_net::PROTOCOL_VERSION
        );
        return;
    }
    let result = lgduel_server::host::parse_args(args).and_then(|a| lgduel_server::host::run(&a));
    if let Err(e) = result {
        eprintln!("lgduel-server: {e}");
        std::process::exit(1);
    }
}
