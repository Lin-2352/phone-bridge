# Audio sharing (both ways) Implementation Plan

**Spec:** docs/superpowers/specs/2026-11-03-audio-sharing-design.md (binding). **Branch:** feature/audio-full (worktree `phone bridge audio`), from feature/audio-sharing (the user's commits 5476745, cc60db8 + discovery fix + spec).

**Goal:** phone audio plays on the PC and PC audio plays on the phone, each session consented on the device whose audio is shared, mic only as an explicit opt-in mode, off by default, with limits and a persistent Stop indicator.

## Global constraints
- No AI attribution in commits; commit as the local git user; never push; explicit `git add <paths>`.
- One cargo and one gradle command at a time across agents (check `tasklist` first). TDD: failing test first.
- Phone rules: serial 79KRFEJVPVOBQCHQ, no settings changes, never uninstall the app (holds the pairing), user taps every permission/consent prompt.
- Wire limits: sample rate 44100 or 48000, channels 1 or 2, frame length 1..=65536 and a multiple of 2 x channels, one audio session per direction. Raw PCM (no Opus).
- Never record the microphone except in the explicit mic mode (direction 0, source 1).

- Consent: microphone asked every time with no skip option; playback audio: per-direction Settings toggle `audio_skip_confirm_phone_to_pc` / `audio_skip_confirm_pc_to_phone` (default off) skips only Relay's own dialog (Android's projection dialog cannot be skipped); the toggle is ignored for source=microphone; Stop indicator always shown.

## Tasks
1. **Protocol + transport hardening** (`pb-protocol/control.rs`, `pb-transport/audio.rs`, tests, docs/WIRE_PROTOCOL.md): AudioStart gains `direction` (0 phone→PC, 1 PC→phone) and `source` (0 playback, 1 mic); drop `bitrate_kbps`; AudioAccept gains `accepted` + `reason` (declined, unavailable, busy, bad_params, no_permission, app_blocks_capture); AudioStop sendable by either side; validation of every peer field on both sides; bounded drop-oldest queue (~300 ms); write failure surfaces as an event; Drop on the view closes the session; frame alignment/size checks; direction-agnostic sender/receiver; tests over a real node pair for both directions: order, decline, busy, bad params, stop from each side, consumer dropped, truncated frame, hostile accept, old-peer unknown tag.
2. **PC player** (`pb-platform-win/audio_out.rs`): lock-free ring (no locks/allocs in the cpal callback), 100 ms prebuffer, target latency 100–150 ms with drop-to-target on overflow, pure resample/remix to the device format (unit-tested F32/I16/mono↔stereo/44.1↔48 kHz), clear error for unsupported formats.
3. **PC capture** (new `pb-platform-win/audio_capture.rs`): WASAPI loopback of the default output device via cpal, 48 kHz stereo i16 frames of 20 ms, started only on demand, stops cleanly; device change handled.
4. **Phone capture** (`AudioStreamer.kt`): remove the silent mic branch; playback capture requires MediaProjection (Android 14: token obtained after the mediaProjection service type) and RECORD_AUDIO; explicit mic mode with `FOREGROUND_SERVICE_MICROPHONE`; error codes to the PC (no_permission, app_blocks_capture/silence detection); stop joins the worker; reusable buffers.
5. **Phone playback** (new `AudioPlayer.kt`): `AudioTrack` low-latency with a jitter buffer 100–150 ms, drop on overflow, underrun to silence, stop/cleanup.
6. **FFI + services** (`pb-ffi/lib.rs`, `PbService.kt`, consent activities): `pump` handles AudioStart for both directions with the consent gate (phone asks for phone→PC via the projection dialog / mic dialog; PC asks for PC→phone via a tray dialog), events, start/stop calls, notification with Stop, one session per direction, Settings toggles default off.
7. **PC UI** (`pb-tray`): tray menu + Home quick action "Phone audio on this PC" / "PC audio on the phone", consent dialog for PC→phone, active-audio card with Stop, Settings toggles, Timeline entry (metadata only), applog events.
8. **Android UI**: Home quick actions, Settings toggles, mic mode picker dialog, active notification, Timeline entry.
9. **Docs**: WIRE_PROTOCOL, THREAT_MODEL rows (each mode, mic excluded by default), DECISIONS.
10. **Verification on real devices** (campus Wi-Fi): phone→PC with a music app, PC→phone with a PC video, mic mode (prompt, notification, Stop), a call app blocking capture (clear error), link drop, rotation, realme background kill; latency and underrun numbers.

## Parallelism
Task 1 first. Then 2+3 (Rust, PC) and 4+5 (Android) in parallel. Then 6, then 7+8 in parallel, then 9, 10.
