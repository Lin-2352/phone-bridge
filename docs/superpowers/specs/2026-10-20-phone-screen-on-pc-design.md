# Phone screen on the PC: design

Status: approved in brainstorming on 2026-10-20 (sub-project 1 of 4). Revised the same day with the external review's points and the user's picture-quality rules (10-bit panel; no artefacts, no shimmering). Next: the brainstorm for sub-project 4, then the implementation plans.

## Purpose and scope
Show the phone's screen in a window on the PC, so the user can see the phone while controlling it from the PC.

- **Priority:** control first. Low lag at 30 fps; reading text must be comfortable.
- **Picture quality rules (user requirement):**
  - no blocky artefacts during motion;
  - no shimmering or "breathing" on still content;
  - no added banding: the phone has a 10-bit panel.
- **Later upgrade (not in this version):** "phone apps on the PC" quality (60 fps, sound).

### The sub-projects
| # | Sub-project | Relation to this one |
|---|---|---|
| 1 | Phone screen on the PC | **This document.** Builds the shared phone → PC video module. |
| 2 | Continuity Camera | Reuses that module (its plan is already drafted). |
| 3 | PC screen on the phone | The reverse direction. |
| 4 | Phone notifications in the Windows notification centre | Separate. Brainstormed next; its implementation plan comes before this one's, because it is small. |

### Out of scope
- Audio.
- 60 fps.
- Truly switching the phone's panel off. A normal app can't do that while capturing; see "Dim / on".
- Any route that needs USB debugging (scrcpy-style).

## Approach
In-app screen capture with hardware video encoding, using public Android APIs only:

```
Android MediaProjection → VirtualDisplay → MediaCodec (H.265, else H.264)
  → QUIC uni stream (pb-transport) → Media Foundation hardware decode (D3D11)
  → Phone Bridge window on the PC
```

**Codec, in order of preference:**
1. H.265 Main10 (10-bit), when the phone's encoder offers it for surface input.
2. H.265 Main (8-bit).
3. H.264 High, if the PC has no H.265 decoder.

The PC reports which codecs and profiles it can decode at start; the phone picks the best one both sides support.

**The dev PC:** has the HEVC Video Extensions installed, plus an RTX 4060 and Intel UHD Graphics (both decode H.265 Main and Main10 in hardware).

## 1. Data flow and protocol

**Start:**
1. The user turns on **"Show phone screen"** in the tray menu or flyout. The window opens.
2. The PC sends `ScreenStart{max_long_side, fps = 30, codecs = [h265_main10, h265, h264], keep_screen_on}`.

**Consent on the phone:**
1. The same ask-first flow as PC control: popup with Allow / Always allow / Deny.
2. Then Android's own capture prompt, which Android shows every session.
3. The phone answers with the chosen codec, size and rotation.

**Stream:** one QUIC unidirectional stream, phone → PC.
- It begins with the header `PBV1`.
- Then records: `u32 len | i64 pts_us | u8 flags | payload`.
- Flags: keyframe, codec config.
- A record over 4 MiB ends the session.
- **Config record** (flag "codec config"): payload = `u16 rotation (0/90/180/270) | u16 width | u16 height | codec headers (VPS/SPS/PPS)`. It is sent at start and after every rotation or encoder reset, and is always followed by an IDR keyframe.

**Shared video module** (framing, pacer, decoder glue), used by both this feature and Continuity Camera:
- The PC sends feedback (the newest presented pts) every 100 ms.
- **Lag counts in-flight frames only:** lag = pts of the newest frame *sent* − the newest pts the PC *confirmed*. It is never wall-clock time, so a still screen (nothing sent) can never look like lag.
- **On a drop:** if lag is over 150 ms, or the send queue (1 MiB cap) is full, the phone drops frames until the next keyframe, asks the encoder for one, and multiplies the bitrate by 0.7 (floor 1.5 Mbit/s).
- **Recovery:** after 1 s with no drops and the send queue under 256 KiB, the bitrate rises by 10% per second, up to the target.
- **Targets** (VBR, at the default size, 30 fps):

| Codec | Target | Peak |
|---|---|---|
| H.265 (Main10 or Main) | about 12 Mbit/s | about 20 Mbit/s |
| H.264 | about 16 Mbit/s | about 25 Mbit/s |

  A still screen sends almost nothing, so the average is far lower.

**Input from the window:**
- A new record type: absolute touch at (x, y), normalised to 0–1 so it doesn't depend on resolution.
- It is carried on the existing input stream.

**Stop:** closing the window, the toggle, or Stop on the phone. Either side ends it through `SessionCloser`, the same pattern as control.

`docs/WIRE_PROTOCOL.md` documents the new messages, the record format, the config record and the codec negotiation.

## 2. The PC window and input

**Window:**
- Borderless, Fluent-style (the existing look), titled with the phone's name.
- Resizable; keeps the phone's aspect ratio and flips with the phone's rotation.
- **Top bar:**
  - **Pin:** always on top, or behave like a normal window. The choice is remembered.
  - **"Phone screen: dim / on".**
- **Bottom bar:** Back, Home and Recents.

**Decode and render (low latency, one GPU):**
- The D3D11 device is created on the adapter that drives the window's monitor (on hybrid laptops usually the Intel iGPU). The same device is given to the decoder through `IMFDXGIDeviceManager`, so frames are never copied between GPUs and the RTX 4060 stays asleep.
- The decoder MFT gets `CODECAPI_AVLowLatencyMode = TRUE` before streaming: one frame in, one frame out.
- Decoded frames stay on the GPU (NV12, or P010 for Main10). The D3D11 video processor converts them with BT.709 limited range, matching what the phone signals.
- The swap chain uses `DXGI_SWAP_CHAIN_FLAG_FRAME_LATENCY_WAITABLE_OBJECT` with a maximum frame latency of 1. Its format is `R10G10B10A2` where the output supports it, else `B8G8R8A8`.
- On a still screen the last frame simply stays; nothing is re-presented.

**Mouse inside the window:**

| Action | On the phone |
|---|---|
| Left press, drag, release | Finger down, move, up at that exact spot (the streamed finger already built for control) |
| Wheel | Smooth scroll: a synthetic finger drag at the cursor (the existing streamed finger), not `ACTION_SCROLL`, so it works in every app |
| Ctrl+Wheel | Pinch out / in, centred on the cursor (a two-finger gesture) |
| Right-click | Back |
| Middle-click | Home |

Outside the window the mouse belongs to the PC; nothing is captured.

**Keyboard while the window has focus:**
- Typing goes through the input-method path (already fixed for control).
- Esc = Back.
- Ctrl+A / C / V / X do the same as now.
- Alt+F4 or Ctrl+W closes the window.

**Files:** dropping files on the window sends them to the phone with the existing file transfer. Progress shows in the status pill.

**Ctrl+Alt+P captured mode** keeps working, with or without the window. While captured, the window also shows the phone's cursor.

**Status texts:**
- "Waiting for phone…" while consent and the capture prompt are pending.
- "Phone stopped sharing its screen" when the phone side ends it.
- "Hidden by the app (secure screen)" while frames are fully black because of a secure screen.
- Status messages that matter when the window isn't in front ("<phone> didn't share its screen", "Phone stopped sharing its screen") also go to the Windows notification centre, once sub-project 4's toast path exists.

## 3. Phone side

**Start flow:**
- After consent, a small transparent activity launches Android's capture prompt. Android requires the prompt every session, from a visible activity.
- If the phone is locked, a notification asks: "Unlock to share your screen with <PC>".
- On Android 14 and later, the prompt may offer "single app" capture. It works through the same virtual display; Home and Recents behave cleanly only with full-screen capture.

**Foreground service type:**
- `mediaProjection` is added only while mirroring, and dropped afterwards. Android 14's rules for this type are followed.
- The manifest declares `FOREGROUND_SERVICE_MEDIA_PROJECTION`.

**Capture:**
- The virtual display renders straight into the encoder's input surface. Zero CPU copies, no preview.
- **Size:**
  - The long side is capped at **1920** by default. A "Sharp" option uses the native resolution.
  - Both dimensions are then rounded to the encoder's reported alignment (`VideoCapabilities` width and height alignment, at least 16) and confirmed with `isSizeSupported`. On the 1080×2412 realme this gives 864×1920.
  - Normalised touches and aspect-correct rendering hide the few pixels lost to rounding.

**Colour and bit depth** (the phone has a 10-bit panel):
- Android composes app screens in 8-bit SDR, so the captured frames are 8-bit. The encoder signals that honestly: `KEY_COLOR_STANDARD = BT709`, `KEY_COLOR_TRANSFER = SDR_VIDEO`, `KEY_COLOR_RANGE = LIMITED`. The PC converts with the same values.
- **10-bit encoding:** HEVC Main10 (`HEVCProfileMain10`) is used when the encoder offers it for surface input. Encoding at 10 bits avoids adding banding in smooth gradients (dark wallpapers, photos), even from 8-bit frames.
- Otherwise 8-bit Main, then H.264 High.
- Main10 with surface input on the realme's encoder is checked first, in the plan's first spike.

**Encoder and rate control** (no artefacts, no shimmering):
- Low latency, no B-frames.
- **VBR** (`BITRATE_MODE_VBR`) at the targets in section 1.
- **A QP ceiling** (`KEY_VIDEO_QP_MAX`, API 31+; the realme is on Android 13 = API 33), so fast motion never turns blocky. When the network can't carry that quality, the pacer lowers the rate and, under pressure, the size and frame rate (section 4), rather than letting artefacts through.
- **No periodic keyframes:** `KEY_I_FRAME_INTERVAL` is set very long, so there is no keyframe "pulse" every few seconds. Keyframes come only when needed: at start, after a pacer drop, on rotation, when the window is restored, and on a decoder error.
- **Still screens never shimmer:**
  - Unchanged frames are not re-sent, so a still screen costs almost nothing.
  - `KEY_REPEAT_PREVIOUS_FRAME_AFTER` (about 100 ms) lets the encoder send a few refining repeats once the screen stops changing, so text is sharp right after a scroll.
  - Repeats are sent (they are tiny). Changed in device tests: dropping a frame the encoder called a repeat smeared the picture, because every frame is a reference for the next. Nothing is ever dropped except whole runs up to the next key frame. The PC keeps showing the last frame, which cannot shimmer.
- **Fallback:** if the realme's encoder doesn't accept VBR with surface input, CBR is used at the same rate, with the same QP ceiling and repeat rule. A device check compares both on static text, gradients and slow scrolling, and the better one is kept.

**Rotation:**
1. Release or reset the codec.
2. Configure it again with width and height swapped, and create a new input surface.
3. `VirtualDisplay.resize(w, h, dpi)` and `setSurface(newSurface)`.
4. Send the config record (rotation, size, headers), then an IDR keyframe.

**Input:** absolute touches drive the streamed finger at exact positions; keys use the input-method path.

**Dim / on:**
- A 1×1 `TYPE_ACCESSIBILITY_OVERLAY` window with `FLAG_NOT_TOUCHABLE | FLAG_NOT_FOCUSABLE | FLAG_KEEP_SCREEN_ON`.
- "Dim": its `screenBrightness` is 0.0. "On": −1 (the system's brightness); it only keeps the phone awake.
- To verify on the realme: that a 1×1 window's brightness applies to the whole display. If it doesn't, a full-screen transparent, not-touchable accessibility overlay is used instead.
- Brightness comes back when the power button turns the screen off and on, and from a "Restore brightness" action on the sharing notification. Physical touches on the phone are not detected (not reliably possible for this app).
- Both are removed when mirroring stops.
- True panel-off is not possible for a normal app: the phone would sleep and the mirror would go black.

**Secure screens** (banking apps, password fields, biometric prompts): Android captures them as black by design. Touch and keyboard input still work, and the PC window shows the "secure screen" note.

**On-phone controls:**
- A notification "Sharing screen with <PC>" with Stop (and "Restore brightness" while dimmed).
- The main-page card shows the sharing state with a Stop button, like control.

## 4. Lightweight budgets

The lesson from 2026-10-20: Android Studio's mirroring drove this phone into swap thrash and watchdog reboots. This feature must stay inside measured limits.

**Budgets** (checked on the realme with the light sampler, during a 10-minute mirror):

| Measure | Limit |
|---|---|
| Phone Bridge memory (PSS) | under 60 MB while mirroring |
| Rust send queue | 1 MiB max |
| Swap | under 200 pages/s on average |
| kswapd | under 10% |
| Kernel CPU | under 15% |
| Free memory | never under 1 GB |

For comparison, Studio's mirroring reached 25,000 swap pages/s, kswapd at 90% and 0.77 GB free.

**Rules:**
- **Step-down:** under memory pressure (`onTrimMemory`, or under 1.2 GB free) or thermal status "severe", drop to 720-class size at 15 fps. The window says why. The QP ceiling stays, so quality per frame holds.
- **Encode only while visible:** a minimised window sends `active = false`, and the phone pauses capture inside the session. On `active = true` the phone requests a keyframe at once (`PARAMETER_KEY_REQUEST_SYNC_FRAME`), so the first frame shown is fresh.
- **One process:** everything runs inside the Phone Bridge process; no helper agents.
- **Testing conditions:** device tests run with Android Studio closed and the light sampler on. Results go into `docs/DECISIONS.md`.

## 5. Errors and testing

**Errors:**
- **Consent declined or capture prompt cancelled:** pill "<phone> didn't share its screen"; the window closes.
- **Phone locked or link lost:** the last frame stays, dimmed, with "Reconnecting…". It retries for 30 s, then shows "Phone stopped sharing its screen".
- **No H.265 decoder, or decoding fails:** the PC asks again with the next codec (Main10 → Main → H.264) automatically.
- **Phone Bridge killed by Android:** the window says so. The existing "not running, tap to turn on" notice covers the phone side.

**Automated tests:**
- Framing, and config-record encode/decode.
- Pacer: drop to keyframe; bitrate back-off with its floor; recovery at +10% per second only after 1 s clean with the queue under 256 KiB; lag stays 0 while nothing is in flight (idle gaps are not lag).
- Size rounding to the encoder's alignment.
- Loopback with two real nodes and synthetic frames: ordering, stop from either side, codec fallback.
- Absolute-touch scaling, including rotation.

**Device tests** (realme, Android Studio closed, light sampler running):
- the consent and capture prompts;
- tap, drag, scroll, Ctrl+Wheel pinch (a photo or map) and type through the window;
- rotate: a correct landscape image, no stretching; pin and unpin; resize;
- dim on and off; brightness returns on the power button;
- drop a file on the window;
- minimise pauses capture; restore shows a fresh frame at once;
- close stops capture and drops the service type;
- a secure screen shows black, with input still working;
- **picture quality:**
  - a still screen for 60 s: no shimmer or keyframe pulse; frames sent drop to 0 after the repeats;
  - a dark gradient wallpaper: no banding (Main10 compared with Main);
  - fast scrolling through a photo feed: no blockiness;
  - text sharp right after a fast scroll stops;
  - VBR compared with CBR if both are available;
- the 10-minute budget run;
- glass-to-glass latency under 100 ms (low-latency decode and the waitable swap chain), measured by filming a stopwatch on the phone as shown in the PC window.

## Decisions recorded here
- H.265 Main10 when the encoder offers it, else H.265 Main, else H.264 (negotiated at start).
- Colour signalled as BT.709 limited range, SDR, on both ends.
- VBR with a QP ceiling and no periodic keyframes (CBR only as a fallback); at most 3 refining repeats, then silence on a still screen.
- Lag measured on in-flight frames only; back-off ×0.7 (floor 1.5 Mbit/s); recovery +10% per second after 1 s clean.
- The config record carries rotation, size and codec headers, followed by an IDR; a fixed rotation sequence.
- Encoder size rounded to the encoder's alignment (864×1920 on the realme).
- PC: low-latency decode, a waitable swap chain, one D3D11 device on the window's adapter shared with the decoder.
- Absolute touch input for the window; wheel as a synthetic drag; Ctrl+Wheel as a pinch. Ctrl+Alt+P relative control is kept alongside it.
- Long side 1920 by default; "Sharp" for native resolution.
- Backlight dimming through a 1×1 accessibility overlay (verified on the device), not panel-off.
- Secure screens show black with a note; input still works.
- The shared video module is built here and reused by Continuity Camera.
