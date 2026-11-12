//! The release manifest parser: whatever bytes and signature arrive, `verify` never panics, and
//! an unsigned or wrongly signed manifest is never `Available`. A second mode signs the fuzzed
//! body with the trusted key, so the JSON, version, hex, link and key-rotation rules are reached.
#![no_main]
use libfuzzer_sys::fuzz_target;
use pb_update::{verify, TrustState, Verdict, Version};

fuzz_target!(|data: &[u8]| {
    let trust = TrustState::with_key(pb_update::public_key(&[7; 32]));
    if let Some((&mode, body)) = data.split_first() {
        if mode & 1 == 1 {
            // Correctly signed, arbitrary content: must not panic, and a rejection changes nothing.
            let sig = pb_update::sign(&[7; 32], body);
            let (verdict, after) = verify(body, &sig, &trust, Version(0, 2, 0), 2_000_000_000);
            if matches!(verdict, Verdict::Rejected(_)) {
                assert_eq!(after, trust, "a rejection leaves the trust state alone");
            }
            return;
        }
    }
    let (sig, body) = if data.len() >= 64 {
        let (s, b) = data.split_at(64);
        (<[u8; 64]>::try_from(s).unwrap(), b)
    } else {
        ([0u8; 64], data)
    };
    let (verdict, after) = verify(body, &sig, &trust, Version(0, 2, 0), 2_000_000_000);
    assert!(
        !matches!(verdict, Verdict::Available(_)),
        "a random signature must not verify"
    );
    assert_eq!(after, trust, "a rejection leaves the trust state alone");
});
