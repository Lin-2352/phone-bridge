# Relay: wire protocol (version 0)

Status: current for Relay 0.3.0. Matches the `pb-protocol` crate at protocol version 0; fields added since 0.2.0 are optional, so an older build ignores them. Two devices talk when each side's `min_compatible` is at most the other's `protocol_version`. The version is still 0, so every build talks to every other.

## Layers
1. **QUIC** (iroh 1.3.0 / noq) provides encryption, authentication of the peer's Ed25519 key, streams and unreliable datagrams.
2. **ALPN** `pb/0` for sessions, `pb-pair/0` for pairing. (Spikes used `pb-spike/0`.) Session connections from peers not in the paired-peer store are closed with application code 403 before any stream is read.
   Application close codes: 0 bye, 200 file saved, 300 transfer cancelled, 400 pairing refused or transfer failed, 403 not paired, 410 this device closed every connection ("Disconnect everything"; the pairing is untouched), 426 incompatible protocol version (`Hello`), 429 busy (the paired peer already has the most sessions it may keep open, 16), 500 the joiner could not save the pairing.
3. **Control stream**: the first bidirectional stream opened by the connecting side. Carries length-prefixed protobuf frames.
4. **Feature streams**: one bidirectional stream per transfer (files, clipboard payloads, camera). Each begins with a frame naming the feature.
5. **Datagrams**: unreliable, unordered, small. Used for audio PCM only (input travels on a reliable stream).

## Control frames
```
frame  = length:u32-LE || body
body   = protobuf message (prost), at most 65536 bytes
```
- A receiver rejects a declared length above 65536 before reading the body.
- Truncated frames wait for more bytes; malformed bodies are an error and close the stream.
- Bulk data never uses control frames; it uses raw bytes on a feature stream.

### `Hello` (both sides send it first)
| Field | Tag | Type | Notes |
|---|---|---|---|
| `protocol_version` | 1 | uint32 | This build's wire version (0) |
| `device_name` | 2 | string | User-visible; only sent after authentication, never in discovery. Receivers show the name saved at pairing, not this field |
| `capabilities` | 3 | repeated string | `clipboard`, `audio`, `discovery2` (keyed discovery) and `resume` (resumable file transfers: `Offer.resume_token`, `OfferReply.resume_from`, `OfferReply.prefix_sha256`, `FileHeader.offset`; the send queue's pause, resume and cancel are local and need nothing on the wire). A build advertises a capability only once it has it. The other planned features (unlock gate, Disconnect everything, pairing limits, quotas, several devices, history, drag-and-drop) change nothing on the wire. `audio` covers the audio messages (the audio messages, tags 25 to 27; a requester that does not see it fails at once instead of waiting for a reply an older peer never sends) |
| `min_compatible` | 4 | uint32 | The oldest peer `protocol_version` the sender talks to. Absent (0) from Relay 0.2.0 |
| `app_version` | 5 | string | The sender's Relay version, for people. Absent from Relay 0.2.0 |

Compatibility: a device whose `protocol_version` is below the peer's `min_compatible` must update. The device that finds this closes with code 426 and reason `update:you` (the receiver must update) or `update:me` (the closer must). A close with any other reason (Relay 0.2.0's) reads as "update both".

Relay 0.2.0 sends fields 1-3 only. A `Hello` with fields 4 and 5 at their defaults encodes to the same bytes, so a 0.2.0 device and this build still talk: tested both ways in `pb-protocol` (`hello_from_0_2_0_reads_with_defaults`, `this_hello_reads_as_0_2_0`) and end to end in `pb-transport` (`a_0_2_0_peer_still_talks`, clipboard text both ways).

### `Control` envelope (session stream)
`oneof msg { Hello = 1; Heartbeat = 2; ClipboardText = 3; Offer = 4; OfferReply = 5; TransferDone = 6; Cancel = 7; ClipboardUpdate = 8; }`
- `Heartbeat { seq: uint64 }` every 5 s from both sides; 15 s of silence ends the session.
- `ClipboardText { text: string (max 60 KiB), sensitive: bool }`, sent only on explicit user action; receivers must not preview or keep sensitive text in history.

### Pairing (`pb-pair/0`, envelope `Pair`)
Pairing code shown by the host: `pb1:<host id hex>:<16-byte token hex>:<addr>,<addr>` (single use, 2 minutes, burned after 3 wrong tokens).
1. joiner -> host `PairStart { token, commitment = SHA-256("phone-bridge/commit/v0" || nonce_j), device_name, addrs }`
2. host -> joiner `PairNonce { nonce_h, device_name }`
3. joiner -> host `PairReveal { nonce_j }` (host verifies the commitment)
4. both show `SAS = 6 digits of SHA-256("phone-bridge/sas/v0" || sorted(id, nonce) pairs)`; each side sends `PairConfirm { accepted }`. The joiner sends its confirm first; the host reads it before sending its own and answers `accepted: true` only when both users accepted and it has saved the pairing. A joiner that declines closes at once (400); the host then stops waiting for its own user.
5. Close order: joiner finishes its stream, host finishes after its confirm, joiner closes after the host's end-of-stream (closing earlier can discard an unread confirm): code 0 once it saved the pairing, 500 when it could not, and the host then removes its own entry.
6. Peer-supplied values are checked on receipt: only addresses that parse as `ip:port` are kept (canonical form, at most 6; the same for the addresses in a pasted code), and device names lose control and hidden bidi/zero-width characters and are cut to 64 characters.

### Discovery (UDP port 47199, no QUIC)

A device that moved network finds its paired peers again with small UDP probes: broadcast on each interface plus every last known peer address. A probe or answer only adds an in-memory *candidate* address; an address is saved only after an authenticated QUIC connection to the peer's key succeeded through it. Probes go out on demand (start, network change, failed dial), never periodically; at most one answer per peer per second.

**PBD1** (Relay 0.2.0 and later, 23 bytes, unkeyed): `"PBD1" | kind u8 (1 probe, 2 answer) | quic port u16 LE | tag[16]`, `tag = SHA-256("pb-discovery/1" || sender id || 30-minute window u64 LE)[..16]` (windows -1, 0 and +1 are accepted). Anyone who knows a device's public key can compute the tag.

**PBD2** (capability `discovery2`, from the build that adds keyed discovery): authenticated with a per-pair key, so only a device that holds the pairing can ask or answer. The key is `k = HKDF-SHA256(salt = "pb-discovery/2", ikm = X25519(own, peer), info = min(id) || max(id))`, with the X25519 keys taken from the two paired Ed25519 identities; an all-zero shared secret is refused. All integers are little-endian, `mac` is 16 bytes.

- probe, 39 bytes: `"PBD2" | 1 | quic port u16 | nonce[16] | mac`
- answer, 56 to 170 bytes: `"PBD2" | 2 | quic port u16 | probe_nonce[16] | nonce[16] | n u8 (at most 6) | n x (family u8 4 or 6 | ip 4 or 16 | port u16) | mac`
- `mac = HMAC-SHA256(k, label || window u64 || every byte before the mac)[..16]`, `label` being `"pb-discovery/2 probe"` or `"pb-discovery/2 answer"`. The mac covers the exact bytes on the wire (the plan listed named fields; covering the whole prefix means the encoding cannot drift from what is authenticated), and the window is the 30-minute window as in PBD1.
- Receive order: at most 256 bytes and a strict parse (exact length, at most 6 addresses, family 4 or 6) first; then the cached keys of the marked peers are tried over windows -1, 0 and +1; only a packet whose mac verified enters the `(peer, nonce)` replay cache (1024 entries), so unauthenticated packets cannot flush it. A probe is answered with this device's addresses under the pair's mac; the source address of a probe is not authenticated and is not learned. Only the MACed addresses of an answer become candidates.
- A probe round sends one PBD2 probe per marked peer, padded with random-mac dummies to at least 4, so the count does not show how many peers are paired. PBD1 probes are still sent while any paired peer is not marked.
- Marking: a peer is marked when a session's `Hello` lists `discovery2`, and unmarked when a later `Hello` does not or when it is unpaired. Marks live in `discovery2.txt` in the data folder (one lowercase hex id per line, at most 64, written atomically), never as a field of `peers.txt`: Relay 0.2.0 drops a `peers.txt` line with an extra column and would lose the pairing. Once a peer is marked, PBD1 is neither sent to it nor answered from it. PBD1 is removed in the next minor release.
- A downgrade that is also a network change: a peer that went back to 0.2.0 and then moved IP is found only through PBD1, which is no longer sent to it while its old mark stands; its mark is cleared the next time it connects (its `Hello` no longer lists `discovery2`), after which PBD1 works again.

### Sharing (AirDrop-style), one session per transfer
1. sender -> `Offer { id: u64 random, kind: File|Text|Link|ClipboardImage, name, size, mime, text, resume_token }` (text/links inline, max 60 KiB). `resume_token` (tag 7) is exactly 16 random bytes chosen by the sender and kept for every retry of one file; it is sent only to a peer that advertised `resume`, only for files, and is empty otherwise.
2. receiver -> `OfferReply { id, accept, too_large, resume_from, prefix_sha256 }` (auto-accept from paired devices is a receiver setting; otherwise the user is asked; offers over the size cap (4 GiB default) or the receiver's free space are declined with `too_large = true` (tag 3; older peers leave it false), so the sender can say why). `resume_from` (tag 4, u64) and `prefix_sha256` (tag 5, 32 bytes) are set only in answer to an offer that carried a token, when the receiver kept a partial of exactly that file (same peer, token, name and size): `resume_from` is how many bytes it holds, `prefix_sha256` their SHA-256. Both absent means 0.
3. File only: sender opens a unidirectional QUIC stream: `FileHeader { id, offset }` frame, then the bytes from `offset` to the end (exactly `size - offset`); `offset` (tag 2, u64) is absent (0) for the whole file, or equals the offered `resume_from` after the sender hashed its first `resume_from` bytes and they matched `prefix_sha256`; on a mismatch the sender sends offset 0 and the whole file and the receiver truncates its partial. Any other offset is a protocol error. Then `TransferDone { id, sha256 }` on the control stream.
4. Receiver writes `.<name>.partial-<16 hex digits chosen locally>` (the offer id is the peer's choice and may repeat), checks byte count and SHA-256 over the whole file (a kept prefix is re-read from disk and hashed, never trusted from a stored hash), then renames to a unique sanitised name (`name (2).ext` on collision); any failure deletes the partial, except a broken or silent connection of a resumable transfer: that partial is kept (named from the peer and the token, never from the sent name) for at most 24 hours, capped per peer, overall and against free space, and continues on the next offer with the same token. A `ClipboardImage` (at most 10 MiB) is received in memory with the same checks and never written to the receive folder.
5. Close order: whoever reads the last message closes (sender after a decline/text reply; receiver after `TransferDone`). The receiver closes with code 200 only once the file is saved under its final name, with 400 if it did not arrive intact or could not be saved, and either side closes with 300 when its user cancels; the sender reports success only on 200 (0 is also what QUIC uses for a dropped connection).

### Universal Clipboard
`ClipboardUpdate { text, hash = SHA-256(text) }`, sent automatically when the setting is on at both ends. Each side remembers the hash of the last text it received and sent and never re-sends it (no ping-pong). Content marked private (Windows `ExcludeClipboardContentFromMonitorProcessing`, Android `EXTRA_IS_SENSITIVE`) is never sent. Android can only read the clipboard while focused: phone->PC sync runs when the app gains focus (only if the clipboard changed since the last sync, persisted) and on the Send clipboard tile/notification.

### Local second-instance requests (Windows only)
`pb-tray --send <files>` / `--pair` / `--send-clipboard` reach the running tray over the named pipe `\\.\pipe\PhoneBridgeTray-<user>` (remote clients rejected; ACL: this user by SID and SYSTEM only). Lines: verb, then one path per line.

### Shared mouse and keyboard
1. controller -> `InputStart { id }` (control tag 9); receiver -> `OfferReply { id, accept }`. The PC accepts only if "Let my phone control this PC" is on. The phone needs its accessibility service ("Phone Bridge control") enabled, and then accepts at once if "Let my PC control this phone" is on; otherwise it asks the user (popup plus notification) and declines after 30 s without an answer.
2. Controller opens one unidirectional stream and writes a no-op move (0, 0) at once, so the stream exists for the receiver even if the user is idle (receivers wait 10 s for it); then fixed 8-byte records: `kind u8 | flags u8 | seq u16 | a i16 | b i16` (little-endian). Kinds: 0 move (dx, dy), 1 button (button, flags&1 = pressed), 2 wheel (dx, dy in 1/120 notches), 3 key (PC set-1 scan code, 0xE0xx extended), 4 character (one UTF-16 unit).
3. Reliable and ordered (not datagrams): a lost key-up or click would be worse than any latency on a local link. Moves queued faster than the link drains are summed into one record. The sender paces itself to stay inside the receiver's limits (receiver: 2000 moves/s dropped beyond, 1000 other events/s and 4000 records/s end the session): token buckets of 800 moves/s and 250 other events/s with bursts of 100, so excess moves merge and a long paste is typed out over a few seconds.
4. The session ends when the stream ends; the receiver closes. Whatever is still pressed is released on the receiver.
5. The receiver can end a session at any time (the user taps Stop or turns control off): it closes the connection, and the controller sees the session end and returns its mouse and keyboard to the PC.

Further messages (camera session) are added with their features.

### Phone notifications
Control messages, tags 10 to 16 of the `Control` envelope:

| Message | Direction | Content |
|---|---|---|
| `Notification` (10) | phone → PC | key, package, app name, title, text, time (Unix ms), conversation, up to 5 lines, flags (quiet, private, call), one-time code, actions (label, takes a reply), icon hash, picture (JPEG) |
| `NotifyIcon` (11) | phone → PC | SHA-256 and PNG of an app icon; sent once per session before the first notification that uses it |
| `NotifyRemoved` (12) | phone → PC | key |
| `NotifySync` (13) | phone → PC | every key active on the phone; the PC removes toasts for the others |
| `NotifySyncRequest` (14) | PC → phone | none |
| `NotifyAction` (15) | PC → phone | id, key, kind (dismiss, button, reply, open, silence), index, text |
| `NotifyActionResult` (16) | phone → PC | id, ok, reason |

**Sessions:**
- **Notify session** (phone → PC):
  1. The phone opens a session when it has something to send.
  2. `NotifySync`, then quiet re-sends of notifications that failed to reach this PC earlier (at most 50).
  3. Live `NotifyIcon` / `Notification` / `NotifyRemoved`.
  4. The phone finishes the session after 15 s with nothing to send.
  
  Several updates of one key waiting in the queue are sent as the last one only.
- **Tray start** (PC → phone): `NotifySyncRequest`. The phone answers on the same session with quiet copies of up to 50 active notifications (newest first, no pictures), then `NotifySync`, then finishes.
- **Action** (PC → phone): `NotifyAction`, answered by `NotifyActionResult` with the same id (the PC waits up to 10 s).
- **Toast arguments** on the PC carry the phone's id and the notification key (percent-encoded), so a toast still works after a tray restart; the tray rebuilds its list of phone toasts from the notification centre at start.

**Limits:**
- Strings: key 256 B, title 512 B, text 4096 B, 5 lines of 512 B each, reply text 4096 B. They are truncated on a character boundary.
- Picture 48 KiB; icon 16 KiB; `NotifySync` 200 keys.
- The phone sends at most 40 notify records per second. The PC closes a session carrying more than 200 per second on arrival (a Wi-Fi stall can bunch paced records). The phone closes one carrying more than 10 actions per second.
- Older peers ignore the unknown messages.

### Phone screen
Control messages, tags 17 to 20:

| Message | Direction | Content |
|---|---|---|
| `ScreenStart` (17) | PC → phone | id, max long side (0 = native), fps, codecs the PC decodes (best first: 1 H.265 Main10, 2 H.265, 3 H.264), dim, sharp |
| `ScreenAccept` (18) | phone → PC | id, codec, width, height, rotation; declined with `OfferReply{accept: false}` |
| `ScreenFeedback` (19) | PC → phone | newest presented pts (µs), every 100 ms |
| `ScreenState` (20) | both | PC → phone: `active` (window visible), `dim`, `codec_failed`; phone → PC: `notice` |

**Session:**
1. After `ScreenAccept`, the phone opens a uni stream of video records: the header `PBV1`, then `u32 len | i64 pts_us | u8 flags | payload` (flags: 1 key, 2 codec config, 4 repeat). A record over 4 MiB ends the session.
2. A config record (`u16 rotation | u16 width | u16 height | codec headers`) comes first, after every rotation or encoder reset, and is always followed by a key frame.
3. The PC opens a uni stream of 8-byte input records (below) for the window's mouse and keyboard; one consent covers both.
4. After a dropped link the phone keeps capturing (paused) for 30 s; a new `ScreenStart` from the same PC within that time resumes without asking again.

### Phone camera
Control messages, tags 21 to 24 (feedback reuses `ScreenFeedback`, 19):

| Message | Direction | Content |
|---|---|---|
| `CameraStart` (21) | PC → phone | id, codecs the PC decodes (best first), lens (0 back, 1 front), width, height (the 16:9 size wanted: 1920x1080 or 1280x720), fps (30) |
| `CameraAccept` (22) | phone → PC | id, codec, width, height, rotation (0/90/180/270, clockwise to upright), lens; declined with `OfferReply{accept: false}` |
| `CameraControl` (23) | PC → phone | lens: switch lenses mid-stream |
| `CameraLens` (24) | phone → PC | lens (0 back, 1 front) now in use: after every switch, from the phone or the PC, and when streaming starts; any other value fails decoding. Older peers skip the unknown tag |

**Session:**
1. After `CameraAccept`, the phone opens a uni stream of video records, the same framing as the phone screen (`PBV1`, `u32 len | i64 pts_us | u8 flags | payload`; config records `u16 rotation | u16 width | u16 height | headers`). There is no input stream.
2. A rotation or lens change sends a new config record with the new rotation; the key frame that follows is decoded with it.
3. A deliberate close by the PC ends the session at once; a lost link keeps the camera on the phone for 30 s for a resume.

**Frames file** (PC only, tray to media source): `%ProgramData%\PhoneBridge\camera\frames`, 4096 bytes of header and two NV12 slots of 1920x1080. Little-endian; every header field is an 8-byte word:

| Offset | Field |
|---|---|
| 0 | magic `PBCF`, `u32` version 1 |
| 8 | `u32 wanted_w`, `u32 wanted_h` (the source writes the size the app chose) |
| 24, 32 | seqlock per slot (odd while being written) |
| 40 | index of the newest complete slot |
| 48, 56 | `pts_us` per slot |
| 64 | tray heartbeat (ms since 1601); the source keeps the last picture for the 30 s reconnect wait and shows black after 35 s without one |
| 72, 80 | `u32 width`, `u32 height` per slot (a frame is delivered only if they equal the wanted size) |
| 88 | count of completed frames |
| 4096, 4096 + 3110400 | the two slots, tightly packed NV12 |

### Audio sharing (either direction)
Control messages, tags 25 to 27. The requester (whoever connected) sends `AudioStart`; the responder answers with `AudioAccept`. Which side sends the PCM is `direction`, not who requested.

| Message | Direction | Content |
|---|---|---|
| `AudioStart` (25) | requester → responder | id, sample_rate (44100 or 48000), channels (1 or 2), direction (0 phone to PC, 1 PC to phone), source (0 playback, 1 microphone; the microphone only with direction 0 and only in the explicit microphone mode). Tag 4 was `bitrate_kbps`, removed (raw PCM) and not reused |
| `AudioAccept` (26) | responder → requester | id, sample_rate and channels (the format that will be sent), accepted (bool; absent reads as refused), reason (when refused: 1 declined, 2 unavailable, 3 busy, 4 bad_params, 5 no_permission, 6 app_blocks_capture; an unknown code reads as declined) |
| `AudioStop` (27) | either side | id: end the session; the receiver closes the connection. Older peers skip unknown tags |
| `ClipboardPull` (28) | PC → phone | want_image: send what is on the phone's clipboard now. The phone opens a see-through window to read it (Android lets only the focused app), then answers with a `ClipboardUpdate` or a clipboard picture, or with `ClipboardPullResult`. Older peers skip the unknown tag, so the PC times out and says so |
| `ClipboardPullResult` (29) | phone → PC | ok, reason: why nothing was sent (locked, sharing off, nothing copied) |

**Session:**
1. Consent is given on the device whose audio is shared, before the request (PC audio) or before the accept (phone audio). There is no auto-accept and no resume after a drop.
2. A requester checks the peer's `Hello` for `audio` first; without it the request fails as unsupported and nothing is sent.
3. After an accepting `AudioAccept`, the PCM travels in QUIC datagrams, never on a stream. Each datagram: `seq u32 LE | sender clock in ms u32 LE | PCM` (raw i16 LE, interleaved, 1 to 65536 bytes in whole sample groups, as many as fit the path's datagram size). Both ends need datagram support: a responder without it refuses with `unavailable`, a requester closes ("no datagrams"). A malformed datagram (shorter than the 8-byte header, or PCM outside the limits) closes the session. The old uni-stream format with the `PBA0` magic is gone.
   - Why datagrams: a reliable stream retransmits in order, so during a Wi-Fi stall QUIC buffered seconds of audio and the listener heard it late. Measured on a simulated 3 s blackout: audio was 3085 ms old one second after the link came back on the stream design, 2 ms on datagrams.
   - `AudioAccept.codec` (tag 6) says what the payload is: 0 raw PCM, split to fit, or 1 one Opus packet per datagram (48 kHz only, 1..=1275 bytes, never split). The sender chooses; the phone sends Opus at about 160 kbit/s and falls back to PCM when it has no encoder. PC to phone stays PCM. The sender skips an Opus packet while more than 4 KiB of Opus waits unsent.
   - Receiver: on an Opus session a lost packet (a gap up to 40 ms) reaches the decoder as an empty frame, which it conceals.
   - Receiver: a packet that arrives ahead of its predecessor is held for up to 15 ms (at most 8 packets) so Wi-Fi swapping neighbours does not cost audio; if the missing packet does not come it is lost. A packet older than the newest one already played (lower `seq`), or a duplicate, is late and dropped. A gap of lost packets up to 40 ms is filled with silence so playback keeps its timing; a longer gap (a stall) is not filled, which would only add delay.
4. Each field is validated by whoever receives it, on both sides: `AudioStart` by the responder (a violation is answered with `reason = bad_params` and the user is not asked), `AudioAccept` by the requester (a format outside the limits closes the connection). One session per direction at a time per device; a second request in a direction in use is refused with `busy`.
5. The sender's QUIC datagram buffer holds about 100 ms (`tuning.rs`, `DATAGRAM_SEND_BUFFER` 20 KiB) and the receiver queues 100 ms; both drop the oldest audio past that, so a slow consumer never stalls the sender and neither end can fall behind. A failed send, a limit violation, `AudioStop`, a lost link, or dropping the session ends it, and each side is told once why.
6. **Roles in this version.** The PC is always the requester (it dials and sends `AudioStart`); the phone is always the responder. So `direction = 0` (phone audio on the PC) is asked for by the PC and consented on the phone, and `direction = 1` (PC audio on the phone) is consented on the PC before it dials. There is no message with which a phone asks the PC to start, so the phone's Home tiles for audio only point at the PC and stop a session that runs. Adding one ("please ask me") would be a new tag and is deliberately not part of this version.
7. **The microphone** (`source = 1`) is part of the wire format and the phone side (Relay dialog on every request, no skip setting, microphone service type), but nothing on the PC sends it: the PC UI offers playback only. It stays unreachable from the apps until a phone-initiated start exists.

## Input record (fixed 8 bytes, little-endian)
```
offset 0  kind   u8    0 MouseMove, 1 MouseButton, 2 Wheel, 3 Key
offset 1  flags  u8    bit 0 = pressed (Button, Key)
offset 2  seq    u16   wraps; receiver drops stale or duplicate events
offset 4  a      i16   MouseMove dx | button id | Wheel dx | scancode (as u16)
offset 6  b      i16   MouseMove dy | 0           | Wheel dy | 0
kind 5 Touch: flags = phase (0 hover, 1 down, 2 move, 3 up); a, b = x, y as u16 (0..=65535 across the screen)
kind 6 Pinch: flags bit 0 = out (zoom in); a, b = centre x, y as u16
```
- Wheel units are 1/120 of a notch.
- Keys are hardware scan codes so layouts do not matter; text arrives as kind 4 (one UTF-16 unit) on the same stream.
- Records travel on the reliable, ordered input stream (see Shared mouse and keyboard), not in datagrams, so none is lost. When the stream or the connection ends, the receiver releases every key and button still pressed. There is no input heartbeat.

## Limits (enforced in code)
- Control frame body: 64 KiB. Inline text/links and clipboard text: 60 KiB, checked by the senders; a receiver takes whatever fits the 64 KiB control frame. Files: 4 GiB default cap. Clipboard images: 10 MiB.
- Input record: 8 bytes; one that does not decode is skipped (it still counts against the flood limit).
- Audio: sample rate 44100 or 48000, channels 1 or 2, PCM per datagram 1 to 65536 bytes and a multiple of `2 x channels` after an 8-byte header, one session per direction, about 100 ms buffered by the sender and 100 ms queued by the receiver.

## Open items
- ALPN naming and per-feature stream headers.
- QR rendering of the pairing code (text only so far).
- Flow control and priorities between control, input and bulk traffic.
- Version negotiation after 1.0.
