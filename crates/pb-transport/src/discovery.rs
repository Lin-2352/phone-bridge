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
        let mut targets = vec![SocketAddr::from((Ipv4Addr::BROADCAST, port))];
        // Directed broadcast on each local IPv4 network (assumes /24, the
        // common home and hotspot size): reaches interfaces the limited
        // broadcast above does not leave through.
        for a in self.addrs() {
            if let Ok(SocketAddr::V4(v4)) = a.parse::<SocketAddr>() {
                let o = v4.ip().octets();
                let t = SocketAddr::from((Ipv4Addr::new(o[0], o[1], o[2], 255), port));
                if !targets.contains(&t) {
                    targets.push(t);
                }
            }
        }
        // And each last known peer address, across routers that drop broadcast.
        for p in lock(&self.inner.store).list() {
            for a in &p.addrs {
                if let Ok(sa) = a.parse::<SocketAddr>() {
                    let t = SocketAddr::new(sa.ip(), port);
                    if !targets.contains(&t) {
                        targets.push(t);
                    }
                }
            }
        }
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

    /// Unverified addresses learned for `peer`, newest first.
    pub(crate) fn candidates(&self, peer: &PeerId) -> Vec<SocketAddr> {
        lock(&self.inner.candidates)
            .get(peer)
            .cloned()
            .unwrap_or_default()
    }

    /// An authenticated connection succeeded through `addrs`: save them first
    /// in the peer's address list and forget the candidates.
    pub(crate) fn confirm_addrs(&self, peer: &PeerId, addrs: &[SocketAddr]) {
        lock(&self.inner.candidates).remove(peer);
        let mut store = lock(&self.inner.store);
        let Some(mut p) = store.get(peer) else {
            return;
        };
        let new: Vec<String> = addrs.iter().map(|a| a.to_string()).collect();
        if p.addrs.starts_with(&new) {
            return;
        }
        p.addrs.retain(|x| !new.contains(x));
        let mut merged = new;
        merged.append(&mut p.addrs);
        merged.truncate(MAX_ADDRS);
        p.addrs = merged;
        store.pin(p);
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
}
