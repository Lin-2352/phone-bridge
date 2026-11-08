//! The signed release manifest: what the update check trusts, and nothing else.
//!
//! A release is described by `release-manifest.json` plus a detached Ed25519 signature over the
//! exact bytes. [`verify`] is the only door: it never touches the network or the disk, it takes
//! the bytes, the signature, the keys the app already trusts and the clock, and answers with a
//! [`Verdict`] plus the trust state to store. The app opens the release page only for
//! `Verdict::Available`; there is no download and no install here.
//!
//! Rules: the manifest is at most 64 KiB; the signature must verify with a trusted key; a
//! manifest dated before the last accepted one, or more than seven days ahead of the clock, is
//! rejected (a frozen or replayed "latest" cannot pass); a version that is not newer than the
//! installed one is never offered; every artifact link must be a release link of this project; a
//! new key is accepted only when the current key signs it, and only one step ahead.
use std::cmp::Ordering;

use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use serde::Deserialize;

/// Largest manifest accepted, in bytes.
pub const MAX_MANIFEST: usize = 64 * 1024;
/// How far ahead of the clock a manifest date may be.
pub const MAX_FUTURE_SECS: u64 = 7 * 24 * 3600;
/// Most artifacts a manifest may list.
pub const MAX_ARTIFACTS: usize = 32;
/// Artifact links must start with this.
/// An artifact link must start with the project's release address and continue with plain path
/// characters only: no `..` or `.` segment, no percent-escapes, no backslash, no query, so the browser
/// cannot be walked out of the release folder.
fn url_ok(url: &str) -> bool {
    let Some(rest) = url.strip_prefix(RELEASE_URL_PREFIX) else {
        return false;
    };
    url.len() <= 512
        && !rest.is_empty()
        && rest
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '/' | '+'))
        && rest.split('/').all(|seg| seg != "." && seg != "..")
}

pub const RELEASE_URL_PREFIX: &str = "https://github.com/Lin-2352/phone-bridge/releases/";
/// The text the current key signs to introduce the next key: this prefix plus the key in hex.
pub const NEXT_KEY_PREFIX: &str = "relay-next-key:";

/// `major.minor.patch`; anything else (pre-release tags included) is not a version we offer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version(pub u32, pub u32, pub u32);

impl Version {
    pub fn parse(s: &str) -> Option<Version> {
        let mut it = s.split('.');
        let v = Version(
            it.next()?.parse().ok()?,
            it.next()?.parse().ok()?,
            it.next()?.parse().ok()?,
        );
        it.next().is_none().then_some(v)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Artifact {
    pub name: String,
    pub url: String,
    pub sha256: [u8; 32],
    pub size: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Manifest {
    pub version: Version,
    /// UTC seconds since 1970 (RULING: an integer, so the trust boundary needs no date parser).
    pub date: u64,
    pub artifacts: Vec<Artifact>,
    pub next_key: Option<[u8; 32]>,
}

/// What the app stores between checks.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TrustState {
    /// The key in use, and at most one pending successor.
    pub keys: Vec<[u8; 32]>,
    /// The date of the newest manifest accepted so far.
    pub last_date: Option<u64>,
    /// The newest version seen so far.
    pub last_version: Option<Version>,
}

impl TrustState {
    pub fn with_key(key: [u8; 32]) -> Self {
        Self {
            keys: vec![key],
            ..Self::default()
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reject {
    TooBig,
    BadJson,
    BadSignature,
    UnknownKey,
    /// A signed manifest older than one already seen (by date or by version).
    Rollback,
    Stale,
    FromTheFuture,
    BadUrl,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Verdict {
    UpToDate,
    Available(Manifest),
    Rejected(Reject),
}

#[derive(Deserialize)]
struct RawArtifact {
    name: String,
    url: String,
    sha256: String,
    size: u64,
}

#[derive(Deserialize)]
struct RawManifest {
    version: String,
    date: u64,
    artifacts: Vec<RawArtifact>,
    next_key: Option<String>,
    next_key_sig: Option<String>,
}

fn unhex<const N: usize>(s: &str) -> Option<[u8; N]> {
    if s.len() != N * 2 || !s.is_ascii() {
        return None;
    }
    let mut out = [0u8; N];
    for (i, o) in out.iter_mut().enumerate() {
        *o = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(out)
}

fn signed_by(key: &[u8; 32], msg: &[u8], sig: &[u8; 64]) -> bool {
    match VerifyingKey::from_bytes(key) {
        Ok(k) => k.verify(msg, &Signature::from_bytes(sig)).is_ok(),
        Err(_) => false,
    }
}

/// Checks a manifest. Returns the verdict and the trust state to keep: unchanged on a rejection.
pub fn verify(
    bytes: &[u8],
    sig: &[u8; 64],
    trust: &TrustState,
    installed: Version,
    now: u64,
) -> (Verdict, TrustState) {
    let reject = |r: Reject| (Verdict::Rejected(r), trust.clone());
    if bytes.len() > MAX_MANIFEST {
        return reject(Reject::TooBig);
    }
    if trust.keys.is_empty() {
        return reject(Reject::UnknownKey);
    }
    let Some(signer) = trust.keys.iter().position(|k| signed_by(k, bytes, sig)) else {
        return reject(Reject::BadSignature);
    };
    let Ok(raw) = serde_json::from_slice::<RawManifest>(bytes) else {
        return reject(Reject::BadJson);
    };
    let Some(version) = Version::parse(&raw.version) else {
        return reject(Reject::BadJson);
    };
    if raw.date > now.saturating_add(MAX_FUTURE_SECS) {
        return reject(Reject::FromTheFuture);
    }
    if trust.last_date.is_some_and(|d| raw.date < d) {
        return reject(Reject::Stale);
    }
    if trust.last_version.is_some_and(|v| version < v) {
        return reject(Reject::Rollback);
    }
    if raw.artifacts.len() > MAX_ARTIFACTS {
        return reject(Reject::BadJson);
    }
    let mut artifacts = Vec::new();
    for a in raw.artifacts {
        if !url_ok(&a.url) || a.name.len() > 128 {
            return reject(Reject::BadUrl);
        }
        let Some(sha256) = unhex::<32>(&a.sha256) else {
            return reject(Reject::BadJson);
        };
        artifacts.push(Artifact {
            name: a.name,
            url: a.url,
            sha256,
            size: a.size,
        });
    }

    // Key rotation: the key that signed this manifest introduces the next one, and only it.
    let signing_key = trust.keys[signer];
    let mut next_key = None;
    if let Some(k) = &raw.next_key {
        let (Some(key), Some(ksig)) = (
            unhex::<32>(k),
            raw.next_key_sig.as_deref().and_then(unhex::<64>),
        ) else {
            return reject(Reject::BadSignature);
        };
        if !signed_by(
            &signing_key,
            format!("{NEXT_KEY_PREFIX}{k}").as_bytes(),
            &ksig,
        ) {
            return reject(Reject::BadSignature);
        }
        next_key = Some(key);
    }

    let mut keys = vec![signing_key];
    if let Some(k) = next_key {
        if k != signing_key {
            keys.push(k);
        }
    } else if signer == 0 {
        // Still the old key signing: a pending successor stays pending.
        keys = trust.keys.clone();
    }
    let manifest = Manifest {
        version,
        date: raw.date,
        artifacts,
        next_key,
    };
    let new_trust = TrustState {
        keys,
        last_date: Some(raw.date.max(trust.last_date.unwrap_or(0))),
        last_version: Some(version.max(trust.last_version.unwrap_or(Version(0, 0, 0)))),
    };
    let verdict = match version.cmp(&installed) {
        Ordering::Greater => Verdict::Available(manifest),
        _ => Verdict::UpToDate,
    };
    (verdict, new_trust)
}

/// Signs `bytes`; used by the `sign-manifest` tool and by tests.
pub fn sign(seed: &[u8; 32], bytes: &[u8]) -> [u8; 64] {
    use ed25519_dalek::Signer;
    ed25519_dalek::SigningKey::from_bytes(seed)
        .sign(bytes)
        .to_bytes()
}

/// The public key for a signing seed.
pub fn public_key(seed: &[u8; 32]) -> [u8; 32] {
    ed25519_dalek::SigningKey::from_bytes(seed)
        .verifying_key()
        .to_bytes()
}

/// Signs a next-key introduction: what goes in `next_key_sig`, in hex.
pub fn sign_next_key(seed: &[u8; 32], next_key_hex: &str) -> String {
    hex(&sign(
        seed,
        format!("{NEXT_KEY_PREFIX}{next_key_hex}").as_bytes(),
    ))
}

pub fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEED_A: [u8; 32] = [1; 32];
    const SEED_B: [u8; 32] = [2; 32];
    const NOW: u64 = 2_000_000_000;
    const SHA: &str = "aa00000000000000000000000000000000000000000000000000000000000000";

    fn manifest(version: &str, date: u64, extra: &str) -> String {
        format!(
            r#"{{"version":"{version}","date":{date},"artifacts":[{{"name":"Relay.zip","url":"{RELEASE_URL_PREFIX}v{version}/Relay.zip","sha256":"{SHA}","size":1234}}]{extra}}}"#
        )
    }

    fn run(
        m: &str,
        seed: &[u8; 32],
        trust: &TrustState,
        installed: Version,
    ) -> (Verdict, TrustState) {
        verify(
            m.as_bytes(),
            &sign(seed, m.as_bytes()),
            trust,
            installed,
            NOW,
        )
    }

    fn trusting_a() -> TrustState {
        TrustState::with_key(public_key(&SEED_A))
    }

    #[test]
    fn a_valid_newer_manifest_is_available() {
        let (v, t) = run(
            &manifest("0.3.0", NOW - 10, ""),
            &SEED_A,
            &trusting_a(),
            Version(0, 2, 0),
        );
        let Verdict::Available(m) = v else {
            panic!("{v:?}")
        };
        assert_eq!(m.version, Version(0, 3, 0));
        assert_eq!(m.artifacts[0].size, 1234);
        assert_eq!(t.last_date, Some(NOW - 10));
    }

    #[test]
    fn never_offers_installed_version() {
        for installed in [Version(0, 3, 0), Version(0, 4, 0)] {
            let (v, _) = run(
                &manifest("0.3.0", NOW, ""),
                &SEED_A,
                &trusting_a(),
                installed,
            );
            assert_eq!(v, Verdict::UpToDate);
        }
    }

    #[test]
    fn a_bad_signature_is_rejected() {
        let m = manifest("0.3.0", NOW, "");
        let mut sig = sign(&SEED_A, m.as_bytes());
        sig[0] ^= 1;
        let (v, t) = verify(m.as_bytes(), &sig, &trusting_a(), Version(0, 2, 0), NOW);
        assert_eq!(v, Verdict::Rejected(Reject::BadSignature));
        assert_eq!(t, trusting_a(), "a rejection changes nothing");
    }

    #[test]
    fn a_manifest_changed_after_signing_is_rejected() {
        let m = manifest("0.3.0", NOW, "");
        let sig = sign(&SEED_A, m.as_bytes());
        let tampered = m.replace(SHA, &"bb".repeat(32));
        let (v, _) = verify(
            tampered.as_bytes(),
            &sig,
            &trusting_a(),
            Version(0, 2, 0),
            NOW,
        );
        assert_eq!(v, Verdict::Rejected(Reject::BadSignature));
    }

    #[test]
    fn the_wrong_key_and_no_key_are_rejected() {
        let m = manifest("0.3.0", NOW, "");
        assert_eq!(
            run(&m, &SEED_B, &trusting_a(), Version(0, 2, 0)).0,
            Verdict::Rejected(Reject::BadSignature)
        );
        assert_eq!(
            run(&m, &SEED_A, &TrustState::default(), Version(0, 2, 0)).0,
            Verdict::Rejected(Reject::UnknownKey)
        );
    }

    #[test]
    fn stale_manifest_rejected() {
        let mut t = trusting_a();
        t.last_date = Some(NOW - 100);
        let (v, _) = run(
            &manifest("0.3.0", NOW - 101, ""),
            &SEED_A,
            &t,
            Version(0, 2, 0),
        );
        assert_eq!(v, Verdict::Rejected(Reject::Stale));
        let (v, _) = run(
            &manifest("0.3.0", NOW - 100, ""),
            &SEED_A,
            &t,
            Version(0, 2, 0),
        );
        assert!(
            matches!(v, Verdict::Available(_)),
            "the same date is not stale"
        );
    }

    #[test]
    fn an_older_version_than_one_seen_is_a_rollback() {
        let mut t = trusting_a();
        t.last_version = Some(Version(0, 4, 0));
        let (v, _) = run(&manifest("0.3.0", NOW, ""), &SEED_A, &t, Version(0, 2, 0));
        assert_eq!(v, Verdict::Rejected(Reject::Rollback));
    }

    #[test]
    fn future_dated_manifest_rejected() {
        let (v, _) = run(
            &manifest("0.3.0", NOW + MAX_FUTURE_SECS + 1, ""),
            &SEED_A,
            &trusting_a(),
            Version(0, 2, 0),
        );
        assert_eq!(v, Verdict::Rejected(Reject::FromTheFuture));
        let (v, _) = run(
            &manifest("0.3.0", NOW + MAX_FUTURE_SECS, ""),
            &SEED_A,
            &trusting_a(),
            Version(0, 2, 0),
        );
        assert!(matches!(v, Verdict::Available(_)));
    }

    #[test]
    fn oversize_manifest_rejected() {
        let big = manifest(
            "0.3.0",
            NOW,
            &format!(r#","pad":"{}""#, "x".repeat(MAX_MANIFEST)),
        );
        assert_eq!(
            run(&big, &SEED_A, &trusting_a(), Version(0, 2, 0)).0,
            Verdict::Rejected(Reject::TooBig)
        );
    }

    #[test]
    fn hostile_json_is_bad_json_not_a_crash() {
        for m in [
            r#"{"version":"0.3.0","version":"0.4.0","date":1,"artifacts":[]}"#.to_owned(),
            r#"{"version":"1.2","date":1,"artifacts":[]}"#.to_owned(),
            r#"{"version":"0.3.0-rc1","date":1,"artifacts":[]}"#.to_owned(),
            "not json".to_owned(),
            format!(
                r#"{{"version":"0.3.0","date":1,"artifacts":[{}]}}"#,
                vec![r#"{"name":"a","url":"u","sha256":"00","size":1}"#; 5000].join(",")
            ),
        ] {
            let (v, _) = run(&m, &SEED_A, &trusting_a(), Version(0, 2, 0));
            assert!(
                matches!(v, Verdict::Rejected(Reject::BadJson | Reject::TooBig)),
                "{v:?}"
            );
        }
    }

    #[test]
    fn deeply_nested_unknown_fields_never_crash() {
        let nested = format!(
            r#"{{"version":"0.3.0","date":1,"artifacts":[],"x":{}{}}}"#,
            "[".repeat(5000),
            "]".repeat(5000)
        );
        let (v, _) = run(&nested, &SEED_A, &trusting_a(), Version(0, 2, 0));
        assert!(
            matches!(
                v,
                Verdict::Available(_) | Verdict::Rejected(Reject::BadJson)
            ),
            "{v:?}"
        );
    }

    #[test]
    fn only_release_links_of_this_project_pass() {
        for url in [
            "http://github.com/Lin-2352/phone-bridge/releases/x",
            "https://evil.example/Relay.zip",
            "https://github.com/Lin-2352/phone-bridge/releases.evil/x",
            "https://github.com/Lin-2352/phone-bridge/releases/../../../evil/x/releases/download/a.zip",
            "https://github.com/Lin-2352/phone-bridge/releases/%2e%2e/%2e%2e/evil/a.zip",
            "https://github.com/Lin-2352/phone-bridge/releases/./x?a=b",
            "https://github.com/Lin-2352/phone-bridge/releases/",
        ] {
            let m = manifest("0.3.0", NOW, "")
                .replace(&format!("{RELEASE_URL_PREFIX}v0.3.0/Relay.zip"), url);
            assert_eq!(
                run(&m, &SEED_A, &trusting_a(), Version(0, 2, 0)).0,
                Verdict::Rejected(Reject::BadUrl),
                "{url}"
            );
        }
    }

    fn rotation_extra(introducer: &[u8; 32], next: &[u8; 32]) -> String {
        let nk = hex(&public_key(next));
        let ks = sign_next_key(introducer, &nk);
        format!(r#","next_key":"{nk}","next_key_sig":"{ks}""#)
    }

    #[test]
    fn rotation_is_accepted_when_the_current_key_introduces_the_next() {
        let m = manifest("0.3.0", NOW, &rotation_extra(&SEED_A, &SEED_B));
        let (v, t) = run(&m, &SEED_A, &trusting_a(), Version(0, 2, 0));
        assert!(matches!(v, Verdict::Available(_)));
        assert_eq!(
            t.keys,
            vec![public_key(&SEED_A), public_key(&SEED_B)],
            "both trusted until B signs"
        );
        // The next release is signed by B: the old key is dropped.
        let m2 = manifest("0.3.1", NOW + 1, "");
        let (v2, t2) = run(&m2, &SEED_B, &t, Version(0, 2, 0));
        assert!(matches!(v2, Verdict::Available(_)));
        assert_eq!(t2.keys, vec![public_key(&SEED_B)]);
        let (v3, _) = run(
            &manifest("0.3.2", NOW + 2, ""),
            &SEED_A,
            &t2,
            Version(0, 2, 0),
        );
        assert_eq!(
            v3,
            Verdict::Rejected(Reject::BadSignature),
            "the old key no longer signs"
        );
    }

    #[test]
    fn rotation_requires_current_key_signature() {
        // The manifest is signed by A, but the introduction of B was signed by B itself.
        let nk = hex(&public_key(&SEED_B));
        let forged = format!(
            r#","next_key":"{nk}","next_key_sig":"{}""#,
            sign_next_key(&SEED_B, &nk)
        );
        let (v, t) = run(
            &manifest("0.3.0", NOW, &forged),
            &SEED_A,
            &trusting_a(),
            Version(0, 2, 0),
        );
        assert_eq!(v, Verdict::Rejected(Reject::BadSignature));
        assert_eq!(t, trusting_a());
        // A next key without any signature is refused too.
        let bare = format!(r#","next_key":"{nk}""#);
        assert_eq!(
            run(
                &manifest("0.3.0", NOW, &bare),
                &SEED_A,
                &trusting_a(),
                Version(0, 2, 0)
            )
            .0,
            Verdict::Rejected(Reject::BadSignature)
        );
    }

    #[test]
    fn rotation_chain_is_one_step_only() {
        // A introduces B; before B signs anything, B cannot introduce C, because a manifest must be
        // signed by a key already trusted AND B's own manifest is the first that B may sign: it can
        // introduce C, but A is dropped at that moment, so the chain never has more than two keys.
        let seed_c = [3u8; 32];
        let (_, t) = run(
            &manifest("0.3.0", NOW, &rotation_extra(&SEED_A, &SEED_B)),
            &SEED_A,
            &trusting_a(),
            Version(0, 2, 0),
        );
        let (_, t2) = run(
            &manifest("0.3.1", NOW + 1, &rotation_extra(&SEED_B, &seed_c)),
            &SEED_B,
            &t,
            Version(0, 2, 0),
        );
        assert_eq!(t2.keys, vec![public_key(&SEED_B), public_key(&seed_c)]);
        assert!(t2.keys.len() <= 2);
    }

    #[test]
    fn unknown_key_is_rejected() {
        let stranger = [9u8; 32];
        let (v, _) = run(
            &manifest("0.3.0", NOW, ""),
            &stranger,
            &trusting_a(),
            Version(0, 2, 0),
        );
        assert_eq!(v, Verdict::Rejected(Reject::BadSignature));
    }
}
