---
name: verify
description: How to run and drive Phone Bridge for runtime verification (pb CLI on Windows, Android app on the real phone over USB tethering)
---
- Toolchain env (Git Bash): `export RUSTUP_HOME=D:/Rust/rustup CARGO_HOME=D:/Rust/cargo PATH="/d/Rust/cargo/bin:$PATH" ANDROID_HOME="$LOCALAPPDATA/Android/Sdk" ANDROID_NDK_HOME="$ANDROID_HOME/ndk/29.0.14206865" JAVA_HOME="C:/Program Files/Java/jdk-21"`
- **The phone (serial 79KRFEJVPVOBQCHQ) is the PC's internet via USB tethering. Never `adb kill-server`, `adb usb/tcpip/root/reboot`, `svc`, `settings put`.** Always `adb -s <serial>`. `ping 1.1.1.1` after phone steps.
- PC: `cargo build --profile spike -p pb-cli` -> `target/spike/pb.exe --dir <dir> pair-host|join|listen|send-clip|peers|unpair`. Default port 47200 (one process per port; a waiting `pair-host` blocks `send-clip`). Seed is DPAPI-sealed in `<dir>/identity.sealed`. Firewall: user allowed `pb.exe`; new binaries need the user to allow them.
- Android app: `pwsh scripts/android-build.ps1`, `adb install -r android/app/build/outputs/apk/debug/app-debug.apk`, `am start -n dev.phonebridge.app/.MainActivity`. Node lives in `PbService` (port 47100). Logs: `adb logcat -d -s PhoneBridge`. Files: `adb shell run-as dev.phonebridge.app ls files`.
- Drive the app over adb: `am start ... --es pair_code "<CODE>"`, `--es confirm yes|no`, `--es send_text "<t>"`. Buttons: `uiautomator dump` then `input tap` (Material theme uppercases labels, e.g. `STOP PHONE BRIDGE`). Shell cannot start the non-exported service directly.
- Screen-off test: `input keyevent KEYCODE_HOME` then `KEYCODE_SLEEP`; wake with `KEYCODE_WAKEUP`. Port 47100 in `/proc/net/udp*` is `:B7FC`.
- Don't `logcat -c` between an action and reading its result.
- Emulator: UDP cannot be adb-forwarded; run CLIs on the emulator as clients to the PC at `10.0.2.2`.
- PowerShell gotcha: the harness blocks `Remove-Item` in some compound commands; avoid it.
