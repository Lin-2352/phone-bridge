# Phone Bridge: threat model (draft, Phase 0)

Status: draft. No external review yet. Update whenever a feature adds a new trust boundary.

## What we protect
| Asset | Why it matters |
|---|---|
| Clipboard contents | Often passwords, tokens, personal text |
| Files in transit | Arbitrary private data |
| Input control of the phone | Remote control of an unlocked phone is full account takeover |
| Camera and microphone streams | Live surveillance if hijacked |
| Phone notifications | Message text, senders, one-time codes; acting on them (reply, answer a call) speaks for the user |
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
| 1 | Pairing | Man in the middle during first contact | QR bootstrap carries the peer public key; 6-digit SAS compared by the user; nonce commitment before reveal (**implemented** in `pb-transport::pairing`, tested; QR rendering still planned) |
| 2 | Pairing | Stranger pairs by guessing or spamming requests | Codes exist only while the host shows one; single use, 2-minute expiry, burned after 3 wrong tokens (**implemented**) |
| 3 | Transport | Eavesdropping or tampering | QUIC with TLS 1.3, peers authenticated by pinned Ed25519 keys (iroh endpoint ids). Only keys in the paired-peer store may open sessions; others are closed with code 403 before any stream is read; revocation takes effect for new connections (**implemented**, tested in-process and on the real phone) |
| 4 | Discovery | Tracking a device by its broadcasts | Rotating HMAC identifiers, no device name in discovery (**planned**) |
| 5 | Relay | Operator reads content | End-to-end encryption between devices; relay only forwards QUIC packets. Self-hostable relay; default build must not use third-party relays without consent (Spike 1 uses none) |
| 6 | Relay | Operator learns metadata | Cannot be eliminated; documented. Users can self-host. |
| 7 | Clipboard | Silent theft of secrets | Universal Clipboard is opt-in (off by default); private content (Windows exclusion formats, Android `EXTRA_IS_SENSITIVE`) never sent; received sensitive text excluded from Win+V history/cloud; echo guard; the Android clipboard-reading activity is not exported so other apps cannot trigger it (**implemented**) |
| 8 | Files | Path traversal, zip bombs, overwriting files | Names sanitised (last component only, reserved/control chars, Windows device names, length), unique names, partial file + SHA-256 check before rename, 4 GiB cap; auto-accept only from paired devices, otherwise prompt (**implemented**, tests incl. hostile names, forged hash, interrupted sender) |
| 9 | Input control | Hijacking the phone | Paired devices only; explicit session started by the user (touchpad screen, or Ctrl+Alt+P on the PC); PC side gated by a setting, phone side by an accessibility service the user must enable in Android settings (off by default, revocable there at once); visible indicators (toast and cursor on the phone, notification on the PC); held keys and buttons released when a session ends; PC hooks installed only during a session (**implemented**) |
| 10 | Accessibility service | Abuse by other apps or policy loss | Only in the `foss` flavor; no data collection; never ships in the Play flavor (**decided**, see DECISIONS.md) |
| 11 | Parsers | Memory corruption or DoS from malformed input | Size limits before allocation, no panics on bad input, fuzz targets (**implemented for control frames and input datagrams**; fuzzing harness **planned**) |
| 12 | Keys at rest | Theft from disk or backup | Seed wrapped by Android Keystore / Windows DPAPI; excluded from backups (**implemented**: DPAPI on Windows, Keystore AES-GCM on Android; unseal failure is an error, never a silent new identity; Android backup disabled) |
| 13 | Updates | Malicious update | Signed releases, reproducible builds where possible (**planned**) |
| 14 | Resource use | Battery or CPU drain used as DoS | Rate limits per peer; idle budget tracked in DECISIONS.md |
| 15 | Phone notifications | Content leaks, forged actions, malformed images | Only to paired peers over the session transport (row 3), and only after the user grants Android Notification access and leaves "Notifications on PC" on. Nothing is stored except toast images (a private temp folder, PNG/JPEG checked by magic bytes, hex-only names, 50 MB cap, emptied at tray start) and the app names seen (for the mute switches). Logs carry the package and a key hash, never text. The phone acts only on keys it currently shows, through the apps' own intents; actions and records are rate-limited (10 actions/s; records paced at 40/s, a session over 200/s on arrival is closed) and size-limited. Toast arguments carry the phone id and notification key so toasts work after a restart; they stay in the user's own notification centre. A toast is only kept in step (and its clearing mirrored to the phone) once Windows has actually kept it. Toast XML escapes all text. Content is hidden while the PC is locked (default) and kept off the lock screen (default). One-time codes copied on the PC are kept out of clipboard history and cloud clipboard, never synced back, and cleared after 60 s if unchanged. The Windows notification centre keeps shown toasts until they are cleared or the phone removes them (**implemented**; device tests pending) |

## Known limitations found so far
- Relay metadata exposure (row 6) is inherent to any relay design.
- `pb --auto-accept` and the app's adb intent hooks skip or script the SAS decision; they are test-only and must not ship in release builds.
- Spike 1 used an unauthenticated test protocol on throw-away keys; it proves transport, not authorisation. Do not treat spike code as secure.
- The first GSO send failure on USB tethering (DECISIONS.md) is an availability bug, not a confidentiality one, but a flaky transport invites insecure retries; do not add silent fallbacks.

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
