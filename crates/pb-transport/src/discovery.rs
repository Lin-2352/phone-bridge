//! Finding paired devices again after a network change (new Wi-Fi, new IP).
//!
//! Relay and public discovery are off by design, so a device only knows the
//! addresses its peers had at pairing time. When those go stale, a node sends
//! a small UDP probe on the local network (broadcast per interface plus every
//! last known address) and the paired peer answers directly.
//!
//! Safety of what is learned: a probe or answer only adds an in-memory
//! *candidate* address. It is saved to the peer store only after an
//! authenticated QUIC connection to the peer's key succeeded through it, so a
//! forged or replayed probe can neither replace good addresses nor cause disk
//! writes; at worst it costs one failed dial.
//!
//! Privacy: a probe carries no name and no key, only
//! `tag = SHA-256("pb-discovery/1" || sender id || 30-minute window)[..16]`.
//! Only a device holding the sender's public key can recognise it; the tag
//! changes every 30 minutes. Probes are sent on demand (start, network change,
//! failed dial), never periodically.
//!
//! Packet (23 bytes): `"PBD1" | kind u8 (1 probe, 2 answer) | quic port u16 LE | tag [16]`.

use std::{
    collections::HashMap,
    io::ErrorKind,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    sync::Arc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use pb_identity::PeerId;
use sha2::{Digest, Sha256};
use tokio::{net::UdpSocket, task::JoinHandle};

use crate::{lock, Node};

/// UDP port for probes on every device.
pub const DISCOVERY_PORT: u16 = 47199;

const MAGIC: &[u8; 4] = b"PBD1";
const PROBE: u8 = 1;
const ANSWER: u8 = 2;
const LEN: usize = 23;
/// Tag window; neighbours on either side are accepted (clock skew).
const WINDOW_SECS: u64 = 30 * 60;
/// Unverified candidate addresses kept per peer.
const MAX_CANDIDATES: usize = 4;
/// Verified addresses kept per peer in the store.
const MAX_ADDRS: usize = 6;

fn window(offset: i64) -> u64 {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() / WINDOW_SECS)
        .unwrap_or(0);
    now.saturating_add_signed(offset)
}

fn tag(id: &PeerId, window: u64) -> [u8; 16] {
    let mut h = Sha256::new();
    h.update(b"pb-discovery/1");
    h.update(id.as_bytes());
    h.update(window.to_le_bytes());
    let d = h.finalize();
    let mut t = [0u8; 16];
    t.copy_from_slice(&d[..16]);
    t
}

fn packet(kind: u8, port: u16, tag: [u8; 16]) -> [u8; LEN] {
    let mut p = [0u8; LEN];
    p[..4].copy_from_slice(MAGIC);
    p[4] = kind;
    p[5..7].copy_from_slice(&port.to_le_bytes());
    p[7..].copy_from_slice(&tag);
    p
}

fn parse(buf: &[u8]) -> Option<(u8, u16, [u8; 16])> {
    if buf.len() != LEN || &buf[..4] != MAGIC || !matches!(buf[4], PROBE | ANSWER) {
        return None;
    }
    let port = u16::from_le_bytes([buf[5], buf[6]]);
    let mut t = [0u8; 16];
    t.copy_from_slice(&buf[7..]);
    Some((buf[4], port, t))
}

/// Directed broadcast address (`ip | !mask`) of each usable local IPv4
/// network, using the interface's real prefix length.
pub(crate) fn directed_broadcasts(ifaces: &[(Ipv4Addr, u8)]) -> Vec<Ipv4Addr> {
    let mut out = Vec::new();
    for &(ip, prefix) in ifaces {
        // /31 and /32 have no broadcast; below /8 is not a LAN.
        if !(8..=30).contains(&prefix)
            || ip.is_loopback()
            || ip.is_link_local()
            || ip.is_unspecified()
            || ip.is_multicast()
            || ip.is_broadcast()
        {
            continue;
        }
        let b = Ipv4Addr::from(u32::from(ip) | (u32::MAX >> prefix));
        if !out.contains(&b) {
            out.push(b);
        }
    }
    out
}

/// Whether `ip` lies inside one of the networks in `ifaces`.
fn on_known_network(ip: Ipv4Addr, ifaces: &[(Ipv4Addr, u8)]) -> bool {
    ifaces.iter().any(|&(net, prefix)| {
        let mask = u32::MAX
            .checked_shl(32 - u32::from(prefix.min(32)))
            .unwrap_or(0);
        prefix > 0 && u32::from(ip) & mask == u32::from(net) & mask
    })
}

/// Every address a probe goes to, in send order: the limited broadcast, the
/// directed broadcast of every local interface (real prefix), a `/24` guess
/// for any iroh address on no known interface (all of them when the
/// interface list is unavailable: the previous behaviour), then each last
/// known peer address.
pub(crate) fn probe_targets(
    port: u16,
    interfaces: &[(Ipv4Addr, u8)],
    iroh_addrs: &[String],
    saved_peer_addrs: &[String],
) -> Vec<SocketAddr> {
    let mut targets = vec![SocketAddr::from((Ipv4Addr::BROADCAST, port))];
    let mut push = |t: SocketAddr| {
        if !targets.contains(&t) {
            targets.push(t);
        }
    };
    for b in directed_broadcasts(interfaces) {
        push(SocketAddr::from((b, port)));
    }
    for a in iroh_addrs {
        if let Ok(SocketAddr::V4(v4)) = a.parse::<SocketAddr>() {
            if on_known_network(*v4.ip(), interfaces) {
                continue;
            }
            let o = v4.ip().octets();
            push(SocketAddr::from((
                Ipv4Addr::new(o[0], o[1], o[2], 255),
                port,
            )));
        }
    }
    // Across routers that drop broadcast.
    for a in saved_peer_addrs {
        if let Ok(sa) = a.parse::<SocketAddr>() {
            push(SocketAddr::new(sa.ip(), port));
        }
    }
    targets
}

/// IPv4 address and prefix length of every up, non-loopback interface
/// (VPN adapters included: harmless). Empty when the OS gives no answer.
pub(crate) fn local_ipv4_interfaces() -> Vec<(Ipv4Addr, u8)> {
    netdev::get_interfaces()
        .into_iter()
        .filter(|i| i.is_up() && !i.is_loopback())
        .flat_map(|i| i.ipv4.into_iter().map(|n| (n.addr(), n.prefix_len())))
        .collect()
}

/// Source addresses we never accept as a peer location.
fn usable(addr: &SocketAddr) -> bool {
    let ip = addr.ip();
    addr.port() != 0
        && !ip.is_unspecified()
        && !ip.is_multicast()
        && !matches!(ip, IpAddr::V4(v4) if v4.is_broadcast())
        && (cfg!(test) || !ip.is_loopback())
}

pub(crate) struct Discovery {
    sock: Arc<UdpSocket>,
    port: u16,
    task: JoinHandle<()>,
}

/// Candidate addresses per peer (learned, not yet verified).
pub(crate) type Candidates = std::sync::Mutex<HashMap<PeerId, Vec<SocketAddr>>>;

impl Node {
    /// Bind the probe socket and answer probes from paired peers. Failure to
    /// bind (port in use) only disables rediscovery.
    pub(crate) async fn start_discovery(&self, port: u16) {
        let Ok(sock) = UdpSocket::bind(SocketAddr::from((Ipv4Addr::UNSPECIFIED, port))).await
        else {
            return;
        };
        let _ = sock.set_broadcast(true);
        let sock = Arc::new(sock);
        let node = self.clone();
        let rx = sock.clone();
        let task = tokio::spawn(async move {
            let mut buf = [0u8; 64];
            // At most one answer per peer per second (no reflection floods).
            let mut answered: Vec<(PeerId, Instant)> = Vec::new();
            loop {
                let (n, from) = match rx.recv_from(&mut buf).await {
                    Ok(v) => v,
                    // Windows reports an earlier send to a closed port (ICMP
                    // unreachable) as an error on the next receive: ignore it.
                    Err(e) if e.kind() == ErrorKind::ConnectionReset => continue,
                    Err(_) => {
                        tokio::time::sleep(Duration::from_millis(200)).await;
                        continue;
                    }
                };
                let Some((kind, quic_port, t)) = parse(&buf[..n]) else {
                    continue;
                };
                let Some(peer) = node.peer_for_tag(&t) else {
                    continue;
                };
                node.add_candidate(peer, SocketAddr::new(from.ip(), quic_port));
                if kind == PROBE {
                    answered.retain(|(_, at)| at.elapsed() < Duration::from_secs(1));
                    if answered.iter().any(|(p, _)| *p == peer) {
                        continue;
                    }
                    let my_port = node.port();
                    if my_port == 0 {
                        continue;
                    }
                    answered.push((peer, Instant::now()));
                    let reply = packet(ANSWER, my_port, tag(&node.inner.me, window(0)));
                    let _ = rx.send_to(&reply, from).await;
                }
            }
        });
        *lock(&self.inner.discovery) = Some(Discovery { sock, port, task });
    }

    /// Stop answering probes and release the port (called from shutdown).
    pub(crate) fn stop_discovery(&self) {
        if let Some(d) = lock(&self.inner.discovery).take() {
            d.task.abort();
        }
    }

    /// Ask paired peers on this network where they are. Answers become
    /// candidate addresses tried on the next connect.
    pub async fn rediscover(&self) {
        let Some((sock, port)) = lock(&self.inner.discovery)
            .as_ref()
            .map(|d| (d.sock.clone(), d.port))
        else {
            return;
        };
        let my_port = self.port();
        if my_port == 0 {
            return;
        }
        let probe = packet(PROBE, my_port, tag(&self.inner.me, window(0)));
        let saved: Vec<String> = lock(&self.inner.store)
            .list()
            .into_iter()
            .flat_map(|p| p.addrs)
            .collect();
        // Interface enumeration is a blocking OS call.
        let ifaces = tokio::task::spawn_blocking(local_ipv4_interfaces)
            .await
            .unwrap_or_default();
        let targets = probe_targets(port, &ifaces, &self.addrs(), &saved);
        for t in targets {
            let _ = sock.send_to(&probe, t).await;
        }
    }

    fn peer_for_tag(&self, t: &[u8; 16]) -> Option<PeerId> {
        let windows = [window(-1), window(0), window(1)];
        lock(&self.inner.store)
            .list()
            .into_iter()
            .map(|p| p.id)
            .find(|id| windows.iter().any(|w| tag(id, *w) == *t))
    }

    fn add_candidate(&self, peer: PeerId, addr: SocketAddr) {
        if !usable(&addr) {
            return;
        }
        let mut c = lock(&self.inner.candidates);
        let list = c.entry(peer).or_default();
        list.retain(|a| *a != addr);
        list.insert(0, addr);
        list.truncate(MAX_CANDIDATES);
    }

    /// What a discovery packet carrying `peer`'s tag from `addr` leaves
    /// (any host on the LAN can send one; tests plant it directly).
    #[cfg(test)]
    pub(crate) fn test_add_candidate(&self, peer: PeerId, addr: SocketAddr) {
        self.add_candidate(peer, addr);
    }

    /// Unverified addresses learned for `peer`, newest first.
    pub(crate) fn candidates(&self, peer: &PeerId) -> Vec<SocketAddr> {
        lock(&self.inner.candidates)
            .get(peer)
            .cloned()
            .unwrap_or_default()
    }

    /// A dial through these candidates failed: forget them.
    pub(crate) fn drop_candidates(&self, peer: &PeerId, failed: &[SocketAddr]) {
        let mut c = lock(&self.inner.candidates);
        if let Some(list) = c.get_mut(peer) {
            list.retain(|a| !failed.contains(a));
            if list.is_empty() {
                c.remove(peer);
            }
        }
    }

    /// An authenticated connection succeeded through `addrs`: save them first
    /// in the peer's address list and forget the candidates.
    ///
    /// Runs off the async threads (the store flushes a file). A save that
    /// fails changes nothing: the old addresses stay, in memory as on disk,
    /// and the next connection tries again. Nobody waits on this to report it.
    pub(crate) async fn confirm_addrs(&self, peer: &PeerId, addrs: &[SocketAddr]) {
        lock(&self.inner.candidates).remove(peer);
        let store = self.inner.store.clone();
        let peer = *peer;
        let new: Vec<String> = addrs.iter().map(|a| a.to_string()).collect();
        let _ = tokio::task::spawn_blocking(move || {
            let mut store = lock(&store);
            let Some(mut p) = store.get(&peer) else {
                return;
            };
            if p.addrs.starts_with(&new) {
                return;
            }
            p.addrs.retain(|x| !new.contains(x));
            let mut merged = new;
            merged.append(&mut p.addrs);
            merged.truncate(MAX_ADDRS);
            p.addrs = merged;
            let _ = store.try_pin(p);
        })
        .await;
    }
}

/// The current probe tag for `id` (tests build probes by hand).
#[cfg(test)]
pub(crate) fn test_tag(id: &PeerId) -> [u8; 16] {
    tag(id, window(0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packets_round_trip_and_reject_noise() {
        let t = [7u8; 16];
        let p = packet(PROBE, 47100, t);
        assert_eq!(parse(&p), Some((PROBE, 47100, t)));
        assert_eq!(parse(&p[..22]), None);
        let mut bad = p;
        bad[0] = b'X';
        assert_eq!(parse(&bad), None);
        let mut kind = p;
        kind[4] = 9;
        assert_eq!(parse(&kind), None);
    }

    #[test]
    fn tags_rotate_and_depend_on_the_id() {
        let a = pb_identity::Identity::from_seed([1; 32]).peer_id();
        let b = pb_identity::Identity::from_seed([2; 32]).peer_id();
        assert_ne!(tag(&a, 100), tag(&a, 101));
        assert_ne!(tag(&a, 100), tag(&b, 100));
        assert_eq!(tag(&a, 100), tag(&a, 100));
    }

    #[test]
    fn unusable_sources_are_rejected() {
        assert!(!usable(&"10.0.0.5:0".parse().unwrap()));
        assert!(!usable(&"0.0.0.0:47100".parse().unwrap()));
        assert!(!usable(&"224.0.0.1:47100".parse().unwrap()));
        assert!(!usable(&"255.255.255.255:47100".parse().unwrap()));
        assert!(usable(&"10.73.190.167:47100".parse().unwrap()));
    }

    fn v4(s: &str) -> Ipv4Addr {
        s.parse().unwrap()
    }

    #[test]
    fn a_slash_24_gives_the_previous_dot_255() {
        assert_eq!(
            directed_broadcasts(&[(v4("192.168.1.5"), 24)]),
            vec![v4("192.168.1.255")]
        );
    }

    #[test]
    fn directed_broadcast_uses_the_real_prefix() {
        assert_eq!(
            directed_broadcasts(&[(v4("10.21.7.75"), 20)]),
            vec![v4("10.21.15.255")]
        );
        assert_eq!(
            directed_broadcasts(&[(v4("192.168.1.5"), 16)]),
            vec![v4("192.168.255.255")]
        );
        assert_eq!(
            directed_broadcasts(&[(v4("10.1.2.3"), 8), (v4("10.0.0.9"), 30)]),
            vec![v4("10.255.255.255"), v4("10.0.0.11")]
        );
    }

    #[test]
    fn point_to_point_loopback_link_local_and_nonsense_give_nothing() {
        let ifaces = [
            (v4("10.0.0.1"), 31),
            (v4("10.0.0.1"), 32),
            (v4("10.0.0.1"), 0),
            (v4("10.0.0.1"), 7),
            (v4("10.0.0.1"), 33),
            (v4("127.0.0.1"), 8),
            (v4("169.254.10.20"), 16),
            (v4("0.0.0.0"), 24),
        ];
        assert!(directed_broadcasts(&ifaces).is_empty());
    }

    #[test]
    fn duplicates_across_interfaces_collapse_in_order() {
        let ifaces = [
            (v4("10.21.7.75"), 20),
            (v4("192.168.1.5"), 24),
            (v4("10.21.12.26"), 20),
            (v4("192.168.1.9"), 24),
        ];
        assert_eq!(
            directed_broadcasts(&ifaces),
            vec![v4("10.21.15.255"), v4("192.168.1.255")]
        );
    }

    fn sa(s: &str) -> SocketAddr {
        s.parse().unwrap()
    }

    #[test]
    fn probe_targets_reach_the_real_subnet_broadcast() {
        // The user's PC: Wi-Fi 10.21.7.75/20 plus a VPN adapter; iroh lists
        // the Wi-Fi address; the only saved peer address is from another network.
        let t = probe_targets(
            47199,
            &[(v4("10.21.7.75"), 20), (v4("10.2.0.2"), 32)],
            &["10.21.7.75:51000".into()],
            &["10.57.6.22:47100".into()],
        );
        assert_eq!(
            t,
            vec![
                sa("255.255.255.255:47199"),
                sa("10.21.15.255:47199"),
                sa("10.57.6.22:47199"),
            ]
        );
        // The wrong /24 guess is gone once the real prefix is known.
        assert!(!t.contains(&sa("10.21.7.255:47199")));
    }

    #[test]
    fn every_interface_is_probed_even_if_iroh_omits_it() {
        let t = probe_targets(
            47199,
            &[(v4("10.21.7.75"), 20), (v4("192.168.137.1"), 24)],
            &["10.21.7.75:51000".into()],
            &[],
        );
        assert!(t.contains(&sa("192.168.137.255:47199")));
        assert!(t.contains(&sa("10.21.15.255:47199")));
    }

    #[test]
    fn without_interface_info_the_old_slash_24_guess_is_kept() {
        let t = probe_targets(
            47199,
            &[],
            &[
                "10.21.7.75:51000".into(),
                "[fe80::1]:51000".into(),
                "10.21.7.80:51001".into(),
            ],
            &["10.57.6.22:47100".into(), "10.21.7.75:47100".into()],
        );
        assert_eq!(
            t,
            vec![
                sa("255.255.255.255:47199"),
                sa("10.21.7.255:47199"),
                sa("10.57.6.22:47199"),
                sa("10.21.7.75:47199"),
            ]
        );
    }

    #[test]
    fn an_iroh_address_outside_every_known_interface_still_gets_the_guess() {
        let t = probe_targets(
            47199,
            &[(v4("10.21.7.75"), 20)],
            &["10.21.7.75:51000".into(), "172.20.1.4:51000".into()],
            &[],
        );
        assert_eq!(
            t,
            vec![
                sa("255.255.255.255:47199"),
                sa("10.21.15.255:47199"),
                sa("172.20.1.255:47199"),
            ]
        );
    }

    /// Prints the live probe targets of this machine (real-world check):
    /// `cargo test -p pb-transport live_probe_targets -- --ignored --nocapture`.
    #[test]
    #[ignore = "reads this machine's interfaces; run by hand"]
    fn live_probe_targets() {
        let ifaces = local_ipv4_interfaces();
        println!("interfaces: {ifaces:?}");
        for t in probe_targets(DISCOVERY_PORT, &ifaces, &[], &[]) {
            println!("target: {t}");
        }
    }
}
