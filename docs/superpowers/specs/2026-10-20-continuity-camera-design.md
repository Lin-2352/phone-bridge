# Continuity Camera: design

Status: written on 2026-10-20 from the user's choices in brainstorming (sub-project 2 of 4), with the user's seven stability, cleanup and quality points folded in. **Awaiting the user's review.** No camera code is written before that review.

## Purpose and scope
Use the phone's camera as a webcam on the PC. Any Windows app (Teams, Zoom, Meet in a browser, OBS, the Camera app) sees a normal camera named **"Phone Bridge Camera"**.

- **Direction:** phone → PC video only.
- **Windows 11 only:** the camera is a Media Foundation virtual camera (`MFCreateVirtualCamera`).
- **Clean system (user requirement):**
  - the camera device exists only while streaming;
  - quitting or closing Phone Bridge removes it;
  - leftovers from a crash are removed at start-up and shutdown;
  - "Remove Phone Bridge Camera" takes everything off the system.
- **Never touches other cameras:** the source is pure software. It never wraps, opens or locks a physical webcam.

### Out of scope
- **Audio:** the phone as a microphone is a separate decision (DECISIONS Q2).
- **Windows 10:** a DirectShow filter; DECISIONS lists it as a fallback, not in v1.
- **A preview window on the PC:** apps show their own preview.
- **Effects, zoom, torch, portrait blur.** Later, if wanted.
- **Hiding the camera from detection of any kind:** it's a normal, named webcam that every app can see and identify.

## Approach
Three pieces, reusing the phone-screen work (sub-project 1).

| Piece | Where | Does |
|---|---|---|
| Phone capture | Android, new `CameraShare.kt` | Camera2 → hardware encoder (H.265/H.264, the screen's `EncoderCaps`) → `pb-video` records over a camera session |
| PC receive | `pb-tray`, new `camera.rs` | Opens the session, decodes with `screen_decode::Decoder`, scales on the GPU to the fixed output size, writes NV12 into the frames file |
| Media source | new crate `pb-camera-source` (Rust `cdylib`, COM in-proc server) | Loaded by Windows' Frame Server; reads the frames file and delivers 30 fps NV12 samples to apps |

The tray owns the virtual camera's lifetime (`IMFVirtualCamera`). The source owns only delivery.

## 1. Protocol and transport
A camera session follows the screen session's pattern (`open_screen` / `serve_screen` in `crates/pb-transport/src/screen.rs`):
- one consent;
- one phone → PC uni stream of `pb-video` records (PBV1 framing, config and key flags, 4 MiB cap);
- control messages both ways on the session.

New `ControlMsg` variants (`crates/pb-protocol/src/control.rs`, next free tags):

| Tag | Message | Fields |
|---|---|---|
| 21 | `CameraStart` (PC → phone) | `id`, `codecs` (best first), `lens` (0 back, 1 front), `width`, `height` (the 16:9 size wanted: 1920×1080 or 1280×720), `fps` (30) |
| 22 | `CameraAccept` (phone → PC) | `id`, `codec`, `width`, `height`, `rotation`, `lens` |
| 23 | `CameraControl` (PC → phone) | `lens` (switch lenses mid-stream) |

- **Feedback:** the existing `ScreenFeedback` (tag 19), so the same `pb_video::Pacer` paces the camera.
- **Declined:** no `CameraAccept`, the stream closes, the PC gets `Error::Declined`.
- **Resume:** a 30 s grace after a lost link only, the same rule as the screen after commit `c102bde`. A deliberate close by either side ends the session.

## 2. Phone side

**Capture**
- `CameraDevice.TEMPLATE_RECORD`.
- `CONTROL_AE_TARGET_FPS_RANGE` = [30, 30] when it's listed in `CONTROL_AE_AVAILABLE_TARGET_FPS_RANGES`; otherwise the highest fixed range ([24, 24], …), so indoor light never drops the stream to 15 fps.
- `CONTROL_VIDEO_STABILIZATION_MODE_ON` when it's offered, else `LENS_OPTICAL_STABILIZATION_MODE_ON` when the lens has OIS.
- **Output:** a 16:9 size from `StreamConfigurationMap.getOutputSizes(MediaCodec::class.java)`: 1920×1080, else 1280×720. It renders straight into the encoder's input surface (no copies).

**Encoder:** the screen's settings (VBR, QP ceiling, BT.709 limited range, key frame on request) with these changes:
- `KEY_FRAME_RATE` 30;
- no `KEY_REPEAT_PREVIOUS_FRAME_AFTER`: a camera never stands still.

**Orientation**
- The phone sends the sensor image as captured, plus a `rotation` (0/90/180/270) from the sensor orientation and the device rotation.
- The PC fits it into the fixed 16:9 frame (section 3).
- A rotation change sends a new config record with the new `rotation`. The size on the wire stays the same.

**Lens switch**
- Close the old camera, open the other, same encoder.
- The PC's source repeats the last frame during the 200–400 ms gap (section 4).

**Consent**
- Android's camera runtime permission is asked once, in the app.
- Each session asks the user ("Use your camera on <PC>?"), unless "Always allow camera for <PC>" is on (opt-in, like the screen's).

**Foreground service**
- PbService adds the `camera` type only while streaming (DECISIONS row 25) and drops it at the end.
- Android's while-in-use rule: a foreground service may open the camera only if it took the camera type while an activity of ours was visible. So `CameraConsentActivity` always comes to the front, even with "Always allow", where it shows for a moment and finishes. It starts the camera type while visible.
- Starting that activity from the background relies on the bound accessibility service's background-start exemption (or a full-screen-intent notification as a fallback). This is verified on the realme in Task 1's spike.

**While streaming**
- A notification "Camera in use by <PC>" with Stop and Switch lens.
- Android's own camera privacy indicator (green dot) shows too.

**Budgets:** the screen's (section 4 of its spec). Step-down to 1280×720 when free memory is under 1.2 GB; back up above 1.6 GB.

## 3. PC receive (`pb-tray`)
- **Tray menu:** "Use phone as camera" / "Stop phone camera", plus "Switch lens" while streaming.
- **Pill:** "Phone camera on". On end, the reason: "<phone> didn't share its camera", "Phone camera stopped".
- **Decode:** `screen_decode::Decoder` (hardware, DXVA), on its own thread, like the screen window's decode thread.
- **Scale and fit on the GPU.** The D3D11 video processor (as in `screen_render.rs`) converts and scales into an NV12 output texture of the **fixed output size**:
  - the output size is the one the app chose, read from the frames file header (`wanted_w`/`wanted_h`);
  - it applies `rotation`;
  - landscape content fills the frame;
  - portrait content is pillarboxed (centred, black bars, never stretched).
- **Readback**
  - Two pre-allocated `D3D11_USAGE_STAGING` NV12 textures, used alternately, so the next GPU copy never waits on the CPU map.
  - `Map` (read), then a row-by-row copy: `width` bytes per row for the Y plane (`height` rows), then the UV plane (`height / 2` rows). Each row starts at `RowPitch × row`, so GPU padding never reaches the file. The result is tightly packed NV12 in the next free slot.
  - Unmap.
- **Cost check:** 1920×1080 NV12 is 3 MB a frame; 30 fps × 3 MB = 90 MB/s of memory copy. That's well within budget, and is measured in the device test.

## 4. Media source (`pb-camera-source`)

**COM server and media source**
- A Rust `cdylib` exporting:
  - `DllGetClassObject` and `DllCanUnloadNow` (COM);
  - `DllRegisterServer` / `DllUnregisterServer`, unused: registration is done by the tray, see section 5.
- One class, `CLSID_PhoneBridgeCameraSource` (a fixed GUID chosen in the plan).
- The source implements `IMFMediaSourceEx`, `IMFGetService` and `IKsControl`. `IKsControl` answers "not supported" to every property: no device to control.
- The stream implements `IMFMediaStream2`.
- **Stream descriptor attributes:**
  - `MF_DEVICESTREAM_STREAM_CATEGORY` = `PINNAME_VIDCAP`;
  - `MF_DEVICESTREAM_STREAM_ID` = 0;
  - `MF_DEVICESTREAM_FRAMESERVER_SHARED` = 1.
- **Pure software:** never calls `IMFVirtualCamera::AddDeviceSourceInfo`, never opens another device.

**Media types**
- Only NV12 at 30 fps: 1920×1080 and 1280×720.
- The type never changes after `Start`.
- On `SetCurrentMediaType`, the source writes the chosen size into the header's `wanted_w`/`wanted_h` for the tray to follow.

**Delivery: a 30 fps heartbeat**
- A timer at 33.3 ms. Each tick answers the outstanding `RequestSample` with:
  - the newest complete frame in the file, if its sequence is newer; else
  - the last delivered frame again; or
  - black NV12 (Y = 16, UV = 128) before the first frame.
- Each sample gets a fresh timestamp (`MFGetSystemTime`) and a 333,333 × 100 ns duration.
- Apps never see a stall: not during a lens switch, a Wi-Fi hiccup, or a phone step-down.

**Frames file:** `%ProgramData%\PhoneBridge\camera\frames`, created at registration (section 5) and opened by both sides through a file handle and `CreateFileMappingW` with no name.
- Named `Local\` objects don't cross from the tray's session (1) into Frame Server's (0).
- A normal user can't create `Global\` objects (no `SeCreateGlobalPrivilege`).
- A file in ProgramData needs neither.
- There's no cross-session event: the source polls on its heartbeat.

**Layout** (little-endian, fixed):

| Offset | Field |
|---|---|
| 0 | magic `PBCF`, `u32` version = 1 |
| 8 | `u32 wanted_w`, `u32 wanted_h` (written by the source) |
| 16 | `u32 width`, `u32 height` of the frames (written by the tray) |
| 24 | `u64 seq[2]`: a seqlock per slot. Odd while it's being written; each completed write adds 2 |
| 40 | `u64 newest`: the slot index of the last complete frame (0 or 1) |
| 48 | `u64 pts_us[2]` |
| 64 | `u64 tray_heartbeat_ms` (the tray's wall clock; the source shows black after 5 s without an update) |
| 4096 | slot 0: NV12, `1920 × 1080 × 3 / 2` bytes, sized for the largest type |
| 4096 + slot | slot 1 |

**Reader rule:**
1. Read `newest`, then `s1 = seq[newest]`; skip if it's odd.
2. Copy the slot.
3. Read `s2 = seq[newest]`; keep the copy only if `s1 == s2`.
4. A torn read is dropped and the last frame is repeated.

The writer writes into the slot that isn't `newest`, then flips `newest`.

## 5. Registration, lifetime and cleanup

**Registration (one UAC prompt, on first use).** "Use phone as camera" with no registration found runs `pb-tray --register-camera` through `ShellExecuteW` with the verb `runas`. One prompt. The elevated process:
1. **Copies the DLL** to `%ProgramData%\PhoneBridge\camera\pb-camera-source-<version>.dll`.
   - The ACL: SYSTEM and Administrators full control; LocalService and Users read & execute.
   - LocalService can't read inside `C:\Users\<user>` or a Cargo target folder, so the DLL never loads from there.
   - The versioned name lets an update register while Frame Server still has the older DLL loaded. Older versions are deleted when not locked; otherwise at the next registration.
2. **Writes HKLM keys:** `Software\Classes\CLSID\{CLSID}\InprocServer32` = that path, `ThreadingModel` = `Both`.
3. **Creates the frames file** at its full size. The ACL: SYSTEM full; LocalService read & write (its writes are only the `wanted` fields); the **registering user** read & write. No one else.
   - v1 supports one Phone Bridge user per PC. Registering from another account replaces the ACL.
   - Per-user files (the source learning the user from the virtual camera's attributes) are a spike item in Task 1. Otherwise they're deferred.
4. **Exits** with a status code the tray reads.

**Streaming lifetime**
- **Start:** `MFCreateVirtualCamera(MFVirtualCameraType_SoftwareCameraSource, MFVirtualCameraLifetime_Session, MFVirtualCameraAccess_CurrentUser, "Phone Bridge Camera", "{CLSID}", [KSCATEGORY_VIDEO_CAMERA], …)`, then `Start(null)`.
  - It runs on a worker thread in a multithreaded COM apartment, never on the UI thread.
  - Session lifetime means Windows also removes the device when the tray process ends for any reason, including a crash.
- **Stop:** `Stop()` → `Remove()` → `Shutdown()`, always all three, in that order. They run on every path:
  - the menu Stop;
  - the phone ended the session;
  - the link lost after the 30 s grace;
  - tray exit;
  - `WM_ENDSESSION` (Windows sign-out or shutdown).
- **The camera device exists only while streaming.** Between sessions no "Phone Bridge Camera" is listed anywhere.

**Leftover sweep (start-up and shutdown)**
- `MFEnumDeviceSources` with `MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_GUID`, matched by our CLSID in the symbolic link or by the friendly name.
- Any match not owned by this process is removed: `MFCreateVirtualCamera` with the same name and CLSID, then `Remove()` and `Shutdown()`.
- Logged as `CAMERA removed a leftover`.

**"Remove Phone Bridge Camera"** (Settings, with a short explanation). In order:
1. Stop and remove the active camera (the full Stop sequence), then run the leftover sweep.
2. `pb-tray --unregister-camera` via `runas` (one prompt) deletes:
   - the HKLM CLSID keys;
   - the frames file;
   - the DLL, or schedules it with `MoveFileExW(MOVEFILE_DELAY_UNTIL_REBOOT)` if Frame Server still holds it.
3. The setting then reads "Not installed".

**Uninstall:** the uninstaller runs the same `--unregister-camera` steps (it's already elevated).

## 6. Errors
| Situation | What the user sees |
|---|---|
| UAC declined | Pill "The camera needs a one-time permission to install". Nothing changed. |
| Phone declines or the dialog times out | Pill "<phone> didn't share its camera". |
| Camera permission not granted on the phone | Phone notification "Allow camera for Phone Bridge"; the PC pill says the phone declined. |
| Camera busy on the phone (another app) | The phone declines. Its notification says the camera is in use by another app; the PC pill says "<phone> didn't share its camera". |
| Link lost | The source repeats the last frame. The tray retries for 30 s, then black frames, the full Stop, and pill "Phone camera stopped". |
| No hardware decoder for H.265 | Codec fallback, as in the screen spec (H.265 → H.264). |
| Frame Server can't load the DLL | `MFCreateVirtualCamera`/`Start` fails. The tray logs the HRESULT, re-checks the registration (path exists, ACL), and offers "Repair camera" (re-run `--register-camera`). |

## 7. Budgets
- **Phone:** the screen's limits (PSS, swap, kswapd, free memory never under 1 GB). The device test runs 10 minutes of streaming.
- **PC:**
  - tray CPU under 5% of one core at 1080p30 (the GPU decodes and scales);
  - the source's CPU under 2% (one 3 MB copy per tick);
  - the frames file is 6.2 MB (two 3.1 MB slots) plus a 4 KiB header.

## 8. Security and privacy (THREAT_MODEL row 17)
- **Who can start it:** paired PCs only, over the session transport. Each session needs the phone user's yes ("Always allow" is opt-in), plus Android's camera permission. Android's camera indicator and our notification with Stop show the whole time.
- **The frames file:**
  - readable and writable only by the registering user, LocalService (Frame Server) and SYSTEM;
  - it holds only the newest two frames;
  - it's overwritten continuously, and zeroed at Stop and at tray start.
- **Who sees the camera:** like any webcam, every app of that Windows user can open it while it exists. It exists only while streaming.
- **The DLL:** in a folder only administrators can write to, so a normal user process can't replace the code that Frame Server loads.
- **Logs:** sizes, codecs, timings and HRESULTs. Never frame content.

## 9. Testing
**Automated:**
- **Frames file:**
  - a seqlock torn read is rejected;
  - the newest complete slot wins;
  - a writer crash mid-frame (odd seq) is skipped;
  - the header round-trips;
  - the 5 s tray heartbeat timeout → black.
- **RowPitch copy:** a texture with `RowPitch` 2048 for width 1920 gives tightly packed Y and UV planes (byte-exact against a reference).
- **Fit geometry:** portrait 1080×1920 into 1920×1080 → a centred 608×1080 rect; landscape fills; rotation 90/270 swaps.
- **Heartbeat:** black before the first frame; repeats the last frame with increasing timestamps; switches to a new frame when its seq advances.
- **Protocol:** `CameraStart`/`CameraAccept`/`CameraControl` round-trip; the oneof tags stay stable.
- **Transport loopback:** two real nodes: records in order, a lens-switch control reaches the phone, a deliberate close vs a lost link (`by_peer`), decline.
- **Teardown order:** a fake `VirtualCamera` trait records the call sequence; every exit path gives Stop, Remove, Shutdown exactly once.
- **Registration (ignored by default, needs admin; run on this PC):**
  - register → `MFCreateVirtualCamera` + `Start` succeed;
  - `MFEnumDeviceSources` lists "Phone Bridge Camera";
  - Stop sequence → not listed;
  - unregister → the keys and DLL are gone.
- **Leftover sweep (ignored by default):** a child process creates the camera and is killed; the sweep finds nothing, because Session lifetime already removed it, or removes it.

**Device tests (realme + this PC):**
- first use: one UAC prompt; consent on the phone; Windows Camera shows the picture;
- Teams/Meet in a browser: 30 fps, no freezes; switch lens mid-call (picture continues, about 300 ms of repeated frame);
- rotate the phone mid-call: the frame stays 16:9; portrait is pillarboxed, never stretched; no app error;
- indoor light: 30 fps holds (AE range locked);
- Stop: the camera disappears from Device Manager, and the camera list re-enumerates in Windows Camera;
- kill the tray (Task Manager): the device is gone (Session lifetime); the next start's sweep logs nothing left;
- quit the tray mid-call: the device is gone;
- a physical webcam (if present): unaffected before, during and after;
- "Remove Phone Bridge Camera": one prompt; the registry keys, DLL and frames file are gone;
- the 10-minute budget run on the phone.

## Decisions recorded here
- A file-backed mapping in ProgramData for frames, because no named object crosses sessions without privileges.
- Fixed 16:9 NV12 types with pillarboxing, because apps break on mid-call type changes.
- A 30 fps heartbeat in the source, because apps drop a camera that stalls.
- Session lifetime plus a sweep plus Stop → Remove → Shutdown, for the user's clean-system rule.
- The DLL is copied to ProgramData with a versioned name, because LocalService can't read user folders, and for in-use updates.
- One UAC prompt on first use (the user's choice), and one for Remove.
- v1 supports a single Phone Bridge user per PC (one frames file). Per-user files are a spike item.
