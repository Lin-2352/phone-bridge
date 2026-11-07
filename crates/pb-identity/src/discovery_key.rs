//! The per-pair key for keyed discovery (PBD2, M7.5). Both sides derive the same 32-byte key from the two paired
//! Ed25519 identities, with no new secret to store and no re-pairing:
//!
//! `dh = X25519(own_secret, peer_public)`, `k = HKDF-SHA256(salt = "pb-discovery/2", ikm = dh, info = min(id) ‖ max(id))`.
//!
//! An all-zero shared secret (a low-order peer key) is refused.

use ed25519_dalek::VerifyingKey;
use hkdf::Hkdf;
use hmac::{Hmac, Mac};
use sha2::Sha256;
use zeroize::Zeroize;

use crate::{Identity, IdentityError, PeerId};

/// The HKDF salt, versioned with the packet format.
const SALT: &[u8] = b"pb-discovery/2";

/// The shared key of one paired pair. Zeroized on drop; `Debug` shows nothing.
pub struct DiscoveryKey([u8; 32]);

impl DiscoveryKey {
    /// HMAC-SHA256 over the concatenation of `parts`, truncated to 16 bytes.
    pub fn mac(&self, parts: &[&[u8]]) -> [u8; 16] {
        // HMAC accepts keys of any length, so this cannot fail for a 32-byte key.
        let mut m =
            <Hmac<Sha256> as Mac>::new_from_slice(&self.0).expect("HMAC takes any key length");
        for p in parts {
            m.update(p);
        }
        let full = m.finalize().into_bytes();
        let mut out = [0u8; 16];
        out.copy_from_slice(&full[..16]);
        out
    }
}

impl Drop for DiscoveryKey {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

impl std::fmt::Debug for DiscoveryKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("DiscoveryKey(..)")
    }
}

impl Identity {
    /// The discovery key shared with the paired `peer`; both sides compute the same one. A peer key of low
    /// order (an all-zero shared secret) is refused with [`IdentityError::WeakPeerKey`].
    pub fn discovery_key(&self, peer: &PeerId) -> Result<DiscoveryKey, IdentityError> {
        let their = VerifyingKey::from_bytes(peer.as_bytes())
            .map_err(|_| IdentityError::InvalidPublicKey)?;
        // Ed25519 keys are Edwards points; X25519 works on the Montgomery form of the same curve.
        let dh = their
            .to_montgomery()
            .mul_clamped(self.key.to_scalar_bytes())
            .to_bytes();
        if dh.iter().all(|b| *b == 0) {
            return Err(IdentityError::WeakPeerKey);
        }
        let own = self.peer_id();
        let (lo, hi) = if own.as_bytes() <= peer.as_bytes() {
            (own.as_bytes(), peer.as_bytes())
        } else {
            (peer.as_bytes(), own.as_bytes())
        };
        let mut info = [0u8; 64];
        info[..32].copy_from_slice(lo);
        info[32..].copy_from_slice(hi);
        let mut okm = [0u8; 32];
        Hkdf::<Sha256>::new(Some(SALT), &dh)
            .expand(&info, &mut okm)
            .map_err(|_| IdentityError::WeakPeerKey)?;
        Ok(DiscoveryKey(okm))
    }
}

#[cfg(test)]
mod tests {
    use crate::{Identity, IdentityError, PeerId};

    fn pair() -> (Identity, Identity) {
        (Identity::from_seed([1; 32]), Identity::from_seed([2; 32]))
    }

    #[test]
    fn both_sides_derive_the_same_key() {
        let (a, b) = pair();
        let ka = a.discovery_key(&b.peer_id()).unwrap();
        let kb = b.discovery_key(&a.peer_id()).unwrap();
        assert_eq!(ka.mac(&[b"probe", b"x"]), kb.mac(&[b"probe", b"x"]));
    }

    #[test]
    fn another_pair_gets_another_key() {
        let (a, b) = pair();
        let c = Identity::from_seed([3; 32]);
        let ab = a.discovery_key(&b.peer_id()).unwrap();
        let ac = a.discovery_key(&c.peer_id()).unwrap();
        assert_ne!(ab.mac(&[b"m"]), ac.mac(&[b"m"]));
    }

    #[test]
    fn a_low_order_peer_key_is_rejected() {
        // The identity point (y = 1) is a valid encoding; its Montgomery u is 0, so the shared secret is all zero.
        let mut id = [0u8; 32];
        id[0] = 1;
        let weak = PeerId::from_bytes(id).expect("the identity point decodes");
        let (a, _) = pair();
        assert_eq!(
            a.discovery_key(&weak).err(),
            Some(IdentityError::WeakPeerKey)
        );
    }

    /// Cross-checked with an independent implementation (Python `cryptography`: Ed25519 to X25519, HKDF-SHA256,
    /// HMAC-SHA256), not with this crate's own code, so a mistake in the derivation cannot hide itself.
    #[test]
    fn known_answer_for_fixed_seeds() {
        let (a, b) = pair();
        let k = a.discovery_key(&b.peer_id()).unwrap();
        let hex: String = k
            .mac(&[b"probe", b"known answer"])
            .iter()
            .map(|x| format!("{x:02x}"))
            .collect();
        assert_eq!(hex, "36939e59410da7fb99696d5f595ca42f");
    }

    #[test]
    fn the_key_never_prints() {
        let (a, b) = pair();
        let k = a.discovery_key(&b.peer_id()).unwrap();
        assert_eq!(format!("{k:?}"), "DiscoveryKey(..)");
    }

    #[test]
    fn the_mac_is_16_bytes_and_covers_every_part() {
        let (a, b) = pair();
        let k = a.discovery_key(&b.peer_id()).unwrap();
        assert_eq!(k.mac(&[b"ab"]).len(), 16);
        assert_ne!(k.mac(&[b"a", b"b"]), k.mac(&[b"a", b"c"]));
    }
}
