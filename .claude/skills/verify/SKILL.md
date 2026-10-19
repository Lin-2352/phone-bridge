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
- Never put the phone screen to sleep (no KEYCODE_SLEEP); background the app with KEYCODE_HOME instead and leave the screen as is.
- Tray: `target/spike/pb-tray.exe --dir <d> [--port N] [--pair] [--test-auto-accept]`; settings in `<d>/settings.txt` (`universal_clipboard=true`, `auto_accept=true`, `save_dir=...`); log `<d>/tray.log` (FILE sent/received with MB/s, CLIPSYNC lines). Firewall rule exists for `target\spike\pb-tray.exe`.
- Drive the tray without clicks: `pb-tray --send "<file>"` (quote paths; Start-Process splits on spaces), `--pair`, `--send-clipboard` go over the pipe `\.\pipe\PhoneBridgeTray-<user>`. Clipboard: `Set-Clipboard`/`Get-Clipboard`.
- Phone share target from adb: `am start -a android.intent.action.SEND -t text/plain --es android.intent.extra.TEXT "<t>" -n dev.phonebridge.app/.ShareActivity`; for files use a MediaStore URI the app owns, e.g. a file it received: `content query --uri content://media/external/downloads --projection _id:_display_name`. Verify with `md5sum '/sdcard/Download/Phone Bridge/<f>'`.
- PowerShell variables are case-insensitive (`$t` == `$T`); use distinct names in test scripts.
- My shell cannot see other windows by handle lookup; to screenshot a window use UIA (find by name) + PrintWindow, never a full-screen capture.
- Before device tests, check that Android Studio is not mirroring the realme (Running Devices) or streaming its logcat: that alone drove it into swap thrash and watchdog reboots (2026-10-20, measured: swap 25k/33k pages/s with it on, 62/24 with Studio quit). Keep phone-side load light: no persistent logcat streams, uiautomator dumps or screen captures while memory is tight.
