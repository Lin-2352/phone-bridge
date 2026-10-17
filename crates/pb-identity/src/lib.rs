//! Device identity: long-term Ed25519 keys, pinned peers and short
//! authentication strings (SAS) for pairing. See `docs/DECISIONS.md`.
//!
//! Secrets never leave this crate except through [`Identity::seed`], which the
//! platform layer must wrap with the OS keystore (Android Keystore / Windows DPAPI).

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
}

impl fmt::Display for IdentityError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Random => "OS random number generator failed",
            Self::InvalidPublicKey => "invalid Ed25519 public key",
            Self::BadSignature => "signature verification failed",
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
}

/// In-memory [`PeerStore`] used in tests and as the base for persistent stores.
#[derive(Default)]
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
        };
        assert_eq!(store.get(&peer.id), None);
        store.pin(peer.clone());
        assert_eq!(store.get(&peer.id), Some(peer.clone()));
        assert_eq!(store.list(), vec![peer.clone()]);
        assert!(store.remove(&peer.id));
        assert!(!store.remove(&peer.id));
        assert_eq!(store.get(&peer.id), None);
    }
}
