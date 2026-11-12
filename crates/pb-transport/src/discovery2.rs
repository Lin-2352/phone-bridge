//! Keyed discovery, PBD2 (M7.6): the packets, the per-peer marks and the replay cache. Pure (no sockets), so
//! every rule is unit-tested; `discovery.rs` drives it.
//!
//! A probe or answer is authenticated with the pair's [`DiscoveryKey`] (derived in `pb-identity` from the two
//! paired identities), so a LAN host that does not hold a pairing cannot make a node dial an address of its
//! choosing, and a passive observer cannot tell which paired peers a device has.
//!
//! Packets (all integers little-endian; `mac` is 16 bytes):
//!
//! * probe, 39 bytes: `"PBD2" | 1 | quic port u16 | nonce[16] | mac`
//! * answer, 56 to 170 bytes: `"PBD2" | 2 | quic port u16 | probe_nonce[16] | nonce[16] | n u8 (at most 6) |
//!   n x (family u8 4 or 6 | ip 4 or 16 | port u16) | mac`
//!
//! `mac = HMAC-SHA256(k, label | window u64 | every byte before the mac)[..16]`, where `label` is
//! `"pb-discovery/2 probe"` or `"pb-discovery/2 answer"`, and `window` is the 30-minute window (neighbours
//! either side are accepted, for clock skew). The mac covers the exact bytes on the wire, so what is
//! authenticated cannot drift from what is parsed.

use std::{
    collections::{HashSet, VecDeque},
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    path::PathBuf,
};

use pb_identity::{DiscoveryKey, PeerId};

pub const MAGIC2: &[u8; 4] = b"PBD2";
pub const PROBE2: u8 = 1;
pub const ANSWER2: u8 = 2;
/// The receive buffer: nothing valid is longer.
pub const MAX_PACKET2: usize = 256;
/// Addresses in one answer.
pub const MAX_ANSWER_ADDRS: usize = 6;
/// A probe round always sends at least this many probes, so the count does not show how many peers are paired.
pub const MIN_PROBES: usize = 4;
/// `(peer, nonce)` pairs remembered to refuse a replay.
pub const REPLAY_CAP: usize = 1024;
/// Peers marked as speaking PBD2.
pub const MAX_MARKS: usize = 64;

const PROBE_LEN: usize = 4 + 1 + 2 + 16 + 16;
const PROBE_LABEL: &[u8] = b"pb-discovery/2 probe";
const ANSWER_LABEL: &[u8] = b"pb-discovery/2 answer";

/// A parsed, not yet authenticated packet. `body` is every byte before the mac.
#[derive(Debug, PartialEq, Eq)]
pub struct Parsed2<'a> {
    pub kind: u8,
    pub port: u16,
    /// For an answer: the nonce of the probe it answers.
    pub probe_nonce: Option<[u8; 16]>,
    pub nonce: [u8; 16],
    pub addrs: Vec<SocketAddr>,
    pub mac: [u8; 16],
    pub body: &'a [u8],
}

fn label(kind: u8) -> &'static [u8] {
    if kind == PROBE2 {
        PROBE_LABEL
    } else {
        ANSWER_LABEL
    }
}

/// Parses a packet strictly: the exact length, at most [`MAX_ANSWER_ADDRS`] addresses, known families only. Anything
/// else is `None` (the caller drops it without any further work).
pub fn parse2(buf: &[u8]) -> Option<Parsed2<'_>> {
    if buf.len() > MAX_PACKET2 || buf.len() < PROBE_LEN || &buf[..4] != MAGIC2 {
        return None;
    }
    let kind = buf[4];
    let port = u16::from_le_bytes([buf[5], buf[6]]);
    let take16 = |at: usize| -> [u8; 16] { buf[at..at + 16].try_into().expect("16 bytes") };
    match kind {
        PROBE2 => {
            if buf.len() != PROBE_LEN {
                return None;
            }
            Some(Parsed2 {
                kind,
                port,
                probe_nonce: None,
                nonce: take16(7),
                addrs: Vec::new(),
                mac: take16(23),
                body: &buf[..23],
            })
        }
        ANSWER2 => {
            // Fixed part: magic, kind, port, probe nonce, nonce, count; then the addresses; then the mac.
            const FIXED: usize = 4 + 1 + 2 + 16 + 16 + 1;
            if buf.len() < FIXED + 16 {
                return None;
            }
            let n = buf[39] as usize;
            if n > MAX_ANSWER_ADDRS {
                return None;
            }
            let mut at = FIXED;
            let mut addrs = Vec::with_capacity(n);
            for _ in 0..n {
                let family = *buf.get(at)?;
                at += 1;
                let ip = match family {
                    4 => {
                        let o: [u8; 4] = buf.get(at..at + 4)?.try_into().ok()?;
                        at += 4;
                        IpAddr::V4(Ipv4Addr::from(o))
                    }
                    6 => {
                        let o: [u8; 16] = buf.get(at..at + 16)?.try_into().ok()?;
                        at += 16;
                        IpAddr::V6(Ipv6Addr::from(o))
                    }
                    _ => return None,
                };
                let p = u16::from_le_bytes(buf.get(at..at + 2)?.try_into().ok()?);
                at += 2;
                addrs.push(SocketAddr::new(ip, p));
            }
            // The mac is exactly the last 16 bytes: nothing may sit between the addresses and it.
            if buf.len() != at + 16 {
                return None;
            }
            Some(Parsed2 {
                kind,
                port,
                probe_nonce: Some(take16(7)),
                nonce: take16(23),
                addrs,
                mac: take16(at),
                body: &buf[..at],
            })
        }
        _ => None,
    }
}

fn mac_of(key: &DiscoveryKey, kind: u8, window: u64, body: &[u8]) -> [u8; 16] {
    key.mac(&[label(kind), &window.to_le_bytes(), body])
}

/// A probe for the pair holding `key`, in `window`.
pub fn build_probe(key: &DiscoveryKey, port: u16, nonce: [u8; 16], window: u64) -> Vec<u8> {
    let mut p = Vec::with_capacity(PROBE_LEN);
    p.extend_from_slice(MAGIC2);
    p.push(PROBE2);
    p.extend_from_slice(&port.to_le_bytes());
    p.extend_from_slice(&nonce);
    let mac = mac_of(key, PROBE2, window, &p);
    p.extend_from_slice(&mac);
    p
}

/// A probe-shaped packet with a random mac: it verifies for no one, and hides how many peers are paired.
pub fn build_dummy_probe(port: u16, nonce: [u8; 16], mac: [u8; 16]) -> Vec<u8> {
    let mut p = Vec::with_capacity(PROBE_LEN);
    p.extend_from_slice(MAGIC2);
    p.push(PROBE2);
    p.extend_from_slice(&port.to_le_bytes());
    p.extend_from_slice(&nonce);
    p.extend_from_slice(&mac);
    p
}

/// An answer to the probe `probe_nonce`, listing at most [`MAX_ANSWER_ADDRS`] of `addrs`.
pub fn build_answer(
    key: &DiscoveryKey,
    port: u16,
    probe_nonce: [u8; 16],
    nonce: [u8; 16],
    addrs: &[SocketAddr],
    window: u64,
) -> Vec<u8> {
    let addrs = &addrs[..addrs.len().min(MAX_ANSWER_ADDRS)];
    let mut p = Vec::with_capacity(MAX_PACKET2);
    p.extend_from_slice(MAGIC2);
    p.push(ANSWER2);
    p.extend_from_slice(&port.to_le_bytes());
    p.extend_from_slice(&probe_nonce);
    p.extend_from_slice(&nonce);
    p.push(addrs.len() as u8);
    for a in addrs {
        match a.ip() {
            IpAddr::V4(ip) => {
                p.push(4);
                p.extend_from_slice(&ip.octets());
            }
            IpAddr::V6(ip) => {
                p.push(6);
                p.extend_from_slice(&ip.octets());
            }
        }
        p.extend_from_slice(&a.port().to_le_bytes());
    }
    let mac = mac_of(key, ANSWER2, window, &p);
    p.extend_from_slice(&mac);
    p
}

fn ct_eq(a: &[u8; 16], b: &[u8; 16]) -> bool {
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Whether `p` carries a valid mac for `key` in `window`.
pub fn verify(key: &DiscoveryKey, window: u64, p: &Parsed2<'_>) -> bool {
    ct_eq(&mac_of(key, p.kind, window, p.body), &p.mac)
}

/// Which peer sent `p`: the first of `keys` that verifies it in any of `windows`. Bounds were checked by
/// [`parse2`] before this runs, and only these cached keys are tried.
pub fn authenticate(
    keys: &[(PeerId, &DiscoveryKey)],
    windows: &[u64],
    p: &Parsed2<'_>,
) -> Option<PeerId> {
    keys.iter()
        .find(|(_, k)| windows.iter().any(|w| verify(k, *w, p)))
        .map(|(id, _)| *id)
}

/// `(peer, nonce)` pairs seen on authenticated packets. Only a packet whose mac verified is ever inserted, so
/// unauthenticated noise cannot flush it and make a replay look new.
#[derive(Default)]
pub struct ReplayCache {
    order: VecDeque<(PeerId, [u8; 16])>,
    seen: HashSet<(PeerId, [u8; 16])>,
}

impl ReplayCache {
    /// Records the pair; `false` when it was already there (a replay).
    pub fn insert(&mut self, peer: PeerId, nonce: [u8; 16]) -> bool {
        if !self.seen.insert((peer, nonce)) {
            return false;
        }
        self.order.push_back((peer, nonce));
        while self.order.len() > REPLAY_CAP {
            if let Some(old) = self.order.pop_front() {
                self.seen.remove(&old);
            }
        }
        true
    }

    pub fn len(&self) -> usize {
        self.order.len()
    }

    pub fn is_empty(&self) -> bool {
        self.order.is_empty()
    }
}

/// How many probes a round sends for `peers` PBD2 peers: one each, and never fewer than [`MIN_PROBES`].
pub fn probes_to_send(peers: usize) -> usize {
    peers.max(MIN_PROBES)
}

/// The peers known to speak PBD2, kept in a file next to the peer list (never a field of the peer list: an
/// older build rejects a line with an extra field and would lose its pairings).
#[derive(Default)]
pub struct Marks {
    peers: Vec<PeerId>,
    path: Option<PathBuf>,
}

impl Marks {
    /// Reads `path` (a missing or unreadable file is empty; bad lines are skipped; at most [`MAX_MARKS`]).
    pub fn load(path: PathBuf) -> Self {
        let mut peers = Vec::new();
        if let Ok(text) = std::fs::read_to_string(&path) {
            for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
                if let Ok(id) = PeerId::from_hex(line) {
                    if !peers.contains(&id) && peers.len() < MAX_MARKS {
                        peers.push(id);
                    }
                }
            }
        }
        Self {
            peers,
            path: Some(path),
        }
    }

    pub fn contains(&self, peer: &PeerId) -> bool {
        self.peers.contains(peer)
    }

    pub fn peers(&self) -> &[PeerId] {
        &self.peers
    }

    /// Marks `peer`; true when that changed anything (and the file was rewritten).
    pub fn mark(&mut self, peer: PeerId) -> bool {
        if self.peers.contains(&peer) || self.peers.len() >= MAX_MARKS {
            return false;
        }
        self.peers.push(peer);
        self.save();
        true
    }

    /// Removes the mark (the peer was unpaired, or stopped advertising PBD2).
    pub fn unmark(&mut self, peer: &PeerId) -> bool {
        let before = self.peers.len();
        self.peers.retain(|p| p != peer);
        let changed = self.peers.len() != before;
        if changed {
            self.save();
        }
        changed
    }

    /// Writes the file atomically (a temporary file, then a rename). A failure leaves the old file.
    fn save(&self) {
        let Some(path) = &self.path else { return };
        let text: String = self.peers.iter().map(|p| format!("{p}\n")).collect();
        let tmp = path.with_extension("tmp");
        if std::fs::write(&tmp, text).is_ok() {
            let _ = std::fs::rename(&tmp, path);
        }
    }
}

/// The PBD2 parser and the checks that follow it, for the fuzz targets in `tools/fuzz/`: whatever the bytes are, parsing
/// and verifying them never panics, and a packet that parses stays within the bounds.
#[cfg(feature = "fuzzing")]
pub fn fuzz_parse2(buf: &[u8]) -> bool {
    let Some(p) = parse2(buf) else {
        return false;
    };
    assert!(buf.len() <= MAX_PACKET2 && p.addrs.len() <= MAX_ANSWER_ADDRS);
    assert_eq!(
        p.body.len() + 16,
        buf.len(),
        "the mac is the last 16 bytes and nothing else follows"
    );
    // Verify against a fixed pair key: this runs the mac over arbitrary bodies.
    let a = pb_identity::Identity::from_seed([1; 32]);
    let b = pb_identity::Identity::from_seed([2; 32]);
    if let Ok(k) = a.discovery_key(&b.peer_id()) {
        let _ = verify(&k, 0, &p);
    }
    true
}

/// What one probe round sends: PBD2 probes, and whether PBD1 is still needed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProbePlan {
    /// PBD2 probes: one per marked peer, padded to [`MIN_PROBES`].
    pub v2: usize,
    /// PBD1 is sent only while some paired peer is not marked (it may be an older build).
    pub v1: bool,
}

/// `paired` peers are paired, `marked` of them speak PBD2. Once a peer has announced PBD2, PBD1 is no longer sent
/// to it; PBD1 stays only for the peers that have not.
pub fn probe_plan(paired: usize, marked: usize) -> ProbePlan {
    ProbePlan {
        v2: probes_to_send(marked),
        v1: marked < paired || paired == 0,
    }
}

#[cfg(test)]
mod tests {
    use pb_identity::Identity;

    use super::*;

    fn pair() -> (Identity, Identity) {
        (Identity::from_seed([11; 32]), Identity::from_seed([12; 32]))
    }

    fn keys(a: &Identity, b: &Identity) -> (DiscoveryKey, DiscoveryKey) {
        (
            a.discovery_key(&b.peer_id()).unwrap(),
            b.discovery_key(&a.peer_id()).unwrap(),
        )
    }

    const W: u64 = 1_000;
    const N: [u8; 16] = [7; 16];

    #[test]
    fn probe_round_trips() {
        let (a, b) = pair();
        let (ka, kb) = keys(&a, &b);
        let pkt = build_probe(&ka, 47100, N, W);
        assert_eq!(pkt.len(), 39);
        let p = parse2(&pkt).expect("parses");
        assert_eq!((p.kind, p.port, p.nonce), (PROBE2, 47100, N));
        assert!(verify(&kb, W, &p), "the other side of the pair verifies it");
        assert!(!verify(&kb, W + 1, &p), "a different window does not");
    }

    #[test]
    fn a_wrong_key_probe_is_ignored() {
        let (a, b) = pair();
        let c = Identity::from_seed([13; 32]);
        let (ka, kb) = keys(&a, &b);
        let (kc, _) = keys(&c, &b);
        let pkt = build_probe(&kc, 47100, N, W);
        let p = parse2(&pkt).unwrap();
        assert!(
            !verify(&kb, W, &p),
            "a stranger's probe does not verify for the pair a-b"
        );
        let ring = vec![(a.peer_id(), &kb), (c.peer_id(), &ka)];
        assert_eq!(authenticate(&ring, &[W - 1, W, W + 1], &p), None);
    }

    #[test]
    fn a_replayed_probe_is_ignored() {
        let (a, _) = pair();
        let mut cache = ReplayCache::default();
        assert!(cache.insert(a.peer_id(), N));
        assert!(
            !cache.insert(a.peer_id(), N),
            "the same (peer, nonce) a second time is a replay"
        );
        assert!(cache.insert(a.peer_id(), [8; 16]), "another nonce is fine");
    }

    #[test]
    fn unauthenticated_noise_cannot_flush_the_replay_cache() {
        // The caller inserts only after the mac verified; this pins that a flood of bad packets adds nothing.
        let (a, b) = pair();
        let (ka, kb) = keys(&a, &b);
        let mut cache = ReplayCache::default();
        let good = build_probe(&ka, 47100, N, W);
        let ring = vec![(a.peer_id(), &kb)];
        let accept = |pkt: &[u8], cache: &mut ReplayCache| -> bool {
            let Some(p) = parse2(pkt) else { return false };
            let Some(peer) = authenticate(&ring, &[W], &p) else {
                return false;
            };
            cache.insert(peer, p.nonce)
        };
        assert!(accept(&good, &mut cache));
        for i in 0..(REPLAY_CAP * 2) {
            let junk = build_dummy_probe(1, (i as u128).to_le_bytes(), [9; 16]);
            assert!(!accept(&junk, &mut cache));
        }
        assert_eq!(cache.len(), 1, "no junk entered the cache");
        assert!(!accept(&good, &mut cache), "so the replay is still refused");
    }

    #[test]
    fn the_replay_cache_is_bounded() {
        let (a, _) = pair();
        let mut cache = ReplayCache::default();
        for i in 0..(REPLAY_CAP as u128 + 50) {
            cache.insert(a.peer_id(), i.to_le_bytes());
        }
        assert_eq!(cache.len(), REPLAY_CAP);
    }

    #[test]
    fn after_discovery2_pbd1_is_neither_sent_nor_answered() {
        // Every paired peer speaks PBD2: PBD1 is dropped from the round.
        assert_eq!(probe_plan(2, 2), ProbePlan { v2: 4, v1: false });
        // One of them is an older build: PBD1 stays, so that one is still found.
        assert!(probe_plan(2, 1).v1);
        assert!(probe_plan(3, 0).v1);
        // Nobody paired: the legacy probe is harmless and keeps a first pairing from looking different.
        assert!(probe_plan(0, 0).v1);
    }

    #[test]
    fn probes_are_padded_to_four() {
        assert_eq!(probes_to_send(0), 4);
        assert_eq!(probes_to_send(1), 4);
        assert_eq!(probes_to_send(4), 4);
        assert_eq!(probes_to_send(9), 9);
        // A dummy has the same shape as a real probe and verifies for nobody.
        let (a, b) = pair();
        let (ka, kb) = keys(&a, &b);
        let real = build_probe(&ka, 47100, N, W);
        let dummy = build_dummy_probe(47100, N, [3; 16]);
        assert_eq!(real.len(), dummy.len());
        assert!(!verify(&kb, W, &parse2(&dummy).unwrap()));
    }

    fn addr(s: &str) -> SocketAddr {
        s.parse().unwrap()
    }

    #[test]
    fn an_answer_offers_only_its_maced_addresses() {
        let (a, b) = pair();
        let (ka, kb) = keys(&a, &b);
        let offered = [addr("192.168.1.20:47100"), addr("[fe80::1]:47100")];
        let pkt = build_answer(&ka, 47100, N, [5; 16], &offered, W);
        let p = parse2(&pkt).expect("parses");
        assert_eq!(p.kind, ANSWER2);
        assert_eq!(p.probe_nonce, Some(N));
        assert_eq!(p.addrs, offered);
        assert!(verify(&kb, W, &p));
        // The address list is covered: only a packet that verified may be used.
        let ring = vec![(a.peer_id(), &kb)];
        assert_eq!(authenticate(&ring, &[W], &p), Some(a.peer_id()));
    }

    #[test]
    fn a_tampered_address_fails_the_mac() {
        let (a, b) = pair();
        let (ka, kb) = keys(&a, &b);
        let mut pkt = build_answer(&ka, 47100, N, [5; 16], &[addr("192.168.1.20:47100")], W);
        // Flip the last byte of the address (its port) without touching the mac.
        let at = pkt.len() - 16 - 1;
        pkt[at] ^= 0x01;
        let p = parse2(&pkt).expect("still well-formed");
        assert!(!verify(&kb, W, &p), "an attacker's address is not accepted");
    }

    #[test]
    fn oversized_or_too_many_addresses_are_dropped() {
        let (a, b) = pair();
        let (ka, _) = keys(&a, &b);
        let seven: Vec<SocketAddr> = (0..7)
            .map(|i| addr(&format!("10.0.0.{}:47100", i + 1)))
            .collect();
        // The builder itself keeps to six.
        let built = build_answer(&ka, 47100, N, [5; 16], &seven, W);
        assert_eq!(parse2(&built).unwrap().addrs.len(), MAX_ANSWER_ADDRS);
        // A packet claiming seven is refused.
        let mut forged = built.clone();
        forged[39] = 7;
        assert!(parse2(&forged).is_none());
        // Too long, trailing bytes, a bad family and a truncated address are all refused.
        assert!(parse2(&vec![0u8; MAX_PACKET2 + 1]).is_none());
        let mut trailing = built.clone();
        trailing.push(0);
        assert!(parse2(&trailing).is_none());
        let mut family = build_answer(&ka, 47100, N, [5; 16], &[addr("10.0.0.1:1")], W);
        family[40] = 9;
        assert!(parse2(&family).is_none());
        assert!(parse2(&built[..built.len() - 17]).is_none());
        assert!(parse2(b"PBD1").is_none());
    }

    /// A cheap stand-in for the fuzz target when libFuzzer is not available: structured and random bytes never
    /// panic the parser, and whatever parses stays within the bounds.
    #[test]
    fn parse2_never_panics_and_stays_in_bounds_on_mutated_packets() {
        let (a, b) = pair();
        let (ka, _) = keys(&a, &b);
        let v4 = [addr("10.0.0.1:1"), addr("10.0.0.2:2")];
        let v6 = [addr("[2001:db8::1]:3")];
        let seeds = [
            build_probe(&ka, 47100, N, W),
            build_answer(&ka, 47100, N, [5; 16], &v4, W),
            build_answer(&ka, 47100, N, [5; 16], &v6, W),
            build_answer(&ka, 47100, N, [5; 16], &[], W),
        ];
        let mut x = 0x9E37_79B9_7F4A_7C15u64;
        let mut next = move || {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x
        };
        for _ in 0..50_000 {
            let mut pkt = seeds[(next() % seeds.len() as u64) as usize].clone();
            for _ in 0..(next() % 4) {
                let at = (next() % pkt.len() as u64) as usize;
                pkt[at] = next() as u8;
            }
            match next() % 4 {
                0 => pkt.truncate((next() % (pkt.len() as u64 + 1)) as usize),
                1 => pkt.extend((0..next() % 8).map(|_| next() as u8)),
                _ => {}
            }
            if let Some(p) = parse2(&pkt) {
                assert!(pkt.len() <= MAX_PACKET2 && p.addrs.len() <= MAX_ANSWER_ADDRS);
                assert_eq!(p.body.len() + 16, pkt.len());
            }
        }
        // And pure noise.
        for _ in 0..20_000 {
            let len = (next() % 300) as usize;
            let noise: Vec<u8> = (0..len).map(|_| next() as u8).collect();
            let _ = parse2(&noise);
        }
    }

    #[test]
    fn the_longest_answer_fits_the_buffer() {
        let (a, b) = pair();
        let (ka, _) = keys(&a, &b);
        let v6: Vec<SocketAddr> = (0..6)
            .map(|i| addr(&format!("[2001:db8::{}]:47100", i + 1)))
            .collect();
        let pkt = build_answer(&ka, 47100, N, [5; 16], &v6, W);
        assert_eq!(pkt.len(), 4 + 1 + 2 + 16 + 16 + 1 + 6 * 19 + 16);
        assert!(pkt.len() <= MAX_PACKET2);
        assert!(parse2(&pkt).is_some());
    }

    #[test]
    fn marks_round_trip_through_the_file_and_are_capped() {
        let dir = std::env::temp_dir().join(format!("pb-marks-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("discovery2.txt");
        let _ = std::fs::remove_file(&path);
        let (a, b) = pair();
        let mut m = Marks::load(path.clone());
        assert!(m.mark(a.peer_id()));
        assert!(!m.mark(a.peer_id()), "marking twice changes nothing");
        assert!(m.mark(b.peer_id()));
        let again = Marks::load(path.clone());
        assert!(again.contains(&a.peer_id()) && again.contains(&b.peer_id()));
        let mut m = again;
        assert!(m.unmark(&a.peer_id()));
        assert!(!Marks::load(path.clone()).contains(&a.peer_id()));
        // Garbage lines are skipped, and the cap holds.
        let mut text = String::from("not a key\n\n");
        for i in 0..(MAX_MARKS as u8 + 10) {
            text += &format!(
                "{}\n",
                Identity::from_seed([i.wrapping_add(50); 32]).peer_id()
            );
        }
        std::fs::write(&path, text).unwrap();
        assert_eq!(Marks::load(path.clone()).peers().len(), MAX_MARKS);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
