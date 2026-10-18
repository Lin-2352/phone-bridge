//! Identity seed storage protected by the OS keystore.
//!
//! The seed is stored as `wrapper.wrap(seed)` in `identity.sealed`. The wrapper
//! is platform-provided: Windows DPAPI (`pb-platform-win`), Android Keystore
//! AES-GCM (Kotlin, through `pb-ffi`). A legacy plain `identity.key` from
//! Phase 1 is migrated on first open and then deleted.

use std::{
    io,
    path::{Path, PathBuf},
};

use crate::Identity;

/// Seals and unseals small secrets with a key the OS protects.
pub trait SeedWrapper: Send + Sync {
    fn wrap(&self, plain: &[u8]) -> io::Result<Vec<u8>>;
    fn unwrap(&self, sealed: &[u8]) -> io::Result<Vec<u8>>;
}

pub const SEALED_FILE: &str = "identity.sealed";
pub const LEGACY_PLAIN_FILE: &str = "identity.key";

fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(tmp, path)
}

fn seed_from(bytes: &[u8], what: &str) -> io::Result<[u8; 32]> {
    bytes.try_into().map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("{what} does not hold a 32-byte seed"),
        )
    })
}

/// Load this device's identity from `dir`, creating it if absent.
///
/// Order: sealed file -> legacy plain file (migrated, then removed) -> new identity.
/// If the sealed file exists but cannot be unsealed (e.g. keystore reset), this
/// fails rather than silently creating a new identity, so the user sees the
/// problem instead of losing every pairing.
pub fn open_identity(dir: &Path, wrapper: &dyn SeedWrapper) -> io::Result<Identity> {
    let sealed: PathBuf = dir.join(SEALED_FILE);
    let legacy = dir.join(LEGACY_PLAIN_FILE);
    match std::fs::read(&sealed) {
        Ok(bytes) => {
            let plain = wrapper.unwrap(&bytes).map_err(|e| {
                io::Error::new(
                    e.kind(),
                    format!("this device's identity could not be unsealed ({e}); the keystore may have been reset or the file damaged"),
                )
            })?;
            return Ok(Identity::from_seed(seed_from(&plain, "sealed identity")?));
        }
        Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
        Err(_) => {}
    }
    let identity = match std::fs::read(&legacy) {
        Ok(bytes) => Identity::from_seed(seed_from(&bytes, "legacy identity file")?),
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            Identity::generate().map_err(io::Error::other)?
        }
        Err(e) => return Err(e),
    };
    write_atomic(&sealed, &wrapper.wrap(&identity.seed())?)?;
    // Only delete the plain copy once the sealed one is safely on disk and
    // round-trips.
    let check = wrapper.unwrap(&std::fs::read(&sealed)?)?;
    if check.as_slice() != identity.seed().as_slice() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "sealed identity did not round-trip",
        ));
    }
    match std::fs::remove_file(&legacy) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    Ok(identity)
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};

    use super::*;

    /// Test wrapper: XOR with a key byte plus a tag, so plain != sealed.
    struct Xor(u8, AtomicBool);
    impl Xor {
        fn new(k: u8) -> Self {
            Self(k, AtomicBool::new(false))
        }
    }
    impl SeedWrapper for Xor {
        fn wrap(&self, plain: &[u8]) -> io::Result<Vec<u8>> {
            let mut v = b"SEALED".to_vec();
            v.extend(plain.iter().map(|b| b ^ self.0));
            Ok(v)
        }
        fn unwrap(&self, sealed: &[u8]) -> io::Result<Vec<u8>> {
            if self.1.load(Ordering::Relaxed) {
                return Err(io::Error::other("keystore unavailable"));
            }
            let body = sealed
                .strip_prefix(b"SEALED")
                .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "bad tag"))?;
            Ok(body.iter().map(|b| b ^ self.0).collect())
        }
    }

    fn dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("pb-vault-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn creates_sealed_identity_and_reuses_it() {
        let d = dir("create");
        let w = Xor::new(0x5a);
        let a = open_identity(&d, &w).unwrap();
        let sealed = std::fs::read(d.join(SEALED_FILE)).unwrap();
        assert!(
            !sealed.windows(32).any(|win| win == a.seed()),
            "seed must not be stored in clear"
        );
        let b = open_identity(&d, &w).unwrap();
        assert_eq!(a.peer_id(), b.peer_id());
    }

    #[test]
    fn migrates_legacy_plain_file_and_deletes_it() {
        let d = dir("migrate");
        let old = Identity::from_seed([7; 32]);
        std::fs::write(d.join(LEGACY_PLAIN_FILE), old.seed()).unwrap();
        let w = Xor::new(0x33);
        let got = open_identity(&d, &w).unwrap();
        assert_eq!(
            got.peer_id(),
            old.peer_id(),
            "pairings must survive migration"
        );
        assert!(!d.join(LEGACY_PLAIN_FILE).exists());
        assert!(d.join(SEALED_FILE).exists());
    }

    #[test]
    fn unsealing_failure_is_an_error_not_a_new_identity() {
        let d = dir("fail");
        let w = Xor::new(1);
        let a = open_identity(&d, &w).unwrap();
        w.1.store(true, Ordering::Relaxed);
        assert!(open_identity(&d, &w).is_err());
        w.1.store(false, Ordering::Relaxed);
        assert_eq!(open_identity(&d, &w).unwrap().peer_id(), a.peer_id());
    }

    #[test]
    fn corrupt_sealed_file_is_an_error() {
        let d = dir("corrupt");
        std::fs::write(d.join(SEALED_FILE), b"garbage").unwrap();
        assert!(open_identity(&d, &Xor::new(1)).is_err());
        std::fs::write(d.join(SEALED_FILE), b"SEALEDshort").unwrap();
        assert!(open_identity(&d, &Xor::new(1)).is_err());
    }
}
