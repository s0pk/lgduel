//! UDP network emulator proxy: clients talk to the proxy, the proxy forwards to
//! the real server, delaying / jittering / dropping datagrams in both
//! directions. Each client gets its own upstream socket so the server sees
//! distinct peers.

use std::collections::{BTreeMap, HashMap};
use std::io;
use std::net::{SocketAddr, UdpSocket};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Profile {
    /// One-way delay per direction.
    pub latency_ms: f64,
    pub jitter_ms: f64,
    /// Drop probability per datagram per direction.
    pub loss: f64,
}

impl Profile {
    pub fn named(name: &str) -> Option<Profile> {
        Some(match name {
            "clean" => Profile {
                latency_ms: 0.0,
                jitter_ms: 0.0,
                loss: 0.0,
            },
            "lossy" => Profile {
                latency_ms: 30.0,
                jitter_ms: 10.0,
                loss: 0.02,
            },
            "highping" => Profile {
                latency_ms: 100.0,
                jitter_ms: 5.0,
                loss: 0.0,
            },
            _ => return None,
        })
    }
}

/// Counters for one proxy.
#[derive(Debug, Default, Clone, Copy)]
pub struct NetemCounters {
    pub forwarded: u64,
    pub dropped: u64,
}

pub struct Netem {
    pub listen_addr: SocketAddr,
    stop: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<NetemCounters>>,
}

impl Netem {
    /// Start a proxy on `listen` forwarding to `upstream` with `profile`.
    pub fn start(
        listen: SocketAddr,
        upstream: SocketAddr,
        profile: Profile,
        seed: u64,
    ) -> io::Result<Netem> {
        let front = UdpSocket::bind(listen)?;
        front.set_nonblocking(true)?;
        let listen_addr = front.local_addr()?;
        let stop = Arc::new(AtomicBool::new(false));
        let stop2 = stop.clone();
        let handle = std::thread::spawn(move || run(front, upstream, profile, seed, stop2));
        Ok(Netem {
            listen_addr,
            stop,
            handle: Some(handle),
        })
    }

    pub fn stop(mut self) -> NetemCounters {
        self.stop.store(true, Ordering::Relaxed);
        self.handle
            .take()
            .map(|h| h.join().unwrap_or_default())
            .unwrap_or_default()
    }
}

enum Dir {
    /// client → server via this client's upstream socket
    Up(SocketAddr),
    /// server → client via the front socket
    Down(SocketAddr),
}

fn run(
    front: UdpSocket,
    upstream: SocketAddr,
    p: Profile,
    seed: u64,
    stop: Arc<AtomicBool>,
) -> NetemCounters {
    let start = Instant::now();
    let mut rng = seed | 1;
    let mut rand = move || {
        rng ^= rng << 13;
        rng ^= rng >> 7;
        rng ^= rng << 17;
        (rng >> 11) as f64 / (1u64 << 53) as f64
    };
    let mut uplinks: HashMap<SocketAddr, UdpSocket> = HashMap::new();
    let mut queue: BTreeMap<(u64, u64), (Dir, Vec<u8>)> = BTreeMap::new();
    let mut seq = 0u64;
    let mut counters = NetemCounters::default();
    let mut buf = vec![0u8; 2048];
    while !stop.load(Ordering::Relaxed) {
        let now = start.elapsed().as_micros() as u64;
        let mut schedule = |dir: Dir,
                            data: Vec<u8>,
                            counters: &mut NetemCounters,
                            rand: &mut dyn FnMut() -> f64| {
            if p.loss > 0.0 && rand() < p.loss {
                counters.dropped += 1;
                return;
            }
            let jitter = (rand() * 2.0 - 1.0) * p.jitter_ms;
            let at = now + ((p.latency_ms + jitter).max(0.0) * 1000.0) as u64;
            seq += 1;
            queue.insert((at, seq), (dir, data));
        };
        // Client → proxy.
        loop {
            match front.recv_from(&mut buf) {
                Ok((n, from)) => {
                    if let std::collections::hash_map::Entry::Vacant(e) = uplinks.entry(from) {
                        let Ok(s) = UdpSocket::bind("127.0.0.1:0")
                            .and_then(|s| s.set_nonblocking(true).map(|_| s))
                        else {
                            continue;
                        };
                        e.insert(s);
                    }
                    schedule(Dir::Up(from), buf[..n].to_vec(), &mut counters, &mut rand);
                }
                Err(e) if e.kind() == io::ErrorKind::ConnectionReset => continue,
                Err(_) => break,
            }
        }
        // Server → proxy (per client uplink).
        let mut downs = Vec::new();
        for (client, s) in &uplinks {
            loop {
                match s.recv_from(&mut buf) {
                    Ok((n, _)) => downs.push((*client, buf[..n].to_vec())),
                    Err(e) if e.kind() == io::ErrorKind::ConnectionReset => continue,
                    Err(_) => break,
                }
            }
        }
        for (client, d) in downs {
            schedule(Dir::Down(client), d, &mut counters, &mut rand);
        }
        // Deliver due datagrams.
        let due: Vec<(u64, u64)> = queue.range(..(now + 1, 0)).map(|(k, _)| *k).collect();
        for k in due {
            let (dir, data) = queue.remove(&k).unwrap();
            counters.forwarded += 1;
            match dir {
                Dir::Up(client) => {
                    if let Some(s) = uplinks.get(&client) {
                        let _ = s.send_to(&data, upstream);
                    }
                }
                Dir::Down(client) => {
                    let _ = front.send_to(&data, client);
                }
            }
        }
        std::thread::sleep(Duration::from_micros(250));
    }
    counters
}

#[cfg(test)]
mod tests {
    use super::*;

    fn echo_server() -> (SocketAddr, Arc<AtomicBool>) {
        let s = UdpSocket::bind("127.0.0.1:0").unwrap();
        s.set_read_timeout(Some(Duration::from_millis(5))).unwrap();
        let addr = s.local_addr().unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let st = stop.clone();
        std::thread::spawn(move || {
            let mut b = [0u8; 64];
            while !st.load(Ordering::Relaxed) {
                if let Ok((n, from)) = s.recv_from(&mut b) {
                    let _ = s.send_to(&b[..n], from);
                }
            }
        });
        (addr, stop)
    }

    #[test]
    fn proxies_with_delay() {
        let (srv, stop) = echo_server();
        let p = Profile {
            latency_ms: 20.0,
            jitter_ms: 0.0,
            loss: 0.0,
        };
        let netem = Netem::start("127.0.0.1:0".parse().unwrap(), srv, p, 1).unwrap();
        let c = UdpSocket::bind("127.0.0.1:0").unwrap();
        c.set_read_timeout(Some(Duration::from_millis(500)))
            .unwrap();
        let t = Instant::now();
        c.send_to(b"hi", netem.listen_addr).unwrap();
        let mut b = [0u8; 8];
        let (n, from) = c.recv_from(&mut b).unwrap();
        let rtt = t.elapsed();
        assert_eq!(&b[..n], b"hi");
        assert_eq!(from, netem.listen_addr);
        assert!(rtt >= Duration::from_millis(40), "{rtt:?}");
        assert!(rtt < Duration::from_millis(120), "{rtt:?}");
        stop.store(true, Ordering::Relaxed);
        let counters = netem.stop();
        assert_eq!(counters.forwarded, 2);
    }

    #[test]
    fn profiles() {
        assert!(Profile::named("lossy").unwrap().loss > 0.0);
        assert!(Profile::named("highping").unwrap().latency_ms >= 100.0);
        assert!(Profile::named("nope").is_none());
    }
}
