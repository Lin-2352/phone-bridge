# Fuzzing

Fuzz targets for the parsers that read what another device or a stranger on
the network sends. This crate is outside the main workspace because it needs
a nightly toolchain:

```bash
rustup toolchain install nightly-2026-11-01 --profile minimal
cargo install cargo-fuzz --locked --version 0.13.2
cd tools/fuzz
cargo +nightly-2026-11-01 fuzz list
cargo +nightly-2026-11-01 fuzz run control_frame -- -max_total_time=60
```

CI runs every target for 20 seconds on each push (`fuzz-smoke`). A crash
leaves its input in `fuzz/artifacts/<target>/`: add it as a unit test next to
the parser once fixed.
