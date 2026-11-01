//! Writes the generated token files from `design/tokens.toml`.
//! Run from anywhere inside the repo: `cargo run -p gen-tokens`.

use std::path::Path;

fn main() -> Result<(), String> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let src =
        std::fs::read_to_string(root.join("design/tokens.toml")).map_err(|e| e.to_string())?;
    let t = gen_tokens::load(&src)?;
    let outs = [
        ("crates/pb-tray/src/tokens.rs", gen_tokens::render_rust(&t)),
        (
            "android/app/src/main/java/dev/phonebridge/app/ui/Tokens.kt",
            gen_tokens::render_kotlin(&t),
        ),
    ];
    for (path, text) in outs {
        std::fs::write(root.join(path), text).map_err(|e| format!("{path}: {e}"))?;
        println!("wrote {path}");
    }
    Ok(())
}
