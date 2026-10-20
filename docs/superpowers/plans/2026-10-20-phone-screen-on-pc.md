# Phone screen on the PC: implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** a window on the PC shows the phone's screen live (30 fps, low lag, no artefacts, no shimmer) and lets the user drive the phone from it with mouse and keyboard.

**Architecture:**
- **Phone:** MediaProjection renders into a hardware encoder (HEVC Main10, else HEVC Main, else H.264). Each encoded unit goes through Rust (`pb-ffi`) to a new shared crate, `pb-video`, which frames the records and runs the pacer. `pb-transport` carries everything on one "screen session":
  - the control stream (consent, feedback, window state);
  - one unidirectional stream phone → PC for video records;
  - one unidirectional stream PC → phone for input records.
- **PC:** the tray decodes with Media Foundation on the GPU (one D3D11 device on the window's adapter) and presents through a waitable flip swap chain.

**Tech Stack:**
- Rust: prost, iroh/pb-transport, uniffi, `windows` 0.62 (Media Foundation, D3D11, DXGI).
- Kotlin: MediaProjection, MediaCodec, VirtualDisplay, AccessibilityService.

**Spec:** `docs/superpowers/specs/2026-10-20-phone-screen-on-pc-design.md`

## Decisions this plan makes where the spec left room
Each one goes into `docs/DECISIONS.md` (Task 14).

1. **One consent covers video and input.** An accepted `ScreenStart` lets the window's mouse and keyboard drive the phone for that session. The input travels on the session's own input stream, not through a separate `InputStart`.
2. **Lag is measured on in-flight frames.** Lag = now − send time of the oldest frame the PC has not yet confirmed (0 when nothing is in flight). The spec's "newest sent pts − newest confirmed pts" would count the gap after an idle screen as lag on the first new frame. This keeps its intent: an idle screen is never lag.
3. **Absolute input record kinds** on the existing 8-byte input stream:
   - `Touch { phase: Hover|Down|Move|Up, x, y }` (kind 5);
   - `Pinch { out, x, y }` (kind 6).
   - x and y are normalised to 0..=65535.
   - The wheel in the window is a `Touch{Hover}` to the cursor's spot, then the existing `Wheel` event (the phone already streams it as a finger drag).
   - Back / Home / Recents reuse `MouseButton` 1 / 2 / 4.
4. **Reconnect without a new capture prompt.** When the link drops, the phone keeps the projection alive (encoder paused) for 30 s. A new `ScreenStart` from the same PC within that time resumes without asking, so a Wi-Fi blip does not need Android's capture prompt again.
5. **Secure-screen note:** once a second the PC reads back a 16×16 downscale of the current frame. It shows "Hidden by the app (secure screen)" while the mean brightness is below 3/255.
6. **"Quiet while the phone-screen window is focused"** (notifications) becomes live: the tray passes `mirroring_focused` from this window.

## Global Constraints
- Never add AI attribution to commits; commit as the local git identity.
- **Gates:**
  - `cargo fmt --all -- --check`;
  - `cargo clippy --workspace --all-targets -- -D warnings` on Windows and Linux CI (Win32 code `#[cfg(windows)]`, including `mod` lines);
  - `cargo test --workspace`;
  - `pwsh scripts/android-build.ps1` plus `gradlew :app:lintRelease`.
- `unsafe` only in modules with `#![allow(unsafe_code)]`, one `// SAFETY:` comment per block.
- **Values from the spec:**
  - 30 fps;
  - long side capped at 1920 by default ("Sharp" = native);
  - sizes rounded to the encoder's alignment (at least 16; 864×1920 on the 1080×2412 realme);
  - record header `u32 len | i64 pts_us | u8 flags | payload`, magic `PBV1`; a record over 4 MiB ends the session;
  - config record payload `u16 rotation | u16 width | u16 height | codec headers`, always followed by an IDR;
  - feedback every 100 ms;
  - drop when lag > 150 ms or the send queue (1 MiB) is full: drop until the next keyframe, request one, bitrate × 0.7 with a floor of 1.5 Mbit/s;
  - recovery: after 1 s without drops and the queue under 256 KiB, +10 %/s up to the target;
  - VBR targets: H.265 12 Mbit/s (peak 20), H.264 16 Mbit/s (peak 25);
  - QP ceiling via `KEY_VIDEO_QP_MAX`;
  - no periodic keyframes; `KEY_REPEAT_PREVIOUS_FRAME_AFTER` ≈ 100 ms, at most 3 repeats forwarded;
  - BT.709 limited range, SDR.
- **Budgets on the realme (10-minute mirror):**
  - Phone Bridge PSS < 60 MB;
  - swap < 200 pages/s average;
  - kswapd < 10 %;
  - kernel CPU < 15 %;
  - free memory ≥ 1 GB.
  - Step down to 720-class, 15 fps on `onTrimMemory` or < 1.2 GB free, or thermal status "severe".
- **Device rules:**
  - always `adb -s 79KRFEJVPVOBQCHQ`;
  - never `KEYCODE_SLEEP`, `settings put`, `svc`, `adb kill-server`, `adb usb|tcpip|root|reboot`;
  - Android Studio closed;
  - light checks only;
  - announce every permission or capture prompt before it appears;
  - never capture the PC's full screen (window captures only).

## Review Focus
1. **Rotation mid-stream:** the phone turns while frames are in flight. The window must flip its aspect with no stretched frame, and touches must land where clicked. Pinned by `touch_mapping_follows_rotation` (Task 12) and `config_record_round_trips_and_resets_size` (Task 1).
2. **Wi-Fi blip of a few seconds:** the last frame stays dimmed with "Reconnecting…", then the session resumes without a new capture prompt. Pinned by `screen_session_resumes_within_grace` (Task 4), plus a device test.
3. **Idle screen for minutes, then a scroll:** never a drop or a bitrate cut on the first new frame. Pinned by `idle_gap_is_not_lag` (Task 1).
4. **Clicks in the letterbox bars, or outside the video:** nothing is sent to the phone. Pinned by `clicks_outside_the_video_send_nothing` (Task 12).
5. **A PC without an HEVC decoder,** or a decoder that fails mid-session: the next codec is asked for automatically, with no endless retry loop. Pinned by `codec_fallback_stops_after_last_codec` (Task 1) and `decoder_failure_asks_next_codec` (Task 13).

---

### Task 1: `pb-video` crate: records, pacer, sizes, codec choice

**Files:**
- Create: `crates/pb-video/Cargo.toml`, `crates/pb-video/src/lib.rs`, `src/record.rs`, `src/pacer.rs`, `src/size.rs`, `src/codec.rs`
- Modify: root `Cargo.toml` (members)

**Interfaces (produces):**
- `record.rs`:
  - `pub const MAGIC: &[u8; 4] = b"PBV1";`
  - `pub const MAX_RECORD: usize = 4 * 1024 * 1024;`
  - `pub const FLAG_KEY: u8 = 1; pub const FLAG_CONFIG: u8 = 2; pub const FLAG_REPEAT: u8 = 4;`
  - `pub struct Record { pub pts_us: i64, pub flags: u8, pub payload: Vec<u8> }`
  - `pub fn encode(r: &Record, out: &mut Vec<u8>)`
  - `pub struct Decoder` with `fn new() -> Self` and `fn push(&mut self, bytes: &[u8]) -> Result<Vec<Record>, RecordError>`. It checks the magic once at the start; `RecordError::{BadMagic, TooLarge(usize)}`.
  - `pub struct Config { pub rotation: u16, pub width: u16, pub height: u16, pub headers: Vec<u8> }`, with `fn encode(&self) -> Vec<u8>` and `fn decode(p: &[u8]) -> Option<Config>`. Rotation must be one of 0/90/180/270.
- `pacer.rs`:
  - `pub struct Pacer` with:
    - `fn new(target_bps: u32) -> Self`
    - `fn on_frame(&mut self, now: Instant, pts_us: i64, key: bool, queued_bytes: usize) -> Decision`
    - `fn on_feedback(&mut self, now: Instant, presented_pts_us: i64)`
    - `fn bitrate(&self) -> u32`
  - `pub struct Decision { pub send: bool, pub request_key: bool, pub new_bitrate: Option<u32> }`
  - Constants: `MAX_LAG = 150 ms`, `QUEUE_CAP = 1 MiB`, `RECOVER_QUEUE = 256 KiB`, `FLOOR_BPS = 1_500_000`, back-off `0.7`, recovery `+10 %/s after 1 s clean`.
  - At most 3 consecutive `FLAG_REPEAT` frames are sent; later ones are dropped silently, without counting as drops.
- `size.rs`: `pub fn fit_size(native: (u32, u32), max_long: u32, align: (u32, u32)) -> (u32, u32)`. It scales so the long side ≤ `max_long`, then rounds each side to the nearest multiple of its alignment (ties up), with alignment at least 16.
- `codec.rs`:
  - `#[repr(u8)] pub enum Codec { H265Main10 = 1, H265 = 2, H264 = 3 }`
  - `pub fn choose(pc: &[Codec], phone: &[Codec]) -> Option<Codec>` (the first in the PC's order that the phone has)
  - `pub fn next_after(failed: Codec, pc: &[Codec]) -> Option<Codec>`

- [ ] **Step 1: Write the tests** (one module per file):
  - `records_round_trip_across_split_reads`: encode 3 records with 1-byte pushes → the same 3 come out.
  - `bad_magic_and_oversized_records_are_errors`: a 5 MiB declared length → `TooLarge`.
  - `config_record_round_trips_and_resets_size`: `Config{rotation: 90, width: 1920, height: 864, headers: vec![1,2,3]}` round-trips; rotation 45 → `None`.
  - `idle_gap_is_not_lag`: frame at t0 confirmed at t0+30 ms; next frame at t0+5 s → `send && !request_key && new_bitrate.is_none()`.
  - `lag_over_150ms_drops_until_key_and_backs_off`: three unconfirmed frames over 200 ms → `send == false`, `request_key == true`, `new_bitrate == Some(8_400_000)` from 12 M; the next non-key frames are not sent; the next key frame is sent.
  - `full_queue_drops`: `queued_bytes = 1 MiB` → drop, request key.
  - `bitrate_has_a_floor`: repeated back-offs from 2 M never go under 1.5 M.
  - `recovery_is_ten_percent_per_second_after_a_clean_second`: after a drop to 8.4 M, 1 s clean with the queue under 256 KiB → 9.24 M, then +10 %/s, capped at 12 M; no recovery while the queue is ≥ 256 KiB.
  - `only_three_repeats_are_sent`: 5 `FLAG_REPEAT` frames → `send` is true for the first 3 only; no back-off.
  - `fit_size_realme_default_and_sharp`: `fit_size((1080,2412),1920,(16,16)) == (864,1920)`; `fit_size((1080,2412),2412,(16,16)) == (1088,2416)`; a landscape input keeps orientation.
  - `codec_choice_follows_pc_order`: `choose(&[H265Main10,H265,H264], &[H265,H264]) == Some(H265)`.
  - `codec_fallback_stops_after_last_codec`: `next_after(H264, &[H265Main10,H265,H264]) == None`.
- [ ] **Step 2: Run** `cargo test -p pb-video`. Expected: FAIL to compile.
- [ ] **Step 3: Implement** the four modules (std only; `thiserror`-free like the other crates).
- [ ] **Step 4: Run** `cargo test -p pb-video` and `cargo clippy -p pb-video --all-targets -- -D warnings`. Expected: PASS.
- [ ] **Step 5: Commit** `video: shared phone→PC video module (records, pacer, sizes, codec choice)`.

### Task 2: Protocol: screen messages and absolute input records

**Files:**
- Modify: `crates/pb-protocol/src/control.rs` (tags 17–20), `crates/pb-protocol/src/lib.rs` (`InputEvent` kinds 5–6), `docs/WIRE_PROTOCOL.md`

**Interfaces (produces):**
- `ControlMsg::ScreenStart(ScreenStart)` = 17, PC → phone:
  - `id: u64`=1;
  - `max_long_side: u32`=2;
  - `fps: u32`=3;
  - `codecs: Vec<u32>`=4 (`Codec` values, in the PC's order);
  - `dim: bool`=5;
  - `sharp: bool`=6.
- `ControlMsg::ScreenAccept(ScreenAccept)` = 18, phone → PC: `id`=1, `codec: u32`=2, `width: u32`=3, `height: u32`=4, `rotation: u32`=5. Declining uses the existing `OfferReply{accept: false}`.
- `ControlMsg::ScreenFeedback(ScreenFeedback)` = 19, PC → phone: `presented_pts_us: i64`=1.
- `ControlMsg::ScreenState(ScreenState)` = 20:
  - both directions: `active: bool`=1 (PC → phone: the window is visible); `dim: bool`=2 (PC → phone);
  - phone → PC: `notice: String`=3 (e.g. "Lower quality to save memory"); `codec_failed: bool`=4 (PC → phone: ask for the next codec).
- `InputEvent::Touch { phase: TouchPhase, x: u16, y: u16 }`: kind 5; flags = phase (0 Hover, 1 Down, 2 Move, 3 Up).
- `InputEvent::Pinch { out: bool, x: u16, y: u16 }`: kind 6; flags bit 0 = out.
- `pub enum TouchPhase { Hover, Down, Move, Up }`.

- [ ] **Step 1: Write the tests:** extend `control_envelope_round_trips_each_variant` with the four messages; add `touch_and_pinch_records_round_trip` (all phases; x = 0 and 65535) and `unknown_touch_phase_is_rejected` (flags 7 → `DatagramError::UnknownKind(5)`).
- [ ] **Step 2: Run** `cargo test -p pb-protocol`. Expected: FAIL to compile.
- [ ] **Step 3: Implement**; document the messages and kinds in `docs/WIRE_PROTOCOL.md` ("Phone screen" section).
- [ ] **Step 4: Run** `cargo test -p pb-protocol`. Expected: PASS.
- [ ] **Step 5: Commit** `protocol: screen session messages and absolute touch/pinch input records`.

### Task 3: Input stream: absolute touches through the sender and receiver

**Files:**
- Modify: `crates/pb-transport/src/input.rs`, `crates/pb-transport/src/tests.rs`

**Interfaces:**
- Consumes: Task 2's `InputEvent::Touch` and `Pinch`.
- Produces:
  - `pub(crate) fn sender_on(out: SendStream) -> InputSender`: the existing pacing task, now reusable without the `InputStart` negotiation, for screen sessions.
  - `pub(crate) async fn read_input(stream: RecvStream, on_event: impl FnMut(InputEvent)) -> Result<(), Error>`: the receiver loop with its limits; `receive_input` uses it.
  - Touch `Hover`/`Move` are merged like mouse moves (latest wins, flushed before any other event) and counted against the move limits. `Down`/`Up`/`Pinch` count as other events.

- [ ] **Step 1: Write the tests in `tests.rs` (`mod input`):**
  - `touch_moves_merge_but_down_and_up_keep_order`: send Down(10,10), 50 × Move, Up → received: Down first, at most 5 Moves (the last one at the final point), Up last.
  - `touch_flood_is_limited_like_mouse_moves`: 5000 Moves within 1 s through a raw stream → the session stays up and at most 2000 are delivered.
- [ ] **Step 2: Run** `cargo test -p pb-transport --lib tests::input`. Expected: FAIL.
- [ ] **Step 3: Implement** the refactor plus touch merging; `open_input_with` keeps its behaviour (all 35+ existing tests stay green).
- [ ] **Step 4: Run** `cargo test -p pb-transport`. Expected: PASS.
- [ ] **Step 5: Commit** `transport: absolute touch records on the input stream; reusable sender and reader`.

### Task 4: Transport: screen sessions

**Files:**
- Create: `crates/pb-transport/src/screen.rs` (`pub mod screen;` in `lib.rs`; `pb-video` added as a dependency)
- Test: `crates/pb-transport/src/tests.rs` (`mod screen`)

**Interfaces:**
- Consumes: Tasks 1–3.
- Produces:
  - **PC side:** `impl Node { pub async fn open_screen(&self, peer: &PeerId, req: ScreenStart, on_asked: impl FnOnce()) -> Result<ScreenView, Error> }`:
    - `Err(Declined)` on `OfferReply{accept: false}`; times out after 40 s (the phone's prompt plus the capture prompt).
    - `pub struct ScreenView { pub accept: ScreenAccept, pub records: mpsc::Receiver<Record>, pub input: InputSender, pub state: mpsc::Receiver<ScreenState>, closer: SessionCloser }`.
    - Methods: `fn feedback(&self, pts_us: i64)`, `fn set_state(&self, active: bool, dim: bool)`, `fn codec_failed(&self)`, `fn close(&self)`.
    - The records channel is bounded (16). When the decoder falls behind, `try_send` drops non-key records and keeps keys and configs.
  - **Phone side:** `pub async fn serve_screen(session: Session, start: ScreenStart, answer: Option<ScreenAccept>) -> Result<ScreenServer, Error>`:
    - `None` declines.
    - `pub struct ScreenServer`, with:
      - `fn send(&self, r: Record) -> SendOutcome` (`Queued(usize) | Full | Closed`, with a byte queue capped at `pacer::QUEUE_CAP`);
      - `fn queued_bytes(&self) -> usize`;
      - `pub events: mpsc::Receiver<ScreenEvent>`, where `ScreenEvent::{Feedback(i64), State(ScreenState), Input(InputEvent), Closed}`;
      - `fn notice(&self, text: &str)`;
      - `fn close(&self)`.
  - **Grace:** `pub const RESUME_GRACE: Duration = Duration::from_secs(30)`. The phone-side resume decision lives in `pb-ffi` (Task 5); transport only exposes `Session::peer`.

- [ ] **Step 1: Write the loopback tests:**
  - `screen_records_arrive_in_order_after_accept`: config, key and 20 frames arrive in order with the magic checked; `accept.width == 864`.
  - `declined_screen_reports_declined`.
  - `either_side_can_stop`: `view.close()` → the phone gets `ScreenEvent::Closed`; `server.close()` → `records` ends.
  - `feedback_and_state_reach_the_phone`: `feedback(33_000)` → `Feedback(33000)`; `set_state(false, true)` → `State{active:false, dim:true}`.
  - `touches_reach_the_phone_through_the_session`: `view.input.send(Touch{Down…})` → `ScreenEvent::Input(Touch…)`.
  - `send_reports_full_at_the_queue_cap`: pushing 1.5 MiB with the PC not reading → `Full` before 1 MiB is exceeded.
  - `screen_session_resumes_within_grace`: close the session from the PC, then open a new one with the same `peer` within 30 s → `serve_screen` works again (the grace bookkeeping itself is tested in Task 5).
- [ ] **Step 2: Run** `cargo test -p pb-transport --lib tests::screen`. Expected: FAIL to compile.
- [ ] **Step 3: Implement:** one task owns the `Session` (control messages both ways), one writes the video uni stream from a bounded byte queue, one reads input with `read_input`.
- [ ] **Step 4: Run** `cargo test -p pb-transport`. Expected: PASS.
- [ ] **Step 5: Commit** `transport: screen sessions (video stream, feedback, state, input on one consent)`.

### Task 5: FFI for the phone

**Files:**
- Modify: `crates/pb-ffi/src/lib.rs`, `crates/pb-ffi/Cargo.toml` (`pb-video`)

**Interfaces:**
- **Event:** `PbEvent::ScreenRequest { id: u64, peer_name: String, max_long_side: u32, sharp: bool, dim: bool, codecs: Vec<u8> }`. If "always allow screen" is set, it is still raised (Kotlin must run the capture prompt anyway). Within the 30 s grace from the same peer, it is not raised; the ongoing capture resumes.
- **`PbNode` methods:**
  - `fn answer_screen(&self, id: u64, accept: Option<PbScreenAccept>) -> Option<Arc<PbScreen>>`
  - `fn set_screen_listener(&self, l: Option<Arc<dyn ScreenListener>>)`
- **Record:** `PbScreenAccept { codec: u8, width: u32, height: u32, rotation: u16 }`.
- **`PbScreen` object:**
  - `fn send_frame(&self, data: Vec<u8>, pts_us: i64, flags: u8) -> PbAdvice`, where `PbAdvice { sent: bool, request_key: bool, new_bitrate: u32 }` (0 = unchanged). The pacer is applied here.
  - `fn notice(&self, text: String)`
  - `fn stop(&self)`
- **`trait ScreenListener` (Kotlin):**
  - `fn on_state(&self, active: bool, dim: bool)`
  - `fn on_codec_failed(&self)`
  - `fn on_touch(&self, phase: u8, x: u16, y: u16)`
  - `fn on_pinch(&self, out: bool, x: u16, y: u16)`
  - `fn on_key(&self, scancode: u16, pressed: bool)`
  - `fn on_char(&self, unit: u16)`
  - `fn on_button(&self, button: u8)`
  - `fn on_wheel(&self, dx: i16, dy: i16)`
  - `fn on_stopped(&self, resumable: bool)` (true while the grace runs)
- **Grace bookkeeping:** `struct Grace { peer: PeerId, until: Instant }`, pure, with `fn admits(&self, peer, now) -> bool`.

- [ ] **Step 1: Write the tests (host):**
  - `grace_admits_same_peer_within_30s` (29 s yes, 31 s no, another peer no);
  - `advice_maps_pacer_decisions` (pure mapping).
- [ ] **Step 2: Run** `cargo test -p pb-ffi`. Expected: FAIL.
- [ ] **Step 3: Implement:** the `pump` arm for `ScreenStart` (busy if a control or screen session runs), `answer_screen`, the event forwarding thread, `PbScreen`.
- [ ] **Step 4: Run** `cargo test -p pb-ffi && pwsh scripts/android-build.ps1`. Expected: PASS and BUILD SUCCESSFUL (bindings contain `answerScreen`).
- [ ] **Step 5: Commit** `ffi: phone screen sessions (request, accept, frames with pacing, listener)`.

### Task 6: Android: encoder capabilities (decides Main10 / VBR / QP)

**Files:**
- Create: `android/app/src/main/java/dev/phonebridge/app/EncoderCaps.kt`
- Modify: `PbService.kt` (log once at start)

**Interfaces (produces):**
- `object EncoderCaps { fun probe(): Caps }`
- `data class Caps(val codecs: List<Int> /* pb Codec values, best first */, val vbr: Boolean, val qpMax: Boolean, val alignW: Int, val alignH: Int, val encoderName: String)`
- **Rules:**
  - `MediaCodecList(REGULAR_CODECS)`, hardware encoders only (`isHardwareAccelerated`).
  - HEVC Main10: `CodecProfileLevel.HEVCProfileMain10` in the profiles, with the `COLOR_FormatSurface` colour format.
  - VBR: `EncoderCapabilities.isBitrateModeSupported(BITRATE_MODE_VBR)`.
  - QP ceiling: API ≥ 31.
  - Alignment: `VideoCapabilities.widthAlignment` / `heightAlignment`, at least 16.

- [ ] **Step 1: Implement** and log `SCREEN encoder caps <name> codecs=<list> vbr=<b> qpMax=<b> align=<w>x<h>` at service start.
- [ ] **Step 2: Build, install and read the log:** `pwsh scripts/android-build.ps1`, `adb -s 79KRFEJVPVOBQCHQ install -r …`, `adb -s 79KRFEJVPVOBQCHQ logcat -d -s PhoneBridge | findstr "encoder caps"`. Expected: one line. Record it in `docs/DECISIONS.md` (Phase 4.1). If Main10 or VBR is missing, the fallbacks in Task 7 apply. No code change is needed, only the note.
- [ ] **Step 3: Commit** `android: probe the hardware encoder (Main10, VBR, QP ceiling, alignment)`.

### Task 7: Android: capture, encode, consent, service type

**Files:**
- Create:
  - `ScreenConsentActivity.kt`: transparent. It asks "Let <PC> see this screen?" with Allow / Always allow / Deny, then calls `MediaProjectionManager.createScreenCaptureIntent()`. While the phone is locked it posts the notification "Unlock to share your screen with <PC>" instead, and waits up to 30 s.
  - `ScreenCapture.kt`: the encoder and virtual display.
- Modify:
  - `PbService.kt`: `startForeground` with `CONNECTED_DEVICE or MEDIA_PROJECTION` while mirroring, then `CONNECTED_DEVICE` only; handles `PbEvent.ScreenRequest`.
  - `AndroidManifest.xml`: `FOREGROUND_SERVICE_MEDIA_PROJECTION`; the service type `connectedDevice|mediaProjection`; the activity.
  - `Prefs.kt`: `allowScreen: Boolean = false`, `screenSharp: Boolean = false`.

**Interfaces:**
- Consumes: Task 5's `answerScreen`, `PbScreen.sendFrame`, `ScreenListener`; Task 6's `EncoderCaps`.
- Produces: `class ScreenCapture(ctx, projection: MediaProjection, screen: PbScreen, codec: Int, maxLong: Int)`, with:
  - `fun start(): PbScreenAccept`
  - `fun pause()` / `fun resume()` (`setSurface(null)` / restore plus `PARAMETER_KEY_REQUEST_SYNC_FRAME`)
  - `fun stepDown()` (720-class, 15 fps; re-configure; notice "Lower quality to save memory")
  - `fun onRotation()`
  - `fun stop()`

**Encoder format (exact keys):**
- MIME: `video/hevc` with profile `HEVCProfileMain10` (or Main), or `video/avc` High.
- `KEY_COLOR_FORMAT = COLOR_FormatSurface`.
- `KEY_BIT_RATE` = 12 M (H.265) / 16 M (H.264); `KEY_BITRATE_MODE = BITRATE_MODE_VBR` (else CBR).
- `KEY_FRAME_RATE = 30`; `KEY_I_FRAME_INTERVAL = 3600` (no periodic keyframes).
- `KEY_REPEAT_PREVIOUS_FRAME_AFTER = 100_000` µs.
- `KEY_MAX_B_FRAMES = 0` (API 29); `KEY_LATENCY = 1` (API 30); `KEY_PRIORITY = 0`.
- `KEY_COLOR_STANDARD = COLOR_STANDARD_BT709`, `KEY_COLOR_TRANSFER = COLOR_TRANSFER_SDR_VIDEO`, `KEY_COLOR_RANGE = COLOR_RANGE_LIMITED`.
- `KEY_VIDEO_QP_MAX = 38` (API 31+, when `qpMax`).
- Size from `fit_size` through the binding `pb_ffi.fitSize(...)` (export it), confirmed with `isSizeSupported`.

**Output loop:**
- On its own thread; `BUFFER_FLAG_CODEC_CONFIG` → a config record (rotation, size, headers), then a key frame request.
- Repeat frames are marked with `FLAG_REPEAT`; `MediaCodec` tags them via presentation-time equality with the previous frame.
- Applies `PbAdvice`: `request_key` → `setParameters(REQUEST_SYNC_FRAME)`; `new_bitrate` → `PARAMETER_KEY_VIDEO_BITRATE`.

**Memory and thermal:** `onTrimMemory` (≥ `TRIM_MEMORY_RUNNING_LOW`), < 1.2 GB `MemoryInfo.availMem`, or `PowerManager.THERMAL_STATUS_SEVERE` → `stepDown()`.

**Resume:** keep `ScreenCapture` alive on `onStopped(resumable = true)`, paused; stop it after 30 s.

- [ ] **Step 1: Implement.**
- [ ] **Step 2: Run** `pwsh scripts/android-build.ps1` and `gradlew :app:lintRelease`. Expected: BUILD SUCCESSFUL, lint clean.
- [ ] **Step 3: Commit** `android: screen capture to the hardware encoder; consent, capture prompt, mediaProjection service type`.

### Task 8: Android: input from the window, dim / on

**Files:**
- Modify: `ControlService.kt`

**Interfaces:**
- Consumes: Task 5's `ScreenListener` callbacks (registered by `PbService` while a screen session runs; ControlService's methods do the work).
- Produces, on `ControlService`:
  - `fun touchAt(phase: Int, x: Int, y: Int)`: the existing `Finger` `press`/`moveTo`/`release` at `x / 65535f * w`; Hover moves the cursor only.
  - `fun pinchAt(out: Boolean, x: Int, y: Int)`: two strokes, 300 ms, starting 120 px apart and ending 420 px apart (out) or the reverse (in).
  - `fun setDim(on: Boolean)`: a 1×1 `TYPE_ACCESSIBILITY_OVERLAY` with `FLAG_NOT_TOUCHABLE or FLAG_NOT_FOCUSABLE or FLAG_KEEP_SCREEN_ON`; `screenBrightness = 0f` (dim) or `-1f`.
  - `fun endDim()`. If the 1×1 override does not dim the whole display on the realme (checked in Task 15), switch to a full-screen transparent not-touchable overlay.
  - The overlay is removed on `ACTION_SCREEN_OFF` (power button) and on "Restore brightness" from the sharing notification.

- [ ] **Step 1: Implement.**
- [ ] **Step 2: Build plus lint.** Expected: clean.
- [ ] **Step 3: Commit** `android: absolute touches, pinch and dim/on for the phone-screen window`.

### Task 9: Android: sharing notification and main-page card

**Files:**
- Modify: `PbService.kt` (notification "Sharing screen with <PC>" with **Stop** and, while dimmed, **Restore brightness**), `MainActivity.kt` (a `ScreenToPc` card: state plus a Stop button, and the "Always allow" switch), `Bridge` state (`sharingWith: MutableState<String?>`)

- [ ] **Step 1: Implement.**
- [ ] **Step 2: Build plus lint.** Expected: clean.
- [ ] **Step 3: Commit** `android: screen-sharing notification and main-page card`.

### Task 10: PC: Media Foundation decoder

**Files:**
- Create: `crates/pb-tray/src/screen_decode.rs` (`#[cfg(windows)]`, `#![allow(unsafe_code)]`)
- Modify: `crates/pb-tray/Cargo.toml` (`windows` features `Win32_Media_MediaFoundation`, `Win32_Graphics_Direct3D`, `Win32_Graphics_Direct3D11`, `Win32_Graphics_Dxgi`, `Win32_Graphics_Dxgi_Common`; `pb-video`)

**Interfaces (produces):**
- `pub fn supported_codecs() -> Vec<Codec>`: `MFTEnumEx(MFT_CATEGORY_VIDEO_DECODER, MFT_ENUM_FLAG_HARDWARE | MFT_ENUM_FLAG_SORTANDFILTER)` for HEVC and H.264. HEVC present → `[H265Main10, H265, H264]`.
- `pub struct Decoder`:
  - `fn new(device: &ID3D11Device, codec: Codec, cfg: &Config) -> Result<Self, String>`: `IMFDXGIDeviceManager` with the window's device; `CODECAPI_AVLowLatencyMode = TRUE` before streaming; output `NV12`, or `P010` for Main10.
  - `fn push(&mut self, r: &Record) -> Result<Option<Frame>, String>`
  - `pub struct Frame { pub texture: ID3D11Texture2D, pub index: u32, pub pts_us: i64 }`
- A config record re-creates the decoder (rotation and size change).

- [ ] **Step 1: Write the test** `decodes_an_h264_stream_from_the_mf_encoder` (`#[cfg(windows)]`): encode 10 synthetic NV12 frames (gradient, 320×240) with the in-box MF H.264 encoder into Annex-B, wrap them as records with a `Config`, decode → 10 frames, each 320×240. HEVC decode is checked on the device in Task 15 (CI has no HEVC decoder).
- [ ] **Step 2: Run** `cargo test -p pb-tray screen_decode`. Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** the test locally. Expected: PASS. Then `cargo run -p pb-tray --profile spike -- --probe-codecs` prints `[H265Main10, H265, H264]` on this PC (a temporary flag, removed in Task 13).
- [ ] **Step 5: Commit** `tray: hardware video decoding with Media Foundation (low latency, shared D3D11 device)`.

### Task 11: PC: renderer

**Files:**
- Create: `crates/pb-tray/src/screen_render.rs` (`#[cfg(windows)]`)

**Interfaces (produces):**
- `pub fn device_for_window(hwnd: HWND) -> Result<ID3D11Device, String>`: the adapter whose output contains the window's monitor (`IDXGIFactory1::EnumAdapters1` → `EnumOutputs` → `MonitorFromWindow`), with `D3D11_CREATE_DEVICE_VIDEO_SUPPORT`.
- `pub struct Renderer`:
  - `fn new(hwnd: HWND, device: ID3D11Device) -> Result<Self, String>`: a flip-model swap chain with `DXGI_SWAP_CHAIN_FLAG_FRAME_LATENCY_WAITABLE_OBJECT` and maximum frame latency 1; `R10G10B10A2` if the output supports it, else `B8G8R8A8`.
  - `fn present(&mut self, f: &Frame, dimmed: bool) -> Result<(), String>`: the video processor, BT.709 limited → full RGB, letterboxed with the aspect kept; `dimmed` darkens to 40 % (Reconnecting…).
  - `fn resize(&mut self, w: u32, h: u32)`
  - `fn mean_brightness(&mut self) -> Option<u8>`: a 16×16 downscale read back at most once a second.

- [ ] **Step 1: Write the test** `secure_screen_detection_threshold` on the pure `fn is_black(samples: &[u8]) -> bool` (mean < 3): all-zero → true; one bright pixel among 256 → false.
- [ ] **Step 2: Run.** Expected: FAIL.
- [ ] **Step 3: Implement** the renderer and `is_black`.
- [ ] **Step 4: Run.** Expected: PASS. Manual check comes with Task 13.
- [ ] **Step 5: Commit** `tray: low-latency D3D11 renderer on the window's adapter`.

### Task 12: PC: the phone-screen window

**Files:**
- Create: `crates/pb-tray/src/screen_geom.rs` (pure, tested on all platforms), `crates/pb-tray/src/screen_window.rs` (`#[cfg(windows)]`)

**Interfaces:**
- **`screen_geom.rs`:**
  - `pub fn video_rect(client: (u32, u32), video: (u32, u32)) -> (i32, i32, u32, u32)` (letterbox)
  - `pub fn map_point(p: (i32, i32), client: (u32, u32), video: (u32, u32)) -> Option<(u16, u16)>`
  - `pub fn fit_window(proposed: (u32, u32), video: (u32, u32), edge: Edge) -> (u32, u32)` (`WM_SIZING`: keep the aspect, from the dragged edge)
- **`screen_window.rs`:**
  - `pub fn open(view: ScreenView, name: String, settings: SharedSettings, cmds: UnboundedSender<Cmd>)`.
  - Its own thread, message loop, decoder and renderer.
  - A borderless Fluent window titled with the phone's name.
  - **Top bar:** Pin (`HWND_TOPMOST` toggle, remembered in `settings.screen_pinned`) and "Phone screen: dim / on" (`view.set_state`).
  - **Bottom bar:** Back / Home / Recents.
  - **Mouse:** left → `Touch` Down/Move/Up at `map_point`; wheel → `Touch{Hover}` then `Wheel`; Ctrl+Wheel → `Pinch`; right-click → `MouseButton 1`; middle → `MouseButton 2`.
  - **Keys:** `WM_KEYDOWN`/`WM_KEYUP` → `Key{scancode}` (bits 16–23, plus `0xE000` when extended); `WM_CHAR` → `Char`. Esc → `MouseButton 1`; Alt+F4 / Ctrl+W close.
  - **Files:** `DragAcceptFiles` → `Cmd::SendFiles`.
  - **Visibility:** minimised → `set_state(active = false)`; restored → `true`.
  - **Feedback:** sent every 100 ms with the last presented pts.
  - **Status texts:** "Waiting for phone…", "Reconnecting…" (last frame dimmed, retry for 30 s through `Cmd::ShowScreen`), "Phone stopped sharing its screen", "Hidden by the app (secure screen)", and the phone's `notice`.
  - `pub fn is_focused() -> bool` (for notifications' `mirroring_focused`).

- [ ] **Step 1: Write the tests in `screen_geom.rs`:**
  - `video_rect_letterboxes_both_ways`: client 1000×1000 with video 864×1920 → width 450, x 275.
  - `clicks_outside_the_video_send_nothing`: points in the bars → `None`.
  - `corners_map_to_the_full_range`: the video's top-left → (0, 0); bottom-right → (65535, 65535).
  - `touch_mapping_follows_rotation`: video 1920×864 (landscape) in client 1000×600 → the centre maps to (32767, 32767), and a point 10 px inside the left edge of the video maps to x < 1000.
  - `fit_window_keeps_aspect_from_the_dragged_edge`.
- [ ] **Step 2: Run** `cargo test -p pb-tray screen_geom`. Expected: FAIL.
- [ ] **Step 3: Implement** `screen_geom.rs`, then `screen_window.rs`.
- [ ] **Step 4: Run** `cargo test -p pb-tray && cargo clippy --workspace --all-targets -- -D warnings`. Expected: PASS.
- [ ] **Step 5: Commit** `tray: phone-screen window (video, touch, keys, bars, pin, dim, file drop)`.

### Task 13: Tray integration

**Files:**
- Modify: `crates/pb-tray/src/core.rs` (`Cmd::ShowScreen`, `Cmd::StopScreen`, `UiEvent::ScreenReady{view, name}`, `UiEvent::ScreenEnded{why}`), `ui.rs` / `menu.rs` / `flyout.rs` ("Show phone screen" toggle), `settings.rs` (`screen_pinned`, `screen_sharp`), `notify.rs` caller (`mirroring_focused: screen_window::is_focused()`), `main.rs` (remove `--probe-codecs`)

**Interfaces:**
- Consumes: Tasks 4, 10 and 12.
- **Flow:**
  - `Cmd::ShowScreen` → `node.open_screen(peer, ScreenStart{max_long_side: if sharp {0} else {1920}, fps: 30, codecs: supported_codecs(), dim, sharp}, on_asked)`. Status pill "Waiting for {phone}…" while asking.
  - On success → `UiEvent::ScreenReady` → `screen_window::open`.
  - `Declined` → pill "{phone} didn't share its screen". `Timeout` → "{phone} didn't answer".
  - **Decoder failure** (`decoder_failure_asks_next_codec`): the window calls `view.codec_failed()`; the phone restarts its encoder with `next_after`. With no next codec → "Can't show this phone's screen on this PC" and close.

- [ ] **Step 1: Write the test** `decoder_failure_asks_next_codec` as a pure state test of `fn on_decode_error(current: Codec, pc: &[Codec]) -> Next` (`Retry(Codec) | GiveUp`) in `screen_geom.rs` or a new `screen_flow.rs` (pure).
- [ ] **Step 2: Run.** Expected: FAIL.
- [ ] **Step 3: Implement** the flow and the menu/flyout toggle.
- [ ] **Step 4: Run** all gates. Expected: PASS.
- [ ] **Step 5: Commit** `tray: Show phone screen (menu, flyout, session flow, codec fallback)`.

### Task 14: Docs

**Files:**
- Modify: `docs/WIRE_PROTOCOL.md` (screen session flow, record format, grace), `docs/THREAT_MODEL.md` (new asset: the screen stream; consent per session plus Android's prompt, secure screens black, dim overlay, input only while the window has focus), `docs/DECISIONS.md` (Phase 4.1: decisions 1–6 above plus Task 6's encoder caps)

- [ ] **Step 1: Edit** the three docs.
- [ ] **Step 2: Commit** `docs: phone screen on the PC (protocol, threat model, decisions)`.

### Task 15: Device verification and budgets

Run on the realme with Android Studio closed and the light sampler running (`thrash_sample.ps1 -Step screen -Seconds 600`). Announce the consent and capture prompts before they appear.

- [ ] **Step 1: Run the spec's device tests and record the results:**
  - consent and capture prompts; the locked-phone notification;
  - tap, drag, scroll, Ctrl+Wheel pinch on a photo or map, typing;
  - rotate (a correct landscape image, touches accurate); pin and unpin; resize (aspect kept);
  - dim on and off (whole display dims with the 1×1 overlay? if not, switch it); the power button restores brightness;
  - drop a file on the window;
  - Ctrl+Alt+P captured mode with the window open: the phone's cursor shows in the mirror (the accessibility overlay is captured with the screen);
  - minimise pauses capture (phone encoder idle); restore shows a fresh frame at once;
  - close stops capture and drops the service type (`dumpsys activity services dev.phonebridge.app` shows `connectedDevice` only);
  - a secure screen (banking app) shows black with the note, and input still works;
  - **picture quality:**
    - a still screen for 60 s: frames sent drop to 0 after 3 repeats, no shimmer;
    - a dark gradient wallpaper: no banding (Main10 vs Main);
    - fast photo-feed scrolling: no blockiness;
    - text sharp right after a scroll;
    - VBR vs CBR if both exist;
  - **Wi-Fi blip:** "Reconnecting…", then resume without a capture prompt within 30 s;
  - **the 10-minute budget run** (limits in Global Constraints);
  - **glass-to-glass latency** under 100 ms: film a stopwatch shown on the phone next to the PC window.
- [ ] **Step 2: Fix failures**, test first where the cause is in pure code.
- [ ] **Step 3: Record the results** in `docs/DECISIONS.md` (Phase 4.1 results).
- [ ] **Step 4: Run all gates, push, and check CI** (`gh run watch <id> --exit-status`). Expected: every job passes.
