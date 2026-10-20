# Continuity Camera Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The phone's camera appears on the Windows 11 PC as "Phone Bridge Camera" while streaming, and leaves no trace when it stops.

**Architecture:**
- The phone encodes Camera2 frames with the screen's encoder path and sends `pb-video` records over a camera session.
- The tray decodes them on the GPU, fits them into a fixed 16:9 NV12 frame, and writes them into a file-backed frames ring in ProgramData.
- A Rust COM media source DLL, loaded by Windows' Frame Server, reads that ring and delivers 30 fps samples.
- The tray owns the `IMFVirtualCamera` lifetime (Session lifetime; Stop → Remove → Shutdown) and the leftover sweep.

**Tech Stack:** Rust (`windows` 0.62 with Media Foundation, D3D11 and COM; prost; iroh via `pb-transport`), Kotlin (Camera2, MediaCodec), UniFFI.

**Spec:** `docs/superpowers/specs/2026-10-20-continuity-camera-design.md`

## Global Constraints
- Windows 11 only (`MFCreateVirtualCamera`). There's no Windows 10 path.
- Virtual camera: `MFVirtualCameraType_SoftwareCameraSource`, `MFVirtualCameraLifetime_Session`, `MFVirtualCameraAccess_CurrentUser`, friendly name exactly `Phone Bridge Camera`, category `KSCATEGORY_VIDEO_CAMERA`.
- The source **never** calls `AddDeviceSourceInfo` and never opens another device.
- Teardown is always `Stop()` → `Remove()` → `Shutdown()`, each exactly once, on every exit path.
- Media types: NV12 only, 1920×1080 and 1280×720, 30 fps. The type never changes after `Start`.
- Heartbeat: one sample per 33.3 ms. The newest frame, else the last frame again, else black NV12 (Y = 16, UV = 128). Duration 333,333 (100 ns units).
- Frames file: `%ProgramData%\PhoneBridge\camera\frames`. The layout is exactly as in spec section 4 (magic `PBCF`, version 1, data at offset 4096, two slots of 3,110,400 bytes).
- DLL path: `%ProgramData%\PhoneBridge\camera\pb-camera-source-<CARGO_PKG_VERSION>.dll`. HKLM `Software\Classes\CLSID\{CLSID}\InprocServer32`, `ThreadingModel=Both`.
- ACLs:
  - the DLL folder: SYSTEM and Administrators full control; LocalService and Users read & execute;
  - the frames file: SYSTEM full; LocalService read & write; the registering user read & write.
- Registration and unregistration run only in `pb-tray --register-camera` / `--unregister-camera`, started with `ShellExecuteW` verb `runas` (one UAC prompt each).
- Protocol tags: `CameraStart` = 21, `CameraAccept` = 22, `CameraControl` = 23. Feedback reuses `ScreenFeedback` (19).
- Phone camera:
  - `TEMPLATE_RECORD`;
  - AE fps range [30, 30] if available, else the highest fixed range;
  - video stabilization or OIS if offered;
  - output 1920×1080, else 1280×720;
  - encoder `KEY_FRAME_RATE` 30, with no repeat-previous-frame.
- Android FGS: the `camera` type only while streaming, entered while `CameraConsentActivity` is visible.
- Resume: a 30 s grace only after a lost link (`ScreenEvent::Closed { by_peer: false }` semantics). A deliberate close ends the session.
- Logs never contain frame content.
- Commits: git user Lin, with **no AI attribution** of any kind.
- Phone tests: never `adb kill-server`/`usb`/`tcpip`/`root`/`reboot`, `svc` or `settings put`; always `adb -s 79KRFEJVPVOBQCHQ`; no `KEYCODE_SLEEP`.

## Review Focus
1. **The tray is killed or crashes mid-call.** The device must disappear (Session lifetime), and the next start's sweep finds nothing or removes it. Owner: Task 8, test `sweep_removes_a_killed_process_camera`.
2. **The phone rotates or switches lens mid-call.** The media type must stay the same. The frame keeps its size; portrait is pillarboxed; no gap longer than one heartbeat without a sample. Owner: Task 3 (`fit_*`, `heartbeat_*`) and Task 6 (`media_type_never_changes_after_start`).
3. **A torn or partial frame is read while the tray writes.** It must be dropped and the last frame repeated, never a mixed image. Owner: Task 3, `torn_read_is_rejected`.
4. **Remove while streaming, or with Frame Server holding the DLL.** Stop the stream first; the DLL deletion is scheduled for reboot rather than failing. Owner: Task 7, `unregister_schedules_locked_dll`; Task 9, `remove_stops_streaming_first`.
5. **The decoder's padded textures** (`RowPitch` > width, height padded to 1088). No skew and no green bars in the frames file. Owner: Task 3, `row_pitch_copy_strips_padding`; Task 9, the device check.

---

### Task 1: Spike: confirm the platform assumptions (throwaway)

**Files:** `spikes/vcam-spike/` (a new workspace member, deleted or kept as a spike; not shipped)

**Interfaces:** Produces rulings only (written to the ledger), consumed by Tasks 6–10.

- [ ] **Step 1:** Write a minimal Rust COM media source (one black NV12 1280×720 stream; `IMFMediaSourceEx`, `IMFGetService`, `IKsControl`, `IMFMediaStream2`). Register it elevated with a copy in ProgramData. Create the camera with `MFCreateVirtualCamera` (Session lifetime). Open Windows Camera.
  - Expected: "Phone Bridge Camera" listed and showing black.
- [ ] **Step 2:** From the source, open a file in `%ProgramData%\PhoneBridge\camera\` (ACL: user read & write, LocalService read & write).
  - Expected: the open succeeds inside Frame Server (log via `OutputDebugString` and read with DebugView, or a log file in the same folder).
- [ ] **Step 3:** Set a custom attribute on `IMFVirtualCamera` (`SetString`) before `Start` and try to read it in the source (activation attributes).
  - Ruling: per-user frames files if it works; otherwise the single-file v1 of the spec.
- [ ] **Step 4:** Kill the creating process (Task Manager).
  - Expected: the device is gone from `MFEnumDeviceSources` within 5 s. Record the time.
- [ ] **Step 5 (phone):** With the app in the background, start a transparent activity from `PbService` on a PC request (accessibility background-start exemption), and enter the FGS `camera` type while it's visible. Open the camera from the service after the activity finishes.
  - Expected: frames flow. Otherwise fall back to a full-screen-intent notification and record the ruling.
- [ ] **Step 6:** Write the ledger rulings (`Task 1: Ruling: …`) with the evidence.
  - No product commit. The spike folder is committed only if a later task copies from it; otherwise delete it.

### Task 2: Protocol: camera messages

**Files:**
- Modify: `crates/pb-protocol/src/control.rs` (`ControlMsg` oneof `tags = "1, …, 20, 21, 22, 23"`; new structs);
- Modify: `crates/pb-protocol/src/lib.rs` (exports);
- Test: the existing round-trip test module in `control.rs`.

**Interfaces:** Produces:
- `CameraStart { id: u64, codecs: Vec<u32>, lens: u32, width: u32, height: u32, fps: u32 }`
- `CameraAccept { id: u64, codec: u32, width: u32, height: u32, rotation: u32, lens: u32 }`
- `CameraControl { lens: u32 }`
- the `ControlMsg::{CameraStart, CameraAccept, CameraControl}` variants.

- [ ] **Step 1: Failing test.** `camera_messages_round_trip`: encode and decode each of the three messages with non-default values; assert equality. Assert the tag bytes: `CameraStart` encodes with a first byte `0xAA, 0x01` (field 21, wire type 2).
- [ ] **Step 2:** Run `cargo test -p pb-protocol camera_messages_round_trip`. Expected: FAIL (types missing).
- [ ] **Step 3:** Add the structs and variants (field tags 1.. in the order above).
- [ ] **Step 4:** Run `cargo test -p pb-protocol`. Expected: PASS, all.
- [ ] **Step 5:** Commit: "protocol: camera session messages".

### Task 3: `pb-camera-frames`: frames file, copy, fit, heartbeat (pure, shared)

**Files:**
- Create `crates/pb-camera-frames/{Cargo.toml, src/lib.rs, src/layout.rs, src/copy.rs, src/fit.rs, src/heartbeat.rs}`.
- Add it to the workspace members.
- No `windows` dependency, `unsafe_code = "deny"`. The ring works on `&mut [u8]` / `&[u8]`, with atomics via `AtomicU64::from_ptr` behind one documented `#[allow(unsafe_code)]` helper.

**Interfaces:** Produces:
- `pub const MAGIC: [u8; 4] = *b"PBCF"`, `pub const VERSION: u32 = 1`, `pub const DATA_OFFSET: usize = 4096`, `pub const SLOT_BYTES: usize = 1920 * 1080 * 3 / 2`, `pub const FILE_BYTES: usize = DATA_OFFSET + 2 * SLOT_BYTES`.
- `pub struct Ring<'a>` over the mapped bytes:
  - `Ring::init(buf: &mut [u8])`;
  - `Ring::open(buf) -> Option<Ring>` (checks the magic and version);
  - `fn write_frame(&self, w: u32, h: u32, pts_us: u64, fill: impl FnOnce(&mut [u8]))`;
  - `fn read_newest(&self, last_seq: u64, out: &mut Vec<u8>) -> Read` (`enum Read { New { seq: u64, w: u32, h: u32, pts_us: u64 }, Same, Torn, Empty }`);
  - `fn set_wanted(&self, w: u32, h: u32)`, `fn wanted(&self) -> (u32, u32)`;
  - `fn beat(&self, now_ms: u64)`, `fn tray_alive(&self, now_ms: u64) -> bool` (5 s rule);
  - `fn zero(&self)`.
- `pub fn copy_nv12(src_y: &[u8], src_uv: &[u8], row_pitch: usize, w: usize, h: usize, dst: &mut [u8])`: `h` Y rows of `w` bytes, then `h / 2` UV rows of `w` bytes, each source row read at `row_pitch * row`. The caller passes `src_uv` starting at `row_pitch * padded_h`.
- `pub fn fit(src: (u32, u32), rotation: u32, out: (u32, u32)) -> Rect { x, y, w, h }`:
  - the source is swapped for rotation 90/270;
  - scale to fit inside `out` while keeping the aspect;
  - `w` and `h` = `2 * round(exact / 2)`;
  - `x = ((out.w - w) / 2)` and `y = ((out.h - h) / 2)`, each rounded down to even.
- `pub struct Heartbeat`:
  - `Heartbeat::new()`;
  - `fn tick(&mut self, read: Read) -> Deliver`;
  - `enum Deliver { Black, Repeat, New }`;
  - plus `fn next_pts(&mut self, now_100ns: i64) -> i64` (strictly increasing).

- [ ] **Step 1: Failing tests.**
  - `header_round_trips`: init, open, magic and version checked; `set_wanted(1280, 720)` → `wanted() == (1280, 720)`.
  - `newest_complete_slot_wins`: write A (seq 2), write B (seq 2 in the other slot) → `read_newest` returns B's bytes and `New`.
  - `torn_read_is_rejected`: start a write with the seq left odd (a test hook `begin_write_for_test`) → `read_newest` returns `Torn` or the previous frame, never the half-written bytes.
  - `same_frame_reads_same`: reading twice with the returned seq → `Same`.
  - `tray_heartbeat_times_out_after_5s`: `beat(1000)`; `tray_alive(5999)` is true; `tray_alive(6001)` is false.
  - `row_pitch_copy_strips_padding`: w 1920, h 1080, `row_pitch` 2048, `padded_h` 1088. Fill each source row with its row index (padding bytes 0xEE) → the dst Y row r is all `r as u8`, the UV row r is all `(1000 + r) as u8`, the dst length is `1920 * 1080 * 3 / 2`, and no 0xEE anywhere.
  - `fit_portrait_is_pillarboxed`: `fit((1080, 1920), 0, (1920, 1080)) == Rect { x: 656, y: 0, w: 608, h: 1080 }`.
  - `fit_landscape_fills`: `fit((1920, 1080), 0, (1280, 720)) == Rect { x: 0, y: 0, w: 1280, h: 720 }`.
  - `fit_rotation_90_swaps`: `fit((1920, 1080), 90, (1920, 1080))` equals the portrait case.
  - `heartbeat_black_before_first_frame`: `tick(Empty) == Black`.
  - `heartbeat_repeats_last_frame`: after `New`, `tick(Same) == Repeat`, `tick(Torn) == Repeat`.
  - `heartbeat_timestamps_increase`: 100 calls to `next_pts` with the same `now` are strictly increasing, by ≥ 333,333 each.
- [ ] **Step 2:** Run `cargo test -p pb-camera-frames`. Expected: FAIL to compile (the crate is empty).
- [ ] **Step 3:** Implement.
  - Seqlock writer: set `seq[slot]` odd (Release), fill, set even (+1, Release), store `newest` (Release).
  - Reader: Acquire loads, the copy, re-check.
  - `fit`: letterbox or pillarbox by aspect, with the rounding rule from the Interfaces block.
- [ ] **Step 4:** Run `cargo test -p pb-camera-frames`. Expected: PASS (12 tests).
- [ ] **Step 5:** Commit: "camera: frames ring, NV12 copy, fit and heartbeat (shared by tray and source)".

### Task 4: Transport: camera sessions

**Files:**
- Create `crates/pb-transport/src/camera.rs`.
- Modify `crates/pb-transport/src/lib.rs` (`pub mod camera`).
- Test: `crates/pb-transport/src/tests.rs` `mod camera`, reusing `mod screen`'s helpers `paired`, `rec` and `next_event` (move them to a shared `mod video_helpers` in tests.rs).

**Interfaces:**
- Consumes: `screen::{ByteQueue, ScreenSink, SendOutcome}` patterns. Reuse `ScreenSink` for sending records; it's generic over records.
- Produces:
  - `Node::open_camera(&self, peer: &PeerId, req: CameraStart, on_asked: impl FnOnce()) -> Result<CameraView, Error>`;
  - `CameraView { accept: CameraAccept, records: mpsc::Receiver<Record>, state … }` with `into_parts() -> (CameraRemote, Receiver<Record>)`;
  - `CameraRemote::{feedback(i64), switch_lens(u32), close()}`;
  - `serve_camera(session, start: CameraStart, answer: Option<CameraAccept>) -> Result<CameraServer, Error>`;
  - `CameraServer::into_parts() -> (ScreenSink, UnboundedReceiver<CameraEvent>)`;
  - `enum CameraEvent { Feedback(i64), Lens(u32), Closed { by_peer: bool } }`.

- [ ] **Step 1: Failing tests** in `mod camera`:
  - `camera_records_arrive_in_order`: config + key + 20 frames; the PC receives 22 in order.
  - `declined_camera_reports_declined`.
  - `lens_switch_reaches_the_phone`: `remote.switch_lens(1)` → `CameraEvent::Lens(1)`.
  - `pc_close_is_by_peer`: `view.close()` → `Closed { by_peer: true }`; the phone's own close → `Closed { by_peer: false }`.
- [ ] **Step 2:** Run `cargo test -p pb-transport camera`. Expected: FAIL (module missing).
- [ ] **Step 3:** Implement, mirroring `screen.rs`: no input stream; control handles `CameraControl` and `ScreenFeedback`; the close reason uses the same `ApplicationClosed` check as `screen.rs:450`.
- [ ] **Step 4:** Run `cargo test -p pb-transport`. Expected: PASS (all, including the 61 existing).
- [ ] **Step 5:** Commit: "transport: camera sessions (records, lens switch, close reason)".

### Task 5: FFI: phone camera sessions

**Files:**
- Modify `crates/pb-ffi/src/lib.rs`.
- Regenerate the bindings via `scripts/android-build.ps1`.

**Interfaces:**
- Consumes: Task 4.
- Produces (UniFFI):
  - `PbEvent::CameraRequest { id: u64, peer_name: String, lens: u32, width: u32, height: u32, codecs: Vec<u8>, resume: bool }`;
  - `PbCameraAccept { codec: u8, width: u32, height: u32, rotation: u16, lens: u8 }`;
  - `PbNode::answer_camera(id: u64, accept: Option<PbCameraAccept>) -> Option<Arc<PbCamera>>`;
  - `PbCamera::{send_frame(payload: Vec<u8>, pts_us: i64, flags: u8) -> PbAdvice, take_confirm_stats() -> PbConfirmStats, stop()}`;
  - the `CameraListener` trait `{ on_lens(lens: u8), on_stopped(resumable: bool) }`, `PbNode::set_camera_listener`.
  - A `Grace` for the camera, separate from the screen's; `camera_user_stop` declines quietly for 30 s.

- [ ] **Step 1: Failing test:** `camera_grace_and_user_stop_are_separate_from_screen` (a camera user-stop does not decline a screen request and vice versa), using the existing `Grace` helpers.
- [ ] **Step 2:** Run `cargo test -p pb-ffi`. Expected: FAIL.
- [ ] **Step 3:** Implement by mirroring the `PbScreen` serve flow (pending buffer of 32, pacer, `ScreenSink`).
- [ ] **Step 4:** Run `cargo test -p pb-ffi && pwsh scripts/android-build.ps1`. Expected: PASS, and `BUILD SUCCESSFUL`.
- [ ] **Step 5:** Commit: "ffi: phone camera sessions".

### Task 6: `pb-camera-source`: the COM media source DLL

**Files:**
- Create `crates/pb-camera-source/{Cargo.toml, src/lib.rs, src/source.rs, src/stream.rs, src/frames.rs, src/guids.rs}`.
- `crate-type = ["cdylib", "rlib"]`; `[lints.rust] unsafe_code = "deny"` with per-file allows, like pb-tray.
- `windows` 0.62 features: `Win32_Media_MediaFoundation`, `Win32_Media_KernelStreaming`, `Win32_System_Com`, `Win32_Foundation`, `Win32_Storage_FileSystem`, `Win32_System_Memory`, `implement`.

**Interfaces:**
- Consumes: Task 3 (`Ring`, `Heartbeat`, `Deliver`); Task 1 rulings.
- Produces:
  - `pub const CLSID_PHONE_BRIDGE_CAMERA_SOURCE: GUID` (fixed; generate once and paste it here in the plan's ledger);
  - the exports `DllGetClassObject`, `DllCanUnloadNow`;
  - `pub fn media_types() -> [(u32, u32); 2]` = `[(1920, 1080), (1280, 720)]`.

- [ ] **Step 1: Failing tests (rlib, no Frame Server):**
  - `media_types_are_fixed_16_9_nv12`: two types; NV12 subtype; frame rate 30/1; sizes as above.
  - `media_type_never_changes_after_start`: create the source in-process (`CoCreateInstance` is not needed: construct it directly), `Start` with 1280×720, then attempt `SetCurrentMediaType(1920×1080)` → `MF_E_INVALIDREQUEST`.
  - `samples_flow_at_30fps_from_an_empty_ring`: in-process with a temp-file ring; request 10 samples → 10 `MEMediaSample` events within 400 ms, all black (Y = 16), timestamps increasing.
  - `samples_show_the_newest_ring_frame`: write a frame of Y = 200 into the ring → the next sample's first byte is 200.
- [ ] **Step 2:** Run `cargo test -p pb-camera-source`. Expected: FAIL.
- [ ] **Step 3:** Implement:
  - the source and stream with `#[implement]`;
  - the event queues via `MFCreateEventQueue`;
  - the stream descriptor attributes (spec section 4);
  - `IKsControl` returns `HRESULT_FROM_WIN32(ERROR_SET_NOT_FOUND)`;
  - the heartbeat on an `MFPutWaitingWorkItem` or timer-queue timer at 33 ms answering queued `RequestSample` tokens;
  - the ring opened with `CreateFileW` + `CreateFileMappingW` (no name) + `MapViewOfFile`;
  - on `SetCurrentMediaType` → `ring.set_wanted`;
  - no call to any `AddDeviceSourceInfo`.
- [ ] **Step 4:** Run `cargo test -p pb-camera-source && cargo build --release -p pb-camera-source`. Expected: PASS; `target\release\pb_camera_source.dll` exists.
- [ ] **Step 5:** Commit: "camera: Media Foundation source DLL (fixed NV12 types, 30 fps heartbeat, frames ring)".

### Task 7: Tray: register and unregister (elevated)

**Files:**
- Create `crates/pb-tray/src/camera_register.rs`.
- Modify `crates/pb-tray/src/main.rs` (the `--register-camera` and `--unregister-camera` arguments; exit codes 0 ok, 2 not elevated, 3 copy failed, 4 registry failed, 5 ACL failed).

**Interfaces:**
- Consumes: Task 6 (CLSID, DLL file name).
- Produces:
  - `pub fn is_registered() -> bool` (HKLM key present **and** the DLL path exists);
  - `pub fn request_register(hwnd) -> Result<(), RegisterError>` (`ShellExecuteExW` runas, waits for the exit code);
  - `pub fn request_unregister(hwnd) -> Result<(), RegisterError>`;
  - `pub fn register_elevated() -> i32`, `pub fn unregister_elevated() -> i32`;
  - `pub fn frames_path() -> PathBuf`, `pub fn dll_dir() -> PathBuf`.

- [ ] **Step 1: Failing tests:**
  - `paths_are_under_programdata`: `dll_dir()` ends with `PhoneBridge\camera` under `%ProgramData%`; the DLL name contains `CARGO_PKG_VERSION`.
  - `sddl_for_frames_grants_only_user_localservice_system`: the built SDDL string contains `(A;;FA;;;SY)`, `(A;;FRFW;;;LS)`, `(A;;FRFW;;;<user SID>)`, and no `WD`/`BU` grants.
  - `unregister_schedules_locked_dll` (ignored, admin): hold the DLL open with `LoadLibraryW` in the test, run `unregister_elevated()` → returns 0; the DLL path is in `PendingFileRenameOperations`.
  - `register_then_unregister_round_trip` (ignored, admin): register → `is_registered()`; unregister → not registered, the folder has no DLL.
- [ ] **Step 2:** Run `cargo test -p pb-tray camera_register`. Expected: FAIL.
- [ ] **Step 3:** Implement:
  - copy the DLL from next to `pb-tray.exe`;
  - apply the ACLs with `ConvertStringSecurityDescriptorToSecurityDescriptorW` + `SetFileSecurityW` (the pattern in `ipc.rs::owner_only`);
  - registry via `RegCreateKeyExW` in `HKEY_LOCAL_MACHINE`;
  - create the frames file with `FILE_BYTES`, then `Ring::init`;
  - the user SID comes from the unelevated caller: pass `--for-sid <SID>` from the requesting process (the elevated token's user may be an admin account). Reuse `ipc.rs::user_sid`.
- [ ] **Step 4:** Run `cargo test -p pb-tray camera_register`, then run the two ignored tests in an elevated shell (`cargo test -p pb-tray camera_register -- --ignored`). Expected: PASS; record the output in the ledger.
- [ ] **Step 5:** Commit: "tray: register/unregister the camera source (ProgramData copy, ACLs, one UAC prompt)".

### Task 8: Tray: virtual camera lifetime and the leftover sweep

**Files:** create `crates/pb-tray/src/camera_device.rs`.

**Interfaces:**
- Consumes: Task 6 CLSID; Task 7 `is_registered`.
- Produces:
  - `pub trait VirtualCamera { fn start(&mut self) -> Result<(), String>; fn stop(&mut self); fn remove(&mut self); fn shutdown(&mut self); }`;
  - `pub struct MfVirtualCamera` (the real one, created on a dedicated MTA thread);
  - `pub struct CameraDevice<C: VirtualCamera>` with `open(c) -> Result<Self, String>` (start) and `close(self)`. `close` and `Drop` run `stop`, `remove`, `shutdown` exactly once.
  - `pub fn sweep_leftovers(log: &dyn Fn(&str)) -> usize`.

- [ ] **Step 1: Failing tests:**
  - `teardown_order_is_stop_remove_shutdown`: a fake records calls; `open` then `close` → `["start", "stop", "remove", "shutdown"]`.
  - `drop_tears_down_once`: open, then drop → the same sequence, once.
  - `failed_start_still_shuts_down`: the fake's `start` errors → the calls are `["start", "remove", "shutdown"]` (no stop of a camera that never started) and `open` returns `Err`.
  - `sweep_removes_a_killed_process_camera` (ignored, needs registration): spawn `pb-tray --test-create-camera` (a hidden test-only argument that creates and sleeps), kill it, call `sweep_leftovers` → afterwards `MFEnumDeviceSources` has no "Phone Bridge Camera".
- [ ] **Step 2:** Run `cargo test -p pb-tray camera_device`. Expected: FAIL.
- [ ] **Step 3:** Implement `MfVirtualCamera` with `MFCreateVirtualCamera(...)` per the Global Constraints, then `Start(None)`.
  - The sweep: enumerate with `MFEnumDeviceSources` (`MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_GUID`); for a friendly-name or CLSID match, create a handle with the same parameters, then `Remove` + `Shutdown`.
  - Call the sweep at tray start (after the UI is up, off the UI thread) and at tray exit or `WM_ENDSESSION`.
- [ ] **Step 4:** Run `cargo test -p pb-tray camera_device` (plus `-- --ignored` after Task 7's registration on this PC). Expected: PASS.
- [ ] **Step 5:** Commit: "tray: virtual camera lifetime (Session, Stop→Remove→Shutdown) and leftover sweep".

### Task 9: Tray: camera session → GPU fit → frames file; menu, pill and settings

**Files:**
- Create `crates/pb-tray/src/camera.rs`.
- Modify:
  - `crates/pb-tray/src/core.rs` (`Cmd::StartCamera`, `Cmd::StopCamera`, `Cmd::SwitchLens`; the `show_camera` flow like `show_screen`, with the 30 s reconnect);
  - `crates/pb-tray/src/ui.rs` (menu items "Use phone as camera" / "Stop phone camera" / "Switch lens"; IDs 17–19; pill texts from spec section 6);
  - `crates/pb-tray/src/settings_window.rs` or its equivalent ("Remove Phone Bridge Camera" row: "Installed" / "Not installed", "Repair camera");
  - `crates/pb-tray/src/ipc.rs` and `main.rs` (`--start-camera`, `--stop-camera` for checks without clicks; tested like show-screen).

**Interfaces:**
- Consumes: `screen_decode::{Decoder, Frame, supported_codecs}`, `screen_render::device_for_window` (or a windowless adapter choice: the default adapter), Task 3 (`Ring`, `copy_nv12`, `fit`), Task 4 `open_camera`, Task 7 `is_registered`/`request_register`, Task 8 `CameraDevice`.
- Produces:
  - `pub struct CameraPipe` (decode thread, GPU fit, two staging textures, ring writer);
  - `pub fn remove_camera(hwnd)`: stops streaming, sweeps, then `request_unregister`.

- [ ] **Step 1: Failing tests:**
  - `requests_round_trip_and_garbage_is_rejected` extended with `StartCamera` and `StopCamera`.
  - `remove_stops_streaming_first`: with a fake device and a fake unregister recorder, `remove_camera` → the call order is `["stop", "remove", "shutdown", "sweep", "unregister"]`.
  - `gpu_fit_writes_tight_nv12` (skips without a hardware D3D11 device, like the decoder test): encode a 1080×1920 H.264 test pattern with the in-box encoder, decode, fit into 1280×720, read back.
    - The ring frame is 1280×720 × 1.5 bytes.
    - Columns 0..435 of row 360 are black (Y = 16): the pillarbox (fit gives x 436, w 406).
    - The centre pixel matches the pattern within ±8.
- [ ] **Step 2:** Run `cargo test -p pb-tray camera`. Expected: FAIL.
- [ ] **Step 3:** Implement:
  - the video processor output to an NV12 render target of `ring.wanted()` (default 1920×1080);
  - the dest rect from `fit`;
  - the background cleared to black (YUV 16/128/128);
  - `CopyResource` to staging[i % 2], `Map`, `copy_nv12`, `ring.write_frame`;
  - `ring.beat()` once a second;
  - `ring.zero()` at stop and at tray start;
  - feedback per decoded frame (as in the screen window).
- [ ] **Step 4:** Run `cargo test -p pb-tray` and `cargo clippy --workspace --all-targets -- -D warnings`. Expected: PASS and clean.
- [ ] **Step 5:** Commit: "tray: phone camera (decode, GPU fit into fixed 16:9 NV12, frames file; menu, pill, Remove)".

### Task 10: Android: camera capture, consent, service type

**Files:**
- Create `android/app/src/main/java/dev/phonebridge/app/{CameraShare.kt, CameraConsentActivity.kt}`.
- Modify:
  - `PbService.kt` (`askCamera`; `enterCamera` / `leaveCamera` service types; notification "Camera in use by <PC>" with Stop and Switch lens);
  - `Prefs.kt` (`allowCamera`);
  - `MainActivity.kt` (the "Camera for my PC" card: the permission state, "Always allow");
  - `AndroidManifest.xml` (`android.permission.CAMERA`, `FOREGROUND_SERVICE_CAMERA`, the PbService type `connectedDevice|mediaProjection|camera`, the activity with a translucent theme).

**Interfaces:**
- Consumes: Task 5 FFI; `EncoderCaps.kt`; Task 1's phone ruling.
- Produces:
  - `object CameraShare { start(ask), switchLens(), stop(userStopped) }`;
  - `class CameraCapture(ctx, ask) { prepare(): PbCameraAccept?, attach(PbCamera), stop() }`.

- [ ] **Step 1:** Implement capture per the Global Constraints:
  - choose the camera by `LENS_FACING`;
  - pick the AE range from `CONTROL_AE_AVAILABLE_TARGET_FPS_RANGES` (a pure helper `pickFpsRange(ranges): Range<Int>`, unit-tested in `app/src/test`: `[[15, 30], [30, 30], [24, 24]] → [30, 30]`; `[[15, 30], [24, 24]] → [24, 24]`);
  - stabilization keys;
  - a 16:9 size helper (`pickSize(sizes)` tested: prefers 1920×1080, then 1280×720);
  - encoder input surface as the capture target;
  - the cached config record re-sent on attach (as `ScreenShare.kt` does since `538a218`).
- [ ] **Step 2:** Run `./gradlew testReleaseUnitTest lintRelease` (in `android/`). Expected: PASS, lint exit 0.
- [ ] **Step 3: Device check** (the realme; the user taps the camera permission and the first consent):
  - `pb-tray --start-camera`;
  - Windows Camera shows the phone picture;
  - logcat `CAMERA stats out=30/s`;
  - `dumpsys activity services dev.phonebridge.app` shows the `camera` type only while streaming.
- [ ] **Step 4:** Commit: "android: phone camera for the PC (Camera2 30 fps lock, stabilization, consent, camera service type only while streaming)".

### Task 11: Docs and device verification

**Files:** `docs/THREAT_MODEL.md` (row 17, from spec section 8), `docs/WIRE_PROTOCOL.md` (camera section: tags 21–23, the frames-file layout), `docs/DECISIONS.md` (Phase 5.0: the Task 1 rulings, measured numbers).

- [ ] **Step 1:** Run every device test in spec section 9 and record the evidence (log lines, numbers, window-only captures) in `DECISIONS.md`. Expected: all pass. A failure is fixed in its owning task's files with a regression test first.
- [ ] **Step 2:** Full gates: `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace --no-fail-fast`, `pwsh scripts/android-build.ps1`, `./gradlew lintRelease`. Expected: all green.
- [ ] **Step 3:** Commit: "docs: Continuity Camera (threat model, wire protocol, decisions)". Push. Confirm CI is green.
