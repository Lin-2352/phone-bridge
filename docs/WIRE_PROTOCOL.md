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
`oneof msg { Hello hello = 1; Heartbeat heartbeat = 2; ClipboardText clipboard_text = 3; }`
- `Heartbeat { seq: uint64 }` every 5 s from both sides; 15 s of silence ends the session.
- `ClipboardText { text: string (max 60 KiB), sensitive: bool }`, sent only on explicit user action; receivers must not preview or keep sensitive text in history.

### Pairing (`pb-pair/0`, envelope `Pair`)
Pairing code shown by the host: `pb1:<host id hex>:<16-byte token hex>:<addr>,<addr>` (single use, 2 minutes, burned after 3 wrong tokens).
1. joiner -> host `PairStart { token, commitment = SHA-256("phone-bridge/commit/v0" || nonce_j), device_name, addrs }`
2. host -> joiner `PairNonce { nonce_h, device_name }`
3. joiner -> host `PairReveal { nonce_j }` (host verifies the commitment)
4. both show `SAS = 6 digits of SHA-256("phone-bridge/sas/v0" || sorted(id, nonce) pairs)`; each side sends `PairConfirm { accepted }`. Pinned only if both accept.
5. Close order: joiner finishes its stream, host finishes after reading the joiner's confirm, joiner closes after the host's end-of-stream (closing earlier can discard an unread confirm).

Further messages (file offer and accept, input mode, camera session) are added with their features.

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
- Control frame body: 64 KiB.
- Input datagram: exactly 8 bytes, anything else is dropped.

## Open items
- ALPN naming and per-feature stream headers.
- QR rendering of the pairing code (text only so far).
- Heartbeat and key-release message.
- Flow control and priorities between control, input and bulk traffic.
- Version negotiation after 1.0.
