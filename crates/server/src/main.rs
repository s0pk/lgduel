fn main() {
    println!(
        "lgduel-server {} (sim tick {} Hz, protocol v{})",
        env!("CARGO_PKG_VERSION"),
        lgduel_sim::DEFAULT_TICK_RATE_HZ,
        lgduel_net::PROTOCOL_VERSION
    );
}
