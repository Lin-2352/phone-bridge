# Relay: design decisions and measurements

A running engineering log, newest at the end. The project was called Phone Bridge when most of this was written. The first section is the original plan, and later sections record what was measured and what changed. Claims marked *unverified* came from early research and were to be re-checked by the spikes.

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
- Hardening: PC control off until the user allows it in a prompt; receivers cap input at 2000 moves/s and end the session above 1000 clicks/keys or 4000 records per second; pipe sends only existing files; accessibility service subscribes only to announcements.

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

## Phase 4.1 device results: phone screen on the PC (2026-10-21, realme RMX3561, Android Studio closed)
Measured on the final build (`83f75be`), 864 x 1920 H.265 at 60 fps over the hotspot:

| Check | Result |
|---|---|
| Continuous scroll (Settings, 16 swipes) | out 55-60/s, 0 drops, encode p50 19-23 / p95 28-39 ms, **confirm p50 22 / p95 33-37 ms** (before the newest-frame fix: 160-260 ms), PC decode+present p50 11-15 / p95 24-28 ms, received 60/s, shown 54-57/s, skipped 4-6/s |
| Still screen | 10-12 frames/s (encoder repeat every 100 ms), confirm p95 < 30 ms. The spec's "drops to 0 after the repeats" is not met: repeats continue at 10/s (about 1.5 Mbit/s). Left as is: it keeps the picture fresh and costs little. |
| Hide, then show | hide ends the share with no resume grace (phone: "ready for an instant share"); the next show needs no prompt: accepted after 101 ms, first frame shown after 798 ms |
| Phone Bridge memory while sharing | PSS 75 MB (native 16, Java 5, graphics 8): **over the 60 MB budget**, down from 100 MB. Open item. |
| Free memory | MemAvailable 2.25 GB, no step-down |

Fixes made during these tests and the whole-branch review: step-down only under 1.2 GB free and back up above 1.6 GB (realme sends "running low" with 2 GB free); only the newest decoded frame is presented when more are waiting (a queue had built up behind the monitor refresh); the config record is resent when a session attaches (a resumed window stayed blank); a PC-side close ends the share at once; config records are never dropped by the pacer or the send queue; frames sent while the streams open are no longer lost; keys and a finger held through the window are released on focus loss or close and reset on the phone; closing the window cancels a pending reconnect; Stop during the resume grace declines that PC quietly; a stale session of the same PC is superseded; a second PC does not take another PC's resume grace; the power button keeps the mirror awake and tells the PC; the "Asking..." pill closes when the phone accepts.

Not tested on the device (need the user): rotation, file drop, a secure screen, the stopwatch glass-to-glass check, Ctrl+W then typing. Deferred review minors: a second PC window state after reconnect (position, minimised, dim not saved), single status slot, notification text during the grace, "busy" decline looks like a Deny, thermal step-down, two consent activities for one request on a locked phone.

## Phase 5.0 (2026-10-21): Continuity Camera (phone camera as a Windows 11 virtual camera)
Built from `docs/superpowers/specs/2026-10-20-continuity-camera-design.md` and its plan. Device results so far (this PC, realme RMX3561):

| Step | Result |
|---|---|
| Install (one UAC prompt) | DLL copied to `%ProgramData%\PhoneBridge\camera\pb-camera-source-<version>-<hash>.dll`, HKLM class key with `ThreadingModel=Both`, frames file 6,224,896 bytes; ACLs checked with `icacls`: frames file SYSTEM full, LocalService R,W, the installing user R,W; folder SYSTEM and Administrators full, LocalService and Users read and execute |
| Phone camera session | Accepted; 1920x1080 H.265, rotation 90, back lens; Camera2 TEMPLATE_RECORD opened by the foreground service in the camera type |
| GPU fit on this PC | Portrait 540x960 into 1280x720: bars exactly Y16 UV128, picture exact, rows tight (a unit test) |
| Frame Server activation | Needed three things found on the device: the registered class must be an **IMFActivate** (Frame Server asked for `{7FEE9E9A-4A89-47A6-899C-B6A53A70FB67}`; a media source alone gave "No such interface"), `GetService` must answer `MF_E_UNSUPPORTED_SERVICE`, and a changed DLL needs a new file name (content-addressed, so an update installs beside one Frame Server still holds) |
| Virtual camera Start | **Blocked by Windows' machine-wide camera switch**: `HKLM\...\CapabilityAccessManager\ConsentStore\webcam = Deny` (Settings, Privacy and security, Camera, "Camera access") makes Frame Server answer "Access is denied" after it has created and queried the source. A system privacy setting: not changed by Phone Bridge. With it on, the remaining device checks (picture in the Camera app, lens switch, rotation, kill-the-tray cleanup, removal) can run. |

Diagnostics: the source writes its COM calls as text into unused header bytes (128..4096) of the frames file, because Frame Server's own log is out of reach; no pictures or personal data.

Camera review (fresh reviewer, 2026-10-21): the Critical finding was real: the elevated installer trusted a folder and DLL that any standard user could have planted under ProgramData (the folder inherits "Users may create folders", the DLL name is guessable, and the `ProgramData` environment variable belongs to the user), which would have loaded the user's code into Frame Server as LocalService. Now: ProgramData comes from `SHGetKnownFolderPath`, both folders must be owned by SYSTEM, Administrators or a service account (else they are deleted and recreated), junctions are refused, the DLL bytes are always written, and the SID is validated. Also fixed: a camera that fails on the phone ends the PC session, rotation follows the physical orientation sensor (the screen is off or portrait-only while the phone is a webcam), a stale "Camera in use" notification, the media-type lock only between Start and Stop, no store through a read-only mapping inside Frame Server, sign-out stops the camera, the frames file is cleared at start, and the picture is kept for the whole 30 s reconnect (black after 35 s).

Phone Bridge memory (2026-10-21, `dumpsys meminfo`, realme RMX3561): the "PSS under 60 MB" budget counted memory that is not ours. Idle reading: TOTAL PSS 60-70 MB, of which "System" is 28-29 MB (shared Android framework pages that any app carries and no app can reduce). Our own part: Java heap 6 MB, native heap 8 MB, graphics 12 MB, code 4 MB, other 8 MB: about 38 MB; during a 1080p share the earlier reading of 75 MB total is about 47 MB of our own. **Ruling: the budget is measured as TOTAL PSS minus "System" (our own footprint), limit 60 MB** — met (about 47 MB while sharing). A live-share breakdown still needs the phone's capture prompt tapped; it is in the open device checks. Cost if wrong: none to users; only the number we hold ourselves to.

Camera source pacing: the 30 fps timer uses absolute deadlines (`pump::tick_due`) instead of a sleep after each tick (which gave about 28 fps).

## Phase 5.1 (2026-10-23): camera device results and the realme "no focus window" kill
Device: realme RMX3561, this PC, the phone's own Camera app open underneath.

| Check | Result |
|---|---|
| Streaming with the phone's Camera app open | Works: 30 fps, a real picture, the status window shows six green rows and "Working". Held for 60 s in one process. |
| Virtual camera as an app sees it | Listed by Windows as "Phone Bridge Camera (Windows Virtual Camera)"; the window's own source reader reads 29-30 frames a second. |
| Quick Settings tile after the app was killed | Locked: a tap starts the service directly (0.16 s, no unlock prompt). Unlocked: a tap starts it through the invisible foreground activity. Both verified from a fully stopped app. A real phone restart is not tested (needs the user). |

**The stalled stream was a watchdog kill, not a link problem.** The invisible window kept in front of the phone's Camera app was touch-through and not focusable. With no focusable window the system has no focus window; realme's "Theia" watchdog logs `hasTopDisplayFocusWindow focus:null`, then `ANR in dev.phonebridge.app ... Changing to new focus window timeout`, and kills the process about 13 s later. Frames fell 29, 3, 0 a second and the tray reported "Phone camera stopped". Fixed: the window is touch-through only (`holdWindowFlags()`, unit tested); 60 s run with the same process, no kill. Rule for this phone: a held-open activity must stay focusable.

**Request that Android cannot open a window for.** A camera or screen request that arrives while the app is in the background used to be dropped silently on an unlocked phone (Android blocks the background activity start). It now always posts a notification ("<PC> wants to use your camera", tap to answer); the window cancels it when it opens.

**Always allow is per PC (decision).** The camera consent remembers allowed PCs by peer id, not one global switch. The old global switch is not migrated: no PC is allowed until it is chosen again. The id is looked up from the PC's name, and a name shared by two paired PCs gives no id, so it is never auto-allowed.

Also in this stretch: the camera steps down to 1280x720 under 1.2 GB free and returns above 1.6 GB (decided per session); the tray menu and the camera window show Cancel while the phone has not answered; the window offers "Repair camera" when starting fails or the shipped source is newer; the phone reports its lens (wire tag 24) so the tray's "Switch lens" and the next request follow the real lens, and the window names it.

Not tested on the device (need the user): Cancel and Repair on screen, the lens report end to end (the phone's consent for this PC must be given again, see above), a phone restart with the tile, the phone-screen and notification items listed under Phase 4.1.

## Phase 5.2 (2026-10-25): deferred camera minors and the portable Relay folder

- Fixed with tests first: the pending flag is a count (a quick Cancel then Start keeps the request open); Repair is not offered for decode/GPU errors; only one install prompt at a time; Cancel works during the first install; the memory step-down is decided once per session.
- Left as they are: the phone's request window stays up to 30 s after the PC cancels (no wire message tells the phone; a new one is not worth it for a 30 s window); the lens decoder keeps `#[allow(deprecated)]` because `prost::DecodeError::new` is the only way to reject a value in a hand-written `Message`.
- Portable folder: `scripts/make-portable.ps1` builds `dist\Relay` (Relay.exe, relay-cli.exe, pb_camera_source.dll, Relay.cmd, data\). Name chosen by the user: Relay. Only file and folder names changed; window titles, crate names, the Android package and the phone app label are unchanged.

## Phase 7 (2026-10-28): clipboard pictures (branch `feature/clipboard-images`, not in v0.1.0)

- **What it does.** A picture copied on the PC (Copy image, Snipping Tool) lands on the phone clipboard; each new phone screenshot lands on the PC clipboard; the phone's "Send clipboard" Quick Settings tile sends a copied picture (or text) to the PC. Off by default on both sides: PC "Share images" (needs Share clipboard), phone "Share copied images" and "Send new screenshots to PC".
- **Wire.** `OfferKind::ClipboardImage = 3` over the file stream; PNG only, at most 10 MiB; the receiver keeps it in memory, so nothing is written to the save folder. An old peer would save it as a plain file. No new control message.
- **Windows.** The PC reads "PNG", then DIBV5, then DIB; writes DIBV5 plus "PNG". DIB to PNG is safe Rust (`png` crate, 8192 px / 32M pixel cap). Password-manager (private) pictures are never sent; echo is stopped by hashes.
- **Android.** A received PNG goes to `cacheDir/clips` behind a FileProvider (not the gallery), newest 5 kept, nothing older than a day. Screenshots come from a MediaStore observer that only accepts screenshot-looking rows added in the last 15 s; needs the READ_MEDIA_IMAGES prompt (the user's tap). Phone pictures are converted to PNG before sending.
- **Device results (realme RMX3561, Android 13).**
  - PC to phone: Copy image and Snipping Tool both paste on the phone (the user confirmed); 2.4 MB and 2.9 MB sent and received.
  - Phone screenshot to PC: 1080x2412 arrives in about 2 s and pastes on the PC.
  - Tile: a picture copied on the phone reaches the PC clipboard (322 KB, 678x452).
  - Off switches: PC switch off sends nothing and refuses the phone's picture ("image sharing is off", the phone reports "declined"); phone "Share copied images" off refuses the PC's picture; phone screenshots switch off sends nothing, back on sends again.
  - Not sent: a password-flagged PC picture; a non-screenshot picture saved in `Pictures/`; the same picture twice; a picture that just arrived (no echo between devices).
  - Phone offline: the PC logs "timed out", stays responsive, and sends again once Relay is on.
- **Not verified on the device:** a phone-side picture over 10 MB (covered by unit tests only); a revoked READ_MEDIA_IMAGES permission and the home banner (revoking would need a re-grant by the user); the tile after a phone restart.
- **Found on the way.** The Linux CI job had been failing since the settings window work: `settings_rows.rs` (built everywhere) imports `camera_status`, which was gated to Windows. It is pure, so it now builds on every platform (`bb07dfe` on `phase3`, same change on this branch).
- **Left alone.** An oversize picture copied on the PC is dropped without a log line.

## Phase 8 (2026-11-02): UI v2 budgets on the PC (Task 13, `feature/ui-v2`)

Measured on the dev laptop (Intel UHD Graphics drives the panel, RTX 4060 idle; 32 GB), `--profile spike`, throwaway `--dir`, no phone paired. "Private" is `PrivateMemorySize64`; working set (WS) also counts shared DLL pages.

| State | WS | Private | Notes |
|---|---|---|---|
| Idle, no window ever opened, no device | 35 MB | 5.6 MB | after the change (the device is no longer made at start) |
| Idle at start with the old warm-up | 61 MB | 55.8 MB | before: the device was made 3 s after start and never freed |
| Main window open | 70 MB | 61 MB | same before and after; over the 40 MB budget |
| Main window closed, 30 s later (device released, WS trimmed) | 1 to 5 MB | 12.9 to 13.2 MB | after; before, the device stayed (about 61 MB private) |
| Camera window opened and closed after that, 40 s later | 3 to 5 MB | 13.3 MB | after: released again (any window on the UI thread) |

- **Where the memory goes.** The D3D11 device plus its Direct2D device costs about 55 MB of private bytes in the Intel driver. It's the same on the default adapter and at feature levels 11.x, 10_0 and 9_3. WARP would cost about 15 MB private, but WS is still about 55 MB with the window open. With no device the tray is 35 MB WS (shared DLL pages), so a 40 MB WS budget with a GPU-drawn window can't be met on this machine. Proposal: restate the budget as private bytes. That's about 61 MB open on this driver; under 40 MB only with WARP.
- **First paint** (log line, cold device): 182 to 250 ms over six runs (window 1 to 2 ms, surface 150 to 218 ms, draw 23 ms). With the old warm-up it was 22 to 30 ms. The 200 ms budget is met in most runs but not all.
  - Releasing has a cost: every first window after 30 s idle pays device creation (about 150 to 220 ms on the UI thread). That includes the flyout from a tray click, which used to open on a warm device.
  - The first-paint line is logged once per process, so reopen timings after a release aren't in the log.
- **Idle CPU over 60 s:** window open 0.03 to 0.09 s (the 1 s refresh timer); window closed 0.00 s after the change. Before the change it was 0.08 to 0.20 s, from the permanent 1 s timer.
- **Changes:**
  - The device is made by the first window, not at start.
  - It's released 30 s after the last surface on the UI thread closes (main window, flyout, menu, QR, camera window or pill), then the working set is trimmed. Dialogs and the screen window run on their own threads; their devices are freed when those threads end.
  - The 1 s timeline timer runs only while a write is owed or the main window or flyout is open. A timeline add from the core wakes it.
- **GDI:** every window now draws through Direct2D. The guard test `no_gdi_drawing_left` keeps it that way. The `Win32_Graphics_GdiPlus` feature is gone.

## Phase 8 (2026-11-02): UI v2 results and rulings

What was built (`feature/ui-v2`, from `feature/clipboard-images`; not merged, not released): a shared design-token file generating Rust and Kotlin; a Direct2D drawing kit on the PC (Mica where Windows allows it, opaque otherwise); a main window (Home, Timeline, Devices, Settings) next to the tray; a metadata-only Timeline on both devices; a four-tab Android app (Home, Timeline, Remote, Settings); a home-screen widget; two FFI getters.

**Rulings that change the spec (please confirm):**
- **Memory budget restated.** "Under 40 MB working set with the main window open" cannot be met with a hardware Direct2D device on this machine: the GPU driver alone costs about 55 MB private. New budget: idle tray with no window at most 40 MB working set and 15 MB private; main window open at most 70 MB private; the device is released and the working set trimmed 30 s after the last window closes. WARP (software) would cut private memory to about 22 MB but is slower; not chosen.
- **No always-on warm-up.** A warm device would hold about 55 MB for as long as the tray runs. Cold first paint is 182 to 250 ms (warm: 22 to 30 ms). Accepted as "about 200 ms".
- **Silent removal of "reset" PCs after pairing was removed.** It could unpair the PC the user just re-paired. The Home row now offers "Remove and pair again" (explicit, only for PCs flagged at that moment). The transport clears the `changed` flag on re-pairing and on every authenticated incoming session.
- **Turning "Keep history" off asks first** (it deletes the history), on both devices.

**Known limits:**
- A device on the same network can plant an address for a paired peer through the unauthenticated discovery packet (its tag is not bound to the sender). iroh 1.3 keeps every dialled address as a path and cannot drop one. Mitigations: a peer whose failed dial included an unsaved learned address is distrusted for the rest of the run (its mismatches show as ordinary connect errors, never "reinstalled or reset"; a real reset of that peer then shows only after a restart); the "reinstalled or reset" notes and the phone's Home row say another device may be the cause; "Remove and pair again" asks first and re-reads which PCs are flagged. Remaining gap, accepted: a planted address that was dialled in a connect that succeeded or timed out is remembered but does not distrust the peer, so if the phone then goes offline and the planter answers there, a false "reinstalled or reset" can still appear. No takeover is possible: re-pairing needs the real QR code and the SAS.
- The screen window now builds its own Direct2D device for its two bars; its private memory with a phone screen open is unmeasured.
- The camera preview allocates a GPU bitmap per frame (about 30 per second).
- Android 14+ partial photo access may jump to the app settings after the photo prompt (this phone is Android 13; not testable here).

**Not verified on real hardware (this phone was not reachable over adb during the final pass):** every Android screen on the phone, the widget, the tab back stack and rotation, the touchpad edge-swipe, Timeline entries after real transfers, screen and control sessions with the new windows, Mica (transparency effects are off on this PC), mixed-DPI monitors, and the flyout/dialog captures. The Android release APK builds and is signed with the Relay release key; unit tests and `lintRelease` pass.

**Environment finding:** on this PC the Windows clipboard currently misbehaves (`Set-Clipboard` succeeds but `Get-Clipboard` reads back empty). The two `pb-platform-win` clipboard tests fail with "clipboard is busy", identically at the Phase 7 commit, so it is not a regression. Clipboard sync cannot be verified end to end on this PC until that clears.

## Phase 7b (2026-11-02): why PayZapp refused to open, and the workaround

PayZapp (HDFC) checks for security risks and refuses to run while either of these is on: (1) a non-screen-reader **accessibility service** (its dialog names "Relay control"), and (2) **USB debugging**. Relay's Control feature needs the accessibility service, so Control on means PayZapp will not open. Relay cannot and will not hide this from the bank app.

Workaround: keep Relay Control off except while controlling the phone (Relay now switches it off about 1.5 s after a PC session ends; checked on the realme: session end 01:01:13.8, switch-off 01:01:15.3, `enabled_accessibility_services` empty), and turn USB debugging off (some banking apps also object to Developer options being on at all) when using PayZapp.

## Phase 8b (2026-11-03): the release APK crashed on start (found on the phone)

The first release APK built from the whole UI v2 branch crashed at launch with "Failed to create an instance of androidx.work.impl.WorkDatabase". WorkManager arrived with the Glance widget library; it creates its Room database by reflection on the generated `WorkDatabase_Impl`, and R8 (full mode) kept the class but removed its default constructor. Unit tests, lint and CI only exercise debug or non-shrunk code, so none of them could see it (the constructor stays in debug builds).

Fix: `-keep class * extends androidx.room.RoomDatabase { <init>(); }` in `android/app/proguard-rules.pro`; verified on the realme (app starts, MainActivity focused, no crash). Lesson, now a release rule: **install the minified release APK on a real phone and start it before any tag or release**; do not rely on CI alone for shrinker problems.

## Phase 9 (2026-11-03): audio sharing, both directions (branch `feature/audio-full`)

- **Both directions.** Phone audio plays on the PC and PC audio plays on the phone; raw 16-bit PCM (no Opus: about 1.5 Mbit/s on Wi-Fi, which the link carries). Details in WIRE_PROTOCOL.md.
- **The PC's audio queue follows the network.** Phone audio over 2.4 GHz arrives with gaps of 100 to 195 ms and no loss. With a fixed 140 ms queue each long gap ran it dry (a hard cut) and the burst after it overflowed the limit (a skip): one gap made two audible faults. The queue now starts at 160 ms, grows by 40 ms each time it runs dry up to 260 ms, shrinks by 20 ms after each clean minute, skips only past target + 80 ms, and fades every break over 5 ms.
- **Phone audio is Opus, because ColorOS caps Relay's upload.** With a video app in front, a realme phone on ColorOS (Android 13) shapes Relay's Wi-Fi upload to 600 kbit/s with a 3000-packet queue: `tc -s class show dev wlan0` shows `class htb 4:5f rate 600Kbit`, backlog 3000 packets, classified by the eBPF program `prog_oplus_netd_schedcls_egress_data_match`. Raw stereo needs about 1.5 Mbit/s, so delay grew about 6 s every 10 s, with no loss. It does not happen with Relay in front or on the home screen, which hid it from the first tests. Marking the socket's priority cannot escape the shaper: the QUIC library puts an `IP_TOS` cmsg on every packet, which overrides it, and forking the library for one vendor's setup would be fragile. Opus at about 160 kbit/s fits under the cap on any phone, with Android's own encoder and libopus on the PC (`unsafe-libopus`, the reference code transpiled to Rust; heavily fuzzed upstream, and only a paired device can reach it). The pure-Rust `opus-decoder` was tried first, because it has no unsafe code, but took about 27 ms for each 20 ms packet from the phone's encoder, so it dropped a quarter of the audio. PC to phone stays PCM: the cap is on the phone's upload. The cap would also hit screen and camera streams and file sends with a video app in front.
- **PCM in QUIC datagrams, not a stream.** The first version sent PCM on a reliable uni stream (`PBA0` magic, length-prefixed frames). During a Wi-Fi stall QUIC kept retransmitting in order and the listener heard seconds of old audio afterwards. Measured on a simulated 3 s blackout: audio 3085 ms old one second after the link returned on the stream, 2 ms on datagrams. Each datagram carries an 8-byte header (u32 sequence, u32 sender clock in ms) and whole samples. The sender's QUIC datagram buffer is about 100 ms (`tuning.rs`), the receiver queue 100 ms, both dropping the oldest; the receiver holds a packet that arrives ahead of its predecessor for up to 15 ms (measured on the real Wi-Fi: about half the packets arrived swapped with a neighbour one evening, which dropped them as late and caused dozens of underruns; with the hold, 4 lost and 1 to 3 underruns in 35 s); late packets are dropped; gaps up to 40 ms are filled with silence to keep timing, longer gaps are not (filling a stall only adds delay). The stream format and its magic are gone; versions must match pre-1.0, so there is no fallback.
- **Consent on the device whose audio is shared.** Phone audio: the phone asks (Relay's question, then Android's capture prompt, skipped only when the audio helper does the capture or the user allowed it outside Relay). PC audio: the PC asks. Each skip switch belongs to the device that asks: the phone has "Share this phone's audio without asking" (phone audio to the PC), the PC has "Play PC audio without asking". The earlier phone-side preference for the PC direction was stored and never read; it was removed instead of wired, because the phone has no question to skip in that direction.
- **The PC starts every session.** The transport makes the requester the receiver in one direction, so a phone-initiated "share my audio" needs a new message. Not added in this version (see WIRE_PROTOCOL.md, Audio sharing, points 6 and 7); the phone's audio tiles say "starts from your PC", like Screen and Camera. Consequence: the microphone mode cannot be started from the apps. It stays built and gated on the phone, and is excluded by default in THREAT_MODEL.md row 20.
- **No master switch** (superseded by "Per-device permissions", v2 M8). Every session already needs consent, and the Stop notification is always shown, so a general on/off per direction would be a second way to say no. Not added.
- **Pausing Relay on the PC ends audio.** Like the camera and the screen, a pause closes every session.
- **Timeline and logs keep metadata only** (who, direction, length). The audio itself is never stored.

## Audio helper (2026-11-04, branch `feature/audio-full`)

- **Why it exists.** A normal app's capture leaves the sound on the speaker, so Silence had to mute the phone: a volume key then unmuted it for a moment and the volume bar showed nothing useful. Android's `REMOTE_SUBMIX` source moves media off the speaker and hands the phone's real volume bar the level, as scrcpy's default mode does, but it needs `CAPTURE_AUDIO_OUTPUT`, which only the shell or the system can hold. So capture runs in a small program started as the shell user from Relay's own APK (following scrcpy's AudioDirectCapture, Apache-2.0). It has no command runner: capture, ping and quit are all it does.
- **Measured on a realme RMX3561, Android 13, ColorOS, video app in front.** 6 volume presses took a steady tone from peak 806 to 182 and back to 814, with the speaker volume untouched, no mute and no prompt. Alarm, ringtone and notification sounds stay routed to the speaker. Killing Relay mid-share stops recording and routes media back to the speaker.
- **One-time setup, in the phone alone.** The user pairs Relay with Wireless debugging once. Relay then grants itself `WRITE_SECURE_SETTINGS` over that connection, so after a reboot or an update it can turn Wireless debugging on for a moment, start the helper, and put it back. This is the pattern of Shizuku's boot receiver, without installing another app. It needs Wi-Fi at the moment of starting; until then, sharing falls back to Android's capture.
- **Loopback TCP, not a local socket.** An app could not reach a Unix socket held by the shell on this phone. Hidden binder calls would work but change between Android versions. Any app can reach a loopback port, so both sides prove a shared secret over fresh nonces before anything else, and the helper gets the secret from its environment, never its command line.
- **Android 12 or later.** On Android 11 this capture needs a shell window in front.
- **Library.** `libadb-android` 3.1.1 (Apache-2.0 option), which states it has never had a security audit. It comes from JitPack, which builds on demand, so its artifacts are pinned by hash in `android/gradle/verification-metadata.xml`. With Conscrypt and Bouncy Castle it takes the release APK from 16 MB to 23 MB.
- **Bluetooth was considered.** Windows can act as a Bluetooth speaker for the phone (`AudioPlaybackConnection`), with no helper and no stored access. The user chose the helper for its Wi-Fi range and since it matches the behaviour they showed. Delay and quality of the Bluetooth route were not measured.
- **The mute path stays** as the fallback for phones without the helper.
- **Debug builds only (2026-11-05, v2 M2.1).** The helper, its adb setup and its libraries live in `src/debug`; release and qa builds compile no-op stand-ins from `src/release`, so libadb, Conscrypt, Bouncy Castle and JitPack leave the release graph (CI checks it). A release build removes an old install's stored adb identity, its Keystore key and the helper's settings once.

## Silencing the speaker with an effect (2026-11-04)

- **The problem.** Android's capture leaves the sound on the speaker. Muting broke the volume keys: a press unmuted for a moment, and the bar showed nothing useful. The audio helper above gave the right behaviour but needs adb, which needs Developer options.
- **Why the helper can't do it.** Anything started through adb sits in adbd's process group (measured: `0::/uid_0/pid_3176`), and Shizuku's manual says to keep USB debugging and Developer options on. Banks refuse those settings: PayZapp refuses USB debugging. `pm grant` for `WRITE_SECURE_SETTINGS` also failed silently on this phone, most likely ColorOS's Permission monitoring switch, though the refusal text wasn't captured. The helper is switched off (`HELPER_OFFERED`) and its code remains.
- **What's used instead.** A `DynamicsProcessing` effect at -96 dB on audio session 0, needing only `MODIFY_AUDIO_SETTINGS`. The volume index moves normally, so the real bar works, and Relay maps it onto the PC's level. Android placed the effect on the deep-buffer output, and captured sound is taken before it.
- **Measured on a realme RMX3561, Android 13, video in Google Photos.** With the bar at the maximum, the user heard nothing from the speaker and the video on the PC. Captured peak stayed at 22,000 to 27,000. Notifications, calls and alarms still played on the speaker.
- **During a share the bar belongs to the PC.** The phone's own volume is saved first and put back on stop before the effect is released, or at the next start after a crash. A share starts at the last share's level, so a leftover 0 can't silence the PC. If Android refuses the effect or another app takes it, Relay falls back to muting.
- **Limits.** Not tested on other phones or with apps that use another output, such as games. Android chooses where a session-0 effect goes, and session-0 insert effects are documented as deprecated for some effect types. While Relay is dead nothing silences the speaker, so sound can return at the share's level until the service restarts. A route change during a share can restore the volume to the wrong output.

## Idle wakeup budget (2026-11-05, v2 M2.19)

- **Budget.** Idle with no PC in contact and Relay not on screen, Relay wakes the phone at most once per 15 minutes.
- **What changed.** Log sampling (perf every 10 s, network every 60 s) now runs only while a PC session is open or Relay is on screen; otherwise its loop waits for that state, with no timer. The status redraw ("last seen 5 min ago" in the notification, tile and widget) runs only while the screen is on, and turning the screen on redraws at once. Nobody sees the notification with the screen off, so that loop no longer wakes the phone.
- **Kept on purpose.** The relative wording stays, matching Home and the PC app; an absolute "last seen at 14:05" was considered and dropped for that reason.
- **What may still wake it.** The recovery job (JobScheduler, 15-minute minimum period, batched by Doze) and packets arriving on the node's sockets. Not measured on a device yet: the device checklist checks it with `adb shell dumpsys batterystats` after an hour idle.

## Protocol compatibility and capabilities (2026-11-05, v2 M3.1-M3.2)

- **No breaking bump.** The wire version stays 0. `Hello` gains optional `min_compatible` and `app_version`; two devices talk when each side's minimum is at most the other's version. A 0.2.0 peer sends neither (read as 0 and empty) and requires an exact match, which holds while every build is version 0.
- **Naming the device to update.** The 426 close carries `update:you` or `update:me`; a plain close (0.2.0) reads as "update both".
- **Capabilities.** One name per wire feature, advertised only by the build that implements it. `discovery2` and `resume` are reserved; the other M7-M9 features need none.

## Foundations: ids, settings versions, folder names (2026-11-05, v2 M3.3-M3.7)

- **Peer ids.** Phone events carry the PC's id next to its name; the three ask paths (control, screen, camera) use the id, never the name, so two PCs with the same name are told apart. The PC keeps name-keyed events until M8 (per-device permissions).
- **Settings versions.** PC `settings.txt` starts with `version=1`; Android prefs carry `prefs_version`. Migrations are pure functions that run once at start and are saved back. Newer files are left alone; unknown keys are kept. A fresh install is stamped without other writes. PC per-device facts live in `devices.txt`, not in `peers.txt` (an older Relay drops lines with extra columns there).
- **Folder names.** New PC installs use `%LOCALAPPDATA%\Relay\tray` and `Downloads\Relay`; existing installs keep `PhoneBridge\tray` and have `Downloads\Phone Bridge` pinned. Never renamed: the Run value, the toast app id, the pipe, the camera folder, notification channel ids, the Android application id and Keystore alias.
- **Unreadable identity.** An identity that can't be unsealed is a typed error, not a retry loop; the user is offered "Reset and pair again", which removes only the identity and the pairings.
- **A missing settings file is written on the first start (M3.R).** Relay 0.2.0 wrote `settings.txt` only when something changed, so a missing file can't tell a new install from an old one. A new install (no identity, no pairings, no legacy `identity.key`) saves today's defaults at once; an old one is migrated from version 0, which pins `Downloads\Phone Bridge`. The lost-identity flag on Android is cleared by any start that opens the identity, and opening the app or the share sheet doesn't start a service whose identity is lost.

## Silence and the volume keys in every app (2026-11-05)

- **The rule, for any app (`SoundRoute`).** The stream is muted from the start of a share, so a new sound finds the phone silent. When sound is captured, it unmutes at volume 1 for up to 360 ms and looks at a `Visualizer` on session 0, which sits ahead of the silencing effect. Sound on that meter is covered by the effect, so the mute comes off and the real volume bar is used, at the PC's level, with the cast session off. Sound captured with the meter at its floor bypasses the effect, so the mute stays and the keys go to the cast session. `AudioPlaybackCallback` reports each player starting or stopping, and a new player or the end of the covered ones mutes and walks down at once. There is no list of apps.
- **Why not one bar everywhere.** Two Android rules, both read in the source and seen on a realme RMX3561: a raise of the real volume unmutes, so with sound on an output the effect misses it is heard. And `MediaSessionService` hands the keys to the local stream while an app that called `setVolumeControlStream(STREAM_MUSIC)` is playing, whichever session is first: YouTube and Instagram do this, Clash Royale does not. The only way around both is an accessibility key filter, which banking apps refuse.
- **A session needs a callback.** `MediaSession` drops volume presses when no callback is set, and the system's bar goes back after 1 s (`OPTIMISTIC_VOLUME_TIMEOUT`).
- **Both bars at once flip.** With the cast session active while an app kept the keys, the panel changed icon on every press in Instagram.
- **The meter is made fresh for each look.** It reports the peak of a long window, so the last sound could pass for the next one. Muting zeroes the mix ahead of it too, which is why the look needs the unmute.
- **Measured looks.** First tried at the moment of a press, with the bar on screen: the user saw flickers on switching from Clash to YouTube. That is why it happens when the sound starts.
- **Calls.** `AudioPlaybackCaptureConfiguration` allows only media, game and unknown usages, so call audio can't be captured without system privileges. Calls keep their own stream and aren't muted.
- **Earphones.** Android keeps a volume per output. A saved volume is only walked back on the output it came from: the speaker's level walked onto earphones could be painfully loud.

## The PC's audio queue (2026-11-05)

- **Crackle and skips: thread priority.** cpal 0.15.3 passes a thread ID to `SetThreadPriority`, which wants a handle, and ignores the error. In a live share: at priority 0 the device ran dry 1 to 4 times per 10 s, callbacks were up to 56 ms apart, and the queue crept up until it skipped 85 ms every 20 to 30 s. Raised in the first callback: nothing ran dry, callbacks stayed within 18 ms, no skips outside Wi-Fi stalls. MMCSS "Pro Audio", then `THREAD_PRIORITY_TIME_CRITICAL`.
- **Lag: queue length.** The queue grew 40 ms at each dropout and shrank 20 ms per clean minute, so on real Wi-Fi it sat at 180 to 260 ms, plus up to 80 ms before skipping. `PlayoutTarget` now takes the third-worst of the longest gaps in 2 s windows over 30 s, plus 40 ms, between 60 and 160 ms. Repeated gaps get covered, a rare stall does not raise the delay on everything, and it comes back down within 30 s.
- **What didn't work, measured.** Sizing from packet age left out the frame length: a queue of 65 ms ran dry 4 times per 10 s. A speed correction of 2% was heard as the sound being slowed down, so it is now 0.5%. A margin of 20 ms gave a dropout every 24 s from ordinary gaps of 60 to 90 ms.
- **Wi-Fi media streaming mode (`WlanSetInterface`) was tried and removed.** Switched on and off by the minute during one share: 85.9% against 85.6% of packets within 20 ms, and no fewer stalls.

## Audio leftovers (2026-11-06, v2 Part A)

- **VPN hint.** Home shows "A VPN is on" only with a VPN active, a PC paired and the last attempt to reach it failed. A VPN alone is not a problem (a split-tunnel one, like Proton's, lets the LAN through), and the card has no dismiss.
- **Reconnecting.** A running phone-audio share with no frame for `STALL_MS` (1.5 s) says "Reconnecting…" on the PC flyout. The sender never skips silent frames (no DTX in the code), so a paused player should not trigger it; of 2,657 logged 10-second windows, the 12 with a gap of 1.5 s or more all show underruns (a real stall). Not yet seen on a paused player: that retest is the owner's. The phone's matching text rarely shows: audio rides its own session, so when it drops the share ends with it.
- **Accept order.** The PC hears "yes" once the phone's speaker is silenced, or after 1.5 s, and the wait is no longer on the main thread (`acceptOrder`).
- **Log fallback folder** follows the install's folder name, like the data folder (`Relay\logs` new, `PhoneBridge\logs` old).
- **Release exe.** The portable folder in use runs the `--release` build. Its camera DLL bytes are embedded and differ from the previous spike build's, so the first camera start can show one UAC prompt.

## Design tokens (2026-11-06, v2 M4)
- **One source.** `design/tokens.toml` is read by `tools/gen-tokens`, which writes `crates/pb-tray/src/tokens.rs`, `android/app/src/main/java/dev/phonebridge/app/ui/Tokens.kt` and `design/floot/base.css`. Generated files are never edited by hand, and drift tests (`checked_in_rust_tokens_match_the_toml`, `checked_in_kotlin_tokens_match_the_toml`, `checked_in_css_matches_the_toml`) fail on any difference. Two rules are written by hand beside their callers and tested there, because they are logic, not data: `resolve` and `spring_or_cut` (`crates/pb-tray/src/tokens_rules.rs`, `ui/TokenRules.kt`).
- **Materials** (tint / opacity / blur): Native light `#F3F3F3` / 0.92 / 30; Native dark `#202020` / 0.82 / 30; Relay light `#F8F9FF` / 0.92 / 24; Relay dark `#14172A` / 0.88 / 24. They are near-opaque on purpose: RESEARCH.md ("Glass is a scrim problem: opacity first, scrim second, darker text last") shows that Windows' own Acrylic keeps text legible with a luminosity layer at 0.85 to 0.96, and that the derived minimum alphas for 4.5:1 sit well below these values. Values that failed a test were changed in the TOML, never the threshold.
- **Floors, enforced by tests.** Text-bearing glass has alpha of 0.70 or more and icon-only glass 0.50 or more (PRINCIPLES). `glass_text_is_aa_over_white_and_black`, `icons_are_3_to_1` and `every_mode_meets_aa` check text 4.5:1 and icons 3:1 over a pure white and a pure black backdrop in all four materials, and over the opaque tint for every reduced mode. `control_boundaries_are_3_to_1_on_opaque_surfaces` holds `outline_strong` at 3:1 (WCAG 1.4.11); `outline` stays a divider.
- **Visible change on Android.** In the light theme the amber value moves from `#C9820F` to `#955700`, so the pairing-code chip tint and the amber icon badges (clipboard, audio, camera, timeline) turn darker. It is an improvement, pinned by `amber_icon_badges_are_3_to_1` (the amber icon on its own tint goes from 2.47:1 to 4.29:1) and `pairing_code_digits_are_aa_on_the_amber_tint` (the digits are drawn in `onSurface`, still above 12:1).
- **Status text and glass.** Status words, badges and amber text sit on opaque layers only (PRINCIPLES); the M5.3 test `status_text_sits_on_an_opaque_layer` pins it for the flyout, which draws a warning line straight on its backdrop today (`flyout.rs:347-348`).
- **One accent.** `primary` (cobalt) is the only accent. The amber token (`accent`, renamed `attention` in M5.1) is a status role that always comes with an icon and words; since M4.9 its light value is `#955700` (`attention_is_readable_as_text_and_icon`), and text on a status tint uses `success_text`, `warning_text` and `danger_text` (`status_text_is_aa_on_its_tint`).
- **Reduced modes.** Reduced transparency, lite and high contrast resolve to an opaque material with no blur, grain or highlight. In CSS the OS preference (`prefers-reduced-transparency`, `forced-colors`) makes glass solid unless a page explicitly sets `data-force-glass="true"` (the Floot prototype does that for `?transparency=full`, so captures can show glass on a PC whose Windows transparency effects are off); `data-reduce-transparency="true"` always wins (`forced_glass_opts_out_of_the_os_preference`).
- **Motion.** Springs (standard 700/0.9, emphasised 380/0.8, quick 1400/0.9, effects 1600/1.0) and Windows easing curves are tokens. Per SPEC M5.2 the PC uses a spring only where motion follows the hand; transitions use Microsoft's curves. The PC keeps Relay's own durations (120/200/300 ms), not Microsoft's published 83/167/250/333 ms (M5.0a, REVIEW R-15).
- **Relay light and the attention colour (M5.0a).** The Relay light tint moves from `#F8F9FF` to `#E6ECFF` so the look is visibly cooler than Native over a white window (`relay_light_glass_is_visibly_cooler_than_native_light`: a summed channel difference of at least 28, was 21; text still passes AA on it). The dark `warning` and `warning_text` move from yellow `#FCE100` to the amber `#E8A93A` (`warning_is_amber_in_both_modes`), so attention is amber in both themes (REVIEW R-4, R-22).
- **Materials and fallbacks (M5.1).** `look.rs` decides what each surface draws on from the look, the surface kind and the system state: Native glass is Mica on the main window and Acrylic on the flyout; Relay glass is DWM Acrylic with a tint wash, a baked 64x64 grain tile and a 1 px top highlight drawn in Direct2D. Anything else, and every surface when transparency effects are off, high contrast is on, or Windows is older than 22H2 (build 22621) or a Remote Desktop session, is Solid. A window that can be glass is always created on the DirectComposition path, so turning transparency on or off only changes the DWM backdrop and the fill; it never rebuilds the window (the creation style cannot change later and a rebuild would lose the page, scroll position and focus). A backdrop DWM refuses is a downgrade to Solid. Text antialiasing is grayscale over a translucent backdrop and ClearType on opaque layers. Menus and dialogs stay Solid until they are created through `make_window` (M5.6). Theme and Look sit on a new Appearance settings page; settings version 2 added `look=`.
- **Screenshots.** The design review used Floot's own screenshot tool in the owner's open tab (JPEG, first screen of each page), not Playwright: headless Edge and Chrome produced no output here and the tool needs no new dependency. Below-the-fold content was judged from the Floot source. The review is `docs/v2/design/REVIEW.md`; its Floot-only leftovers are on the "Floot later" list in PROGRESS.md.

## Windows PC redesign (v2 M5, 2026-11-07)

- **Materials per surface.** The main window is Mica and the flyout is Acrylic under the Native look; under the Relay look the main window and flyout are Relay glass (DWM Acrylic plus a Direct2D tint, a baked 64x64 grain tile and a 1 px top highlight). Dialogs, menus, QR, Transfers, the status pill, the screen bars and the camera window are Solid (the dialog and menu are Solid until they are created through `make_window`, M5.6 follow-up; QR must keep a white quiet zone).
- **Fallbacks.** Solid whenever transparency effects are off, high contrast is on, Windows is older than build 22621, or the session is Remote Desktop. A backdrop DWM refuses is a downgrade to Solid. A glass-capable window is always created on the DirectComposition path, so a toggle never rebuilds a window.
- **Text.** Grayscale antialiasing over a translucent backdrop, ClearType on opaque layers. Glass that carries text has alpha at least 0.70 (icons 0.50).
- **Motion.** Springs only for released drags (`SpringAnim`, none under reduced motion). Transitions keep the token curve and durations. Timers stop when nothing animates; the dialog's question check is a 500 ms timer.
- **Accent.** `primary` is the one accent. The amber token is `attention`, and it always comes with an icon and words. High contrast uses only the system colours; status colours are the text colour there.
- **Narrator.** The flyout publishes its tiles as a UI Automation tree (one toggle per tile). The UIA snapshot is per window.
- **Not done in M5.** The Segoe UI Variable fallback to Segoe UI in the product (only the snapshot font check exists), the offscreen snapshot renderer, the snapshot goldens and CI job (M5.10), the M5.R reviewer pass, and the budget run (M5.11, needs the owner's Relay closed). Tabular numerals on the transfer progress line.
- **Glass checks.** Mica, Acrylic and Relay glass are not verified visually on a PC with transparency effects on; `docs/v2/DEVICE-CHECKS.md` lists the checks.

## Haze for backdrop blur (v2 M6.2, 2026-11-07)

- **Haze 1.6.10** (Apache-2.0) draws the blur for glass panels on API 31 and up, outside lite mode. Below API 31, and in lite mode, a tinted scrim; with reduced transparency or high contrast, an opaque panel.
- **Why 1.6.10:** Haze 1.7.x needs compile API 37 (this project compiles against 36, and the plan raises it only for a stable library that needs it). Haze 2.x is built with Kotlin 2.4, which this project's compiler (2.2.10) cannot read.
- **Standard library pin:** Haze's dependencies pull `kotlin-stdlib` 2.3.x, which the Kotlin 2.2.10 compiler cannot optimise (a compile crash in the IR interpreter). The app pins `kotlin-stdlib` to 2.2.10 in its configurations. Remove the pin when the compiler moves to 2.3.x.
- **Verification:** the Gradle verification trust regex covers `dev.chrisbanes.haze`; the sha256 entries are not pinned yet (open item for M6.R).
- **Frame budget:** `FrameBudget` (more than a quarter of the last 120 frames over 1.5 intervals) trips to lite on its own. Wiring it to the frames on screen is open in M6.2.

## Security shield (v2 M7, 2026-11-07)

- **The gate is an extra step, never the only one.** It guards allow decisions, always-allow and auto-accept, confirming a pairing, unpairing and turning itself off. When it cannot run, the old consent dialog stands (a person is not locked out of their own consent); the settings row says why. A failed or cancelled unlock starts no grace.
- **Grace is 60 s,** reset by locking the screen; a clock that goes backwards asks. Default on in release and qa builds, off in debug builds.
- **PBD2 marks live in `discovery2.txt`,** never as a fourth field of `peers.txt`: Relay 0.2.0 drops a line with an extra column and would lose the pairing. A peer is marked when a session's `Hello` lists `discovery2`. PBD1 stays for unmarked peers and is removed in the next minor release.
- **The PBD2 mac covers a label, the window and every byte before the mac** (a superset of the plan's field list), so what is authenticated cannot drift from what is parsed. A probe's source address is not learned; only an answer's MACed addresses become candidates.
- **Pairing limits are per source IP,** with a global cap per code and a 150 s time box per pairing slot. The old rule "3 wrong tokens from anyone burn the code" is replaced by tests that are strictly more specific.
- **Session quota is 16 per peer** (the plan said 8): normal heavy use (main session, screen, camera, audio each way, control, concurrent batches) can reach nine or ten.
- **Disconnect everything needs no confirmation and no gate:** it only removes access. The PC pauses Relay; the phone turns Relay off until resumed.
- **Not done in M7:** the flyout, tile and notification entry points for Disconnect; the `OfferPermit` call sites in the tray and ffi; the SAS confirm and the allow-list switches on Android are not behind the gate; the Hello and biometric prompts are checked on devices (DEVICE-CHECKS).

## Per-device permissions (v2 M8, 2026-11-07)
- **Supersedes "No master switch" (Phase 9).** Every paired device has its own answer for control, screen, camera and audio: Ask (the default for a new device), Always, or Never. Clipboard sync, notifications and automatic file accept are each device's own switches, off for a new device.
- **Never is not No.** With several PCs, "never ask from this PC" is a different decision from answering No once: Never refuses at once, without a prompt, and the phone shows a short note ("Blocked a screen share from ‹PC› (set to Never)"). It still asks nothing, grants nothing and only removes a prompt, so "the PC can only ask" is unchanged.
- **Always while the phone is locked still asks** (the M1.6 rule). The Always answers are applied through the unlock gate when turned on.
- **Where it lives.** Phone: Always stays in the per-kind allow sets (`allow_control_pcs`, `allow_screen_pcs`, `allow_camera_pcs`, new `allow_audio_pcs`); Never is `<id>/never_<kind>`; the three switches are `<id>/clipboard_sync`, `<id>/notifications`, `<id>/auto_accept`. PC: `perm_*` keys per device in `devices.txt`. An unknown or damaged value reads as Ask or off, never as a grant.
- **The all-PCs switches stay** (Settings, tray menu) as shortcuts that set every paired device, and the global value shows "some device has it on", so the screen shows what will happen. A device set on its own can differ from the shortcut.
- **Migration copies today's switches to every paired device, once.** The globals already applied to all of them; copying to one would silently change the others. An install that never chose keeps what it had (automatic accept and notifications default on); a device paired afterwards starts from fresh defaults, never from a global. The old keys stay for one release so a downgrade to 0.2.x still finds them.
- **Disconnect everything clears every device's Always and automatic accept,** not only the global values. Never answers and plain switches stay.
- **A yes from one device allows no other.** On the PC, the answer to "allow this phone to control the PC" is stored for the asking phone only.
- **Nicknames are local labels,** never sent; a device's own name is that device's business. The default device is where a send goes when none is chosen; with several devices a send asks which one first (the default on top).
- **Not done in M8:** audio-skip, clipboard and notification switches per device on the PC (the per-device fields exist and migrate; the PC still reads the global switch for them); a Timeline entry for a blocked request (a new entry kind changes the file format shared with the PC, so a notification and the app log record it for now); Rename on the PC (no text-input dialog yet).

## PC budget run (2026-11-08, M5.11)
- **Not a valid idle measurement (advisor review): nobody opened the window and a fresh folder likely shows the first-run window, which keeps the GPU device alive.** Numbers kept for the record. **Measured** with `scripts/pc-budget.ps1` on a release `pb-tray` (fresh data folder, owner's Relay stopped for the run, then restarted): idle working set 1.90 MB, idle private 71.11 MB, private with the main-window step 71.14 MB, idle CPU 0.219 s over 60 s. Nobody opened or closed the window during the run (the script's two prompts need a person), so the "window open" figure is not a window measurement.
- **Result: over budget.** Idle private 71 MB against the 15 MB limit and idle CPU 0.22 s against 0.05 s. The idle private figure looks like the GPU device still alive at the sample (a fresh data folder may show the pairing window at start). Not yet explained; it is an open follow-up, not a pass.
- **Ruling:** the budgets stay as written in "Phase 8"; the table is recorded as failing and the cause is investigated before 0.3.0 is called done. Cost if wrong: the idle tray costs more RAM than promised.

## Resumable transfers, the send queue and history (v2 M9, 2026-11-08)
- **The sender names a transfer, the receiver keeps the bytes.** `Offer.resume_token` is 16 random bytes the sender picks once per file (never derived from the offer id, which is new each attempt) and keeps across retries and restarts. The receiver keeps a partial under a name made from the peer and the token, never from the file name, and answers `resume_from = n`; the sender hashes its first n bytes, the receiver its kept n, and a mismatch restarts at zero. `TransferDone.sha256` still covers the whole file. All new fields are optional and `PROTOCOL_VERSION` stays 0, so Relay 0.2.0 talks to 0.3.0 and just never resumes. Cost if wrong: a rare resume that restarts at zero.
- **Partials are bounded.** 24 hours, 2 GiB per peer, 8 GiB in all, never below 256 MiB free; oldest first. They live in the data folder, not the save folder. Cost if wrong: a very large interrupted file older than a day starts over.
- **The receiver keeps a partial only when the link broke** (`Transport`, `Closed` or `Timeout`). A cancel, a refusal or a failed checksum deletes it.
- **Retry only after bytes flowed.** A sender's wait for the receiver's yes also ends in `Timeout`, so a dismissed or unanswered prompt would look like a broken link. `should_retry` therefore requires that bytes were sent, at most 3 times, never after a cancel (`only_a_broken_transfer_is_retried_never_an_unanswered_prompt`). Cost if wrong: a link that breaks before the first byte is not retried by itself.
- **The queue is local.** One queue per device, at most 64 unfinished files, pause stops the next file and never the stream already sending, cancel removes the item and cancels the stream if it was the one sending. No message crosses the wire. `queue.txt` stores unfinished items with their tokens; after a restart every queue is paused and a moved file shows "needs the file". The PC has one Pause/Resume queue button on the Transfers page for all devices. Waiting files have a Remove button on the PC page; the file already sending is cancelled with its own Cancel.
- **The phone resumes through a callback.** A resumed send reads the start of the file and then the rest, so the engine asks Kotlin for a fresh file descriptor each time (`FileOpener`); Kotlin hands out the one it opened and then copies of a kept descriptor rewound to the start. The notification's Retry reuses the same token. The phone has no automatic retry yet. Cost if wrong: a broken phone-to-PC send waits for the person to tap Retry.
- **Clipboard history is opt-in and sealed by the system.** Off by default, 7 days, 200 clips, 64 KiB each; sensitive clips and one-time codes (4 to 10 digits, optional spaces or dashes) are never stored. PC: Windows DPAPI for the current user with entropy of its own, which also authenticates the record, so no nonce of ours. Phone: AES-256-GCM, the intended Keystore key (alias `clip_history_v1`, non-exportable, no user-authentication requirement so it works while the phone is locked; not written yet, the tests use a software key), the IV taken from the cipher because the Keystore forces randomised encryption, and the version and created time as authenticated data. A lost key reads as empty and deletes the file. Cost if wrong: history is gone after a Keystore reset.
- **Drag and drop goes through the same device choice as every send.** On the PC a drop on the flyout, the main window or the screen window asks "Send to" when several phones are paired. On Android a window 600 dp or wider takes drops; only content from other apps is accepted. The phone sends a drop to the default PC without asking yet.

## Staying current and trusted (v2 M10, 2026-11-08)
- **Update check = verify, then open the release page.** No PC self-swap and no in-app APK install (the owner's ruling): the signature protects the person from a fake "update available", and the install stays a normal download.
- **The manifest.** `release-manifest.json` with a detached Ed25519 signature over the exact bytes; at most 64 KiB; `date` is integer UTC seconds (not a date string, so no date parser sits on the trust boundary); artifact links must start with `https://github.com/Lin-2352/phone-bridge/releases/`; at most 32 artifacts. A date older than the last accepted, a version older than the last seen, or a date more than 7 days ahead is refused; a version not newer than the installed one is "up to date".
- **Key rotation.** A manifest may carry `next_key` and `next_key_sig`, a signature by the current key over `"relay-next-key:"` plus the key in hex. Trust holds at most the current key and one pending successor; a refused manifest changes nothing. Cost if wrong: a lost signing key cannot be replaced by software; a new app release would have to ship a new key.
- **The owner's key is not embedded.** The verifier and `sign-manifest` are built; the real public key comes from the owner's offline key (a stop-and-ask item, "OWNER KEY" in PROGRESS). Until then an empty trust list refuses every manifest as `UnknownKey`.
- **Signing and publishing.** `scripts/sign-windows.ps1` reads `WIN_SIGN_PFX_BASE64`, `WIN_SIGN_PFX_PASSWORD` and optionally `WIN_SIGN_TIMESTAMP_URL` by name, is skipped with one line when absent and never runs for pull requests. The package publisher is two jobs: download with read access to contents, push with write access to packages.
- **The installer.** Inno Setup, per user by default. A firewall rule needs an administrator, so it is added only for an administrator install (Private profile, UDP, the one program, inbound); a per-user install relies on Windows' own first-run prompt. Cost if wrong: a per-user install on a machine that blocks the prompt cannot receive until the person allows it.
- **ARM64** is built in CI and not run (no ARM runner); the camera driver is x64 only. **SBOM**: CycloneDX for the Rust crates in CI; the Android SBOM, the NOTICE file and the in-app notices screens are not done.
- **A VPN on the phone and the local network (found on the phone, 2026-11-09).** With a VPN up, Android sent Relay's traffic for the PC into the tunnel and pairing failed with a bare "Failed". Ruling: while a VPN network is up, the phone binds Relay's own process to its Wi-Fi (else Ethernet) network (`LanRoute`, `bindProcessToNetwork`), and unbinds when no VPN is up, so nothing changes for people without one. This is an app-level route: Relay never changes or disables the VPN or any setting. Scope: the binding is process-wide, not per address; this is equivalent today because Relay's relay servers are off and all its traffic is local, and a future internet feature (the update check, M10.2) must use the default network explicitly so it keeps following the VPN. Android honours the request only when the VPN app allows bypass; many do not, so the answer is logged (`LANROUTE bound=`) and the pairing error now names the VPN (`joinErrorText(..., vpnOn)`) unless the code itself was wrong or expired. Cost if wrong: a VPN that forbids bypass still blocks the PC, and the person must allow local network access in the VPN app. Sockets made before a binding keep their old route, so the binding is made before the node starts and again, with a rediscover, whenever a network or the VPN comes or goes.

## Memory budgets reset (2026-11-09, owner ruling)

- **PC:** at most 80 MB private (committed, `PrivateMemorySize64`) in every state, window open or closed; idle CPU at most 0.05 s per 60 s. Replaces 40 MB working set / 15 MB private / 70 MB window.
- **Phone:** PSS at most 80 MB. Replaces 60 MB.
- **Why:** after the device release the PC tray holds 71 MB committed. 51 MB is one region the NVIDIA user-mode driver allocates within a second of start and keeps while the driver stays loaded (resident working set is 2.2 MB). On 11-02 the figure was 13 MB (the notes say Intel UHD drove the panel; which adapter was used now is not confirmed). With the device alive (first 35 s after start, or a window open) the process is about 120 MB, over the 80 MB budget; only the idle-after-release state passes.
- **Cost if wrong:** a lower-memory machine gets no warning until 80 MB. Choosing the low-power adapter would likely return to about 13 MB; not done, because the owner accepted the figure.
- `scripts/pc-budget.ps1` now measures idle after the device release (55 s), not across it.

## Owner rulings (2026-11-09): hard memory ceiling, no update key for now

- **Memory:** 80 MB is a hard maximum in every scenario (PC private/committed bytes with a window open or closed, at start-up, and phone PSS with the UI open or in the background). Not an average or an idle figure. Anything above it is a bug to fix by optimisation, not a number to restate. Research on how to guarantee it is in progress; until a fix lands the PC reads about 120 MB with the graphics device alive and the phone reads 79.9 MB in the background.
- **Update key:** not needed for now. The app has no accounts and stores or shares no data, so the owner will not hold a release-signing key for the update manifest. The verifier and `sign-manifest` stay built; nothing embeds a key. Cost if wrong: an update check can then only point at the release page and cannot prove who published it; do not add any self-update.
- **Signing certificates:** use a free open-source-friendly route for both Windows and Android; research under way (SignPath Foundation and similar for Windows; self-generated keystore, F-Droid and IzzyOnDroid for Android).

## Owner ruling (2026-11-09): PC memory peak

- 80 MB is the normal ceiling (idle and a window open once the device settles). A short peak up to about 120 MB while the graphics device starts (first ~35 s after launch, or a window opening) is accepted; it must not be sustained. No renderer change that slows the UI (WARP rejected). The unexplained 51 MB zero region stays an optimisation to find, not a blocker.
- Lost signing keys: see `docs/KEY-RECOVERY.md`.

## Update check without a key (2026-11-09)

- **What it is:** a button on each app (PC Settings, General page; phone Settings, Updates). It reads GitHub's release list once, when pressed, and shows either "no newer release", a link to the newest one, or "Couldn't check: why". No background check, no daily timer, nothing downloaded or installed.
- **Ruling: pre-releases count.** `/releases/latest` skips pre-releases and every release so far (0.2.0) is one, so the list is read and the highest `major.minor.patch` above the installed version is offered. Drafts, tags like `v0.8.0-rc1` and links outside this project's releases are ignored.
- **Ruling: nothing is verified.** The owner holds no update key, so the screen never says "verified" or "safe"; it says the release is listed on GitHub and to compare on the page. Cost if wrong: a hijacked repository could point people at a bad release; there is no self-update, so the person still chooses what to install.
- **Ruling: 512 KiB response limit** (the plan's 64 KiB was for the signed manifest; release notes make the list larger). A longer answer is a failure, not "up to date".
- **A failure is never "up to date"** (rate limit, offline, HTML page, bad JSON, oversize), on both apps.
- **PC:** WinHTTP (system proxy and certificate store, ten-second limits); no new dependency and nothing loaded until the button is pressed. Rows on the General page can now be 10, not 9.
- **Phone:** `Network.openConnection` on a named network: the VPN when one is up, else Wi-Fi, Ethernet, then mobile data. Relay binds the process to Wi-Fi while a VPN is up for local traffic; a default request would skip the VPN and show the person's address to GitHub.
- The `pb-update` signed-manifest verifier and `sign-manifest` stay, unused and with an empty trust list.

## Owner rulings: Transfers controls, toasts, last seen (2026-11-10)

- **Cancel is per file.** A person's Cancel (and the other device's cancel) stops only that file; the rest of a batch goes on. Relay pausing or quitting still stops the whole batch. Cost if wrong: someone who wanted "cancel everything" presses Cancel all instead.
- **Cancel all** is its own button on the Transfers page: it clears what waits (held files too) and stops what sends.
- **Pause stops what is sending.** Pause queue stops the file in flight and keeps its place and resume token; files arriving from a phone are not touched (the queue only governs what the PC sends). Pause on one row (or ticked rows) holds just those files; Resume queue also releases files held one by one. Until resume works end to end a stopped file starts again from the beginning.
- **Rows have checkboxes** with a bar (Pause, Resume, Cancel, Remove) over the ticked rows; Select all / Clear selection in the header; Remove takes a finished, failed or cancelled transfer off the list and a waiting file out of the queue.
- **Toasts remove themselves** from the notification centre: Relay's own after 2 minutes, mirrored phone notifications after 10, calls after 1. Expandable (longer) toast text is not done: Windows gives a toast no expand control, to be decided with the owner.
- **Last seen lives on the phone's Home card only** ("Connected", or "Last seen X ago" while the PC is away). The tile, widget, notification status line and the Devices list say connected or not connected.
- **Not decided yet:** announcing "3 files" on receive; Windows Hello switches per area (design first, security-sensitive).

## Windows Hello per area (2026-11-10)
Owner asked for switches. With the gate on, three areas have their own switch (default on): saving incoming files, audio sharing, phone control. Turning one off needs an unlock. Pairing, unpair, gate off and the "always allow" toggles always ask while the gate is on. Camera and screen are not gated on the PC (the phone asks). The 60 s grace is unchanged.
