//! `lgduel-server [--config server.toml] [--port N] [--bind ADDR] [--duration-secs N] [--stats-file F]`
//! (same as `lgduel host …`), or `lgduel-server rcon [--password PW] ADDRESS COMMAND…`
//! (password also from the `LGDUEL_RCON_PASSWORD` environment variable).

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
    let result = if args.first().map(String::as_str) == Some("rcon") {
        rcon(&args[1..])
    } else {
        lgduel_server::host::parse_args(args).and_then(|a| lgduel_server::host::run(&a))
    };
    if let Err(e) = result {
        eprintln!("lgduel-server: {e}");
        std::process::exit(1);
    }
}

/// Send one remote console command and print the reply.
fn rcon(args: &[String]) -> Result<(), String> {
    use lgduel_net::protocol::Connectionless;
    let mut password = std::env::var("LGDUEL_RCON_PASSWORD").unwrap_or_default();
    let mut rest = args;
    if rest.first().map(String::as_str) == Some("--password") {
        password = rest.get(1).ok_or("--password needs a value")?.clone();
        rest = &rest[2..];
    }
    let usage = "usage: lgduel-server rcon [--password PW] ADDRESS[:PORT] COMMAND...";
    let (addr, command) = rest.split_first().ok_or(usage)?;
    if command.is_empty() {
        return Err(usage.into());
    }
    if password.is_empty() {
        return Err("no password (--password or LGDUEL_RCON_PASSWORD)".into());
    }
    let addr = if addr.contains(':') {
        addr.clone()
    } else {
        format!("{addr}:27960")
    };
    let target = std::net::ToSocketAddrs::to_socket_addrs(&addr)
        .map_err(|e| format!("{addr}: {e}"))?
        .next()
        .ok_or(format!("{addr}: no address"))?;
    let socket = std::net::UdpSocket::bind("0.0.0.0:0").map_err(|e| e.to_string())?;
    socket
        .set_read_timeout(Some(std::time::Duration::from_secs(2)))
        .map_err(|e| e.to_string())?;
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(7, |d| d.as_nanos() as u64);
    let req = Connectionless::Rcon {
        nonce,
        password,
        command: command.join(" "),
    };
    socket
        .send_to(&req.encode(), target)
        .map_err(|e| e.to_string())?;
    let mut buf = [0u8; 2048];
    loop {
        let (n, _) = socket.recv_from(&mut buf).map_err(|_| {
            "no reply (wrong password, rcon_password not set on the server, or not reachable)"
                .to_string()
        })?;
        if let Some(Connectionless::RconReply { nonce: got, text }) =
            Connectionless::decode(&buf[..n])
            && got == nonce
        {
            println!("{text}");
            return Ok(());
        }
    }
}
