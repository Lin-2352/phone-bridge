# Phone Bridge: wire protocol (draft v0)

Status: draft. Matches the `pb-protocol` crate at protocol version 0. Versions must match exactly until 1.0; there is no compatibility promise yet.

## Layers
1. **QUIC** (iroh 1.3.0 / noq) provides encryption, authentication of the peer's Ed25519 key, streams and unreliable datagrams.
2. **ALPN** `pb/0` for sessions, `pb-pair/0` for pairing. (Spikes used `pb-spike/0`.) Session connections from peers not in the paired-peer store are closed with application code 403 before any stream is read.
3. **Control stream**: the first bidirectional stream opened by the connecting side. Carries length-prefixed protobuf frames.
4. **Feature streams**: one bidirectional stream per transfer (files, clipboard payloads, camera). Each begins with a frame naming the feature.
5. **Datagrams**: unreliable, unordered, small. Used for input and low-latency media.

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
| `protocol_version` | 1 | uint32 | Must equal the receiver's version, else close |
| `device_name` | 2 | string | User-visible; only sent after authentication, never in discovery |
| `capabilities` | 3 | repeated string | e.g. `clipboard`, `files`, `input`, `camera` |

### `Control` envelope (session stream)
`oneof msg { Hello = 1; Heartbeat = 2; ClipboardText = 3; Offer = 4; OfferReply = 5; TransferDone = 6; Cancel = 7; ClipboardUpdate = 8; }`
- `Heartbeat { seq: uint64 }` every 5 s from both sides; 15 s of silence ends the session.
- `ClipboardText { text: string (max 60 KiB), sensitive: bool }`, sent only on explicit user action; receivers must not preview or keep sensitive text in history.

### Pairing (`pb-pair/0`, envelope `Pair`)
Pairing code shown by the host: `pb1:<host id hex>:<16-byte token hex>:<addr>,<addr>` (single use, 2 minutes, burned after 3 wrong tokens).
1. joiner -> host `PairStart { token, commitment = SHA-256("phone-bridge/commit/v0" || nonce_j), device_name, addrs }`
2. host -> joiner `PairNonce { nonce_h, device_name }`
3. joiner -> host `PairReveal { nonce_j }` (host verifies the commitment)
4. both show `SAS = 6 digits of SHA-256("phone-bridge/sas/v0" || sorted(id, nonce) pairs)`; each side sends `PairConfirm { accepted }`. Pinned only if both accept.
5. Close order: joiner finishes its stream, host finishes after reading the joiner's confirm, joiner closes after the host's end-of-stream (closing earlier can discard an unread confirm).

### Sharing (AirDrop-style), one session per transfer
1. sender -> `Offer { id: u64 random, kind: File|Text|Link, name, size, mime, text }` (text/links inline, max 60 KiB)
2. receiver -> `OfferReply { id, accept, too_large }` (auto-accept from paired devices is a receiver setting; otherwise the user is asked; offers over the size cap (4 GiB default) or the receiver's free space are declined with `too_large = true` (tag 3; older peers leave it false), so the sender can say why)
3. File only: sender opens a unidirectional QUIC stream: `FileHeader { id }` frame, then exactly `size` raw bytes; then `TransferDone { id, sha256 }` on the control stream.
4. Receiver writes `.<name>.partial-<id>`, checks byte count and SHA-256, then renames to a unique sanitised name (`name (2).ext` on collision); any failure deletes the partial.
5. Close order: whoever reads the last message closes (sender after a decline/text reply; receiver after `TransferDone`). The receiver closes with code 0 only once the file is saved under its final name, and with 400 if it did not arrive intact or could not be saved; the sender reports success only on 0.

### Universal Clipboard
`ClipboardUpdate { text, hash = SHA-256(text) }`, sent automatically when the setting is on at both ends. Each side remembers the hash of the last text it received and sent and never re-sends it (no ping-pong). Content marked private (Windows `ExcludeClipboardContentFromMonitorProcessing`, Android `EXTRA_IS_SENSITIVE`) is never sent. Android can only read the clipboard while focused: phone->PC sync runs when the app gains focus (only if the clipboard changed since the last sync, persisted) and on the Send clipboard tile/notification.

### Local second-instance requests (Windows only)
`pb-tray --send <files>` / `--pair` / `--send-clipboard` reach the running tray over the named pipe `\\.\pipe\PhoneBridgeTray-<user>` (remote clients rejected; ACL: this user by SID and SYSTEM only). Lines: verb, then one path per line.

### Shared mouse and keyboard
1. controller -> `InputStart { id }` (control tag 9); receiver -> `OfferReply { id, accept }`. The PC accepts only if "Let my phone control this PC" is on. The phone needs its accessibility service ("Phone Bridge control") enabled, and then accepts at once if "Let my PC control this phone" is on; otherwise it asks the user (popup plus notification) and declines after 30 s without an answer.
2. Controller opens one unidirectional stream and writes a no-op move (0, 0) at once, so the stream exists for the receiver even if the user is idle (receivers wait 10 s for it); then fixed 8-byte records: `kind u8 | flags u8 | seq u16 | a i16 | b i16` (little-endian). Kinds: 0 move (dx, dy), 1 button (button, flags&1 = pressed), 2 wheel (dx, dy in 1/120 notches), 3 key (PC set-1 scan code, 0xE0xx extended), 4 character (one UTF-16 unit).
3. Reliable and ordered (not datagrams): a lost key-up or click would be worse than any latency on a local link. Moves queued faster than the link drains are summed into one record. The sender paces itself to stay inside the receiver's limits (receiver: 1000 moves/s dropped beyond, 400 other events/s and 2000 records/s end the session): token buckets of 800 moves/s and 250 other events/s with bursts of 100, so excess moves merge and a long paste is typed out over a few seconds.
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

## Input datagram (fixed 8 bytes, little-endian)
```
offset 0  kind   u8    0 MouseMove, 1 MouseButton, 2 Wheel, 3 Key
offset 1  flags  u8    bit 0 = pressed (Button, Key)
offset 2  seq    u16   wraps; receiver drops stale or duplicate events
offset 4  a      i16   MouseMove dx | button id | Wheel dx | scancode (as u16)
offset 6  b      i16   MouseMove dy | 0           | Wheel dy | 0
```
- Wheel units are 1/120 of a notch.
- Keys are hardware scan codes so layouts do not matter; text entry will use a separate reliable path.
- Datagrams are lossy: a lost key-up must not leave a key stuck. The sender sends a periodic state heartbeat and the receiver releases all keys and buttons if the heartbeat stops (250 ms) or the connection drops. **Heartbeat message not yet defined.**
- Maximum datagram payload is about 1162 to 1414 bytes depending on the path MTU, so these are far below the limit.

## Limits (enforced in code)
- Control frame body: 64 KiB. Inline text/links and clipboard text: 60 KiB. Files: 4 GiB default cap.
- Input datagram: exactly 8 bytes, anything else is dropped.

## Open items
- ALPN naming and per-feature stream headers.
- QR rendering of the pairing code (text only so far).
- Heartbeat and key-release message.
- Flow control and priorities between control, input and bulk traffic.
- Version negotiation after 1.0.
