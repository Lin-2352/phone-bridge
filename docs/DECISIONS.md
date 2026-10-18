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

## Spike 1 and 2 results (2026-10-18)
Setup: Windows 11 PC <-> realme RMX3561 (Android 13, arm64) over the phone's USB-tether link (`rndis0`, 10.107.168.0/24). iroh 1.3.0 (`presets::Minimal`, `tls-ring`, relay disabled unless passed), `spikes/iroh-spike`, spike profile (no LTO). Numbers are from this one link and are lower bounds: USB tethering, not our code, likely caps throughput.

| Measurement | Result |
|---|---|
| iroh builds and runs on Android arm64 and x86_64 | Yes (cargo-ndk, NDK r29, ELF LOAD alignment 0x4000) |
| Handshake PC -> phone (direct) | 2-5 ms |
| Datagram RTT (200 pings, 0 loss) | p50 0.55-0.75 ms, p95 0.8-1.1 ms |
| Bulk stream 64 MiB, hash verified | PC -> phone ~36-40 MB/s, phone -> PC ~30-37 MB/s |
| Idle phone endpoint (30 s, no peers) | ~0.03% of one core (1 tick), 18 MB RSS, 9 threads |
| Idle Windows endpoint (30 s) | 0.00 CPU-seconds, 14.8 MB working set, 36 threads |
| Relay-only dial (self-hosted `iroh-relay --dev`, loopback via `adb reverse`) | connected in 16 ms, then moved to a direct path; data verified |
| Loopback on the PC (reference) | ~390 MB/s each way, 1.7 ms handshake (after making the test pattern generator word-wise; the earlier 72 MB/s download was a test artifact) |
| Android emulator x86_64 (CLI to PC, and the APK) | Works; APK starts with JNI context; CLI 32 MiB verified at ~14 MB/s (emulator NAT limit) |

Idle thread count on Windows (36) is high for a resident daemon; cap tokio workers in the real daemon.

### Android app smoke test (real APK on the phone, 2026-10-18)
`spikes/ffi-smoke` (UniFFI 0.32 + iroh 1.3.0, `spike-abort` profile = `panic = "abort"`) packaged by `scripts/android-smoke.ps1` (AGP 9.4.1, Gradle 9.6.0, JNA 5.17.0).

| Check | Result |
|---|---|
| `.so` loads; UniFFI async + tokio works; `panic = "abort"` with JNI context installed | Yes (finding 2 below is confirmed) |
| 16 KB alignment inside the APK | `libpb_smoke.so` and JNA's `libjnidispatch.so` both 0x4000 |
| PC -> in-app server (GSO off), 64 MiB, hash verified | ~38-39 MB/s both ways, RTT p50 0.74-0.88 ms, 0 datagram loss |
| App memory (PSS, debug build, activity open) | 76 MB total; native heap 23 MB (includes Rust), Java heap 9 MB. RSS (210 MB) overstates because of shared mappings |
| App idle CPU (30 s, Kotlin polls once per second) | ~0.5% of one core; an upper bound, the real app must not poll |

### Findings that change the design
1. **UDP GSO breaks the first large send on `rndis0`.** The first GSO `sendmsg` (12780 B, 1420 B segments) fails with `EIO` on the phone's USB-tether interface. noq-udp then halts GSO, but the connection never recovers and dies at the 30 s idle timeout. Reproduced 11 of 11 times on a fresh phone process (first download always fails, later transfers work); with `enable_segmentation_offload(false)` 3 of 3 fresh runs passed. Decision: **disable GSO on Android** (now the default in the spike: 3/3 fresh-process runs pass without flags) (`QuicTransportConfig::builder().enable_segmentation_offload(false)`) until the upstream recovery is fixed; measure the cost on Wi-Fi. Upstream report drafted in `docs/upstream/noq-gso-eio.md` (not filed yet). Not testable here: GSO behaviour on the phone's Wi-Fi, because enabling Wi-Fi would disturb the USB tethering used for this PC's internet.
2. **iroh needs an Android JNI context.** A bare native binary aborted with `android context was not initialized` (from `ndk-context`, used by iroh-dns and netdev). iroh only survives this by catching a panic, so it needs `panic = "unwind"`; our release profile uses `panic = "abort"`. Decision: the Android library installs the JNI context (`iroh::dns::install_android_jni_context`, an `unsafe` FFI call in a dedicated crate) and the Android build must not rely on unwind to recover. Verified inside a real APK (above).
3. **Rust MSRV is 1.91** (iroh 1.3.0). Workspace `rust-version` raised.
4. **Same-uplink limitation.** PC and phone share one uplink here, so real cross-network hole-punching is untested; the hole-punch success rate remains an unverified claim.

## Performance budgets (provisional)
The sources disagree (idle CPU 0.1% vs 0.5%, RAM 25 vs 45 MB, camera 55 vs 35 ms). The measured idle numbers above are encouraging but cover only an endpoint with no peers and no relay connection. Set final budgets after a longer soak test with a relay connection and the real app.

## Unverified claims to re-check
- Android 17 "Advanced Protection" auto-revoking non-accessibility accessibility services.
- Hole-punch success rates (iroh 1.3.0 builds and runs on Android; see results above).
- Details of the cited CISPA/USENIX paper and quinn advisories.

## Rules for code
- `unsafe` forbidden outside audited FFI/Win32 wrappers.
- Every parser has size limits, timeouts and a fuzz target.
- Sensitive clipboard content is never synced without explicit user action.
