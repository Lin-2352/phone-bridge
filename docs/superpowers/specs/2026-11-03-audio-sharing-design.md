# Audio sharing, both ways (design)

Builds on commits 5476745 and cc60db8 (`feature/audio-sharing`): protocol tags 25-27, `pb-transport/audio.rs`, `AudioOut`, `AudioStreamer.kt`. Those are pieces only: nothing calls them, there is no PC to phone direction, and the Android class silently records the microphone. Review findings: `.superpowers/sdd` is local; the findings are restated here.

## Decisions (user, 2026-11-03)
- **Both directions in this feature**: phone audio plays on the PC, and PC audio plays on the phone.
- **Microphone is a separate opt-in mode.** Never a silent fallback. Chosen on the phone each time, own notification and Android service type, off by default.

## Consent rule (user, 2026-11-03)
- **Microphone: asked every single time**, no setting can remove it (Relay dialog on the phone each session, plus Android's own RECORD_AUDIO grant and the microphone notification).
- **System/playback audio: Relay's own confirmation can be skipped by a per-direction Settings toggle** ("Don't ask when ... starts", default off). Phone audio to PC: Android's MediaProjection dialog still appears every time (platform rule, a single-use token on Android 14; no app can skip it), so the toggle only removes Relay's extra dialog. PC audio to phone: the toggle removes the tray confirmation entirely. The toggle never applies to the microphone, and the active indicator plus Stop stay on regardless.

## Rules that hold for every mode
- **Consent every session, on the device whose audio is shared.** Phone audio: the phone asks (system projection dialog for playback capture; a Relay dialog for the microphone). PC audio: the PC asks (tray dialog). No auto-accept, no resume after a drop.
- **Default off** (a setting per direction), a persistent indicator while streaming (phone notification with Stop; PC tray state with Stop), the session ends on Stop from either side, on a link drop, or when the sharing app is closed.
- **Never** the microphone without the explicit mic mode. If playback capture is unavailable (Android below 10, no projection token, an app that blocks capture) the requester gets a clear error, never mic audio.
- Limits are enforced by the receiver of each field: sample rate 44100 or 48000, channels 1 or 2, frame length 1..=65536 and a multiple of 2 x channels, one audio session per direction at a time. Violations close the stream with a reason; they never allocate from a peer-supplied size.

## Protocol (additions, docs/WIRE_PROTOCOL.md and THREAT_MODEL.md updated)
- `AudioStart` gains `direction` (0 phone to PC, 1 PC to phone) and `source` (0 playback, 1 microphone; microphone only with direction 0); `bitrate_kbps` is removed (raw PCM, so it never meant anything). Tags stay 25-27.
- `AudioAccept` gains `accepted` plus a `reason` code (declined, unavailable, busy, bad parameters, no capture permission, app blocks capture).
- `AudioStop` gets a sender on either side; both sides can send it.
- Old peers ignore unknown tags, so a requester times out in 40 s: add a short "unsupported" probe by feature flag in `Hello` instead of waiting.

## Components
- **Transport** (`pb-transport/audio.rs`): bounded drop-oldest queue (about 300 ms), write failure becomes an event, `Drop` on the view closes the session, frame alignment and size checks, phone-side stop. One implementation serves both directions (the sender is whichever side owns the audio).
- **PC player** (`AudioOut`): lock-free ring buffer, prebuffer of 100 ms, target latency 100-150 ms, resample and remix to the device format (pure, unit-tested conversion functions), clear error for unsupported formats.
- **PC capture** (new): WASAPI loopback of the default output device via cpal, same framing, started only after the phone accepts and the PC user confirmed.
- **Phone capture** (`AudioStreamer.kt`): playback capture requires the MediaProjection token and the `mediaProjection` service type (Android 14: single-use token, obtained after the service type is set); `RECORD_AUDIO` requested at runtime; the silent mic branch is removed and replaced by the explicit mic mode with `FOREGROUND_SERVICE_MICROPHONE`.
- **Phone playback** (new): `AudioTrack` with a jitter buffer (target 100-150 ms), drop on overflow, underrun to silence.
- **FFI and services**: `pump` handles `AudioStart`; `PbEvent` for ask and stop; start/stop calls for both directions; `PbService` consent flow and notification.
- **UI**: tray menu and Home quick action "Phone audio on this PC" and "PC audio on the phone"; Settings toggles (default off); Home shows an active-audio card with Stop (PC) and a notification with Stop (phone). New wording uses "Relay".

## Verification
- TDD for every pure piece: validation, framing, drop-oldest queue, conversions, ring buffer, state machine for consent and stop.
- Transport tests over a real node pair for both directions: ordering, decline, busy, bad parameters, stop from each side, consumer dropped, truncated frame, hostile accept.
- Real-device: phone to PC with a music app, PC to phone with a PC video, mic mode (explicit prompt, notification, Stop), a call app blocking capture (clear error), link drop, rotation, realme background kill.
- Docs: WIRE_PROTOCOL, THREAT_MODEL (new rows for each mode, mic excluded by default), DECISIONS.

## Out of scope now
Opus compression (raw PCM at about 1.5 Mbit/s over Wi-Fi), mixing several sources, per-app audio selection, Bluetooth audio routing, background auto-start.
