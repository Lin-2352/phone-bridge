# Relay

[![ci](https://github.com/Lin-2352/phone-bridge/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/Lin-2352/phone-bridge/actions/workflows/ci.yml)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue)](#license)

Relay connects an Android phone and a Windows PC directly over your local network. It does the things you'd otherwise reach for a cable, a chat app or several separate tools for, with no account and no cloud service in the middle.

**Status: beta.** It is used daily on one Windows 11 PC and one Android 13 phone (realme RMX3561). Other phones and PCs are untested, and it has had no external security audit. See [Known issues](#known-issues).

## What it does

| | |
|---|---|
| **Clipboard** | Copy on one device, paste on the other: text and pictures. Off until you turn it on. |
| **Files and links** | Send from Explorer's *Send to* or Android's share sheet. |
| **Notifications** | Phone notifications appear in Windows. Reply and call buttons exist but are less tested. |
| **Phone screen** | View the phone in a window on the PC and control it with your mouse and keyboard. |
| **Control** | Use the phone from the PC, after the phone asks you. It can also work as a touchpad for the PC. |
| **Camera** | Use the phone as a Windows 11 webcam, seen by any app as a normal camera. |
| **Audio** | Hear the phone on the PC, or the PC on the phone. While sharing, the phone's speaker stays silent and its volume bar is meant to set the level on the PC. |

## Install

Download both parts from the [latest release](https://github.com/Lin-2352/phone-bridge/releases):

1. **PC:** unzip `Relay-<version>-windows-x64-portable.zip` on a local drive and run `Relay.exe`. Allow it through the firewall on private networks. Everything it keeps stays in the `data` folder beside it.
2. **Phone:** install `Relay-<version>.apk`. It isn't on an app store, so Android asks you to allow the install.
3. **Pair:** choose *Pair a phone* from the tray icon, scan the QR code with the phone, and check that both screens show the same six digits.

Requires Android 10 or later and Windows 10 2004 or later. The webcam needs Windows 11, and icons look best there too.

## Privacy and security

- **No account, no server.** Devices talk directly, so they must be able to reach each other, such as on the same Wi-Fi or over USB tethering. Traffic is encrypted over QUIC and TLS 1.3, with each device's key pinned at pairing.
- **Pairing needs both screens.** A QR code carries the key, and you compare a six-digit code to catch anyone in the middle.
- **Sensitive features ask first.** Screen, control, camera and microphone each need a yes on the phone, unless you choose *always allow* for that PC. The microphone always asks.
- **You can see what's running.** Every live share shows on the phone's notification and Home screen, with a Stop button.

Details are in the [threat model](docs/THREAT_MODEL.md). To report a vulnerability, see [SECURITY.md](SECURITY.md).

### Banking apps

Some banking apps refuse to open while Android's accessibility service or USB debugging is on. Relay doesn't ask for developer settings, though this hasn't yet been tried with them switched off and a banking app open. Its *Control* feature does use an accessibility service, which it switches on only during a control session. Relay does not try to hide any of this from those apps.

## How it works

A shared Rust core handles identity, pairing and transport. Each platform adds a thin native layer.

```
Android (Kotlin, Compose)              Windows (Win32, Direct2D)
        |                                      |
     pb-ffi (UniFFI)                        pb-tray
        \____________ pb-transport ___________/
                 pb-protocol, pb-identity
```

| Path | What's there |
|---|---|
| `crates/pb-transport` | Connections, pairing, file, audio, screen and camera sessions |
| `crates/pb-protocol`, `pb-identity` | Wire format; device keys and the paired-device store |
| `crates/pb-tray` | The Windows app |
| `crates/pb-platform-win`, `pb-video`, `pb-camera-*` | Windows audio, video decoding, the virtual camera |
| `crates/pb-ffi` | Bindings used by the Android app |
| `android/` | The Android app |
| `docs/` | [Wire protocol](docs/WIRE_PROTOCOL.md), [threat model](docs/THREAT_MODEL.md), [design decisions and measurements](docs/DECISIONS.md) |

The project started under the name Phone Bridge, which is why crates are prefixed `pb-` and the Android package is `dev.phonebridge.app`.

## Build from source

You need Rust (the version pinned in `.github/workflows/ci.yml`) and, for Android, JDK 21, the Android SDK and NDK, `cargo-ndk`, and the `aarch64-linux-android` and `x86_64-linux-android` Rust targets.

```powershell
# Windows app
cargo build --release -p pb-tray -p pb-cli -p pb-camera-source
pwsh scripts/make-portable.ps1 -Zip      # dist\Relay and a zip

# Android app (debug APK)
pwsh scripts/android-build.ps1
```

Checks that CI runs:

```powershell
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cd android; ./gradlew lintRelease testDebugUnitTest
```

See [CONTRIBUTING.md](CONTRIBUTING.md) before opening a pull request.

## Known issues

- **One PC at a time.** You can pair several, but status and sends use the first one, and there's no picker yet.
- **A share starts at half volume the first time.** If the speaker effect doesn't reach an app, that app plays at that level.
- **Silencing the speaker has been tested on one phone, with one app.** Android decides where the effect that silences it applies, so an app using a different audio output, such as a game, may still be heard. If the effect is refused or taken away, Relay falls back to muting, where volume keys work less smoothly.
- **Headphones during a share.** Plugging them in or removing them mid-share can leave the phone's volume at a different level afterwards.
- **If Relay is killed mid-share**, the phone can play at the share's volume until Relay restarts and puts it back.
- **PC audio plays on the phone without a question on the phone.** The request comes from a PC you paired, and it shows on the phone with a Stop button.

The [changelog](CHANGELOG.md) lists more for each release.

## License

Licensed under either of [Apache License 2.0](LICENSE-APACHE) or [MIT](LICENSE-MIT), at your option.
