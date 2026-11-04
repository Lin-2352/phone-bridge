# Relay: threat model

No external review yet. Update whenever a feature adds a new trust boundary.

## What we protect
| Asset | Why it matters |
|---|---|
| Clipboard contents | Often passwords, tokens, personal text |
| Files in transit | Arbitrary private data |
| Input control of the phone | Remote control of an unlocked phone is full account takeover |
| Camera and microphone streams | Live surveillance if hijacked |
| Phone notifications | Message text, senders, one-time codes; acting on them (reply, answer a call) speaks for the user |
| The phone's screen | Everything on it, live; input from the window acts on the unlocked phone |
| Device identity keys | Whoever holds one can impersonate the device |
| Metadata (who pairs with whom, when, from where) | Location and relationship leakage |

## Actors
- **Owner**: the user, with both devices.
- **Network attacker**: on the same Wi-Fi, or controlling a router/ISP/relay. Can read, drop, modify and inject packets. Cannot break standard cryptography.
- **Nearby attacker**: within BLE/Wi-Fi range, can broadcast discovery packets and attempt pairing.
- **Malicious relay operator**: sees connection metadata (IPs, timing, sizes) of anyone using their relay. Must never see content.
- **Malicious app on the same device**: another app trying to read clipboard, abuse the accessibility service, or talk to our local sockets.
- **Thief with the unlocked or locked device**: physical access.

Out of scope (for now): a fully compromised OS or kernel, hardware attacks, nation-state targeted malware on an endpoint, supply-chain compromise of the Rust toolchain.

## Trust boundaries and main threats
| # | Boundary | Threat | Mitigation (status) |
|---|---|---|---|
| 1 | Pairing | Man in the middle during first contact; a paired or pasted peer planting extra store entries | QR bootstrap carries the peer public key; 6-digit SAS compared by the user; nonce commitment before reveal. Addresses a peer sends (or a pasted code carries) are kept only when they parse as `ip:port`, at most 6; names lose control and hidden bidi characters and are capped at 64 characters; the store writes one peer per line and refuses lines with extra fields (**implemented** in `pb-transport::pairing` and `pb-identity`, tested; QR rendering still planned) |
| 2 | Pairing | Stranger pairs by guessing or spamming requests | Codes exist only while the host shows one; single use, 2-minute expiry, burned after 3 wrong tokens (**implemented**) |
| 3 | Transport | Eavesdropping or tampering | QUIC with TLS 1.3, peers authenticated by pinned Ed25519 keys (iroh endpoint ids). Only keys in the paired-peer store may open sessions; others are closed with code 403 before any stream is read; revocation takes effect for new connections and closes open ones; a removal or pairing that cannot be saved to disk is reported and changes nothing, so a revoked device cannot come back after a restart (**implemented**, tested in-process and on the real phone) |
| 4 | Discovery | Tracking a device by its broadcasts; planting addresses | No device name in discovery. Probes carry a tag `SHA-256("pb-discovery/1" \|\| peer id \|\| 30-minute window)[..16]`, which changes every 30 minutes but is **not keyed**: anyone who knows a device's public key (for example a formerly paired device) can compute it, ask whether that device is on the LAN and get its IP and QUIC port, or answer with a planted address. Planted addresses are only dial candidates: a connection must still prove the paired key, and only the address an authenticated connection uses is saved (**implemented**; keying the tag with a secret exchanged at pairing is **planned**) |
| 5 | Relay | Operator reads content | End-to-end encryption between devices; relay only forwards QUIC packets. Self-hostable relay; default build must not use third-party relays without consent (Spike 1 uses none) |
| 6 | Relay | Operator learns metadata | Cannot be eliminated; documented. Users can self-host. |
| 7 | Clipboard | Silent theft of secrets | Universal Clipboard is opt-in (off by default); private content (Windows exclusion formats, Android `EXTRA_IS_SENSITIVE`) never sent; received sensitive text excluded from Win+V history/cloud; echo guard; the Android clipboard-reading activity is not exported so other apps cannot trigger it (**implemented**) |
| 8 | Files | Path traversal, zip bombs, overwriting files | Names sanitised (last component only, reserved/control chars, Windows device names, length), unique names, partial file + SHA-256 check before rename, 4 GiB cap; auto-accept only from paired devices, otherwise prompt (**implemented**, tests incl. hostile names, forged hash, interrupted sender) |
| 9 | Input control | Hijacking the phone | Paired devices only; explicit session started by the user (touchpad screen, or Ctrl+Alt+P on the PC); PC side gated by a setting, phone side by an accessibility service the user must enable in Android settings (off by default, revocable there at once); visible indicators (toast and cursor on the phone, notification on the PC); held keys and buttons released when a session ends; PC hooks installed only during a session (**implemented**) |
| 10 | Accessibility service | Abuse by other apps or policy loss | Only in the `foss` flavor; no data collection; never ships in the Play flavor (**decided**, see DECISIONS.md) |
| 11 | Parsers | Memory corruption or DoS from malformed input | Size limits before allocation, no panics on bad input, fuzz targets (**implemented for control frames, input records and audio datagrams**; fuzzing harness **planned**) |
| 12 | Keys at rest | Theft from disk or backup | Seed wrapped by Android Keystore / Windows DPAPI; excluded from backups (**implemented**: DPAPI on Windows, Keystore AES-GCM on Android; unseal failure is an error, never a silent new identity; Android backup disabled) |
| 13 | Updates | Malicious update | Signed releases, reproducible builds where possible (**planned**) |
| 14 | Resource use | Battery or CPU drain used as DoS | At most 16 incoming handshakes at once (5 s each) and 2 pairing connections from their own pool; input and notification actions are rate-limited per session; control frames 64 KiB. Not per peer: a paired device can open many sessions and each file offer is checked against free space on its own (**partly implemented**); idle budget tracked in DECISIONS.md |
| 15 | Phone notifications | Content leaks, forged actions, malformed images | Only to paired peers over the session transport (row 3), and only after the user grants Android Notification access and leaves "Notifications on PC" on. Nothing is stored except toast images (a private temp folder, PNG/JPEG checked by magic bytes, hex-only names, 50 MB cap, emptied at tray start) and the app names seen (for the mute switches). Logs carry the package and a key hash, never text. The phone acts only on keys it currently shows, through the apps' own intents; actions and records are rate-limited (10 actions/s; records paced at 40/s, a session over 200/s on arrival is closed) and size-limited. Toast arguments carry the phone id and notification key so toasts work after a restart; they stay in the user's own notification centre. A toast is only kept in step (and its clearing mirrored to the phone) once Windows has actually kept it. Toast XML escapes all text. Content is hidden while the PC is locked (default), always for notifications the app marks private, and kept off the lock screen (default); toasts that arrived while locked are filled in quietly at unlock. One-time codes copied on the PC are kept out of clipboard history and cloud clipboard, never synced back, and cleared after 60 s if unchanged. The Windows notification centre keeps shown toasts until they are cleared or the phone removes them (**implemented**; device tests pending) |
| 16 | Phone screen on the PC | Unwanted viewing or control of the phone | Paired PCs only, over the session transport (row 3). Each session needs the phone user's yes ("Always allow" is opt-in) and Android's own capture prompt, which Android shows every session. One consent covers the window's input for that session only. A sharing notification with Stop shows the whole time; Stop on the phone ends it, and the phone then declines that PC quietly for 30 s. After a dropped link the capture waits (paused) 30 s for the same PC only. Secure screens (banking, passwords) are captured as black by Android. Records are size-limited (4 MiB) and input is rate-limited like PC control (**implemented**; device tests pending) |
| 17 | Phone camera on the PC | Unwanted use of the camera; other users or programs reading the pictures; a replaced source DLL; leftovers on the PC | Paired PCs only, over the session transport (row 3). Each session needs the phone user's yes ("Always allow" is opt-in) plus Android's camera permission; Android's camera indicator and a notification with Stop show the whole time, and the service takes the `camera` type only while streaming. On the PC the camera ("Phone Bridge Camera") exists only while streaming: Session lifetime, Stop, Remove and Shutdown on every exit path, a sweep for leftovers at start-up and exit, and one UAC prompt each to install and to remove. The frames file lives in `%ProgramData%\PhoneBridge\camera` and is readable and writable only by SYSTEM, LocalService (Frame Server) and the installing user; it holds two frames and is zeroed at stop. The source DLL sits in a folder only administrators can write, with a versioned name. Like any webcam, apps of that Windows user can open the camera while it exists. Logs carry sizes, timings and HRESULTs, never pictures (**implemented**; device tests pending) |
| 18 | Phone audio on the PC | Hearing what the phone plays without the owner knowing; capturing a call or a protected app | Paired PCs only, over the session transport (row 3), started from the PC. The phone asks every session: Relay's own question (the only part the Settings switch "Share this phone's audio without asking" can remove) and then Android's screen-capture prompt, which no setting removes and which is single use. Apps that forbid capture (calls, some media apps) stay silent and Relay ends the session after about 2 seconds of silence with a message; it never falls back to the microphone. A notification with Stop shows the whole time (Android shows its cast indicator), Stop on either side or a lost link ends it, nothing resumes by itself. Audio travels as raw PCM in QUIC datagrams (an 8-byte header, then at most 64 KiB of whole samples), each checked before use; a malformed one ends the session. About 100 ms is buffered by the sender and 100 ms queued by the receiver, oldest dropped first, so a Wi-Fi stall leaves no backlog of old sound. The timeline and the logs keep who, which way and how long, never audio (**implemented**; device tests in the audio report) |
| 19 | PC audio on the phone | The PC playing sound on the phone, or capturing PC audio, unasked | Paired phones only. The PC asks its own user every session ("Play this PC's audio on <phone>?"), unless the user turned on "Play PC audio without asking" in Relay settings (off by default). The phone checks the format and that no PC audio session runs, then plays through its media output; it shows a notification with Stop, and Stop on either side or a lost link ends it. The PC captures only the default output device (loopback) and only while the session runs; the capture thread is stopped on every exit path, and pausing Relay ends it. Nothing is stored beyond who, which way and how long (**implemented**; device tests in the audio report) |
| 20 | Microphone | Silent recording of the phone's microphone | **Excluded by default and unreachable in this version.** The microphone is a separate mode (`source = 1`), never a fallback for playback capture. The phone would ask on every request with its own dialog that no setting skips, plus Android's RECORD_AUDIO grant, the microphone foreground-service type and Android's microphone indicator, with a "Microphone is being shared" notification and Stop. No PC UI sends this request, so the microphone cannot be started from the apps today; a request from a hand-made client still meets all of those gates (**implemented on the phone, not reachable from the apps**) |

## Known limitations found so far
- Relay metadata exposure (row 6) is inherent to any relay design.
- `pb --auto-accept` and the app's adb intent hooks skip or script the SAS decision; they are test-only and must not ship in release builds.
- Spike 1 used an unauthenticated test protocol on throw-away keys; it proves transport, not authorisation. Do not treat spike code as secure.
- The first GSO send failure on USB tethering (DECISIONS.md) is an availability bug, not a confidentiality one, but a flaky transport invites insecure retries; do not add silent fallbacks.

## Audio capture without Android's prompt (2026-11-04)
Both of these remove a protection Android normally gives, so both are the user's explicit choice, and Relay's notification with Stop is then the only sign of a share. The rule that the PC can only ask, and that the microphone asks every time, is unchanged.

**The hands-free grant.** If the user allows `PROJECT_MEDIA` for Relay outside the app, Android approves its capture prompt automatically, for audio and for screen sharing alike, with the capture icon still shown. Only a paired PC can request a capture, subject to Relay's own settings. Undone with `appops set ... PROJECT_MEDIA default`.

**The audio helper (switched off, see `HELPER_OFFERED`; kept here for if it returns).** After its one-time setup:
- Relay holds a paired adb identity, which amounts to shell access to this phone. It is kept outside backups, encrypted under a key in Android's keystore, and used only for fixed commands. Anything that takes over Relay's process can use it: that is the main cost of this feature, and why it is off until the user sets it up after being told this.
- Relay holds `WRITE_SECURE_SETTINGS`, which covers far more than Wireless debugging. Same reasoning.
- While starting the helper, Wireless debugging is on for a few seconds, listening on the network and open to paired keys only. It goes back to how the user had it.
- The helper records everything routed to `REMOTE_SUBMIX`, including apps that opt out of capture, with no capture icon. Android routes media there; alarm, ringtone and notification sounds stayed on the speaker when measured.
- Any local app can reach the helper's loopback port. It gets a random nonce and nothing more without the secret (tested). Any app could also occupy the port first: the helper then can't start and Relay falls back, and Relay takes no audio from whatever holds the port, because it must prove the secret too.
- Other apps can't read the secret: it is in Relay's private storage and the shell's environment, and it never goes on a command line or in a log.
- A dead helper or Relay stops recording: recording exists only while Relay holds the connection. The helper exits when the APK it came from goes away.
- `libadb-android` has never had a security audit, and it runs in Relay's process during setup and starts.
- Turning the helper off stops it, revokes the grant when it can reach adb, and deletes the identity. The phone's Paired devices list still shows Relay until the user removes it, though the key is gone.

**Silencing the speaker.** The effect that silences the phone during a share needs no special access. Its risk is sound: if Relay dies, the speaker can play at the share's level until the service restarts and puts the volume back. Relay's notification stays the sign that a share is running.

## Rules that follow from this
1. No feature acts on a peer that is not in the paired-peer store.
2. Anything that reads from or controls the other device needs its own explicit, revocable permission.
3. All parsing of remote bytes is bounded and fuzzed before a feature ships.
4. No telemetry, no third-party services by default.

## Review findings fixed (2026-10-19)
- Exported `MainActivity` honoured test extras (`pair_code`, `confirm`) in release builds: any installed app could pair the phone with an attacker PC without the SAS check. Now only in debuggable builds.
- Unauthenticated LAN hosts could hold pairing connections open indefinitely: pairing ALPN refused unless a code is shown; handshake steps time-boxed (incoming QUIC handshake 5 s); at most 16 incoming connections in flight, and a paired peer's slot is released as soon as it is authenticated.
- Unpairing only blocked new connections: a removed device kept any session it had open (control, transfer). Now `unpair` closes its live connections at once.
- Discovery: a forged or stale candidate address no longer delays connects (candidates and saved addresses are dialled together) and is dropped after a failed dial; only the address the authenticated connection actually uses is saved. Dial-triggered probes at most every 30 s.
- PC -> phone control needs the accessibility service, plus either "Let my PC control this phone" or an approval on the phone for each request (30 s, no answer = deny). The phone shows who is in control and can end it at once (Stop, or turning the switch off).
- Android: allowBackup=false did not cover Android 12+ device-to-device transfer; data extraction rules now exclude everything.
- Tray pipe used the default DACL (other local accounts could connect): now this user (by SID, so an elevated tray still admits the user's unelevated "Send to") + SYSTEM only.
- Clipboard text no longer written to logs (tray.log, logcat).
