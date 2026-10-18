//! Seed protection with DPAPI (`CryptProtectData`, current-user scope).
//!
//! Only the same Windows user on the same machine can unseal; an
//! application-specific entropy value keeps other DPAPI users from
//! unsealing our blob by accident.
#![allow(unsafe_code)]

use std::io;

use pb_identity::vault::SeedWrapper;
use windows::{
    core::PCWSTR,
    Win32::{
        Foundation::{LocalFree, HLOCAL},
        Security::Cryptography::{
            CryptProtectData, CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
        },
    },
};

const ENTROPY: &[u8] = b"phone-bridge/identity-seed/v1";

/// DPAPI-backed [`SeedWrapper`].
#[derive(Default)]
pub struct Dpapi;

fn blob(data: &[u8]) -> CRYPT_INTEGER_BLOB {
    CRYPT_INTEGER_BLOB {
        cbData: data.len() as u32,
        pbData: data.as_ptr() as *mut u8,
    }
}

/// Copy a DPAPI-allocated output blob into a Vec and free it.
fn take(out: CRYPT_INTEGER_BLOB) -> Vec<u8> {
    if out.pbData.is_null() {
        return Vec::new();
    }
    // SAFETY: on success DPAPI returns `cbData` readable bytes at `pbData`,
    // allocated with LocalAlloc, which we free exactly once below.
    let v = unsafe { std::slice::from_raw_parts(out.pbData, out.cbData as usize) }.to_vec();
    // SAFETY: `pbData` came from DPAPI's LocalAlloc and is not used afterwards.
    unsafe {
        LocalFree(Some(HLOCAL(out.pbData.cast())));
    }
    v
}

impl SeedWrapper for Dpapi {
    fn wrap(&self, plain: &[u8]) -> io::Result<Vec<u8>> {
        let input = blob(plain);
        let entropy = blob(ENTROPY);
        let mut out = CRYPT_INTEGER_BLOB::default();
        // SAFETY: input/entropy point to live slices for the call; `out` is a
        // valid out-pointer; no prompt struct; UI is forbidden.
        unsafe {
            CryptProtectData(
                &input,
                PCWSTR::null(),
                Some(&entropy),
                None,
                None,
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut out,
            )
        }
        .map_err(io::Error::other)?;
        Ok(take(out))
    }

    fn unwrap(&self, sealed: &[u8]) -> io::Result<Vec<u8>> {
        let input = blob(sealed);
        let entropy = blob(ENTROPY);
        let mut out = CRYPT_INTEGER_BLOB::default();
        // SAFETY: as in `wrap`; DPAPI only reads `input` and writes `out`.
        unsafe {
            CryptUnprotectData(
                &input,
                None,
                Some(&entropy),
                None,
                None,
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut out,
            )
        }
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        Ok(take(out))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_and_does_not_store_plaintext() {
        let seed = [0x42u8; 32];
        let sealed = Dpapi.wrap(&seed).unwrap();
        assert!(sealed.len() > seed.len());
        assert!(!sealed.windows(32).any(|w| w == seed));
        assert_eq!(Dpapi.unwrap(&sealed).unwrap(), seed);
    }

    #[test]
    fn tampered_blob_is_rejected() {
        let mut sealed = Dpapi.wrap(&[1u8; 32]).unwrap();
        let last = sealed.len() - 1;
        sealed[last] ^= 0xff;
        assert!(Dpapi.unwrap(&sealed).is_err());
        assert!(Dpapi.unwrap(b"not a dpapi blob").is_err());
    }

    #[test]
    fn works_with_the_vault() {
        let dir = std::env::temp_dir().join(format!("pb-dpapi-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let a = pb_identity::vault::open_identity(&dir, &Dpapi).unwrap();
        let b = pb_identity::vault::open_identity(&dir, &Dpapi).unwrap();
        assert_eq!(a.peer_id(), b.peer_id());
    }
}
