# Phone Bridge: architecture decisions

Status: Phase 0. Derived from `pre_build_research/RESEARCH.txt`, `pre_build_research/phone_bridge_deep_research.md` and a shared Gemini report. Claims marked *unverified* came from those reports and must be re-checked in Phase 0 spikes.

## Scope
- Android phone + Windows PC only. LAN and cross-network.
- Features: Universal Clipboard, AirDrop-style transfer, Handoff, Universal Control, Continuity Camera.
- Priorities: performance first (minimal background footprint), then privacy and security.

## Decisions
| ID | Decision |
|----|----------|
| Stack | Rust core shared by both platforms. Android: thin Kotlin layer via UniFFI. Windows: native Win32 tray daemon, no resident WebView2/Electron. |
| Q1 Transport | iroh (noq QUIC + self-hostable iroh-relay) behind our own `pb-transport` trait. Re-check iroh on Android in Spike 1. |
| Q2 Virtual mic | Stage 1: user-mode WASAPI into an existing virtual cable. Stage 2 (hardening phase): attestation-signed sysvad-derived driver. |
| Q3 Wake | Hardware-offloaded BLE scan filter nearby; `connectedDevice` foreground service with long-interval keepalive; optional UnifiedPush. No FCM. |
| Q4 Distribution | Gradle flavors `play` and `foss`. Full build on F-Droid and GitHub Releases. `play` flavor ships without the accessibility service. |
| Q5 License | `MIT OR Apache-2.0`. Clean-room only: never copy code from GPL/AGPL projects (KDE Connect, Lan Mouse, Deskflow, Sunshine, RustDesk). |
| Q6 Framing | prost (Protobuf) for control messages; fixed-layout structs for input and media datagrams. |
| Q7 Windows UI | Win32 tray + on-demand native settings window. |
| Android input | Tier 1: AccessibilityService + overlay + companion IME (foss flavor). Advanced mode: Shizuku/wireless-ADB `/dev/uhid`. |
| Pairing | Ed25519 identity keys, QR bootstrap, 6-digit SAS, pinned certificates over TLS 1.3 (rustls). snow/Noise not used (unaudited). |
| Discovery | Rotating-HMAC mDNS and BLE announcements; no cleartext device names. |
| Windows camera | `MFCreateVirtualCamera` (Windows 11, CurrentUser, off the UI thread); DirectShow fallback for Windows 10 only. |
| Android service | `connectedDevice` FGS type; add `camera`/`microphone` only while streaming. 16 KB page alignment. |

## Performance budgets (provisional)
The sources disagree (idle CPU 0.1% vs 0.5%, RAM 25 vs 45 MB, camera 55 vs 35 ms). Set real budgets after Spike 2 measurements.

## Unverified claims to re-check
- Android 17 "Advanced Protection" auto-revoking non-accessibility accessibility services.
- iroh/noq version numbers and Android behaviour; hole-punch success rates.
- Details of the cited CISPA/USENIX paper and quinn advisories.

## Rules for code
- `unsafe` forbidden outside audited FFI/Win32 wrappers.
- Every parser has size limits, timeouts and a fuzz target.
- Sensitive clipboard content is never synced without explicit user action.
