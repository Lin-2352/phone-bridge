//! Signs a release manifest with the owner's offline Ed25519 key.
//!
//! Usage: `sign-manifest <seed-file> <release-manifest.json>`
//!
//! `<seed-file>` holds the 32-byte signing seed as 64 hex characters (the owner keeps it offline;
//! this tool only reads the path it is given and never prints, logs or copies the key). The
//! signature is written next to the manifest as `<release-manifest.json>.sig` (64 raw bytes), and
//! it is checked against the manifest before the tool reports success.
//!
//! `sign-manifest --next-key <seed-file> <next-public-key-hex>` prints the `next_key_sig` value
//! that introduces a successor key (signed by the current key).
use std::process::ExitCode;

fn read_seed(path: &str) -> Result<[u8; 32], String> {
    let text =
        std::fs::read_to_string(path).map_err(|e| format!("cannot read the key file: {e}"))?;
    let t = text.trim();
    if t.len() != 64 || !t.is_ascii() {
        return Err("the key file must hold 64 hex characters".into());
    }
    let mut seed = [0u8; 32];
    for (i, b) in seed.iter_mut().enumerate() {
        *b = u8::from_str_radix(&t[i * 2..i * 2 + 2], 16)
            .map_err(|_| "the key file must hold 64 hex characters".to_string())?;
    }
    Ok(seed)
}

fn run(args: &[String]) -> Result<String, String> {
    match args {
        [flag, seed, next] if flag == "--next-key" => {
            let seed = read_seed(seed)?;
            Ok(pb_update::sign_next_key(&seed, next))
        }
        [seed, manifest] => {
            let seed = read_seed(seed)?;
            let bytes = std::fs::read(manifest).map_err(|e| format!("cannot read the manifest: {e}"))?;
            if bytes.len() > pb_update::MAX_MANIFEST {
                return Err("the manifest is over 64 KiB".into());
            }
            let sig = pb_update::sign(&seed, &bytes);
            let trust = pb_update::TrustState::with_key(pb_update::public_key(&seed));
            let (verdict, _) = pb_update::verify(&bytes, &sig, &trust, pb_update::Version(0, 0, 0), u64::MAX / 2);
            if matches!(verdict, pb_update::Verdict::Rejected(r) if r != pb_update::Reject::FromTheFuture) {
                return Err("the manifest does not pass its own checks; fix it first".into());
            }
            let out = format!("{manifest}.sig");
            std::fs::write(&out, sig).map_err(|e| format!("cannot write the signature: {e}"))?;
            Ok(format!("signed: {out}"))
        }
        _ => Err("usage: sign-manifest <seed-file> <release-manifest.json>\n       sign-manifest --next-key <seed-file> <next-public-key-hex>".into()),
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(msg) => {
            println!("{msg}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}
