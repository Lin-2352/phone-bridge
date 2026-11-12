# Is QUIC (congestion control and datagram queueing) adding delay to Relay's real-time audio?

Scope: Relay phone-to-PC Opus audio (about 408-byte datagrams including an 8-byte header, 50/s, about 160 kbit/s) over iroh 1.3 / noq 1.3. Data: the bad 2.4 GHz share of 2026-11-05 (22:25:33 to 22:33:41 PC time), the good 5 GHz share from 22:38:53, and a second 2.4 GHz share from 23:27:18.

Paths used below:
- noq-proto = `D:\Rust\cargo\registry\src\index.crates.io-1949cf8c6b5b557f\noq-proto-1.3.0\src\`
- noq = `...\noq-1.3.0\src\`, iroh = `...\iroh-1.3.0\src\`
- project = `D:\Desktop\Project\phone bridge audio\crates\pb-transport\src\`
- PC log = `D:\Desktop\Project\phone bridge audio\dist\Relay\data\tray.log` (seconds since 12:33:56 PC time)
- capture = `C:\WINDOWS\Temp\claude\70f6f07ec3dd5acb\844092cd-aa24-4f63-b66a-5a7e7e87878b\scratchpad\capture\` (the phone clock runs about 1 s ahead of the PC)

Provenance caveat: `audio.rs` has uncommitted changes (git status `M`). The Opus send guard (`OPUS_SEND_BACKLOG` = 4 KiB, skip when the backlog is over it) is in committed `055db63` (2026-11-03) at the same logic, and the phone's `libpb_ffi.so` (2026-11-05 03:41) contains the session strings of this code ("control channel silent", "audio send error"). So the phone ran this guard. That is inferred from the build date and strings; the exact build hash is unknown.

## Verdict (summary for the writer)

The hypothesis as stated ("a shrunken Cubic window throttled 160 kbit/s and datagrams waited seconds in QUIC's queue") is mostly **wrong** for the long arrival gaps. It is **partly right** in a different form, during real outages:

1. **Proven:** QUIC's own send queue can never hold more than about 4.5 KiB of Opus (about 11 packets, about 220 ms of audio). The app guard stops adding at 4 KiB, so QUIC's 20 KiB drop-oldest limit is never reached. In 43 of the 47 phone 10-s windows of the bad share, the sender accepted every frame (no skips), so the queue never even passed 4 KiB there. Most 500-1995 ms gaps came with zero skips and zero lost packets, so roughly 25-75 packets per gap were already below QUIC (socket, qdisc, driver, Wi-Fi retries, access point, PC). That is air and OS delay, not QUIC queueing.
2. **Proven (self-inflicted, but a different mechanism):** when the link stalls, the guard skips the **newest** frames and keeps the **oldest** 4 KiB in QUIC's FIFO. When the link comes back, the first audio delivered is as old as the stall, and the new audio from the stall is gone. At 23:28:18 the phone skipped 120 frames (2.4 s), and the PC saw `max_age_ms=2654` and `max_sent_gap_ms=1471`. The same happened at 22:27:53-22:28:03 (36 and 37 frames) and 22:33:33 (80 frames).
3. **Inferred (the window was not small):** for the sender to keep accepting all frames through a 1.5 s gap, `in_flight` must have reached about 75 × 450 B ≈ 30 KB, so the congestion window was at least about 30 KB. A large window lets more stale audio pile up below QUIC; it does not starve audio.
4. **The coordinator's 23:27 question:** the 65 `lost` packets were **not** QUIC send-buffer drops. The drop-oldest path cannot fire under the guard. The phone's `quic_lost` rose by 55 packets in the same window, so the datagrams were sent and lost on the air (several datagrams can share one QUIC packet). `sentgap` was not small there (1471 ms); the guard's skips plus the losses explain it.
5. **What the window and pacer could do:** the controller *can* throttle this flow, but only after persistent congestion or repeated loss events, and only at high round trips. The cwnd check reserves a whole MTU (`bytes_to_send = segment_size`), so at the floor (2 × MTU) only about 3 audio packets fit in flight. That is below 50/s whenever the round trip is over about 60 ms. No evidence shows this happened. It needs the new sender log line to confirm or rule out.

## Key question 1: By reading noq-proto, when can a datagram sit in the sender's queue, for how long, and can the controller throttle 160 kbit/s at 100-400 ms after the losses seen?

### Takeaway
A datagram waits in `DatagramState.outgoing` (a FIFO) whenever the path is blocked by the congestion window, the pacer or a blocked socket. It waits until the earlier datagrams have left. Cubic can throttle 160 kbit/s only near its 2 × MTU floor, or after several loss events at round trips over about 160-380 ms. Under Relay's guard, at most about 11 packets can wait, so queueing in QUIC is bounded in packets, not in time.

### Cited findings
- `send_datagram` (noq `connection.rs:679-694`) calls `datagrams().send(data, true)`. `drop = true` means `make_space_for` pops the oldest queued datagram until the new one fits under `datagram_send_buffer_size` (noq-proto `connection/datagrams.rs:40-54, 216-224`). The queue is a `VecDeque`; `write` pops from the front (`datagrams.rs:262-281`). Nothing in noq expires a datagram by age; the only other removal is `drop_oversized` after an MTU drop (`datagrams.rs:240-256`).
- Relay sets `DATAGRAM_SEND_BUFFER = 20 * 1024` (project `tuning.rs:59`, applied at `tuning.rs:90`). The library default is 1 MiB (noq-proto `config/transport.rs:582`).
- Relay's Opus guard: when `DATAGRAM_SEND_BUFFER - datagram_send_buffer_space() > OPUS_SEND_BACKLOG` (4 KiB, `audio.rs:55`), `send_frame` returns `Ok(())` without sending. It returns **before** the sequence number is taken and before `packets` is counted (`audio.rs:553-559`, seq at `:569`, count at `:575`). So the queue holds at most 4096 + 408 B, and the 20 KiB drop-oldest is unreachable for Opus.
- Datagrams are written into a packet only when the path may send data (`connection/mod.rs:6504-6517`). Before a new UDP datagram is started, `path_congestion_check` runs (`mod.rs:1539-1554`). It blocks when `in_flight.bytes + bytes_to_send >= congestion.window()` (`mod.rs:1966-1977`), where `bytes_to_send = transmit.segment_size()`, i.e. one full MTU (`mod.rs:1963`), not the datagram's size. Then the pacer is checked (`mod.rs:1980-1992`). Loss probes bypass the cwnd check (`mod.rs:1962-1966`).
- `app_limited` is set when nothing was sent and nothing was blocked (`mod.rs:1424`; `send_blocked` set by Congestion or Pacing at `:1375-1385`). Cubic ignores ACKs while `app_limited` and during recovery (`congestion/cubic.rs:114-122`), as RFC 9002 §7.8 and RFC 9438 §5.8 require ("CUBIC does not increase its congestion window if a flow is application limited") — [RFC 9438](https://www.rfc-editor.org/rfc/rfc9438.html), [RFC 9002](https://www.rfc-editor.org/rfc/rfc9002.html). When the window really blocks, `app_limited` is false and the window grows.
- Cubic on loss: `ssthresh = max(0.7 × window, 2 × MTU)`, window = ssthresh. On persistent congestion the window drops to `minimum_window() = 2 × current_mtu` (`cubic.rs:99-101, 220-239`). At most one reduction per round trip (recovery_start_time, `cubic.rs:193-200`). A spurious-loss undo exists (`cubic.rs:242-248`, called at `mod.rs:2973-2978`). The initial window is 14720 B (`cubic.rs:300`). NewReno halves instead (`loss_reduction_factor: 0.5`, `congestion/new_reno.rs:167`).
- Persistent congestion = two ack-eliciting packets lost more than `PTO × 3` apart with nothing acked between (`mod.rs:3327-3371`; threshold 3 at `config/transport.rs:577`).
- Pacer (Cubic reports no rate): token bucket refilled at `1.25 × window / smoothed_rtt` (`connection/pacing.rs:132-134`). Capacity is about 10 ms of that rate but at least one MTU (`pacing.rs:221-243, 266-279`). At 50 packets of about 450 B every 20 ms, the pacer never binds unless the window is near its floor.
- The MTU is from path MTU discovery: initial 1200 (`lib.rs:357`), upper bound 1452 by default (`config/transport.rs:889`). The actual MTU in this session is unknown.

### Inferences (computed)
- Audio on the wire is about 450 B per packet, 50 per second ≈ 22.5 KB/s. In-flight needed = 22.5 KB/s × RTT: 2.25 KB at 100 ms, 6.75 KB at 300 ms, 9 KB at 400 ms.
- Usable in-flight with Relay's small packets is about `window − MTU`, because of the one-MTU reservation (`mod.rs:1963, 1968`).
  - At the floor (2 × 1452 = 2904 B): about 1452 B usable, about 3 packets per RTT. That is below 50/s when RTT > 60 ms, e.g. 10 packets/s (20 % of the audio) at 300 ms. The hypothesis's 64 kbit/s figure understates the throttle at the floor.
  - After one ordinary loss from 14720 B: 10304 B, about 19 packets usable, enough up to RTT ≈ 380 ms. After two loss events: 7213 B → about 13 packets → enough up to about 260 ms. After three: 5049 B → about 8 packets → enough up to about 160 ms.
- How long a datagram can wait in QUIC under the guard: at most about 11 packets ahead of it, drained at whatever rate the path allows. In a full stall (no ACKs), they wait for the whole stall. That is the mechanism behind finding 2 of the verdict.

### Gaps
- The congestion window, `congestion_events` and in-flight bytes during the share were never logged. Whether Cubic ever reached its floor is unknown.
- The negotiated MTU was not logged.

## Key question 2: Does the configured datagram send buffer allow seconds of stale audio to queue? Does anything drop stale audio?

### Takeaway
For Opus, no: the 4 KiB guard keeps QUIC's queue to about 220 ms of audio. But it keeps the *oldest* audio and throws away the *newest*, which is the wrong end for real-time. For PCM (no guard), 20 KiB is about 107 ms of 48 kHz stereo, dropped oldest first. Nothing drops by age anywhere on the sender. Audio already handed to the socket cannot be recalled, and the congestion window decides how much of that there is.

### Cited findings
- The guard skips new frames and leaves the queued ones (`audio.rs:555-559`). QUIC sends FIFO (`datagrams.rs:267`).
- 23:27 share, phone window ending 23:28:18.968 (phone clock): `captured_frames` 5000→6000 (500 Opus frames due), `packets` 2493→2873 (+380), so **120 frames skipped**. `quic_lost` 2→57 (+55). `encoder_dropped=0`, `max_frame_gap_ms=27` (capture normal). Source: `phone-share-stats.txt`.
- The matching PC window, `[39262.187]` (23:28:18): `packets +317`, `lost` 2→65, `max_gap_ms=3943`, `max_age_ms=2654`, `quic_lost=0` (the PC's own sending lost nothing), `rtt_ms=88`. Playout at `[39262.187]`: `max_sent_gap_ms=1471`. Phone Wi-Fi link speed fell to 34 Mbit/s at 23:28:13.969 (`phone-health.txt`). PC pings to the phone were lost at 23:28:12 and 23:28:14 (`pc-ping.txt`).
- Bad share, skipped frames by phone window (phone clock, `deficit = captured/2 − packets`): 22:28:03 (36), 22:28:43 (15), 22:28:53 (4), 22:33:33 (80, with `quic_lost` +78). All other windows are within ±1. In total 136 frames (2.7 s) over 47 windows (`phone-share-stats.txt`).
- WebTransport gives senders `outgoingMaxAge`: queued datagrams older than it are removed before sending — [W3C WebTransport §4.3, §5.1](https://www.w3.org/TR/webtransport/). RFC 9221 §5.4 lets implementations "optionally support allowing the application to specify a sending expiration time... beyond which a congestion-controlled DATAGRAM frame ought to be dropped" — [RFC 9221](https://www.rfc-editor.org/rfc/rfc9221.html). noq has no such option (`datagrams.rs` only).

### Inferences
- In the 23:28 outage, the about 10 packets held in QUIC were from the start of the stall. When the link came back they went out first, followed by a 2.4 s hole. `max_sent_gap_ms=1471` is that hole as the receiver saw it, not a capture stall (the phone's `max_frame_gap_ms` was 27). Some of the 2654 ms `max_age` may also be packets that sat in Wi-Fi or driver queues. The logs cannot split the two.
- Fix: keep the newest, not the oldest. Remove the skip-new guard for Opus and let QUIC's own drop-oldest do the bounding. Size the datagram buffer to about 2-3 KiB (5-7 packets, 100-140 ms) for the Opus connection. Or, in the app, check the buffer space and accept that the oldest is dropped. Expected effect in the 23:28 event: the first audio after recovery would be at most about 140 ms stale instead of up to the stall length (about 2.6 s). The amount of audio lost is the same.
- This does not change audio already in flight below QUIC. Bounding that needs a capped window (key question 6).

### Gaps
- The time each datagram spends in the queue is not observable: noq has no hook when a datagram leaves the queue.

## Key question 3: Can the logged numbers separate air delay from queue delay? What is `rtt_ms`? What smallest log line to add?

### Takeaway
Partly. The phone's skip count (`captured/2 − packets`) bounds QUIC queueing in each window, and it shows the long gaps were mostly below QUIC. `rtt_ms` is a smoothed estimate. The PC's value in particular is nearly blind: the PC sends almost nothing ack-eliciting. QUIC's round trip starts when a packet is built, so it never includes time in the datagram queue. A small sender-side line would close the question.

### Cited findings
- `rtt_ms` = `conn.rtt(selected path)` (`audio.rs:514-520`) = `RttEstimator::get()` = smoothed RTT (noq-proto `connection/paths.rs:801-803`), updated as `7/8 × old + 1/8 × sample` (`paths.rs:838`), matching RFC 9002 §5.3.
- A sample is taken only when an ACK newly acknowledges the largest packet and includes an ack-eliciting packet (`mod.rs:3038-3057`; RFC 9002 §5.1). It is measured from the packet's `time_sent`, set when the packet is built (`connection/packet_builder.rs:292`). So time in `DatagramState.outgoing` is not in the RTT. Lost packets give no sample.
- The PC is the receiver. Its ack-eliciting traffic is the 5 s heartbeat (`lib.rs:43, 1066-1076`) and keep-alives (`tuning.rs:87`); ACK-only packets are not ack-eliciting. So the PC's RTT gets a sample every few seconds and moves 1/8 per sample. That explains `rtt_ms=60` at `[35647.991]` (22:28:03) next to `max_gap_ms=1995` and `max_age_ms=1980`. The phone (sender, 50 ack-eliciting packets/s) reported `rtt_ms=95` and then `148` in the same windows (`phone-share-stats.txt` 22:28:03.580, 22:28:13.575).
- `max_age_ms` is arrival minus sender clock, against a 15 s rolling minimum (`audio.rs:194-220`). It includes everything after `send_frame`: QUIC queue, socket, air, PC.
- Join of the PC and phone windows (bad share, after 22:27:50): 33 PC windows; 30 had `max_gap_ms ≥ 400`; 24 of those had `max_sent_gap_ms ≤ 60`; 9 had a gap ≥ 1000 ms. The phone skipped no frames in the overlapping windows for all but the four windows listed under key question 2. `lost` was 0 in most, e.g. `[35698.182]` gap 1387 with `lost` +1; `[35750.250]` gap 1594 with `lost` +0 (tray.log).
- Stats noq exposes: `PathStats { rtt, cwnd, congestion_events, spurious_congestion_events, lost_packets, lost_bytes, udp_tx, current_mtu, ... }` (noq-proto `connection/stats.rs:220-250`; `path_stats` fills `cwnd` at `mod.rs:2685-2692`). These are reachable through iroh's `Connection::stats()`/`congestion_state()` (iroh `endpoint/connection.rs:1062-1068`) or per path (noq `path.rs:178`).

### Inferences
- A 1.5 s gap with no skips and `lost=0` means about 75 packets were held somewhere and then delivered together. At most about 11 of those could have been in QUIC's queue. The rest were in flight below QUIC, so the window was at least about 30 KB then. Most of each long gap was air and OS delay.
- Smallest line to add, on the **sender** (the phone's 10 s `stats_line`, FFI `pb-ffi/src/audio.rs:90-104` → `AudioSession::stats`). Sample at every `send_frame`, report per window:
  `AUDIO send skipped=<frames the guard skipped> backlog_max_b=<max(DATAGRAM_SEND_BUFFER − datagram_send_buffer_space())> cwnd_min=<min path_stats.cwnd> cwnd_now=<cwnd> cong_events=<congestion_events Δ> spurious=<Δ> lost_pkts=<lost_packets Δ> udp_tx=<udp_tx.datagrams Δ> mtu=<current_mtu> rtt_ms=<smoothed>`.
  How to read it: if a gap shows `udp_tx` still rising and `backlog_max_b` small, the audio is stuck below QUIC (air or driver). If `udp_tx` is flat and `backlog_max_b` near 4 KiB, QUIC is holding it. If `cwnd_min ≤ rate × RTT + MTU`, the window throttled.
- On the receiver, use the phone's sender-side RTT, not the PC's, as the path's round trip.

### Gaps
- Per-datagram queue time needs a noq change (a timestamp on `Datagram` and a stat when it is written). It cannot be done from the app.

## Key question 4: Does the 15 s control-stream watchdog end a share while audio could still flow? How should it change? Do migration and iroh path handling apply to the 2412 → 5580 MHz roam?

### Takeaway
On 22:33:41 the watchdog fired during a real outage: no audio was arriving either. PTO backoff is capped at 1 s steps here, so it does not stretch the stream's recovery much. But the outage lasted about 40 s, past both the 15 s watchdog and the 30 s QUIC idle timeout. The roam kept the same IP, so the connection could have resumed in place with longer timeouts. Relay never tells iroh about network changes on Android.

### Cited findings
- Watchdog: `Session::recv` fails with `Timeout` after `HEARTBEAT × 3` = 15 s since the last control frame (`lib.rs:1093-1100`). The audio control loop ends the share on it (`audio.rs:851-866`) and closes the connection (`audio.rs:870`).
- PC: `[35984.957] AUDIO phone_to_pc ended after 488 s: LinkLost (control channel silent for 15 s (heartbeat timeout))`. The last `AUDIO arrivals` line is `[35963.704]` (22:33:19); that line is printed only when a frame arrives (`pb-tray/src/audio.rs:312-345`), so no audio arrived for about 20 s before the end. Phone: 22:33:42.010 `frame delivery failed: reason=connection closed`, then `LINK_LOST` (`events-log.txt`). The phone's last window, 22:33:33, had `quic_lost` +78 and 80 frames skipped.
- Roam: phone logcat 22:34:04.888 `NL80211_CMD_ROAM`, 22:34:05.006 `CTRL-EVENT-CONNECTED - Connection to 70:bc:48:1c:25:8f`. State 22:34:13.627 `ip=10.21.12.26 freq=5580` (same IP as before). PC pings answer again from 22:34:05 (`pc-ping.txt`).
- PTO: `pto_base = srtt + max(4 × rttvar, 1 ms)` (`paths.rs:819-821`) plus max_ack_delay (25 ms default, `transport_parameters.rs:62`). It doubles per probe, but each step is capped at `+1 s` when the idle timeout is ≤ 25 s (`mod.rs:3587-3600, 3643-3657`; constants `mod.rs:7589-7598`). iroh sets a 15 s path idle timeout (iroh `socket.rs:117`; set in `endpoint/quic.rs:157-160`), so the 1 s cap applies.
- Idle: noq's default `max_idle_timeout` is 30 s (`config/transport.rs:560`). Neither iroh's builder (`endpoint/quic.rs:155-163`) nor Relay (`tuning.rs:85-91`) changes it. The idle timer is `max(timeout, 3 × PTO)` (`mod.rs:3865-3878`), as RFC 9000 §10.1 requires — [RFC 9000](https://www.rfc-editor.org/rfc/rfc9000.html). The per-path idle timer cannot close the last open path locally: `close_path_inner` returns `LastOpenPath` (`mod.rs:635-642`, used at `:2542-2547`).
- Network changes: on Android, netwatch's route monitor does nothing ("Very sad monitor. Android doesn't allow us to do this", `netwatch-0.19.3/src/netmon/android.rs`). iroh asks apps to call `Endpoint::network_change()` (iroh `endpoint.rs:1650-1671`). That call pings the paths, resets the PTO backoff for recoverable paths (noq-proto `mod.rs:5948-5967`), and opens a new path when the local IP is gone (`mod.rs:5907-5945`; iroh's hint at `socket.rs:1698-1756`). Relay's `PbService` network callback only calls `rediscoverSoon()` (`android/.../PbService.kt:288-299`), and no Rust code in the project calls `network_change`.

### Inferences
- Change the watchdog to "pause, don't end". Keep the share while the QUIC connection is alive. Show "reconnecting" after about 3 s without audio. End only after a long limit (about 60 s), or when the connection closes. Raise `max_idle_timeout` to about 60 s, so QUIC does not kill the connection first. The 22:33 outage (about 22:33:20 to 22:34:05, about 45 s) would have been survived with 60 s limits, and audio would have resumed on the same path, since the IP did not change.
- Call `Endpoint::network_change()` from `onAvailable` and `onLinkPropertiesChanged` through the FFI. After a roam, probes would go out at once instead of waiting for the next backed-off PTO, at most about 1 s plus the PTO.
- If the connection still dies, the PC can re-dial and re-request the same share id with consent already given, keeping its output device open. Relay has no such resume today; it would be a protocol addition.

### Gaps
- When the PC last received any packet was not logged, so the margin to the QUIC idle timeout is unknown.
- Whether the roam's 10.21.x.x address and port reached the PC unchanged was not captured (only the phone's IP).

## Key question 5: What do real-time QUIC systems do? Is a fixed or no-op controller acceptable? Custom controller per connection? Separate connection or UDP?

### Takeaway
Standards require congestion control for QUIC datagrams, and the real-time ones favour delay-based control plus dropping stale media at the sender. libwebrtc does not pace audio at all. A per-connection custom controller is possible in noq and iroh. A separate connection for audio is sensible, mainly so a capped, latency-oriented controller does not slow file transfers. A no-op controller is defensible on a LAN at 160 kbit/s only with a circuit breaker.

### Cited findings
- RFC 9221 §5.4: "DATAGRAM frames employ the QUIC connection's congestion controller. The sender MUST either delay sending the frame until the controller allows it or drop the frame without sending it" — [RFC 9221](https://www.rfc-editor.org/rfc/rfc9221.html).
- RTP over QUIC (draft-ietf-avtcore-rtp-over-quic-14, 2025-03-20, expired while in WG last call): "Senders are required to employ some form of congestion control" (§8.1). "For real-time traffic, it is best that the QUIC implementation uses a congestion controller that aims at keeping queues" low (§8.1), naming SCReAM, NADA, GCC and L4S. It warns about "the interaction of two nested congestion controllers" (§8.2). It notes RTP packets get "buffered while QUIC responds to detected packet loss" (§3.1.2) — [draft](https://datatracker.ietf.org/doc/html/draft-ietf-avtcore-rtp-over-quic), [status](https://datatracker.ietf.org/doc/draft-ietf-avtcore-rtp-over-quic/).
- Media over QUIC transport (draft-23, 2026-11-02) §14.1: CUBIC and Reno are prone to bufferbloat (§14.1.1). Latency-sensitive media is application-limited, and controllers that grow only when fully used underestimate bandwidth (§14.1.2). BBR's PROBE_RTT and Reno's halving cause rate drops that hurt latency-sensitive applications (§14.1.3) — [draft](https://datatracker.ietf.org/doc/html/draft-ietf-moq-transport).
- WebTransport offers `congestionControl` = `"default" | "throughput" | "low-latency"` (§6.9) and a per-datagram `outgoingMaxAge` with expiry before sending — [W3C WebTransport](https://www.w3.org/TR/webtransport/).
- libwebrtc: "Don't pace audio by default... After experimentation, not pacing audio is better" (Evan Shrubsole, 2020-01-08) — [commit](https://webrtc.googlesource.com/src/+/6ef59d1ced564efd9db76d6d2396c67eb0e65884%5E%21/). The pacer header says "Currently audio traffic is not accounted by pacer and passed through" — [pacing_controller.h](https://webrtc.googlesource.com/src/+/HEAD/modules/pacing/pacing_controller.h).
- GCC: delay-based estimate (inter-group delay variation, Kalman filter) plus a loss-based one: keep the rate at 2-10 % loss, cut above 10 %, grow 5 % under 2 %. A pacer sends bursts every 5 ms — [draft-ietf-rmcat-gcc-02](https://datatracker.ietf.org/doc/html/draft-ietf-rmcat-gcc-02). SCReAM: self-clocked window with a queue-delay target (0.1 s low, 0.4 s high). The sender "SHOULD" discard RTP packets when throughput becomes very low — [RFC 8298](https://www.rfc-editor.org/rfc/rfc8298.html). NADA: rate-based, uses delay, loss and ECN, evaluated over Wi-Fi — [RFC 8698](https://www.rfc-editor.org/rfc/rfc8698.html).
- Safety: RFC 8085 §3.1.9 lets applications skip congestion control on reserved capacity, but this is "by no means a safe practice for operation over the wider Internet". §3.1.10: applications without congestion control SHOULD implement a circuit breaker — [RFC 8085](https://www.rfc-editor.org/rfc/rfc8085.html). RTP circuit breakers (RFC 8083): stop after no reports for 3 intervals, cut 10× when the rate exceeds 10 × the TCP-friendly rate, stop if the media is unusable — [RFC 8083](https://www.rfc-editor.org/rfc/rfc8083.html).
- Counterpoint: "Never* use Datagrams" (Luke Curley, 2024-02-17) argues for delay-based control (BBR) plus prioritised streams over datagrams — [moq.dev](https://moq.dev/blog/never-use-datagrams/).
- noq: `congestion_controller_factory` takes any `ControllerFactory` (noq-proto `config/transport.rs:346-350`; trait at `congestion.rs:150-153`; `Controller` trait at `congestion.rs:17-131`). A controller is built for each path (`paths.rs:299`). iroh re-exports the trait and `RttEstimator` (iroh `endpoint/quic.rs:51-72`, builder `:420-424`).
- Per connection: the dialler can pass its own config with `ConnectOptions::with_transport_config` (iroh `endpoint.rs:1792-1810`, `connect_with_opts` at `:1108`). The acceptor can use `Incoming::accept_with(ServerConfig)` (iroh `endpoint/connection.rs:165-172`). The PC dials (`pb-tray/src/audio.rs:172` → `open_audio` → `connect`, `lib.rs:583-642`). The phone accepts and is the audio sender, so the phone's acceptor is where the audio controller must be set.

### Inferences
- A no-op controller (a huge window) would remove any throttling, but it would also let unlimited stale audio pile up below QUIC during a stall. That is the dominant delay found here. It is the wrong direction for latency.
- Better: a separate audio connection (its own ALPN, accepted with `accept_with`) using a fixed-rate controller. Window = audio rate × max(srtt, 50 ms) × 2 + 2 MTU, capped at about 200-300 ms of audio (about 6-9 KB). Pacing just above the audio rate. Datagram buffer about 2-3 KiB with drop-oldest. Add an app-level circuit breaker: stop or drop to a lower Opus bitrate if loss is over about 10-20 % for several seconds, or if no ACK arrives for several seconds. Files and control keep Cubic on the main connection, so the cap does not limit them, and their losses do not shrink audio's window.
- Plain UDP would add its own encryption, NAT and identity work that QUIC/iroh already does. Not worth it.

### Gaps
- No public numbers were found for nested or capped controllers over Wi-Fi. The Engelbart & Ott EPIQ'21 paper exists ([TUM](https://www.ce.cit.tum.de/cm/news-single-view/article/paper-on-early-exploration-of-real-time-media-congestion-control-for-quic-accepted-at-the-acm-epiq-workshop/)), but its results could not be fetched.
- No quinn or noq issue on real-time datagram controllers was found.

## Key question 6: Which controller is best here (Cubic, BBR3, NewReno, or a custom one with a floor rate)? Expected effect and risks.

### Takeaway
With these logs, changing the controller would remove little measured delay. Two changes target what was actually seen: drop-oldest at the sender, and capping in-flight audio. If you keep one connection, Cubic is fine and NewReno is worse. BBR3 is untested here. A custom "floor and cap" controller on a separate audio connection is the best fit.

### Cited findings
- NewReno halves on loss (`new_reno.rs:100, 167`), with the same 2 × MTU floor (`new_reno.rs:38-40, 104-106`). It is more prone to throttling than Cubic's 0.7 cut.
- BBR3 drives the pacer by rate (`pacing.rs:97-99`; metrics at `bbr3/mod.rs:1859-1866`). App-limited samples cannot lower `max_bw` (`bbr3/mod.rs:752-760`). ProbeRTT every 5 s for 200 ms at 0.5 × BDP (`bbr3/mod.rs:74-82`). Its window floor is `min_pipe_cwnd = 4 × SMSS` (`bbr3/mod.rs:641`). It reacts to loss over 2 % with β 0.7 (`bbr3/mod.rs:86-90`). Relay can already select it (`tuning.rs:101-109`). MoQ §14.1.3 names ProbeRTT as a latency risk.

### Inferences (expected effects)
- **Drop newest → drop oldest:** in an outage, the first audio after recovery becomes ≤ about 140 ms stale instead of up to the outage length (about 2.6 s at 23:28, about 1-2 s at 22:27:53-22:28:03). Packets lost: the same. Risk: none known; it is how QUIC's own buffer already works.
- **Capped audio window (about 250 ms of audio in flight) on its own connection:** during a stall, at most about 250 ms of audio piles up below QUIC instead of the about 1-1.5 s seen (≥30 KB in flight). After recovery, that audio arrives at most about 250 ms stale plus the air delay. The cost is more audio skipped during stalls, which is the intended trade. Risk: on a link whose round trip really goes over about 250 ms, the cap itself throttles. So set the cap from the RTT (e.g. max(250 ms, 1.5 × srtt) of audio) and pair it with a floor.
- **Floor rate (window ≥ audio rate × srtt + 2 MTU):** stops Cubic's 2 × MTU floor from starving audio after persistent congestion at RTT > 60 ms. The gain is unmeasured in these logs, and likely small. Risk: it keeps sending 160 kbit/s into a truly saturated link. That is under 1 % of even a degraded 2.4 GHz link (phone link speed 34-143 Mbit/s here), and a circuit breaker covers the extreme case.
- **BBR3 in place of Cubic:** unknown effect. Measure it with `tools/pb-bench` and the new sender line before choosing it.

### Gaps
- None of the controller comparisons were measured on this phone. The sender line from key question 3 plus a bench sweep is needed before a choice.
- **Other context, out of scope:** ColorOS's per-app shaper (`OplusLimitSpeedController`, HTB) was active for Relay's UID 10767 at 600 kbit/s from 22:30:47 to 22:31:46, and again at 147 → 1997 → 998 → 600 kbit/s from 22:33:13 to 22:33:37 (phone logcat `DataLimitCmd` lines). That is above Opus's 160 kbit/s; its effect on these gaps is unknown. In `pc-ping.txt`, "lost" pings were stamped about 40 ms after successful ones, not after the 800 ms timeout (`cap_pc.py:320-321`). So they were fast errors, not timeouts, and ping loss is a poor proxy for the audio path. This is for the Wi-Fi-layer researcher.
