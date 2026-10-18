---
name: verify
description: How to run and drive Phone Bridge for runtime verification (PC CLI, real phone over USB tethering, Android app)
---
- Toolchain env (Git Bash): `export RUSTUP_HOME=D:/Rust/rustup CARGO_HOME=D:/Rust/cargo PATH="/d/Rust/cargo/bin:$PATH" ANDROID_HOME="$LOCALAPPDATA/Android/Sdk" ANDROID_NDK_HOME="$ANDROID_HOME/ndk/29.0.14206865"`
- **The phone (serial 79KRFEJVPVOBQCHQ) is the PC's internet via USB tethering. Never `adb kill-server`, `adb usb/tcpip/root/reboot`, `svc`, `settings put`.** Always `adb -s <serial>`. Check `ping 1.1.1.1` after phone steps.
- PC CLI: `cargo build --profile spike -p iroh-spike` -> `target/spike/iroh-spike.exe serve|connect`.
- Phone CLI: `cargo ndk -t arm64-v8a --platform 29 build --profile spike -p iroh-spike`, `adb push` to `/data/local/tmp`, **chmod +x after every push**, run `serve --port N`; PC connects to `10.107.168.189:N` (phone rndis0 IP, check `ip -o addr` if it changed).
- Android app: `pwsh scripts/android-smoke.ps1`, `adb install -r`, `am start -n dev.phonebridge.smoke/.MainActivity`, read `adb logcat -d -s PbSmoke` for ENDPOINT_ID; app serves on port 47100.
- Emulator: UDP cannot be adb-forwarded; run the CLI on the emulator as client to the PC at `10.0.2.2`.
- PowerShell gotcha: the harness blocks `Remove-Item` in some compound commands; avoid it.
