//! Ask the home router to forward a UDP port, so a host needs no manual port
//! forwarding: UPnP IGD first (SSDP discovery, then SOAP over HTTP), then
//! NAT-PMP (RFC 6886). Standard library only.
//!
//! Everything a router (or anything on the LAN pretending to be one) sends is
//! untrusted: responses are size-capped, every socket has a timeout, and the
//! device description is only fetched from the very address that answered the
//! SSDP search, which must be a private/loopback IPv4 address.

use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, TcpStream, UdpSocket};
use std::time::{Duration, Instant};

/// Largest HTTP response (device description / SOAP reply) we read.
const MAX_HTTP: usize = 128 * 1024;
/// Description for the router's port-mapping table.
const DESCRIPTION: &str = "LGDuel";
/// Lease asked for; renewed at half of it.
pub const LEASE_SECS: u32 = 3600;
/// NAT-PMP / PCP server port.
pub const NATPMP_PORT: u16 = 5351;

/// Where to look for a router. `system` is the real network; tests point it at fakes.
#[derive(Debug, Clone)]
pub struct Probe {
    /// SSDP multicast group (239.255.255.250:1900).
    pub ssdp: SocketAddr,
    /// Candidate gateway addresses for NAT-PMP.
    pub gateways: Vec<Ipv4Addr>,
    pub natpmp_port: u16,
    /// How long one network step waits for an answer.
    pub timeout: Duration,
    /// Accept an external address that isn't public (tests only).
    pub allow_private_external: bool,
}

impl Probe {
    pub fn system(lan: Ipv4Addr) -> Self {
        let mut gateways = Vec::new();
        if let Some(g) = default_gateway() {
            gateways.push(g);
        }
        let o = lan.octets();
        let guess = Ipv4Addr::new(o[0], o[1], o[2], 1);
        if !gateways.contains(&guess) {
            gateways.push(guess);
        }
        Self {
            ssdp: SocketAddr::from(([239, 255, 255, 250], 1900)),
            gateways,
            natpmp_port: NATPMP_PORT,
            timeout: Duration::from_millis(1500),
            allow_private_external: false,
        }
    }
}

/// An open port mapping. `close` removes it again.
#[derive(Debug)]
pub struct Mapping {
    /// The public address friends can reach this host at.
    pub external: SocketAddrV4,
    /// "UPnP" or "NAT-PMP".
    pub via: &'static str,
    internal: u16,
    lease: u32,
    how: How,
    timeout: Duration,
}

#[derive(Debug)]
enum How {
    Pmp { gateway: SocketAddrV4 },
    Upnp { ctl: Control, lan: Ipv4Addr },
}

#[derive(Debug, Clone)]
struct Control {
    addr: SocketAddrV4,
    path: String,
    /// Service type, e.g. "urn:schemas-upnp-org:service:WANIPConnection:1".
    urn: String,
}

/// Public (globally routable) IPv4: not private, loopback, link-local,
/// carrier-grade NAT (100.64/10), multicast or reserved.
pub fn is_public(ip: Ipv4Addr) -> bool {
    let o = ip.octets();
    !(ip.is_private()
        || ip.is_loopback()
        || ip.is_link_local()
        || ip.is_unspecified()
        || ip.is_broadcast()
        || ip.is_documentation()
        || o[0] == 0
        || o[0] >= 224
        || (o[0] == 100 && (o[1] & 0xC0) == 64))
}

/// Ask the router to forward UDP `port` to `lan:port`. Tries UPnP, then NAT-PMP.
pub fn open(p: &Probe, lan: Ipv4Addr, port: u16) -> Result<Mapping, String> {
    let mut errors = Vec::new();
    match upnp_open(p, lan, port) {
        Ok(m) => return finish(p, m),
        Err(e) => errors.push(format!("UPnP: {e}")),
    }
    match natpmp_open(p, port) {
        Ok(m) => return finish(p, m),
        Err(e) => errors.push(format!("NAT-PMP: {e}")),
    }
    Err(errors.join("; "))
}

/// A mapping on a router whose own address is private is useless (the ISP
/// shares one public address between customers): undo it and say why.
fn finish(p: &Probe, m: Mapping) -> Result<Mapping, String> {
    if p.allow_private_external || is_public(*m.external.ip()) {
        return Ok(m);
    }
    let ip = *m.external.ip();
    m.close();
    Err(format!(
        "the router's own address is {ip}, which isn't reachable from the internet (your ISP shares one address between customers, or there is a second router in front)"
    ))
}

impl Mapping {
    /// How long to wait before the next `renew`.
    pub fn renew_after(&self) -> Duration {
        Duration::from_secs(u64::from((self.lease / 2).max(60)))
    }

    /// Refresh the lease (the router may also move us to another external port).
    pub fn renew(&mut self) -> Result<(), String> {
        match &self.how {
            How::Upnp { ctl, lan } => add_mapping(
                ctl,
                *lan,
                self.external.port(),
                self.internal,
                self.lease,
                self.timeout,
            )
            .map_err(|e| e.1),
            How::Pmp { gateway } => {
                let (port, lease) = pmp_map(
                    *gateway,
                    self.internal,
                    self.external.port(),
                    LEASE_SECS,
                    self.timeout,
                )?;
                self.external.set_port(port);
                self.lease = lease;
                Ok(())
            }
        }
    }

    /// Remove the mapping from the router (best effort).
    pub fn close(&self) {
        match &self.how {
            How::Upnp { ctl, .. } => {
                let args = [
                    ("NewRemoteHost", String::new()),
                    ("NewExternalPort", self.external.port().to_string()),
                    ("NewProtocol", "UDP".into()),
                ];
                let _ = soap(ctl, "DeletePortMapping", &args, self.timeout);
            }
            How::Pmp { gateway } => {
                let _ = pmp_map(*gateway, self.internal, 0, 0, self.timeout);
            }
        }
    }
}

// ---------------------------------------------------------------- NAT-PMP

/// One NAT-PMP request/answer exchange (retries with doubling waits).
fn pmp_exchange(
    gw: SocketAddrV4,
    req: &[u8],
    want_op: u8,
    len: usize,
    timeout: Duration,
) -> Result<Vec<u8>, String> {
    let sock = UdpSocket::bind("0.0.0.0:0").map_err(|e| e.to_string())?;
    let mut wait = (timeout / 4).max(Duration::from_millis(50));
    for _ in 0..3 {
        sock.send_to(req, gw).map_err(|e| e.to_string())?;
        let until = Instant::now() + wait;
        let mut buf = [0u8; 64];
        loop {
            let left = until.saturating_duration_since(Instant::now());
            if left.is_zero() {
                break;
            }
            sock.set_read_timeout(Some(left))
                .map_err(|e| e.to_string())?;
            let Ok((n, from)) = sock.recv_from(&mut buf) else {
                break;
            };
            if from != SocketAddr::V4(gw) || n != len || buf[0] != 0 || buf[1] != want_op {
                continue;
            }
            let result = u16::from_be_bytes([buf[2], buf[3]]);
            if result != 0 {
                return Err(format!("router refused (result code {result})"));
            }
            return Ok(buf[..n].to_vec());
        }
        wait *= 2;
    }
    Err("no answer".into())
}

/// Map UDP `internal` → external `want` (0 = any); returns (external port, lease).
fn pmp_map(
    gw: SocketAddrV4,
    internal: u16,
    want: u16,
    lease: u32,
    timeout: Duration,
) -> Result<(u16, u32), String> {
    let mut req = vec![0u8, 1, 0, 0];
    req.extend_from_slice(&internal.to_be_bytes());
    req.extend_from_slice(&want.to_be_bytes());
    req.extend_from_slice(&lease.to_be_bytes());
    let r = pmp_exchange(gw, &req, 129, 16, timeout)?;
    if u16::from_be_bytes([r[8], r[9]]) != internal {
        return Err("answer for another port".into());
    }
    Ok((
        u16::from_be_bytes([r[10], r[11]]),
        u32::from_be_bytes([r[12], r[13], r[14], r[15]]),
    ))
}

fn natpmp_open(p: &Probe, port: u16) -> Result<Mapping, String> {
    let mut last = "no gateway address known".to_string();
    for g in &p.gateways {
        let gw = SocketAddrV4::new(*g, p.natpmp_port);
        let ext = match pmp_exchange(gw, &[0, 0], 128, 12, p.timeout) {
            Ok(r) => Ipv4Addr::new(r[8], r[9], r[10], r[11]),
            Err(e) => {
                last = format!("{g}: {e}");
                continue;
            }
        };
        let (ext_port, lease) = pmp_map(gw, port, port, LEASE_SECS, p.timeout)?;
        return Ok(Mapping {
            external: SocketAddrV4::new(ext, ext_port),
            via: "NAT-PMP",
            internal: port,
            lease,
            how: How::Pmp { gateway: gw },
            timeout: p.timeout,
        });
    }
    Err(last)
}

// ------------------------------------------------------------------- UPnP

fn upnp_open(p: &Probe, lan: Ipv4Addr, port: u16) -> Result<Mapping, String> {
    let locations = ssdp_search(p)?;
    let mut last = String::new();
    for (from, loc) in locations.into_iter().take(4) {
        let Some((addr, path)) = parse_http_url(&loc) else {
            continue;
        };
        // Only talk to the device that answered, and only on the local network.
        if addr.ip() != &from || !(from.is_private() || from.is_loopback() || from.is_link_local())
        {
            continue;
        }
        let ctl = match fetch_control(addr, &path, p.timeout) {
            Ok(Some(c)) => c,
            Ok(None) => continue, // a TV, printer...: not a gateway
            Err(e) => {
                last = e;
                continue;
            }
        };
        let reply = soap(&ctl, "GetExternalIPAddress", &[], p.timeout).map_err(|e| e.1)?;
        let ext: Ipv4Addr = tag(&reply, "NewExternalIPAddress")
            .and_then(|s| s.trim().parse().ok())
            .ok_or("the router didn't report its external address")?;
        let (ext_port, lease) = add_any(&ctl, lan, port, p.timeout)?;
        return Ok(Mapping {
            external: SocketAddrV4::new(ext, ext_port),
            via: "UPnP",
            internal: port,
            lease,
            how: How::Upnp { ctl, lan },
            timeout: p.timeout,
        });
    }
    Err(if last.is_empty() {
        "no router offering port mapping answered (UPnP is probably switched off in the router's settings)".into()
    } else {
        last
    })
}

/// Multicast an SSDP search for internet gateways; returns (responder, LOCATION).
fn ssdp_search(p: &Probe) -> Result<Vec<(Ipv4Addr, String)>, String> {
    let sock = UdpSocket::bind("0.0.0.0:0").map_err(|e| e.to_string())?;
    for st in [
        "urn:schemas-upnp-org:device:InternetGatewayDevice:1",
        "urn:schemas-upnp-org:device:InternetGatewayDevice:2",
        "upnp:rootdevice",
    ] {
        let msg = format!(
            "M-SEARCH * HTTP/1.1\r\nHOST: 239.255.255.250:1900\r\nMAN: \"ssdp:discover\"\r\nMX: 1\r\nST: {st}\r\n\r\n"
        );
        sock.send_to(msg.as_bytes(), p.ssdp)
            .map_err(|e| e.to_string())?;
    }
    let until = Instant::now() + p.timeout;
    let mut found: Vec<(Ipv4Addr, String)> = Vec::new();
    let mut buf = [0u8; 2048];
    while found.len() < 8 {
        let left = until.saturating_duration_since(Instant::now());
        if left.is_zero() {
            break;
        }
        sock.set_read_timeout(Some(left))
            .map_err(|e| e.to_string())?;
        let Ok((n, SocketAddr::V4(from))) = sock.recv_from(&mut buf) else {
            break;
        };
        let text = String::from_utf8_lossy(&buf[..n]);
        let Some(loc) = text.lines().find_map(|l| {
            let (k, v) = l.split_once(':')?;
            k.trim()
                .eq_ignore_ascii_case("location")
                .then(|| v.trim().to_string())
        }) else {
            continue;
        };
        if !found.iter().any(|(_, l)| *l == loc) {
            found.push((*from.ip(), loc));
        }
    }
    if found.is_empty() {
        return Err("no router answered the search (UPnP may be turned off)".into());
    }
    Ok(found)
}

/// `http://a.b.c.d[:port]/path` → (address, path). Hostnames are refused.
fn parse_http_url(u: &str) -> Option<(SocketAddrV4, String)> {
    let rest = u.strip_prefix("http://")?;
    let (hostport, path) = match rest.find('/') {
        Some(i) => (&rest[..i], rest[i..].to_string()),
        None => (rest, "/".to_string()),
    };
    let addr = match hostport.rsplit_once(':') {
        Some((h, p)) => SocketAddrV4::new(h.parse().ok()?, p.parse().ok()?),
        None => SocketAddrV4::new(hostport.parse().ok()?, 80),
    };
    Some((addr, path))
}

/// Contents of the first `<name>…</name>`.
fn tag<'a>(s: &'a str, name: &str) -> Option<&'a str> {
    let open = format!("<{name}>");
    let start = s.find(&open)? + open.len();
    let end = s[start..].find(&format!("</{name}>"))? + start;
    Some(&s[start..end])
}

/// Download the device description and pick the WAN connection service.
fn fetch_control(
    addr: SocketAddrV4,
    path: &str,
    timeout: Duration,
) -> Result<Option<Control>, String> {
    let req = format!("GET {path} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n");
    let (status, body) = http(addr, &req, timeout)?;
    if status != 200 {
        return Err(format!("device description: HTTP {status}"));
    }
    let mut best: Option<(u8, String, String)> = None;
    let mut rest = body.as_str();
    while let Some(i) = rest.find("<serviceType>") {
        rest = &rest[i..];
        let Some(urn) = tag(rest, "serviceType") else {
            break;
        };
        // Services don't nest: this one's controlURL comes before the next service.
        let block_end = rest[1..]
            .find("<serviceType>")
            .map_or(rest.len(), |n| n + 1);
        let rank = if urn.contains(":WANIPConnection:") {
            2
        } else if urn.contains(":WANPPPConnection:") {
            1
        } else {
            0
        };
        if rank > 0
            && let Some(url) = tag(&rest[..block_end], "controlURL")
            && best.as_ref().is_none_or(|b| rank > b.0)
        {
            best = Some((rank, urn.trim().to_string(), url.trim().to_string()));
        }
        rest = &rest[1..];
    }
    let Some((_, urn, url)) = best else {
        return Ok(None);
    };
    let (ctl_addr, ctl_path) = if url.starts_with("http://") {
        parse_http_url(&url).ok_or("bad control URL")?
    } else if url.starts_with('/') {
        (addr, url)
    } else {
        let dir = &path[..path.rfind('/').map_or(0, |i| i + 1)];
        (addr, format!("{dir}{url}"))
    };
    if ctl_addr.ip() != addr.ip() {
        return Err("the control URL points at another device".into());
    }
    Ok(Some(Control {
        addr: ctl_addr,
        path: ctl_path,
        urn,
    }))
}

/// SOAP action; the error is (UPnP error code, text).
fn soap(
    ctl: &Control,
    action: &str,
    args: &[(&str, String)],
    timeout: Duration,
) -> Result<String, (Option<u32>, String)> {
    let mut inner = String::new();
    for (k, v) in args {
        inner += &format!("<{k}>{v}</{k}>");
    }
    let body = format!(
        "<?xml version=\"1.0\"?><s:Envelope xmlns:s=\"http://schemas.xmlsoap.org/soap/envelope/\" s:encodingStyle=\"http://schemas.xmlsoap.org/soap/encoding/\"><s:Body><u:{action} xmlns:u=\"{urn}\">{inner}</u:{action}></s:Body></s:Envelope>",
        urn = ctl.urn
    );
    let req = format!(
        "POST {path} HTTP/1.1\r\nHost: {addr}\r\nContent-Type: text/xml; charset=\"utf-8\"\r\nSOAPAction: \"{urn}#{action}\"\r\nContent-Length: {len}\r\nConnection: close\r\n\r\n{body}",
        path = ctl.path,
        addr = ctl.addr,
        urn = ctl.urn,
        len = body.len()
    );
    let (status, reply) = http(ctl.addr, &req, timeout).map_err(|e| (None, e))?;
    if status == 200 {
        return Ok(reply);
    }
    let code = tag(&reply, "errorCode").and_then(|c| c.trim().parse().ok());
    let text = tag(&reply, "errorDescription").unwrap_or("").trim();
    Err((
        code,
        format!(
            "{action}: HTTP {status} {} {text}",
            code.map_or(String::new(), |c| c.to_string())
        ),
    ))
}

fn add_mapping(
    ctl: &Control,
    lan: Ipv4Addr,
    ext: u16,
    int: u16,
    lease: u32,
    timeout: Duration,
) -> Result<(), (Option<u32>, String)> {
    let args = [
        ("NewRemoteHost", String::new()),
        ("NewExternalPort", ext.to_string()),
        ("NewProtocol", "UDP".into()),
        ("NewInternalPort", int.to_string()),
        ("NewInternalClient", lan.to_string()),
        ("NewEnabled", "1".into()),
        ("NewPortMappingDescription", DESCRIPTION.into()),
        ("NewLeaseDuration", lease.to_string()),
    ];
    soap(ctl, "AddPortMapping", &args, timeout).map(|_| ())
}

/// AddPortMapping with the usual router quirks handled: only permanent leases
/// (725), external port already taken (718).
fn add_any(
    ctl: &Control,
    lan: Ipv4Addr,
    port: u16,
    timeout: Duration,
) -> Result<(u16, u32), String> {
    let mut ext = port;
    let mut lease = LEASE_SECS;
    let seed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(7, |d| d.subsec_nanos());
    for attempt in 0..5u32 {
        match add_mapping(ctl, lan, ext, port, lease, timeout) {
            Ok(()) => return Ok((ext, lease)),
            Err((Some(725), _)) if lease != 0 => lease = 0,
            Err((Some(718), _)) => {
                ext = 20_000
                    + (seed
                        .wrapping_mul(2_654_435_761)
                        .wrapping_add(attempt * 7919)
                        % 40_000) as u16;
            }
            Err((_, e)) => return Err(e),
        }
    }
    Err("the router has no free port to give".into())
}

/// One HTTP exchange → (status, body). Reads until the connection closes (or
/// goes quiet), at most `MAX_HTTP` bytes, and un-chunks the body.
fn http(addr: SocketAddrV4, req: &str, timeout: Duration) -> Result<(u16, String), String> {
    let t = timeout.max(Duration::from_millis(200));
    let mut s =
        TcpStream::connect_timeout(&SocketAddr::V4(addr), t).map_err(|e| format!("{addr}: {e}"))?;
    s.set_read_timeout(Some(t)).ok();
    s.set_write_timeout(Some(t)).ok();
    s.write_all(req.as_bytes())
        .map_err(|e| format!("{addr}: {e}"))?;
    let mut raw = Vec::new();
    // One byte over the cap tells "too big" from "exactly full".
    let _ = (&mut s).take(MAX_HTTP as u64 + 1).read_to_end(&mut raw);
    if raw.len() > MAX_HTTP {
        return Err(format!("{addr}: response too large"));
    }
    let text = String::from_utf8_lossy(&raw).into_owned();
    let (head, body) = text
        .split_once("\r\n\r\n")
        .ok_or(format!("{addr}: no HTTP response"))?;
    let status: u16 = head
        .lines()
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|c| c.parse().ok())
        .ok_or(format!("{addr}: bad HTTP status"))?;
    let chunked = head.lines().any(|l| {
        let l = l.to_ascii_lowercase();
        l.starts_with("transfer-encoding:") && l.contains("chunked")
    });
    Ok((
        status,
        if chunked {
            dechunk(body)
        } else {
            body.to_string()
        },
    ))
}

fn dechunk(mut s: &str) -> String {
    let mut out = String::new();
    while let Some((size, rest)) = s.split_once("\r\n") {
        let Ok(n) = usize::from_str_radix(size.split(';').next().unwrap_or("").trim(), 16) else {
            break;
        };
        if n == 0 || n > rest.len() || !rest.is_char_boundary(n) {
            if n != 0 {
                out.push_str(rest);
            }
            break;
        }
        out.push_str(&rest[..n]);
        s = rest[n..].trim_start_matches("\r\n");
    }
    out
}

// ---------------------------------------------------------------- gateway

/// The default gateway of this machine, if the OS tells us.
fn default_gateway() -> Option<Ipv4Addr> {
    #[cfg(target_os = "linux")]
    {
        let t = std::fs::read_to_string("/proc/net/route").ok()?;
        for l in t.lines().skip(1) {
            let f: Vec<&str> = l.split_whitespace().collect();
            if f.len() > 3 && f[1] == "00000000" && f[3].len() == 4 {
                let flags = u16::from_str_radix(f[3], 16).ok()?;
                let gw = u32::from_str_radix(f[2], 16).ok()?;
                if flags & 2 != 0 && gw != 0 {
                    return Some(Ipv4Addr::from(gw.to_ne_bytes()));
                }
            }
        }
        None
    }
    #[cfg(not(target_os = "linux"))]
    {
        let mut cmd;
        if cfg!(windows) {
            cmd = std::process::Command::new("route");
            cmd.args(["print", "-4", "0.0.0.0"]);
            #[cfg(windows)]
            std::os::windows::process::CommandExt::creation_flags(&mut cmd, 0x0800_0000);
        } else {
            cmd = std::process::Command::new("route");
            cmd.args(["-n", "get", "default"]);
        }
        let out = String::from_utf8_lossy(&cmd.output().ok()?.stdout).into_owned();
        for l in out.lines() {
            let f: Vec<&str> = l.split_whitespace().collect();
            if f.len() > 2 && f[0] == "0.0.0.0" && f[1] == "0.0.0.0" {
                return f[2].parse().ok(); // Windows
            }
            if f.len() == 2 && f[0] == "gateway:" {
                return f[1].parse().ok(); // macOS / BSD
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};

    #[test]
    fn public_address_rules() {
        for ip in ["8.8.8.8", "93.184.216.34", "100.63.0.1", "100.128.0.1"] {
            assert!(is_public(ip.parse().unwrap()), "{ip}");
        }
        for ip in [
            "10.0.0.1",
            "192.168.1.1",
            "172.16.5.5",
            "127.0.0.1",
            "169.254.1.1",
            "100.64.0.1",
            "100.127.255.255",
            "0.0.0.0",
            "224.0.0.1",
            "255.255.255.255",
            "192.0.2.5",
        ] {
            assert!(!is_public(ip.parse().unwrap()), "{ip}");
        }
    }

    #[test]
    fn parses_urls_and_xml() {
        let (a, p) = parse_http_url("http://192.168.1.1:5000/rootDesc.xml").unwrap();
        assert_eq!(
            (a.to_string(), p.as_str()),
            ("192.168.1.1:5000".into(), "/rootDesc.xml")
        );
        assert_eq!(parse_http_url("http://10.0.0.1").unwrap().0.port(), 80);
        assert!(parse_http_url("http://router.example/x").is_none());
        assert!(parse_http_url("https://10.0.0.1/x").is_none());
        assert_eq!(tag("<a><b>hi</b></a>", "b"), Some("hi"));
        assert_eq!(tag("<a></a>", "b"), None);
        assert_eq!(
            dechunk("5\r\nhello\r\n6\r\n world\r\n0\r\n\r\n"),
            "hello world"
        );
        assert_eq!(dechunk("zz\r\nhello"), "");
    }

    // ------------------------------------------------------- fake routers

    #[derive(Clone, Copy, PartialEq)]
    enum Quirk {
        None,
        /// Refuses leases (error 725) until asked for a permanent one.
        PermanentOnly,
        /// External port taken (error 718) unless it's another one.
        PortTaken,
        /// Router sits behind carrier-grade NAT.
        Cgnat,
    }

    struct FakeIgd {
        ssdp: SocketAddr,
        mappings: Arc<Mutex<Vec<(u16, u16, u32)>>>,
        stop: Arc<AtomicBool>,
        threads: Vec<std::thread::JoinHandle<()>>,
    }

    impl Drop for FakeIgd {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::Relaxed);
            for t in self.threads.drain(..) {
                let _ = t.join();
            }
        }
    }

    fn fake_igd(quirk: Quirk, chunked: bool) -> FakeIgd {
        let http_l = TcpListener::bind("127.0.0.1:0").unwrap();
        http_l.set_nonblocking(true).unwrap();
        let http_port = http_l.local_addr().unwrap().port();
        let udp = UdpSocket::bind("127.0.0.1:0").unwrap();
        udp.set_read_timeout(Some(Duration::from_millis(20)))
            .unwrap();
        let ssdp = udp.local_addr().unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let mappings = Arc::new(Mutex::new(Vec::<(u16, u16, u32)>::new()));
        let ext_ip = if quirk == Quirk::Cgnat {
            "100.64.3.4"
        } else {
            "93.184.216.34"
        };
        let t1 = {
            let stop = stop.clone();
            std::thread::spawn(move || {
                let mut buf = [0u8; 1024];
                while !stop.load(Ordering::Relaxed) {
                    if let Ok((_, from)) = udp.recv_from(&mut buf) {
                        let r = format!(
                            "HTTP/1.1 200 OK\r\nCACHE-CONTROL: max-age=120\r\nLocation: http://127.0.0.1:{http_port}/rootDesc.xml\r\nST: upnp:rootdevice\r\n\r\n"
                        );
                        let _ = udp.send_to(r.as_bytes(), from);
                    }
                }
            })
        };
        let t2 = {
            let stop = stop.clone();
            let mappings = mappings.clone();
            std::thread::spawn(move || {
                while !stop.load(Ordering::Relaxed) {
                    let Ok((mut s, _)) = http_l.accept() else {
                        std::thread::sleep(Duration::from_millis(5));
                        continue;
                    };
                    s.set_nonblocking(false).unwrap();
                    s.set_read_timeout(Some(Duration::from_millis(500)))
                        .unwrap();
                    let mut req = Vec::new();
                    let mut b = [0u8; 4096];
                    loop {
                        let n = s.read(&mut b).unwrap_or(0);
                        req.extend_from_slice(&b[..n]);
                        let t = String::from_utf8_lossy(&req).into_owned();
                        if n == 0 {
                            break;
                        }
                        if let Some((h, body)) = t.split_once("\r\n\r\n") {
                            let want = h
                                .lines()
                                .find_map(|l| {
                                    l.to_ascii_lowercase()
                                        .strip_prefix("content-length:")
                                        .map(|v| v.trim().parse::<usize>().unwrap())
                                })
                                .unwrap_or(0);
                            if body.len() >= want {
                                break;
                            }
                        }
                    }
                    let t = String::from_utf8_lossy(&req).into_owned();
                    let (status, body) = if t.starts_with("GET /rootDesc.xml") {
                        (200, "<root><device><deviceList><device><deviceList><device><serviceList>\
                            <service><serviceType>urn:schemas-upnp-org:service:WANCommonInterfaceConfig:1</serviceType><controlURL>/ctl/common</controlURL></service>\
                            <service><serviceType>urn:schemas-upnp-org:service:WANIPConnection:1</serviceType><controlURL>/ctl/ip</controlURL></service>\
                            </serviceList></device></deviceList></device></deviceList></device></root>".to_string())
                    } else if t.contains("#GetExternalIPAddress") {
                        (
                            200,
                            format!(
                                "<s:Envelope><s:Body><u:R><NewExternalIPAddress>{ext_ip}</NewExternalIPAddress></u:R></s:Body></s:Envelope>"
                            ),
                        )
                    } else if t.contains("#AddPortMapping") {
                        let num = |k: &str| -> u32 { tag(&t, k).unwrap().parse().unwrap() };
                        let (ext, int, lease) = (
                            num("NewExternalPort") as u16,
                            num("NewInternalPort") as u16,
                            num("NewLeaseDuration"),
                        );
                        if quirk == Quirk::PermanentOnly && lease != 0 {
                            (500, "<e><errorCode>725</errorCode><errorDescription>OnlyPermanentLeasesSupported</errorDescription></e>".into())
                        } else if quirk == Quirk::PortTaken && ext == int {
                            (500, "<e><errorCode>718</errorCode><errorDescription>ConflictInMappingEntry</errorDescription></e>".into())
                        } else {
                            let mut m = mappings.lock().unwrap();
                            m.retain(|x| x.0 != ext);
                            m.push((ext, int, lease));
                            (200, "<ok/>".into())
                        }
                    } else if t.contains("#DeletePortMapping") {
                        let ext: u16 = tag(&t, "NewExternalPort").unwrap().parse().unwrap();
                        mappings.lock().unwrap().retain(|x| x.0 != ext);
                        (200, "<ok/>".into())
                    } else {
                        (404, String::new())
                    };
                    let reply = if chunked {
                        let (a, b) = body.split_at(body.len() / 2);
                        format!(
                            "HTTP/1.1 {status} X\r\nTransfer-Encoding: chunked\r\n\r\n{:x}\r\n{a}\r\n{:x}\r\n{b}\r\n0\r\n\r\n",
                            a.len(),
                            b.len()
                        )
                    } else {
                        format!(
                            "HTTP/1.1 {status} X\r\nContent-Length: {}\r\n\r\n{body}",
                            body.len()
                        )
                    };
                    let _ = s.write_all(reply.as_bytes());
                }
            })
        };
        FakeIgd {
            ssdp,
            mappings,
            stop,
            threads: vec![t1, t2],
        }
    }

    fn probe(ssdp: SocketAddr) -> Probe {
        Probe {
            ssdp,
            gateways: vec![],
            natpmp_port: 1,
            timeout: Duration::from_millis(800),
            allow_private_external: false,
        }
    }

    #[test]
    fn upnp_opens_renews_and_closes_a_mapping() {
        for chunked in [false, true] {
            let igd = fake_igd(Quirk::None, chunked);
            let mut m = open(&probe(igd.ssdp), "192.168.1.50".parse().unwrap(), 27960).unwrap();
            assert_eq!(
                (m.via, m.external.to_string().as_str()),
                ("UPnP", "93.184.216.34:27960")
            );
            assert_eq!(
                *igd.mappings.lock().unwrap(),
                vec![(27960, 27960, LEASE_SECS)]
            );
            assert_eq!(m.renew_after(), Duration::from_secs(1800));
            m.renew().unwrap();
            assert_eq!(igd.mappings.lock().unwrap().len(), 1);
            m.close();
            assert!(igd.mappings.lock().unwrap().is_empty());
        }
    }

    #[test]
    fn upnp_router_quirks() {
        let igd = fake_igd(Quirk::PermanentOnly, false);
        let m = open(&probe(igd.ssdp), "192.168.1.50".parse().unwrap(), 27960).unwrap();
        assert_eq!(*igd.mappings.lock().unwrap(), vec![(27960, 27960, 0)]);
        m.close();

        let igd = fake_igd(Quirk::PortTaken, false);
        let m = open(&probe(igd.ssdp), "192.168.1.50".parse().unwrap(), 27960).unwrap();
        assert_ne!(m.external.port(), 27960);
        assert_eq!(igd.mappings.lock().unwrap()[0].1, 27960);
    }

    #[test]
    fn mapping_behind_carrier_grade_nat_is_undone() {
        let igd = fake_igd(Quirk::Cgnat, false);
        let e = open(&probe(igd.ssdp), "192.168.1.50".parse().unwrap(), 27960).unwrap_err();
        assert!(e.contains("100.64.3.4"), "{e}");
        assert!(igd.mappings.lock().unwrap().is_empty());
    }

    #[test]
    fn no_router_is_a_clear_error() {
        let quiet = UdpSocket::bind("127.0.0.1:0").unwrap();
        let mut p = probe(quiet.local_addr().unwrap());
        p.timeout = Duration::from_millis(300);
        p.gateways = vec![Ipv4Addr::LOCALHOST];
        let e = open(&p, "192.168.1.50".parse().unwrap(), 27960).unwrap_err();
        assert!(e.contains("UPnP") && e.contains("NAT-PMP"), "{e}");
    }

    #[test]
    fn hostile_ssdp_answers_are_ignored() {
        // A device on the LAN points us at a different host: refused.
        let evil = UdpSocket::bind("127.0.0.1:0").unwrap();
        evil.set_read_timeout(Some(Duration::from_millis(300)))
            .unwrap();
        let ssdp = evil.local_addr().unwrap();
        let t = std::thread::spawn(move || {
            let mut b = [0u8; 512];
            if let Ok((_, from)) = evil.recv_from(&mut b) {
                let _ = evil.send_to(
                    b"HTTP/1.1 200 OK\r\nLOCATION: http://8.8.8.8/x.xml\r\n\r\n",
                    from,
                );
            }
        });
        let mut p = probe(ssdp);
        p.timeout = Duration::from_millis(400);
        let e = upnp_open(&p, "192.168.1.50".parse().unwrap(), 27960).unwrap_err();
        t.join().unwrap();
        assert!(e.contains("no router"), "{e}");
    }

    #[test]
    fn oversized_http_responses_are_refused() {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let SocketAddr::V4(addr) = l.local_addr().unwrap() else {
            unreachable!()
        };
        let t = std::thread::spawn(move || {
            let (mut s, _) = l.accept().unwrap();
            let mut b = [0u8; 256];
            let _ = s.read(&mut b);
            let _ = s.write_all(b"HTTP/1.1 200 OK\r\n\r\n");
            let junk = vec![b'a'; 64 * 1024];
            for _ in 0..4 {
                if s.write_all(&junk).is_err() {
                    break;
                }
            }
        });
        let e = http(addr, "GET / HTTP/1.1\r\n\r\n", Duration::from_secs(2)).unwrap_err();
        let _ = t.join();
        assert!(e.contains("too large"), "{e}");
    }

    #[test]
    fn natpmp_opens_and_closes_a_mapping() {
        let router = UdpSocket::bind("127.0.0.1:0").unwrap();
        router
            .set_read_timeout(Some(Duration::from_millis(50)))
            .unwrap();
        let port = router.local_addr().unwrap().port();
        let stop = Arc::new(AtomicBool::new(false));
        let seen = Arc::new(Mutex::new(Vec::<(u16, u16, u32)>::new()));
        let t = {
            let (stop, seen) = (stop.clone(), seen.clone());
            std::thread::spawn(move || {
                let mut b = [0u8; 64];
                while !stop.load(Ordering::Relaxed) {
                    let Ok((n, from)) = router.recv_from(&mut b) else {
                        continue;
                    };
                    if n == 2 {
                        let _ = router.send_to(&[0, 128, 0, 0, 0, 0, 0, 1, 93, 184, 216, 35], from);
                    } else if n == 12 && b[1] == 1 {
                        let int = u16::from_be_bytes([b[4], b[5]]);
                        let ext = u16::from_be_bytes([b[6], b[7]]);
                        let lease = u32::from_be_bytes([b[8], b[9], b[10], b[11]]);
                        seen.lock().unwrap().push((int, ext, lease));
                        let mut r = vec![0, 129, 0, 0, 0, 0, 0, 2];
                        r.extend_from_slice(&int.to_be_bytes());
                        r.extend_from_slice(&(if ext == 0 { int } else { ext }).to_be_bytes());
                        r.extend_from_slice(&lease.to_be_bytes());
                        let _ = router.send_to(&r, from);
                    }
                }
            })
        };
        let quiet = UdpSocket::bind("127.0.0.1:0").unwrap();
        let p = Probe {
            ssdp: quiet.local_addr().unwrap(),
            gateways: vec![Ipv4Addr::LOCALHOST],
            natpmp_port: port,
            timeout: Duration::from_millis(400),
            allow_private_external: false,
        };
        let mut m = open(&p, "192.168.1.50".parse().unwrap(), 27960).unwrap();
        assert_eq!(
            (m.via, m.external.to_string().as_str()),
            ("NAT-PMP", "93.184.216.35:27960")
        );
        m.renew().unwrap();
        m.close();
        stop.store(true, Ordering::Relaxed);
        t.join().unwrap();
        let seen = seen.lock().unwrap();
        assert_eq!(seen[0], (27960, 27960, LEASE_SECS));
        assert_eq!(*seen.last().unwrap(), (27960, 0, 0));
    }
}
