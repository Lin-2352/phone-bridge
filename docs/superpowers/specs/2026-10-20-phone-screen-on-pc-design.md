# Phone screen on the PC: design

Status: approved in brainstorming on 2026-10-20 (sub-project 1 of 4). Next: implementation plan.

## Purpose and scope
Show the phone's screen in a window on the PC, so the user can see the phone while controlling it from the PC.

- **Priority:** control first. Low lag at 30 fps; reading text must be comfortable.
- **Later upgrade (not in this version):** "phone apps on the PC" quality (60 fps, sound).

### The sub-projects, in build order
| # | Sub-project | Relation to this one |
|---|---|---|
| 1 | Phone screen on the PC | **This document.** Builds the shared phone → PC video module. |
| 2 | Continuity Camera | Reuses that module (its plan is already drafted). |
| 3 | PC screen on the phone | The reverse direction. |
| 4 | Phone notifications in the Windows notification centre | Separate. |

### Out of scope
- Audio.
- 60 fps.
- Truly switching the phone's panel off. A normal app can't do that while capturing; see "Dim / on".
- Any route that needs USB debugging (scrcpy-style).

## Approach
In-app screen capture with hardware video encoding, using public Android APIs only:

```
Android MediaProjection → VirtualDisplay → MediaCodec (H.265, else H.264)
  → QUIC uni stream (pb-transport) → Media Foundation hardware decode (D3D11 NV12)
  → Phone Bridge window on the PC
```

**Codec:** H.265 by default. If the PC has no H.265 decoder, the phone falls back to H.264 automatically. The PC reports which codecs it can decode at start.

**The dev PC:** has the HEVC Video Extensions installed, plus an RTX 4060 and Intel UHD Graphics (both decode H.265 in hardware).

## 1. Data flow and protocol

**Start:**
1. The user turns on **"Show phone screen"** in the tray menu or flyout. The window opens.
2. The PC sends `ScreenStart{max_long_side, fps = 30, codecs = [h265, h264], keep_screen_on}`.

**Consent on the phone:**
1. The same ask-first flow as PC control: popup with Allow / Always allow / Deny.
2. Then Android's own capture prompt, which Android shows every session.
3. The phone answers with the chosen codec, size and rotation.

**Stream:** one QUIC unidirectional stream, phone → PC.
- It begins with the header `PBV1`.
- Then records: `u32 len | i64 pts_us | u8 flags | payload`.
- Flags: keyframe, codec config, rotation change.
- A record over 4 MiB ends the session.

**Shared video module** (framing, pacer, decoder glue), used by both this feature and Continuity Camera:
- The PC sends feedback (the last presented pts) every 100 ms.
- If the phone is more than 150 ms behind, or its send queue (1 MiB cap) is full, it drops frames until the next keyframe, asks the encoder for one, and multiplies the bitrate by 0.7.
- It then recovers gradually.
- Targets: about 4 Mbit/s for H.265 and about 6 Mbit/s for H.264, at the default size.

**Input from the window:**
- A new record type: absolute touch at (x, y), normalised to 0–1 so it doesn't depend on resolution.
- It is carried on the existing input stream.

**Stop:** closing the window, the toggle, or Stop on the phone. Either side ends it through `SessionCloser`, the same pattern as control.

`docs/WIRE_PROTOCOL.md` documents the new messages, the record format and the codec negotiation.

## 2. The PC window and input

**Window:**
- Borderless, Fluent-style (the existing look), titled with the phone's name.
- Video: hardware-decoded NV12 is rendered on the GPU (D3D11) with the correct aspect. No CPU copy; low-latency present.
- Resizable; keeps the phone's aspect ratio and flips with the phone's rotation.
- **Top bar:**
  - **Pin:** always on top, or behave like a normal window. The choice is remembered.
  - **"Phone screen: dim / on".**
- **Bottom bar:** Back, Home and Recents.

**Mouse inside the window:**

| Action | On the phone |
|---|---|
| Left press, drag, release | Finger down, move, up at that exact spot (the streamed finger already built for control) |
| Wheel | Smooth scroll |
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

## 3. Phone side

**Start flow:**
- After consent, a small transparent activity launches Android's capture prompt. Android requires the prompt every session, from a visible activity.
- If the phone is locked, a notification asks: "Unlock to share your screen with <PC>".

**Foreground service type:**
- `mediaProjection` is added only while mirroring, and dropped afterwards. Android 14's rules for this type are followed.
- The manifest declares `FOREGROUND_SERVICE_MEDIA_PROJECTION`.

**Capture:**
- The virtual display renders straight into the encoder's input surface. Zero CPU copies, no preview.
- Size: the long side is capped at **1920** by default (about 860×1920 on the 1080×2412 realme). A "Sharp" option uses the native resolution.

**Encoder:**
- Low latency, no B-frames, a keyframe every 2 s.
- Unchanged frames are not re-sent, so a still screen costs almost nothing.

**Rotation:** the virtual display is resized and the rotation flag is sent.

**Input:** absolute touches drive the streamed finger at exact positions; keys use the input-method path.

**Dim / on:**
- "Dim": an invisible overlay window sets the backlight to minimum and keeps the phone awake.
- "On": only keeps it awake.
- Both are removed when mirroring stops.
- True panel-off is not possible for a normal app: the phone would sleep and the mirror would go black.

**On-phone controls:**
- A notification "Sharing screen with <PC>" with a Stop action.
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
- **Step-down:** under memory pressure (`onTrimMemory`, or under 1.2 GB free) or thermal status "severe", drop to 720-class size at 15 fps. The window says why.
- **Encode only while visible:** a minimised window sends `active = false`, and the phone pauses capture inside the session.
- **One process:** everything runs inside the Phone Bridge process; no helper agents.
- **Testing conditions:** device tests run with Android Studio closed and the light sampler on. Results go into `docs/DECISIONS.md`.

## 5. Errors and testing

**Errors:**
- **Consent declined or capture prompt cancelled:** pill "<phone> didn't share its screen"; the window closes.
- **Phone locked or link lost:** the last frame stays, dimmed, with "Reconnecting…". It retries for 30 s, then shows "Phone stopped sharing its screen".
- **No H.265 decoder, or decoding fails:** the PC asks again with H.264 automatically.
- **Phone Bridge killed by Android:** the window says so. The existing "not running, tap to turn on" notice covers the phone side.

**Automated tests:**
- Framing and pacer: drop to keyframe, bitrate back-off and recovery.
- Loopback with two real nodes and synthetic frames: ordering, stop from either side, codec fallback.
- Absolute-touch scaling, including rotation.

**Device tests** (realme, Android Studio closed, light sampler running):
- the consent and capture prompts;
- tap, drag, scroll and type through the window;
- rotate; pin and unpin; resize; dim on and off;
- drop a file on the window;
- minimise pauses capture;
- close stops capture and drops the service type;
- the 10-minute budget run;
- glass-to-glass latency under 100 ms, measured by filming a stopwatch on the phone as shown in the PC window.

## Decisions recorded here
- H.265 by default, with automatic H.264 fallback (negotiated at start).
- Absolute touch input for the window; Ctrl+Alt+P relative control is kept alongside it.
- Long side 1920 by default; "Sharp" for native resolution.
- Backlight dimming, not panel-off.
- The shared video module is built here and reused by Continuity Camera.
