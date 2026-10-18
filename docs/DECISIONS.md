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

## Phase 1 results (2026-10-18)
Real phone (RMX3561 app) and Windows `pb` CLI over the USB-tether link:
- Pairing: both sides showed SAS 771917; pinned on both.
- Clipboard text PC -> phone (plain and sensitive; sensitive is not displayed) and phone -> PC; still works after restarting the app without re-pairing.
- PC revokes the phone: the phone's next send fails with "peer is not paired with this device".
- Windows Firewall blocks inbound UDP for new binaries on Public networks (the tether link is Public); the user must allow the app. The installer must create a scoped rule (UDP, our binary only).
- Bug found and fixed: closing the pairing connection right after reading discarded the other side's unread Confirm (~5% of pairings failed on one side). Fixed with an ordered close; 40/40 runs pass.

## Phase 2 results (2026-10-18)
- Identity seeds sealed: Windows DPAPI (current user + app entropy, `pb-platform-win`), Android Keystore AES-256-GCM (`KeystoreWrapper`). Phase 1 plain seeds migrated on first start with the same ids (pairings kept) and deleted.
- Android `connectedDevice` foreground service owns the node; events pushed from Rust (no polling). With the app in the background and the screen dozing, PC -> phone clipboard arrived as a notification (sensitive text hidden).
- Bug found by /verify and fixed: the JNI context may only be installed once per process; a second `PbService` creation in the same process (Stop then start, or sticky restart) aborted the app. Now idempotent (`Once`). Also: the Rust node is destroyed explicitly on service stop so its UDP socket is released immediately.
- Idle with screen off (60 s, paired, no traffic): 1 CPU tick (~0.02% of one core), 33 threads, PSS 67.6 MB (native heap 18 MB).

### Windows tray (`pb-tray`, milestone 3)
Win32 tray (no WebView), message-only window, EcoQoS on, 2 tokio workers. Tested PC-locally against the `pb` CLI: pairing (SAS shown, auto-accept test flag), received text written to the Windows clipboard (sensitive text gets the history/cloud/monitor exclusion formats and is never logged), menu "Send clipboard" delivered, clipboard content marked private by its source app (`ExcludeClipboardContentFromMonitorProcessing`) is refused. Real phone (after the user added a scoped UDP firewall rule for `pb-tray.exe`): paired (SAS 892543 on both), phone -> tray text landed on the Windows clipboard, tray -> phone delivered with the phone screen off. Idle 60 s: 0.000 CPU-seconds, 17.2 MB working set, 3.2 MB private, 7 threads (CLI spike default was 36).

## Phase 3 results (2026-10-18): AirDrop-style sharing + Universal Clipboard
Real phone (RMX3561, Compose app 0.0.2) and Windows tray over the USB-tether link:
- PC -> phone 100 MiB via Explorer path (`pb-tray --send`): 35.7 MB/s, saved to Downloads/Phone Bridge, MD5 equal.
- Phone -> PC same file via Android share sheet (detached fd, no copy): 37.6 MB/s, MD5 equal. Link shared from phone -> PC clipboard.
- Universal Clipboard: PC copy reached the phone with the screen off; opening the app did not bounce it back; password-manager content never synced (PC-local test); loopback file throughput ~140 MB/s.
- Bugs found and fixed: reply-before-close race for declines/text (same rule as pairing); tray wrote shared links to the clipboard outside the echo guard (bounced to the phone); app re-sent an unchanged clipboard on focus after a restart (fingerprint now persisted); second-instance forwarding moved from window lookup to a user-scoped named pipe.
- Idle: phone ~0.03% CPU (2 ticks/60 s), PSS 101 MB with the Compose UI resident in the background (68 MB before Compose); tray 0 CPU-s, 20.6 MB, 7 threads. APK (debug, 2 ABIs) 45 MB.
- Not done / known: Windows share sheet target needs package identity (later, with the installer); flyout and Send-to entry need a manual click to verify; incoming link on PC goes to the clipboard (no "open in browser" prompt yet).

## Phase 3.1 (2026-10-18): user feedback
- Quick Settings tile is the on/off switch for Phone Bridge (service + node); long-press opens a compact panel. Status notification reduced to one silent line (Android requires it for a foreground service).
- Theme choice, save-folder choice (SAF), opt-in password/code sync (`ClipboardUpdate.sensitive`, kept out of history on arrival), links open from a PC notification instead of overwriting the clipboard. Tray uses NOTIFYICON_VERSION_4 so one click opens one surface.
- Phone memory in background (UI backgrounded, not swiped): PSS 111 MB = native 27 (Rust core) + graphics 21 (UI surfaces kept alive) + code 29 + Java 14. Trim-memory does not release the graphics while the activity exists. Fix candidate: run the service in a separate `:bridge` process so Android can drop the UI process; expected background ~55-65 MB. Superseded, see Phase 3.2.

## Phase 3.2 (2026-10-18): all-day footprint
- Android library built with a size profile (`opt-level="s"`, fat LTO, abort, strip): 8.4 -> 4.3 MB. Release APK with R8 minify + resource shrink: 45 -> 12 MB.
- UniFFI's tokio runtime capped to 2 workers (`TOKIO_WORKER_THREADS`, set in JNI init before the first runtime).
- Background PSS (UI backgrounded): 111 -> 55 MB (Java 14->3, code 29->17, native 27->21, graphics 21->8). This meets the <=60 MB goal, so the `:bridge` process split is not needed; revisit only if long soaks show growth.
- Idle 5 min: phone 0 CPU ticks, ~0.3 context switches/s for the whole process; tray 0 ms CPU, 3.4 MB private, 4 threads. Relay disabled; heartbeats only exist while a session is open.
- Share sheet: sparse package registered per user (Developer Mode, no certificate); signed MSIX comes with the installer.

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
