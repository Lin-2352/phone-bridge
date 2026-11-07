//! Device identity: long-term Ed25519 keys, pinned peers and short
//! authentication strings (SAS) for pairing. See `docs/DECISIONS.md`.
//!
//! Secrets never leave this crate except through [`Identity::seed`], which the
//! platform layer must wrap with the OS keystore (Android Keystore / Windows DPAPI).

pub mod vault;

mod discovery_key;

pub use discovery_key::DiscoveryKey;

use std::{collections::BTreeMap, fmt};

use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use sha2::{Digest, Sha256};

/// Errors from identity operations.
#[derive(Debug, PartialEq, Eq)]
pub enum IdentityError {
    /// The OS random number generator failed.
    Random,
    /// The bytes are not a valid Ed25519 public key.
    InvalidPublicKey,
    /// The signature did not verify for this peer and message.
    BadSignature,
    /// The peer's key has low order: it would give a shared secret of zero.
    WeakPeerKey,
}

impl fmt::Display for IdentityError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Random => "OS random number generator failed",
            Self::InvalidPublicKey => "invalid Ed25519 public key",
            Self::BadSignature => "signature verification failed",
            Self::WeakPeerKey => "peer key has low order",
        })
    }
}

impl std::error::Error for IdentityError {}

/// A peer's public identity: its Ed25519 public key.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PeerId([u8; 32]);

impl PeerId {
    pub fn from_bytes(bytes: [u8; 32]) -> Result<Self, IdentityError> {
        VerifyingKey::from_bytes(&bytes).map_err(|_| IdentityError::InvalidPublicKey)?;
        Ok(Self(bytes))
    }

    /// Parse the 64-character lowercase or uppercase hex form ([`fmt::Display`]).
    pub fn from_hex(hex: &str) -> Result<Self, IdentityError> {
        if hex.len() != 64 || !hex.is_ascii() {
            return Err(IdentityError::InvalidPublicKey);
        }
        let mut bytes = [0u8; 32];
        for (i, b) in bytes.iter_mut().enumerate() {
            *b = u8::from_str_radix(&hex[2 * i..2 * i + 2], 16)
                .map_err(|_| IdentityError::InvalidPublicKey)?;
        }
        Self::from_bytes(bytes)
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// Human-comparable fingerprint: first 16 bytes of SHA-256 of the key, as
    /// eight groups of four upper-case hex digits (`ABCD-EF01-...`).
    pub fn fingerprint(&self) -> String {
        let digest = Sha256::digest(self.0);
        digest[..16]
            .chunks(2)
            .map(|c| format!("{:02X}{:02X}", c[0], c[1]))
            .collect::<Vec<_>>()
            .join("-")
    }

    /// Verify `signature` over `message` was made by this peer.
    pub fn verify(&self, message: &[u8], signature: &[u8; 64]) -> Result<(), IdentityError> {
        let key = VerifyingKey::from_bytes(&self.0).map_err(|_| IdentityError::InvalidPublicKey)?;
        key.verify(message, &Signature::from_bytes(signature))
            .map_err(|_| IdentityError::BadSignature)
    }
}

impl fmt::Debug for PeerId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "PeerId({})", self.fingerprint())
    }
}

impl fmt::Display for PeerId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for b in self.0 {
            write!(f, "{b:02x}")?;
        }
        Ok(())
    }
}

/// This device's long-term identity. The signing key is zeroized on drop.
pub struct Identity {
    key: SigningKey,
}

impl Identity {
    pub fn generate() -> Result<Self, IdentityError> {
        let mut seed = [0u8; 32];
        getrandom::fill(&mut seed).map_err(|_| IdentityError::Random)?;
        Ok(Self::from_seed(seed))
    }

    pub fn from_seed(seed: [u8; 32]) -> Self {
        Self {
            key: SigningKey::from_bytes(&seed),
        }
    }

    /// The 32-byte secret seed, for storage wrapped by the OS keystore.
    pub fn seed(&self) -> [u8; 32] {
        self.key.to_bytes()
    }

    pub fn peer_id(&self) -> PeerId {
        PeerId(self.key.verifying_key().to_bytes())
    }

    pub fn sign(&self, message: &[u8]) -> [u8; 64] {
        self.key.sign(message).to_bytes()
    }
}

impl fmt::Debug for Identity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Never print key material.
        write!(f, "Identity({})", self.peer_id().fingerprint())
    }
}

/// Six-digit short authentication string both users compare during pairing.
///
/// Symmetric: both sides get the same string regardless of which is "local".
/// Each side contributes a random nonce. The pairing protocol must have each
/// side commit to its nonce before the other reveals theirs, otherwise a man in
/// the middle can grind nonces to force a chosen SAS; that is not enforced here.
pub fn sas(local: (&PeerId, &[u8; 32]), remote: (&PeerId, &[u8; 32])) -> String {
    let (first, second) = if (local.0, local.1) <= (remote.0, remote.1) {
        (local, remote)
    } else {
        (remote, local)
    };
    let mut h = Sha256::new();
    h.update(b"phone-bridge/sas/v0");
    for (id, nonce) in [first, second] {
        h.update(id.as_bytes());
        h.update(nonce);
    }
    let d = h.finalize();
    let n = u32::from_be_bytes([d[0], d[1], d[2], d[3]]) % 1_000_000;
    format!("{n:06}")
}

/// A peer the user has explicitly paired with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PairedPeer {
    pub id: PeerId,
    /// User-visible label chosen at pairing time. Never sent in discovery.
    pub name: String,
    /// Last known direct addresses (`ip:port`), used to dial the peer.
    pub addrs: Vec<String>,
}

/// Addresses kept per peer (also the most a peer may hand us at pairing).
pub const MAX_PEER_ADDRS: usize = 6;
/// Characters kept of a peer-chosen device name.
pub const MAX_PEER_NAME: usize = 64;

/// Invisible characters that reorder or hide text (bidi overrides and
/// isolates, zero-width marks, the BOM). `char::is_control` misses them.
pub fn is_hidden_format(c: char) -> bool {
    matches!(
        c,
        '\u{061C}'
            | '\u{180E}'
            | '\u{200B}'..='\u{200F}'
            | '\u{202A}'..='\u{202E}'
            | '\u{2060}'..='\u{2064}'
            | '\u{2066}'..='\u{206F}'
            | '\u{FEFF}'
    )
}

/// A device name another device sent: controls become spaces, hidden format
/// characters go, at most [`MAX_PEER_NAME`] characters. Never holds a tab
/// or a line break.
pub fn clean_peer_name(name: &str) -> String {
    name.chars()
        .filter(|c| !is_hidden_format(*c))
        .map(|c| if c.is_control() { ' ' } else { c })
        .take(MAX_PEER_NAME)
        .collect()
}

/// Addresses another device sent: only those that parse as `ip:port`
/// (written back in canonical form), no repeats, at most [`MAX_PEER_ADDRS`].
pub fn clean_peer_addrs<S: AsRef<str>>(addrs: &[S]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for a in addrs {
        let Ok(sa) = a.as_ref().parse::<std::net::SocketAddr>() else {
            continue;
        };
        let s = sa.to_string();
        if !out.contains(&s) {
            out.push(s);
        }
        if out.len() == MAX_PEER_ADDRS {
            break;
        }
    }
    out
}

impl PairedPeer {
    /// The same peer with its name and addresses made safe to keep and show.
    pub fn cleaned(self) -> Self {
        Self {
            name: clean_peer_name(&self.name),
            addrs: clean_peer_addrs(&self.addrs),
            id: self.id,
        }
    }
}

/// Persistent set of pinned peers. Only peers in the store may connect.
pub trait PeerStore {
    fn get(&self, id: &PeerId) -> Option<PairedPeer>;
    /// Pin a peer, replacing any existing entry with the same id.
    fn pin(&mut self, peer: PairedPeer);
    /// Remove a peer; returns whether it was present. Revocation takes effect
    /// for new connections immediately; callers must also close live ones.
    fn remove(&mut self, id: &PeerId) -> bool;
    fn list(&self) -> Vec<PairedPeer>;
    /// [`Self::pin`] for a decision the user is told about: an error means the
    /// peer is not pinned (nothing changed, in memory or on disk).
    fn try_pin(&mut self, peer: PairedPeer) -> std::io::Result<()> {
        self.pin(peer);
        Ok(())
    }
    /// [`Self::remove`] for a decision the user is told about: an error means
    /// the peer is still paired (nothing changed, in memory or on disk).
    fn try_remove(&mut self, id: &PeerId) -> std::io::Result<bool> {
        Ok(self.remove(id))
    }
}

/// In-memory [`PeerStore`] used in tests and as the base for persistent stores.
#[derive(Clone, Default)]
pub struct MemoryPeerStore {
    peers: BTreeMap<PeerId, PairedPeer>,
}

impl PeerStore for MemoryPeerStore {
    fn get(&self, id: &PeerId) -> Option<PairedPeer> {
        self.peers.get(id).cloned()
    }

    fn pin(&mut self, peer: PairedPeer) {
        self.peers.insert(peer.id, peer);
    }

    fn remove(&mut self, id: &PeerId) -> bool {
        self.peers.remove(id).is_some()
    }

    fn list(&self) -> Vec<PairedPeer> {
        self.peers.values().cloned().collect()
    }
}

/// [`PeerStore`] persisted to a small text file (one peer per line:
/// `hex id<TAB>name<TAB>addr,addr`). Every change is written atomically
/// and durably (temp file, flushed to disk, then rename). What is kept is
/// [`PairedPeer::cleaned`]: names without controls, only `ip:port` addresses.
pub struct FilePeerStore {
    path: std::path::PathBuf,
    mem: MemoryPeerStore,
}

impl FilePeerStore {
    /// Open the store, creating an empty one if the file does not exist.
    /// Malformed lines are skipped rather than failing the whole store.
    pub fn open(path: impl Into<std::path::PathBuf>) -> std::io::Result<Self> {
        let path = path.into();
        let mut mem = MemoryPeerStore::default();
        match std::fs::read_to_string(&path) {
            Ok(text) => {
                for line in text.lines() {
                    if let Some(peer) = parse_peer_line(line) {
                        mem.pin(peer);
                    }
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
        Ok(Self { path, mem })
    }

    fn save(&self, peers: &MemoryPeerStore) -> std::io::Result<()> {
        let mut out = String::new();
        for p in peers.list() {
            // Never a tab, line break or comma inside a field, whatever the
            // caller pinned: one line is exactly one peer.
            let p = p.cleaned();
            out.push_str(&format!("{}\t{}\t{}\n", p.id, p.name, p.addrs.join(",")));
        }
        write_atomic(&self.path, out.as_bytes())
    }
}

/// Replace `path` so that a crash or power loss leaves the old content or the
/// new one, never an empty file: the bytes reach the disk before the rename.
pub(crate) fn write_atomic(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let tmp = path.with_extension("tmp");
    let written = (|| {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        drop(f);
        rename_over(&tmp, path)
    })();
    if written.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    written?;
    // The rename itself should survive a power loss too. Best effort: once
    // the rename succeeded the new content is what is on disk, and an error
    // now would tell the caller "nothing changed" when it did (a pairing
    // reported failed but kept, a removal reported failed but done).
    // Windows cannot open a folder as a file and flushes the entry with the
    // rename.
    #[cfg(unix)]
    if let Some(dir) = path.parent() {
        let _ = std::fs::File::open(dir).and_then(|d| d.sync_all());
    }
    Ok(())
}

/// A virus scanner or backup agent holding the target for a moment makes a
/// rename on Windows fail with "access denied"; a short retry rides that out.
fn rename_over(from: &std::path::Path, to: &std::path::Path) -> std::io::Result<()> {
    let mut tries = 0;
    loop {
        match std::fs::rename(from, to) {
            Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied && tries < 4 => {
                tries += 1;
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            other => return other,
        }
    }
}

/// One store line: `id<TAB>name<TAB>addrs` (every version wrote three
/// fields; two are accepted). More fields mean the line was tampered with.
/// The `peers.txt` line parser, for the fuzz targets in `fuzz/`.
#[cfg(feature = "fuzzing")]
pub fn fuzz_parse_peer_line(line: &str) -> bool {
    parse_peer_line(line).is_some()
}

fn parse_peer_line(line: &str) -> Option<PairedPeer> {
    let parts: Vec<&str> = line.split('\t').collect();
    if !(2..=3).contains(&parts.len()) {
        return None;
    }
    let addrs: Vec<String> = parts
        .get(2)
        .map_or(Vec::new(), |a| a.split(',').map(str::to_owned).collect());
    Some(
        PairedPeer {
            id: PeerId::from_hex(parts[0]).ok()?,
            name: parts[1].to_owned(),
            addrs,
        }
        .cleaned(),
    )
}

impl PeerStore for FilePeerStore {
    fn get(&self, id: &PeerId) -> Option<PairedPeer> {
        self.mem.get(id)
    }

    /// Best effort, for what nobody is told about (a new address for a peer
    /// that is already paired): on I/O failure the in-memory change is kept
    /// and reaches the file with the next successful save. Use
    /// [`PeerStore::try_pin`] when the user is told the peer is paired.
    fn pin(&mut self, peer: PairedPeer) {
        self.mem.pin(peer.cleaned());
        let _ = self.save(&self.mem);
    }

    /// False also when the removal could not be saved: the peer is still paired.
    fn remove(&mut self, id: &PeerId) -> bool {
        self.try_remove(id).unwrap_or(false)
    }

    fn list(&self) -> Vec<PairedPeer> {
        self.mem.list()
    }

    fn try_pin(&mut self, peer: PairedPeer) -> std::io::Result<()> {
        let mut next = self.mem.clone();
        next.pin(peer.cleaned());
        self.save(&next)?;
        self.mem = next;
        Ok(())
    }

    fn try_remove(&mut self, id: &PeerId) -> std::io::Result<bool> {
        let mut next = self.mem.clone();
        if !next.remove(id) {
            return Ok(false);
        }
        self.save(&next)?;
        self.mem = next;
        Ok(true)
    }
}

/// Load the identity seed from `path`, or create a new identity and save it.
/// The file holds the raw 32-byte seed; wrapping it with the OS keystore is
/// tracked in docs/THREAT_MODEL.md.
pub fn load_or_create_identity(path: &std::path::Path) -> std::io::Result<Identity> {
    match std::fs::read(path) {
        Ok(bytes) => {
            let seed: [u8; 32] = bytes.as_slice().try_into().map_err(|_| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "identity file is not 32 bytes",
                )
            })?;
            Ok(Identity::from_seed(seed))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let id = Identity::generate().map_err(std::io::Error::other)?;
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir)?;
            }
            write_atomic(path, &id.seed())?;
            Ok(id)
        }
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(seed: u8) -> Identity {
        Identity::from_seed([seed; 32])
    }

    #[test]
    fn sign_and_verify_round_trip() {
        let a = id(1);
        let sig = a.sign(b"hello");
        assert_eq!(a.peer_id().verify(b"hello", &sig), Ok(()));
    }

    #[test]
    fn verify_rejects_wrong_message_and_wrong_peer() {
        let (a, b) = (id(1), id(2));
        let sig = a.sign(b"hello");
        assert_eq!(
            a.peer_id().verify(b"hullo", &sig),
            Err(IdentityError::BadSignature)
        );
        assert_eq!(
            b.peer_id().verify(b"hello", &sig),
            Err(IdentityError::BadSignature)
        );
    }

    #[test]
    fn seed_round_trip_keeps_identity() {
        let a = Identity::generate().unwrap();
        let b = Identity::from_seed(a.seed());
        assert_eq!(a.peer_id(), b.peer_id());
    }

    #[test]
    fn generated_identities_differ() {
        let (a, b) = (Identity::generate().unwrap(), Identity::generate().unwrap());
        assert_ne!(a.peer_id(), b.peer_id());
    }

    #[test]
    fn fingerprint_is_stable_and_well_formed() {
        let fp = id(7).peer_id().fingerprint();
        assert_eq!(fp, id(7).peer_id().fingerprint());
        assert_eq!(fp.len(), 8 * 4 + 7);
        assert!(fp
            .chars()
            .all(|c| c == '-' || c.is_ascii_digit() || ('A'..='F').contains(&c)));
    }

    #[test]
    fn debug_never_leaks_the_seed() {
        let a = id(9);
        let shown = format!("{a:?}");
        let seed_hex: String = a.seed().iter().map(|b| format!("{b:02x}")).collect();
        assert!(!shown.contains(&seed_hex));
    }

    #[test]
    fn peer_id_rejects_invalid_public_key() {
        // y = 2 is not on the curve for Ed25519.
        let mut bad = [0u8; 32];
        bad[0] = 2;
        assert_eq!(
            PeerId::from_bytes(bad),
            Err(IdentityError::InvalidPublicKey)
        );
    }

    #[test]
    fn sas_is_symmetric_six_digits_and_nonce_sensitive() {
        let (a, b) = (id(1).peer_id(), id(2).peer_id());
        let (na, nb) = ([3u8; 32], [4u8; 32]);
        let one = sas((&a, &na), (&b, &nb));
        let other_side = sas((&b, &nb), (&a, &na));
        assert_eq!(one, other_side);
        assert_eq!(one.len(), 6);
        assert!(one.chars().all(|c| c.is_ascii_digit()));
        assert_ne!(one, sas((&a, &na), (&b, &[5u8; 32])));
    }

    #[test]
    fn store_pin_get_remove() {
        let mut store = MemoryPeerStore::default();
        let peer = PairedPeer {
            id: id(1).peer_id(),
            name: "Pixel".into(),
            addrs: vec!["10.0.0.2:47100".into()],
        };
        assert_eq!(store.get(&peer.id), None);
        store.pin(peer.clone());
        assert_eq!(store.get(&peer.id), Some(peer.clone()));
        assert_eq!(store.list(), vec![peer.clone()]);
        assert!(store.remove(&peer.id));
        assert!(!store.remove(&peer.id));
        assert_eq!(store.get(&peer.id), None);
    }

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("pb-identity-test-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn hex_round_trip_and_rejects_bad_input() {
        let p = id(3).peer_id();
        assert_eq!(PeerId::from_hex(&p.to_string()), Ok(p));
        assert_eq!(PeerId::from_hex(&p.to_string().to_uppercase()), Ok(p));
        assert!(PeerId::from_hex("abc").is_err());
        assert!(PeerId::from_hex(&"zz".repeat(32)).is_err());
        assert!(PeerId::from_hex(&"é".repeat(32)).is_err());
    }

    #[test]
    fn file_store_persists_across_reopen_and_sanitises_names() {
        let dir = temp_dir("store");
        let path = dir.join("peers.txt");
        let peer = PairedPeer {
            id: id(1).peer_id(),
            name: "My\tPhone\n".into(),
            addrs: vec!["10.0.0.2:47100".into(), "[::1]:5".into()],
        };
        {
            let mut s = FilePeerStore::open(&path).unwrap();
            assert!(s.list().is_empty());
            s.pin(peer.clone());
        }
        let s = FilePeerStore::open(&path).unwrap();
        let got = s.get(&peer.id).unwrap();
        assert_eq!(got.name, "My Phone ");
        assert_eq!(got.addrs, peer.addrs);
        let mut s = s;
        assert!(s.remove(&peer.id));
        assert!(FilePeerStore::open(&path).unwrap().list().is_empty());
    }

    #[test]
    fn file_store_skips_malformed_lines() {
        let dir = temp_dir("malformed");
        let path = dir.join("peers.txt");
        let good = id(2).peer_id();
        std::fs::write(&path, format!("garbage\nnothex\tx\t\n{good}\tLaptop\t\n")).unwrap();
        let s = FilePeerStore::open(&path).unwrap();
        assert_eq!(s.list().len(), 1);
        assert_eq!(s.get(&good).unwrap().addrs, Vec::<String>::new());
    }

    fn peer(seed: u8) -> PairedPeer {
        PairedPeer {
            id: id(seed).peer_id(),
            name: format!("peer {seed}"),
            addrs: vec!["10.0.0.2:47100".into()],
        }
    }

    /// A folder where the temp file goes makes every save fail, on any OS.
    fn break_saves(path: &std::path::Path) -> std::path::PathBuf {
        let tmp = path.with_extension("tmp");
        std::fs::create_dir_all(&tmp).unwrap();
        tmp
    }

    #[test]
    fn a_remove_that_cannot_be_saved_reports_it_and_changes_nothing() {
        let path = temp_dir("remove-fail").join("peers.txt");
        let mut s = FilePeerStore::open(&path).unwrap();
        let p = peer(1);
        s.try_pin(p.clone()).unwrap();
        let block = break_saves(&path);
        assert!(s.try_remove(&p.id).is_err(), "the failure must be reported");
        assert_eq!(s.get(&p.id), Some(p.clone()), "still paired, as on disk");
        assert!(!s.remove(&p.id), "the plain call does not claim success");
        assert_eq!(s.get(&p.id), Some(p.clone()));
        assert!(FilePeerStore::open(&path).unwrap().get(&p.id).is_some());
        // Trying again once the problem is gone works, and survives a restart.
        std::fs::remove_dir(&block).unwrap();
        assert!(s.try_remove(&p.id).unwrap());
        assert!(FilePeerStore::open(&path).unwrap().get(&p.id).is_none());
    }

    #[test]
    fn a_pin_that_cannot_be_saved_reports_it_and_changes_nothing() {
        let path = temp_dir("pin-fail").join("peers.txt");
        let mut s = FilePeerStore::open(&path).unwrap();
        let old = peer(1);
        s.try_pin(old.clone()).unwrap();
        let block = break_saves(&path);
        assert!(s.try_pin(peer(2)).is_err());
        assert_eq!(s.get(&peer(2).id), None);
        let renamed = PairedPeer {
            name: "other".into(),
            ..old.clone()
        };
        assert!(s.try_pin(renamed.clone()).is_err());
        assert_eq!(s.get(&old.id), Some(old), "the old entry is kept");
        std::fs::remove_dir(&block).unwrap();
        s.try_pin(renamed.clone()).unwrap();
        assert_eq!(FilePeerStore::open(&path).unwrap().list(), vec![renamed]);
    }

    #[test]
    fn a_peer_supplied_address_cannot_plant_a_second_peer() {
        let path = temp_dir("plant").join("peers.txt");
        let planted = id(2).peer_id();
        let evil = PairedPeer {
            id: id(1).peer_id(),
            name: format!("A\n{planted}\tB\t"),
            addrs: vec![
                format!("1.2.3.4:1\n{planted}\tMy Phone\t10.0.0.9:1"),
                "10.0.0.2:47100".into(),
                "1.2.3.4:5,6.7.8.9:10".into(),
                "not an address".into(),
            ],
        };
        let mut s = FilePeerStore::open(&path).unwrap();
        s.try_pin(evil.clone()).unwrap();
        s.pin(evil.clone());
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(text.lines().count(), 1, "{text:?}");
        let back = FilePeerStore::open(&path).unwrap();
        assert_eq!(back.list().len(), 1);
        assert!(back.get(&planted).is_none());
        let got = back.get(&evil.id).unwrap();
        assert_eq!(got.addrs, vec!["10.0.0.2:47100".to_string()]);
        assert!(!got.name.contains(['\t', '\n', '\r']));
        assert_eq!(s.get(&evil.id), Some(got), "memory matches the file");
    }

    #[test]
    fn names_and_address_lists_are_bounded() {
        let addrs: Vec<String> = (1..=9).map(|i| format!("10.0.0.{i}:1")).collect();
        assert_eq!(clean_peer_addrs(&addrs), addrs[..MAX_PEER_ADDRS].to_vec());
        assert_eq!(
            clean_peer_addrs(&["[::1]:5", "[::1]:5", "x:1", "10.0.0.1"]),
            vec!["[::1]:5".to_string()]
        );
        let long = "é".repeat(500);
        assert_eq!(clean_peer_name(&long).chars().count(), MAX_PEER_NAME);
        assert_eq!(
            clean_peer_name("Pixel\u{202e}gpj.exe\t\n"),
            "Pixelgpj.exe  "
        );
    }

    #[test]
    fn open_refuses_lines_with_extra_fields_and_keeps_old_ones() {
        let path = temp_dir("fields").join("peers.txt");
        let (a, b, c) = (id(1).peer_id(), id(2).peer_id(), id(3).peer_id());
        std::fs::write(
            &path,
            format!(
                "{a}\tOld\t10.0.0.2:47100,[::1]:5\n{b}\tTwo fields\n{c}\tX\t10.0.0.3:1\textra\n"
            ),
        )
        .unwrap();
        let s = FilePeerStore::open(&path).unwrap();
        let ids: Vec<PeerId> = s.list().iter().map(|p| p.id).collect();
        assert!(ids.contains(&a) && ids.contains(&b), "old lines still load");
        assert!(!ids.contains(&c), "a line with a fourth field is refused");
        assert_eq!(
            s.get(&a).unwrap().addrs,
            vec!["10.0.0.2:47100".to_string(), "[::1]:5".to_string()]
        );
    }

    #[test]
    fn identity_file_is_created_once_then_reused() {
        let dir = temp_dir("seed");
        let path = dir.join("sub").join("identity.key");
        let a = load_or_create_identity(&path).unwrap();
        let b = load_or_create_identity(&path).unwrap();
        assert_eq!(a.peer_id(), b.peer_id());
        std::fs::write(&path, [0u8; 5]).unwrap();
        assert!(load_or_create_identity(&path).is_err());
    }
}
