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

## Phase 3.3 (2026-10-18): stability check and hardening
- PC lag investigation: pb-tray 26 MB / 2 CPU-s over an hour; 300 flyout/menu/settings open-close cycles: private 4.5 -> 4.6 MB, handles 315 flat, GDI 18 flat, idle 0 CPU. The lag came from the development workflow: Gradle + Kotlin daemons left running (3.3 GB, ~54 CPU-min) and full-LTO Android builds using every core. Builds now run below-normal priority on half the cores, Gradle without a daemon, Kotlin in-process.
- Android on the AVD: 2 monkey runs (7,000+ events), 0 Phone Bridge crashes/ANRs, PSS flat (47.6 -> 48.0 MB), idle 0 ticks/60 s.
- Realme SystemUI only reads `ACTIVE_TILE`, `TOGGLEABLE_TILE` and `com.oplus.systemui.qs.hide_tile` from third-party tiles; the ▾ detail panel (`OplusQSDetail`) is for system tiles only. Long-press stays the way into our panel.
- Hardening: PC control off until the user allows it in a prompt; receivers cap input at 1000 moves/s and end the session above 400 clicks/keys per second; pipe sends only existing files; accessibility service subscribes only to announcements.

## Phase 3.4 (2026-10-18/19): realme reboots, review, fixes
- Phone rebooted twice (20:17, 22:56): `kernel_panic,sysrq_trigwarmd_crash`, realme's hang watchdog. No pre-reboot log kept. Both happened with no test load from us; the phone was under heavy memory pressure (LMK thrashing kills of launcher/GMS/WhatsApp), HDFC PayZapp crash-looping every few minutes, Instagram ~700 MB, GMS ~650 MB. Phone Bridge ~45 MB, not in the top 10. Integrity after: verified boot green/locked, no filesystem errors.
- USB debugging off at 23:16:17: realme system_server disables adb 83 ms after `USB_STATE=DISCONNECTED`; not app-triggerable (needs WRITE_SECURE_SETTINGS).
- MediaTek kernel: 16 threads (wdtk-*, hang_detect) sit permanently in D state, so load average ~18 at 8.5% real CPU.
- Three parallel code reviews (Rust core, Android, tray). Confirmed and fixed the critical one in UniFFI 0.32.2 source: a unit-returning callback interface panics when Kotlin throws (`ffi_converter_traits.rs:393`), which aborts the app under `panic = "abort"`. Callbacks now return `Result`; Kotlin hands events to one background thread. Other fixes: received-file copies off the Rust runtime threads, guarded startForeground, async shutdown without orphaned nodes, debug intent hooks only in debuggable builds (exported activity allowed silent pairing), connection caps and handshake timeouts, flood budget on every record, free-space check, tray hook thread, owner-only pipe DACL, icon restore on Explorer restart.
- Realme results after fixes (fresh boot, release build): no crashes/ANRs/LMK kills in 30+ min of monitoring; idle 1 tick/120 s; PSS 38-65 MB depending on UI use; tray private 3.4-4.1 MB, leak test flat, stuck-pipe client no longer blocks (48 ms).

## Phase 3.5 (2026-10-19): network change, link test on Wi-Fi, realme throttling
- Moving both devices to another Wi-Fi broke the link (stored pairing-time addresses only). Added LAN rediscovery (discovery.rs): on-demand probes with 30-minute rotating tags only paired devices can recognise; learned addresses stay in-memory candidates until an authenticated QUIC connection succeeds through them. Verified on the realme: found again without re-pairing.
- Link test on a 2.4 GHz hotspot (independently audited): clipboard 10/10 each way, sensitive clip flagged and absent from all logs, files PC->phone 4/4 MD5 OK (saved to the user-chosen folder), phone->PC 3x100 MB MD5 OK at 7-10 MB/s, touchpad OK, no crash/ANR/kill/disconnect.
- realme throttles background apps' network: during a background 100 MB receive it installed per-uid rate limits halving every 5 s (78 Mbit/s down to 1.2 Mbit/s, 3.1 MB/s overall). With an ongoing progress notification during transfers, a 300 MB background receive ran at 8.8 MB/s (link ceiling) with no throttle rules.
- Measurement artefact found: `dumpsys meminfo` forces a GC in the app and blocks realme's freezer; samplers now run every 5 minutes.

## Phase 3.6 (2026-10-19): memory-pressure test on the realme (5-year-old phone, 8 GB)
- 40 min with the user's apps open (Instagram, WhatsApp, Clash Royale, Google Pay, a reader, Play Store) including a thrashing burst (30+ LMK kills in 15 s): Phone Bridge stayed alive at oom_score_adj 200 the whole time (cached apps 900+), never killed; Android trimmed it to 12.9 MB PSS (1.9 MB native heap); every clipboard ping from the PC (every 2 min) arrived within 4-6 s.
- Transfers under that pressure: 300 MB PC->phone in the background at 7.3 MB/s and 2 x 100 MB phone->PC, all MD5 OK; 0 thrashing reports during the transfers, 0 Phone Bridge kills, no freeze or reboot (phone up 3 h).
- Next: speed is bound by the 2.4 GHz hotspot double hop (~9 MB/s); Wi-Fi Direct + BLE presence (both devices support it) proposed as a feasibility spike.

## Phase 3.7 (2026-10-19): transfer speed, measured and tuned
Tool: `tools/pb-bench` (QUIC bulk with transport knobs, plus raw paced UDP), with the phone side cross-built and run from `/data/local/tmp` over adb. The phone listens and the PC connects out, so no firewall change is needed. Path: phone and PC both on a third phone's hotspot (Xiaomi 11 Lite NE), 5 GHz channel 36, 80 MHz; phone link 720-1201 Mbps, PC link 600-1201 Mbps; every byte crosses the air twice.
- **Ceilings without any congestion control:** raw UDP phone -> PC about 220-310 Mbit/s (28-39 MB/s); PC -> phone limited by the PC's Wi-Fi send queue at about 165-215 Mbit/s (20-27 MB/s). Plain TCP: 14-16 MB/s PC -> phone, 19-20 MB/s phone -> PC.
- **Sweep (medians of 3 runs, MB/s up / down):**
  - default 16.4 / 14.8
  - **4 MiB stream window 21.9 / 23.5**
  - 8 MiB 19.8 / 22.1; 16 MiB 21.3 / 21.6 (more queueing and loss)
  - BBRv3 1.6 / 5.0 (broken in noq 1.3 here)
  - GSO on the phone (down only) 19.9
  - MTU 1472 14.1 / 15.4 (no gain)
  - 2 streams 21.3 / 20.6; 4 streams 19.7 / 22.4 (same effect as the window)
  - EcoQoS on the PC 18.3 / 16.3 (no effect)
  - 4 MiB + phone GSO 20.6-22.5 / 27-33.5
- **Cause:** one file is one QUIC stream, and the library's per-stream receive window (1.25 MB) bounds it to window / RTT. Round trips under load on this path are 40-110 ms. STREAM_DATA_BLOCKED frames are not emitted by noq, so the evidence is the window sweep itself.
- **Adopted:** `Tuning::default().stream_window` = 4 MiB (`crates/pb-transport/src/tuning.rs`). The real apps on the same path: PC -> phone 300 MB at **21.5 MB/s** (was 16.1), phone -> PC 2 x 100 MB at **32.9 / 36.8 MB/s** (was 17.5), MD5 OK, no crash. Phone -> PC is now at the path's UDP ceiling, and PC -> phone at the PC's send ceiling.
- **Not adopted yet:** GSO on Android (+~30% on the phone's sending side in the bench) until it is re-tested on USB tethering with iroh 1.3 (finding 1 was recorded on an earlier version). Rejected: BBRv3, larger windows, a larger MTU, parallel streams.
- **Hardware notes (Intel AX211):** one 2x2 radio, so one channel at a time; no Soft AP (Mobile Hotspot is a Wi-Fi Direct GO, 5 GHz capable); a single GO port, so the Wi-Fi Direct spike and Mobile Hotspot cannot run together; no TDLS, so two clients of the same router cannot link directly.
- **Direct path (PC Mobile Hotspot, 5 GHz channel 157; phone link 1201 Mbps).** The PC's own Wi-Fi stayed on the Xiaomi network at 2.4 GHz channel 1, so its single radio hops between the two channels.
  - Raw UDP: PC -> phone about 360 Mbit/s (45 MB/s); phone -> PC about 450-460 Mbit/s (57 MB/s).
  - Bench, PC -> phone: about 48 MB/s, with any window.
  - Bench, phone -> PC: 16-23 MB/s default; 26-34 with 4 MiB; **61-64 MB/s with 4 MiB plus phone GSO**. Without GSO the phone is limited by one system call per packet.
- **Receive pipeline.** The real app's PC -> phone was 26.7 MB/s against the bench's 48, and the same in the foreground, so not throttling. Every few-KB piece QUIC handed over was hashed and written through `tokio::fs` (one thread hop and one small write each), strictly in sequence with the network reads.
  - Now network reads are batched to 1 MiB and hashed and written on a blocking thread, overlapped with the network (`receive_bytes` in `files.rs`).
  - Real app PC -> phone 300 MB: **33.8 / 36.2 / 38.1 MB/s** with gaps between runs. Back-to-back runs vary (27-46) because the phone is still copying the previous file into MediaStore.
- **Seen twice:** the first bench run after the phone had been dozing ran at 0.2 MB/s with round trips of 0.5-0.7 s for minutes, then recovered. Likely realme power saving with the screen off (Wi-Fi power save and CPU limits). To be measured with the screen off; the user turns it off.
- **Next:** GSO on Android, re-tested on USB tethering; the screen-off transfer test; MediaStore copy overlap.

## Phase 3.8 (2026-10-19): PC -> phone control, ask first, smooth cursor
- **Ask-first control.** The phone's "Let my PC control this phone" switch (default off) decides how a request is handled. On: accepted at once. Off: a popup plus a high-priority notification (Allow / Always allow / Deny), and no answer within 30 s counts as Deny.
  - The phone can end a session at any time: Stop on the main page, or turning the switch off. It closes the connection through a `SessionCloser`, and the PC returns its mouse and keyboard.
  - The main page shows "<PC> is controlling this phone".
- **Cursor.** Before, pointer moves were applied with one `WindowManager.updateViewLayout` per main-loop turn: a relayout through system_server, not tied to display frames. Now one full-screen, non-touchable accessibility overlay exists for the session, and the cursor moves by translation once per display frame (`Choreographer`), easing towards the true position (about 2 frames to catch up). Taps use the true position.
  - A Wi-Fi lock (low latency + high performance) is held only during control, so power save cannot batch pointer packets.
- **Tested on the AVD over loopback** (pairing via `Bridge.join`, a test controller from `pb control`): all paths pass — ask, deny, allow, always allow, switch on with no prompt, Stop, switch off mid-session, the 30 s timeout, and Wi-Fi locks held only while controlled.
  - Timing: 175 moves/s arriving, gaps p50 6 ms / p95 7 ms; the cursor redrawn every display frame (57-59/s on the AVD's 60 Hz screen).
  - The 120 Hz feel on the realme is still to be checked by the user.
- **Phone freeze and reboot at 21:14.** realme hang-watchdog kernel panic again: kswapd at 97%, system_server with 310k major faults.
  - Trigger: HDFC PayZapp crash-looping (247 crashes; its tamper check fails while USB debugging is on), plus heavy background apps. Phone Bridge was not involved (PSS about 12 MB).
  - With PayZapp force-stopped, the next 10 minutes had 0 kills, 0 ANRs and 0 crashes.
  - The phone-watch sampler (which forces a GC in every app) was stopped.

## Phase 3.9 (2026-10-19): state-mismatch bugs and a full bug hunt
**Reported by the user and confirmed on the realme:**
- **The Quick Settings tile showed the opposite state.**
  - Cause: `onClick` repainted from the pre-toggle `running` value. realme SystemUI also ignores later `requestListeningState` calls (`Couldn't find tile`), so the wrong paint stuck.
  - Fix: the tile paints the target state at once (with "Starting…"), repaints while still bound, and paints from the real state on every listen.
  - Result: on-device state matrix 0 of 6 mismatches (old build: 3 of 4).
- **The recovery job's background restart was refused silently** (no battery exemption). It now posts one "Phone Bridge is not running. Tap to turn it on".
- **realme Active apps → Stop** (exit reason USER_REQUESTED, "by user request") now counts as turning Phone Bridge off, so the tile, the app and recovery agree.
- **The accessibility service was stuck "crashed"** after a kill. The card now says so and shows how to fix it.

**Review agents found more** (Android lifecycle, Rust core, Windows tray), all fixed with tests where possible:
- one control session or request at a time (stops a second session hijacking Stop and the cursor);
- turning accessibility off mid-session ends control;
- the popup times from the request, follows new requests, and closes when answered from the notification;
- "saved" uses its own close code (200), because QUIC also closes with 0 on drop or shutdown;
- `InputSender::cancel` drops a queued paste (`stop` still flushes key releases);
- stream and connection windows capped;
- the Android runtime really multi-threaded (async-compat `multi-thread`; it had been a single thread);
- unpair racing a connect;
- pending ids local, partial file names local;
- prompts end when the sender leaves;
- stall-tolerant input limits;
- honest touchpad, share-failure and pairing messages;
- clipboard retried after a failed sync (phone and PC);
- rotation-aware control;
- service shutdown waits for queued events before the next start empties `incoming/`;
- orphan node cleanup survives cancellation;
- tray:
  - control request "Asking…" with cancel and accurate refusal messages;
  - turning "Let my phone control this PC" off ends the session;
  - all held keys and buttons released;
  - foreground keyboard layout;
  - "Paired", not "Linked";
  - real save folder;
  - stale partial files removed at start.

**Still open:**
- the tray does not track reachability, so it shows "Paired" instead;
- several devices: actions go to the first peer;
- keys stuck if the tray is killed mid phone→PC control;
- a dead tray core shows no fatal state.

**Memory:** the realme rebooted again at 23:41 (hang watchdog, swap 91%) while UI automation (uiautomator dumps, screenshots, log streams) ran on it, with PayZapp crash-looping since boot. On-device checks are now kept light while memory is tight.

## Phase 3.10 (2026-10-20): what really froze the realme
- **Cause, measured A/B:** Android Studio's device mirroring ("Running Devices") of the realme, plus its Logcat streaming.
  - Mirroring on: kernel CPU 41-62%, kswapd 73-93%, swap 25k in / 33k out pages/s, 26k major faults/s, free memory down to 0.77 GB. That is the same signature as the four hangs and reboots (hang-watchdog panics).
  - Studio quit: kernel 7%, swap 62/24 pages/s, free memory 3.0-3.3 GB within seconds.
  - The mirror agent (166 MB) and the MediaTek video encoder push this memory-tight phone into a swap storm. Unplugging USB ended it, which is why the phone calmed down at once.
- **Phone Bridge was not running during the last hang** and was not a factor. PayZapp's crash loop (killed by its tamper check while USB debugging is on) added load earlier; it has been uninstalled.
- **Rule:** no Studio mirroring or Logcat on the realme during device tests; keep test tooling light (verify skill notes, project memory).
- **Keyboard (PC → phone):** typing now goes through the accessibility input method (Android 13+): text is committed at the cursor and keys are real key events. It replaces whole-field rewrites from a stale snapshot, which jumbled text and took WhatsApp's "Message" placeholder for text. Confirmed working on the realme.
- **PC feedback:** answers to the user's own actions show in a status pill that Do Not Disturb cannot hide.
  - Shell balloons were dropped and never reached the notification centre under Do Not Disturb.
  - Next: real Windows notifications with an app identity, so the notification centre keeps a history.

## Phase 4.0 (2026-10-20): phone notifications in the Windows notification centre
Spec: `docs/superpowers/specs/2026-10-20-phone-notifications-on-pc-design.md`; plan: `docs/superpowers/plans/2026-10-20-phone-notifications-on-pc.md`.
- **Real toasts:** WinRT `ToastNotificationManager` under the AppUserModelID `PhoneBridge.Tray`. A Start-menu shortcut carries the id and a toast-activator CLSID, and a per-user COM server (HKCU) delivers clicks, buttons and replies, also for toasts left in the notification centre after a tray restart. No admin rights. Phone Bridge's own messages use the same toasts; balloons only if toast setup fails.
- **Short sessions, not a long-lived stream.** A stream held open forever means heartbeats every 5 s both ways, waking the phone's Wi-Fi all day.
  - The phone opens a notify session to each PC when it has something to send, and finishes it after 15 s idle.
  - Each session starts with `NotifySync` (the active keys) and quietly re-sends what failed to arrive earlier (at most 50).
  - At start the tray asks each phone once (`NotifySyncRequest`).
  - Updates of one key waiting in the queue go out as the last one only.
- **Limits:** everything fits the existing 64 KiB control frame: pictures 48 KiB JPEG, icons 16 KiB PNG, text 4096 B, 5 lines, truncated on character boundaries.
- **Muting is per PC** and filtered on the PC, so the phone keeps no per-PC state.
- "Quiet while the phone-screen window is focused" exists and waits for sub-project 1.
- **Notification-centre clears** are noticed by comparing Windows' toast history every 5 s. This is skipped at Windows' 20-toasts-per-app cap, so toasts pushed out by the cap are never cleared on the phone.
- **Auto-copy** of one-time codes skips quiet (re-sent) notifications, so a tray start never overwrites the clipboard with an old code.
- **Fixed after the whole-branch review:**
  - **Clears are mirrored only for toasts Windows actually kept.** A toast counts as cleared only after it was seen in Windows' history; toasts never kept (notifications off for the app, not in the notification centre, toasts unavailable) are dropped quietly and never cleared on the phone.
  - **Restart:** toast arguments carry the phone id and key, and the tray rebuilds its list from the notification centre at start, so buttons and replies work after a restart and the phone's sync removes stale toasts.
  - **Pacing:** the phone paces notify records at 40/s; the PC's flood limit is 200/s on arrival, so a 50-notification sync is never cut off.
  - **Calls:** ringing calls (posted as ongoing by dialers) are mirrored; when the call is answered the toast goes and a silenced ringer comes back. Silence never changes a phone that is already on vibrate or silent.
  - **Phone list:** the phone keeps its list of active notifications current while sharing is off, and a re-seed replaces it.
  - **Own messages:** Phone Bridge's own messages rotate through 5 toasts, so they never crowd out phone notifications.
  - **Clicks:** clicking a toast is not taken for a clear.
  - **Groups:** the toast group is stable per notification key.
  - **Locking:** WinRT calls never run under the UI's notification lock.
  - **Errors:** a clear made while the phone is away fails quietly.
  - **Older phones:** the start-up sync session ends after 30 s if an older phone app never answers.
- **Found in device tests (2026-10-20):**
  - **Private notifications:** WhatsApp marks every notification private, so the public version ("N new messages") was all the PC got. The phone now sends the full content flagged private, and the PC hides private content while it is locked, even in "Full" mode.
  - **Unlock refresh:** toasts that arrived while the PC was locked are filled in quietly at unlock.
  - **App names and icons:** these needed the manifest's launcher `<queries>`, with the notification's app info as a fallback.
  - **Several chats:** WhatsApp's several-chats notification is InboxStyle; its lines are now sent.
  - **Phone off Wi-Fi:** the realme dropped off Wi-Fi twice (mobile data only); nothing can arrive until it is back, by design (local network only).

## Phase 4.1 (2026-10-20): phone screen on the PC
Plan: `docs/superpowers/plans/2026-10-20-phone-screen-on-pc.md`.
- **The realme's encoder, probed:** `c2.mtk.hevc.encoder`, codecs H.265 Main and H.264, **no HEVC Main10** with surface input, VBR supported, QP ceiling available (API 33), alignment 16×16.
  - So the stream is 8-bit HEVC Main with VBR and a QP ceiling.
  - The spec's 10-bit wish cannot be met by this phone's hardware. Android composes the screen in 8-bit anyway, so Main10 would only have avoided added encoder banding; the QP ceiling and generous VBR keep that low.
  - The PC still offers Main10 first, so a phone whose encoder has it uses it.
- **This PC decodes** H.265 Main10, H.265 and H.264 (synchronous MFTs with D3D11/DXVA, low-latency mode); an H.264 stream encoded in-box decodes on the GPU in a test.
- **Decisions made in the plan or while building:**
  - **One consent:** one consent per session covers video and the window's input.
  - **Lag:** measured on frames in flight only (an idle screen is never lag).
  - **Input records:** absolute `Touch` and `Pinch` input records; the wheel is a hover then the existing wheel.
  - **The PC never drops records** (a missing P-frame would smear until the next key frame). A slow decoder slows the stream, and the phone's pacer drops cleanly to a key frame and lowers the bitrate.
  - **Reconnect:** the PC retries for 30 s; the phone resumes without a prompt within its grace, and declines quietly for 30 s after its user pressed Stop.
- **Device tests 1–2 (2026-10-20):**
  - **Lag 1:** the PC sent window state on every resize, and each message made the phone ask for a key frame. Now state is sent on change only.
  - **Lag 2:** the 120 Hz virtual display fed the encoder at 120 frames/s. Now capped with `max-fps-to-encoder` at 60 (the user chose 60 fps).
  - **Frozen, smeared picture, from four bugs:**
    - The PC confirmed frames only every 100 ms while the pacer called 150 ms late. Now every frame is confirmed and the limit is 300 ms.
    - Drop mode asked for a key frame only once. Now it asks every 300 ms.
    - Dropped "repeats" and refused frames left P-frames without their reference. Now nothing is dropped except whole runs up to a key frame.
    - The step-down fired on level 20 (UI_HIDDEN, our own UI going to the background). Now it fires only on RUNNING_LOW and RUNNING_CRITICAL.
  - **Secure screens:** noticed from fully black 16×16 samples, once a second.
  - **The v1 window** is a normal resizable window with the Fluent frame; the video is on a child window (flip swap chain), and the bars are painted on the parent.

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
