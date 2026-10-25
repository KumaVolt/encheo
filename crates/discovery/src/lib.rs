//! Find media servers and file shares on the local network, so adding one is a click instead of
//! typing an address.
//!
//! - Jellyfin and Emby answer a UDP broadcast on port 7359 ("who is JellyfinServer?").
//! - Plex answers GDM, an HTTP-like UDP broadcast on port 32414.
//! - SMB shares and WebDAV servers announce themselves over mDNS (`_smb._tcp`, `_webdav._tcp`,
//!   `_webdavs._tcp`), which is also how macOS and most NAS boxes advertise them.
//!
//! Everything here is blocking and bounded by a timeout; run it on a worker thread.

use std::collections::BTreeSet;
use std::net::{Ipv4Addr, SocketAddr, UdpSocket};
use std::time::{Duration, Instant};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind {
    Jellyfin,
    Emby,
    Plex,
    Smb,
    WebDav,
}

/// Something found on the network.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Found {
    pub kind: Kind,
    pub name: String,
    /// What to put in the address field: `http://192.168.1.5:8096`, `smb://nas.local`, …
    pub address: String,
}

/// Look for everything at once for `timeout`.
pub fn discover(timeout: Duration) -> Vec<Found> {
    let (tx, rx) = std::sync::mpsc::channel();
    let jobs: Vec<Box<dyn FnOnce() -> Vec<Found> + Send>> = vec![
        Box::new(move || ask_media_server(Kind::Jellyfin, "who is JellyfinServer?", timeout)),
        Box::new(move || ask_media_server(Kind::Emby, "who is EmbyServer?", timeout)),
        Box::new(move || plex_gdm(timeout)),
        Box::new(move || mdns(timeout)),
    ];
    for job in jobs {
        let tx = tx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(job());
        });
    }
    drop(tx);
    let mut all: BTreeSet<Found> = BTreeSet::new();
    for found in rx {
        all.extend(found);
    }
    all.into_iter().collect()
}

/// Broadcast a UDP query and collect replies until the timeout.
fn broadcast(port: u16, query: &[u8], timeout: Duration, mut on_reply: impl FnMut(&[u8], SocketAddr)) {
    let Ok(sock) = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)) else { return };
    let _ = sock.set_broadcast(true);
    let _ = sock.send_to(query, (Ipv4Addr::BROADCAST, port));
    collect(&sock, timeout, &mut on_reply);
}

fn collect(sock: &UdpSocket, timeout: Duration, on_reply: &mut impl FnMut(&[u8], SocketAddr)) {
    let deadline = Instant::now() + timeout;
    let mut buf = [0u8; 4096];
    while let Some(left) = deadline.checked_duration_since(Instant::now()) {
        let _ = sock.set_read_timeout(Some(left.max(Duration::from_millis(1))));
        match sock.recv_from(&mut buf) {
            Ok((n, from)) => on_reply(&buf[..n], from),
            Err(_) => break,
        }
    }
}

/// Jellyfin and Emby: each answers its own question with JSON {Address, Id, Name}.
fn ask_media_server(kind: Kind, query: &str, timeout: Duration) -> Vec<Found> {
    let mut out = Vec::new();
    broadcast(7359, query.as_bytes(), timeout, |data, from| {
        if let Some(f) = parse_jellyfin_reply(kind.clone(), data, from) {
            out.push(f);
        }
    });
    out
}

pub(crate) fn parse_jellyfin_reply(kind: Kind, data: &[u8], from: SocketAddr) -> Option<Found> {
    let v: serde_json::Value = serde_json::from_slice(data).ok()?;
    let address = v["Address"].as_str().map(|a| a.trim_end_matches('/').to_string()).unwrap_or_else(|| format!("http://{}:8096", from.ip()));
    Some(Found { kind, name: v["Name"].as_str().unwrap_or("Media server").to_string(), address })
}

/// Plex: GDM answers with "HTTP/1.0 200 OK" and headers like `Name:` and `Port:`.
pub fn plex_gdm(timeout: Duration) -> Vec<Found> {
    let mut out = Vec::new();
    broadcast(32414, b"M-SEARCH * HTTP/1.1\r\n\r\n", timeout, |data, from| {
        if let Some(f) = parse_gdm_reply(data, from) {
            out.push(f);
        }
    });
    out
}

pub(crate) fn parse_gdm_reply(data: &[u8], from: SocketAddr) -> Option<Found> {
    let text = std::str::from_utf8(data).ok()?;
    if !text.starts_with("HTTP/1.0 200") && !text.starts_with("HTTP/1.1 200") {
        return None;
    }
    let header = |name: &str| {
        text.lines().find_map(|l| {
            let (k, v) = l.split_once(':')?;
            k.trim().eq_ignore_ascii_case(name).then(|| v.trim().to_string())
        })
    };
    let port = header("Port").unwrap_or_else(|| "32400".into());
    Some(Found { kind: Kind::Plex, name: header("Name").unwrap_or_else(|| "Plex".into()), address: format!("http://{}:{port}", from.ip()) })
}

/// SMB and WebDAV over mDNS.
pub fn mdns(timeout: Duration) -> Vec<Found> {
    let Ok(daemon) = mdns_sd::ServiceDaemon::new() else { return vec![] };
    let mut out = Vec::new();
    let services = [("_smb._tcp.local.", Kind::Smb, "smb"), ("_webdav._tcp.local.", Kind::WebDav, "http"), ("_webdavs._tcp.local.", Kind::WebDav, "https")];
    let receivers: Vec<_> = services.iter().filter_map(|(ty, k, s)| Some((daemon.browse(ty).ok()?, k.clone(), *s))).collect();
    let deadline = Instant::now() + timeout;
    while let Some(left) = deadline.checked_duration_since(Instant::now()) {
        let mut any = false;
        for (rx, kind, scheme) in &receivers {
            while let Ok(event) = rx.recv_timeout(Duration::from_millis(20).min(left)) {
                any = true;
                if let mdns_sd::ServiceEvent::ServiceResolved(info) = event
                    && let Some(f) = from_mdns(kind.clone(), scheme, &info)
                {
                    out.push(f);
                }
            }
        }
        if !any {
            std::thread::sleep(Duration::from_millis(30).min(left));
        }
    }
    let _ = daemon.shutdown();
    out
}

fn from_mdns(kind: Kind, scheme: &str, info: &mdns_sd::ResolvedService) -> Option<Found> {
    let host = info.get_hostname().trim_end_matches('.').to_string();
    let port = info.get_port();
    let name = info.get_fullname().split("._").next().unwrap_or(&host).replace("\\032", " ").to_string();
    let address = match kind {
        Kind::Smb if port == 445 => format!("smb://{host}"),
        Kind::Smb => format!("smb://{host}:{port}"),
        _ => {
            let path = info.get_property_val_str("path").unwrap_or("/");
            let default = if scheme == "https" { 443 } else { 80 };
            if port == default { format!("{scheme}://{host}{path}") } else { format!("{scheme}://{host}:{port}{path}") }
        }
    };
    Some(Found { kind, name, address })
}

/// How fast data arrives from `reader`: reads up to `max_bytes` or for `max_time`, whichever
/// comes first. Returns megabits per second.
pub fn measure(reader: &mut dyn std::io::Read, max_bytes: u64, max_time: Duration) -> std::io::Result<f64> {
    let start = Instant::now();
    let mut buf = vec![0u8; 1 << 20];
    let mut total = 0u64;
    while total < max_bytes && start.elapsed() < max_time {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        total += n as u64;
    }
    let secs = start.elapsed().as_secs_f64().max(1e-6);
    Ok(total as f64 * 8.0 / secs / 1e6)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn addr() -> SocketAddr {
        "192.168.1.20:7359".parse().unwrap()
    }

    #[test]
    fn jellyfin_reply() {
        let f = parse_jellyfin_reply(Kind::Jellyfin, br#"{"Address":"http://192.168.1.20:8096/","Id":"x","Name":"Living Room"}"#, addr()).unwrap();
        assert_eq!(f, Found { kind: Kind::Jellyfin, name: "Living Room".into(), address: "http://192.168.1.20:8096".into() });
        assert!(parse_jellyfin_reply(Kind::Emby, b"nonsense", addr()).is_none());
    }

    #[test]
    fn gdm_reply() {
        let r = b"HTTP/1.0 200 OK\r\nContent-Type: plex/media-server\r\nName: MiqFlix\r\nPort: 32400\r\nResource-Identifier: abc\r\n\r\n";
        let f = parse_gdm_reply(r, "10.0.0.7:32414".parse().unwrap()).unwrap();
        assert_eq!(f, Found { kind: Kind::Plex, name: "MiqFlix".into(), address: "http://10.0.0.7:32400".into() });
        assert!(parse_gdm_reply(b"HTTP/1.0 404 Not Found\r\n", addr()).is_none());
    }

    /// A fake Jellyfin answering on localhost: the query and the reply format end to end.
    #[test]
    fn udp_round_trip() {
        let server = UdpSocket::bind("127.0.0.1:0").unwrap();
        let port = server.local_addr().unwrap().port();
        std::thread::spawn(move || {
            let mut buf = [0u8; 256];
            let (n, from) = server.recv_from(&mut buf).unwrap();
            assert_eq!(&buf[..n], b"who is JellyfinServer?");
            server.send_to(br#"{"Address":"http://127.0.0.1:8096","Name":"Test"}"#, from).unwrap();
        });
        let sock = UdpSocket::bind("127.0.0.1:0").unwrap();
        sock.send_to(b"who is JellyfinServer?", ("127.0.0.1", port)).unwrap();
        let mut got = vec![];
        collect(&sock, Duration::from_millis(500), &mut |d: &[u8], from| got.extend(parse_jellyfin_reply(Kind::Jellyfin, d, from)));
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].name, "Test");
    }

    /// Advertise a fake SMB server over mDNS and find it.
    #[test]
    fn mdns_finds_an_advertised_share() {
        let daemon = mdns_sd::ServiceDaemon::new().unwrap();
        let host = format!("encheo-test-{}.local.", std::process::id());
        let info = mdns_sd::ServiceInfo::new("_smb._tcp.local.", "Encheo Test NAS", &host, "127.0.0.1", 445, None::<std::collections::HashMap<String, String>>)
            .unwrap();
        daemon.register(info).unwrap();
        let found = mdns(Duration::from_secs(3));
        let _ = daemon.shutdown();
        assert!(
            found.iter().any(|f| f.kind == Kind::Smb && f.name == "Encheo Test NAS" && f.address.starts_with("smb://encheo-test-")),
            "{found:?}"
        );
    }

    #[test]
    fn measures_speed() {
        let mut r = std::io::repeat(0).take(10 << 20);
        let mbps = measure(&mut r, 8 << 20, Duration::from_secs(2)).unwrap();
        assert!(mbps > 10.0, "{mbps}");
    }

    use std::io::Read;
}
