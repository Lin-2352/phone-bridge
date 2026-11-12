# Loss recovery, PLC and adaptive jitter buffering for low-delay music/game audio on bursty Wi-Fi

Method note: about 30 tool calls (over the usual budget). Web fetches go through a summarising model, so numbers quoted below are what the summaries reported, not verified against full source text. Anything marked (memory) comes from my own background knowledge with no source retrieved, and anything marked (inference) is my reasoning. Not found at all: peer-reviewed listening tests of time-scale modification on music, Apple's jitter-buffer policy, and the libopus CELT PLC source (celt_decoder.c). Treat those as gaps.

## Relay's current setup (read from the code in the "phone bridge audio" worktree, not "phone bridge")

Note for the writer: the files named in the brief are in `D:\Desktop\Project\phone bridge audio`, and the main checkout has no audio code.

### Takeaway
Relay sends one Opus packet per 20 ms as an unreliable QUIC datagram. There is no FEC, no redundancy, no in-band FEC and no time-stretch. The only recovery is decoder PLC (decode with NULL), silence fill for gaps up to 40 ms, and the PC-side queue with its trim and skip logic. The Android encoder is MediaCodec, so most libopus controls are not reachable.

### Cited Findings
- PCM or Opus goes in QUIC datagrams, never a stream, to avoid the seconds of buffered audio a stream would build up during a Wi-Fi stall. Each datagram has an 8-byte header (u32 seq plus u32 sender ms). The sender's QUIC buffer holds about 100 ms, and the receiver queue drops the oldest audio. — [crates/pb-transport/src/audio.rs](D:\Desktop\Project\phone bridge audio\crates\pb-transport\src\audio.rs) lines 1-24
- Constants: QUEUE_MS 100; FILL_MAX_MS 40 (a gap up to 40 ms is filled with silence, a longer one is treated as a stall); OPUS_SEND_BACKLOG 4 KiB (about 200 ms at 160 kbit/s, packets beyond that are skipped on the sender); REORDER_WAIT 15 ms with REORDER_HOLD 8 packets. — same file, lines 43-96
- Receiver audio_out constants: START_TARGET 120 ms, MIN_TARGET 60 ms, MAX_TARGET 160 ms, SLACK 60 ms, FADE 5 ms, RING 500 ms, MAX_TRIM 5000 ppm (0.5%), TRIM_PPM_PER_MS 1000, target-window 2 s x 15 windows with 2 outliers ignored, margin 40 ms. — [audio_out.rs](D:\Desktop\Project\phone bridge audio\crates\pb-platform-win\src\audio_out.rs) constants
- The PC decoder is libopus transpiled to Rust (`unsafe_libopus`). `opus_decode(NULL)` is used for loss, with fec=0 always. A pure-Rust decoder was tried and took 27 ms per 20 ms packet. — [opus.rs](D:\Desktop\Project\phone bridge audio\crates\pb-platform-win\src\opus.rs)
- The Android encoder is MediaCodec (`MIMETYPE_AUDIO_OPUS`, 48 kHz, 160 kbit/s, CBR). It sets only bit rate, PCM encoding and bit-rate mode. There is no complexity, FEC, signal type or frame-duration setting. The comment says ColorOS capped Relay at 600 kbit/s with a 3000-packet queue behind it during video, which is why Opus is used. — [OpusEncoder.kt](D:\Desktop\Project\phone bridge audio\android\app\src\main\java\dev\phonebridge\app\OpusEncoder.kt)
- AOSP's software Opus encoder (C2SoftOpusEnc) is a wrapper over libopus. It calls OPUS_SET_APPLICATION(OPUS_APPLICATION_AUDIO), OPUS_SET_COMPLEXITY (default 10, range 1-10), OPUS_SET_SIGNAL(OPUS_AUTO), OPUS_SET_BITRATE and VBR or CBR. It does not set inband FEC, packet loss percentage or bandwidth, so libopus defaults apply (FEC off). DTX is effectively off. Seek pre-roll is 80 ms. The frame duration constant was not visible to the summariser, so 20 ms is not confirmed from AOSP (Relay's own code assumes 20 ms). — [AOSP C2SoftOpusEnc.cpp](https://android.googlesource.com/platform/frameworks/av/+/refs/heads/main/media/codec2/components/opus/C2SoftOpusEnc.cpp)

### Inferences
- With FEC off and defaults, Relay's mode is AUDIO + AUTO signal, so libopus picks CELT or hybrid by bitrate. At 160 kbit/s stereo, it will be CELT-only fullband (inference from libopus mode logic; not source-verified here).
- Gaps in the bursts (480-1995 ms) are far longer than the 40 ms silence fill or any PLC, so the observed artefacts come from the queue policy (skip and trim), not from the codec's concealment.

### Gaps
- Whether the phone's MediaCodec encoder is c2.android.opus.encoder on the user's devices (Nothing, realme) or a vendor encoder was not checked.

## 1. WebRTC NetEq: target delay, time-stretching, concealment

### Takeaway
NetEq targets the 95th percentile of inter-arrival time from a slowly forgetting histogram, takes the max with a detected delay peak, and uses pitch-synchronous (WSOLA-like) accelerate and preemptive-expand operations that change duration without changing pitch. For a 0.5-2 s outage it does not time-stretch through it: it expands, then merges, and with a deep buffer it fast-accelerates. It was designed for speech, and its stretch only fires when the signal is periodic enough.

### Cited Findings
- Per-tick (10 ms) decision flow: use buffered sync samples if at least 10 ms; else decode the next packet and compare the filtered buffer level to the target, time-stretching if too high or low; else comfort noise for DTX; else conceal by extrapolating or using decoder PLC. Operations: Normal, Accelerate, Preemptive Expand, Expand, Merge, CNG. Late packets (e.g. reordered) are discarded. — [NetEq g3doc](https://webrtc.googlesource.com/src/+/HEAD/modules/audio_coding/neteq/g3doc/index.md)
- Default target-delay mode: a 65-bucket histogram (kMaxIat=64) of inter-arrival time in packet units, forget factor 32745/32768 (about 0.9993 per packet), target = the 0.95 quantile. A peak detector takes the max of the target and the recent peak height. Floor is 1 packet, base start is 4 packets, and it is capped at 75% of packet-buffer capacity or at max_delay (0 means unset). A field trial switches to relative-arrival-delay mode: 100 buckets of 20 ms, target = 1 + bucket*20/packet_len. — [delay_manager.cc (webrtc-uwp mirror)](https://raw.githubusercontent.com/webrtc-uwp/webrtc/master/modules/audio_coding/neteq/delay_manager.cc)
- Decision thresholds: Accelerate when the filtered level is at or above high_limit; Fast Accelerate when at or above 4 x high_limit; Preemptive Expand when below low_limit. Time-stretch is only allowed if the previous mode was not Expand. After Expand the next packet gets Merge. After many consecutive expands the decoder resets. — [decision_logic.cc](https://raw.githubusercontent.com/webrtc-uwp/webrtc/master/modules/audio_coding/neteq/decision_logic.cc)
- Time-stretch core: downsample to 4 kHz, autocorrelate, pick one peak as the pitch period; compare two period-length segments at the 15 ms point; "active speech" if the average energy exceeds 8x background noise energy (or a fixed 75000 if no noise estimate). Needs at least about 30 ms of input. Accelerate removes one period by cross-fading it into the signal after a 15 ms head, so each operation changes length by one pitch period. Fast mode (the threshold falls to 0.5 correlation, 8192 in Q14) removes the largest multiple of the pitch period that fits in 15 ms. Passive (low-energy) signals are stretched anyway with correlation set to 0. — [time_stretch.cc](https://raw.githubusercontent.com/webrtc-uwp/webrtc/master/modules/audio_coding/neteq/time_stretch.cc), [accelerate.cc](https://raw.githubusercontent.com/webrtc-uwp/webrtc/master/modules/audio_coding/neteq/accelerate.cc)
- A Rust port of NetEq describes the preemptive-expand lower threshold as the larger of 75% of target and target minus 85 ms, attributed to WebRTC tuning; the port's constant, not verified in WebRTC itself. — [neteq crate search result](https://docs.rs/crate/neteq/0.8.3/source/src/neteq.rs)
- The normal-mode correlation threshold is about 0.9 (kCorrelationThreshold = 14746 in Q14) and Preemptive Expand uses the same machinery with duplication of a period instead of removal (memory; the fetched summary did not show the normal value).
- Expand (concealment) is a pitch-repeat plus noise-shaped synthesis with a mute factor that fades, and Merge cross-correlates the new packet with the expanded signal to align and fade in (memory, general NetEq knowledge; not verified from the retrieved text).
- Academic: WSOLA applied packet by packet in a voice jitter buffer was reported to cause no loss in subjective quality, and the authors argue larger playout jitter is perceptually tolerable when audio is time-scaled. This is a speech result. — [Liang, Farber, Girod, 2003](https://web.stanford.edu/~bgirod/pdfs/LiangMM2003.pdf) (as summarised by the search snippet)
- A black-box study of how much time-scale modification WebRTC's jitter buffer introduces and its effect on perceived speech quality exists (Cinar, Melvin, Pocta); I did not retrieve its numbers. — [University of Galway record](https://research.universityofgalway.ie/en/publications/a-black-box-analysis-of-the-extent-of-time-scale-modification-int/)

### Inferences
- Comparison with Relay: NetEq's per-operation change is tiny (one pitch period, about 2.5-15 ms, per 10 ms tick), but it can run every tick, so it can catch up at a much faster effective rate than 0.5% (up to tens of percent) while keeping pitch. Relay's 0.5% trim (5000 ppm, about 5 ms per second) would take 20 s to remove 100 ms, which is why the hard skip carries all the work in a 480-1995 ms burst. Raising the trim is the cheap lever, but with sample-rate interpolation any trim above about 0.5-1% shifts pitch (about 8.6 cents per 0.5%; (inference, arithmetic)), so the user hearing "speeding up and slowing down" is plausibly the trim and skip cycles. A pitch-preserving stretch only avoids this if it finds a good period.
- For polyphonic music and game mixes the single-pitch-peak correlation will often be low, so NetEq-style stretch would often decline (normal output) or leave artefacts. Use a WSOLA with a longer search window and a larger overlap (10-20 ms), or restrict catch-up to low-energy moments (inference). NetEq's own limit is that its design is speech-centric: 4 kHz downsampled pitch search, a speech VAD, and a 15 ms reference.
- NetEq handles a delay spike by: (a) the peak detector raising the target for a while, (b) Expand then Merge when the packet arrives, (c) Fast Accelerate when the buffer is above 4x the high limit. With a hard cap of 150 ms it cannot absorb a 500-2000 ms outage; no buffer can. What matters is recovery behaviour after the gap.

### Gaps
- Exact `high_limit`/`low_limit` derivation, the peak detector window and constants, and the precise Merge and Expand algorithms were not retrieved. Read modules/audio_coding/neteq/*.cc directly.

## 2. Other jitter buffers: Snapcast, Roc, GStreamer, Moonlight, Apple

### Takeaway
Policies split into two families. Fixed buffer with hard resync plus tiny drift correction (Snapcast: 500 ms buffer, drop or insert silence above 500 ms error, soft sync of at most 0.05%), and adaptive with tuner profiles (Roc: adaptive latency, scaling clamped at 0.5%, cooldowns of 5-15 s). Moonlight keeps no big buffer: it gives up on a missing block quickly and relies on 4+2 Reed-Solomon. Apple was not found.

### Cited Findings
- Snapcast client: target buffer defaults to 500 ms. Age = (server now - chunk start) - buffer + DAC time. Hard sync if |age| > 500 ms, or if median-based windows (500/100/20 samples) exceed 2 ms, 5 ms or 50 ms. Hard sync inserts silence if early or seeks forward and drops old chunks if late. Soft sync adjusts the rate by up to 0.05% (cap 0.0005) by duplicating or skipping single frames. Chunks older than 5 s + buffer are dropped. — [snapcast client/stream.cpp](https://raw.githubusercontent.com/badaix/snapcast/master/client/stream.cpp)
- Roc Toolkit latency config: adaptive mode when target_latency is 0; tuner profiles Intact, Responsive (fast, for lower latency and jitter) and Gradual (slow, for higher latency and jitter); min/max target latency bounds; scaling_interval 5 ms; scaling_tolerance 0.005 (freq coefficient clamped to 1 +/- 0.5%); latency_decrease_relative_threshold 1.7 (decrease target only when current latency is at least 1.7x the estimated optimum); starting_timeout 5 s, cooldown_dec_timeout 5 s, cooldown_inc_timeout 15 s; mean_jitter_overhead 3.0 and max_jitter_overhead 1.2 for estimating expected latency from jitter stats; stale_tolerance disables checks when the queue is stalling. — [roc latency_config.h](https://raw.githubusercontent.com/roc-streaming/roc-toolkit/develop/src/internal_modules/roc_audio/latency_config.h)
- Roc 0.3 added adaptive latency (dynamic jitter buffer size by network condition) and notes better scaling precision gives smaller latency oscillations. Roc supports FECFRAME with Reed-Solomon and LDPC-Staircase via OpenFEC (CeCILL licence caveat). — [Roc 0.3 notes](https://gavv.github.io/articles/roc-0.3/), [roc repo](https://github.com/roc-project/roc)
- Moonlight audio queue (RtpAudioQueue.c): 4 data + 2 parity Reed-Solomon shards per block (blocks aligned to 4 sequence numbers). In-order packets are returned with no buffering. A block is given up as soon as the next block is queued unless out-of-sequence packets were seen; then it waits one block duration plus an OOS wait time. Given-up packets are emitted as zero-length placeholders so the caller runs PLC. — [RtpAudioQueue.c](https://raw.githubusercontent.com/moonlight-stream/moonlight-common-c/master/src/RtpAudioQueue.c); same 4+2 scheme documented for Wolf/Sunshine: [Wolf RTP/Opus](https://games-on-whales.github.io/wolf/stable/protocols/rtp-opus.html)
- GStreamer rtpjitterbuffer: waits for missing packets up to `latency`, treats later arrivals as lost; `drop-on-latency` drops the oldest buffers when full (default FALSE in 1.14); `rtx-delay` is the extra wait before asking for retransmission (default -1 = use max jitter); default mode slave. — [rtpjitterbuffer docs](https://GStreamer.freedesktop.org/documentation/rtpmanager/rtpjitterbuffer.html)

### Inferences
- The Snapcast and Roc numbers confirm Relay's 0.5% trim is the conventional bound for resample-based drift correction; both treat anything bigger as a reset. The difference is that they size the reset around a latency of 500 ms or more, while Relay is at 60-160 ms.
- Roc's asymmetry (decrease only at 1.7x the optimum, cooldown 5 s down and 15 s up) is a template for Relay's target: grow immediately on a burst, shrink slowly and rarely.
- Moonlight is the nearest analogue (game audio, tiny buffer, Wi-Fi). It uses FEC and gives up on a block quickly rather than buffering; its audio packets are 5 ms by default (memory; not retrieved), so each lost packet is short.

### Gaps
- Apple's jitter-buffer policy (AirPlay, FaceTime, CoreAudio) was not found in any retrievable source. Roc's tuner internals and the Moonlight 5 ms figure were not retrieved.

## 3. Best listening experience for 0.5-2 s bursts with a target of 150 ms or less

### Takeaway
No published perceptual study on this exact case was found. Reasoning from the mechanisms: for a late burst longer than the buffer, nothing can hide the gap; the choice is how the gap ends. Fade out cleanly, hold silence (not repeat, not a loop), and on resume do not play a stale backlog or play it fast: drop the backlog to the newest frames and fade in. Then let the target grow modestly and shrink slowly.

### Cited Findings
- Opus/CELT PLC: the patent texts describing CELT concealment say pitch-based concealment is used for the first five consecutive lost frames and a noise-like method after that, with a fade-out factor. These are patent filings, not the libopus source, so the numbers may not match Opus exactly. — [patent 10867613](https://image-ppubs.uspto.gov/dirsearch-public/print/downloadPdf/10867613) (note: this is a different codec's description from the same family of search results; do not rely on it for libopus)
- WSOLA-based time-scaling inside a voice jitter buffer is reported perceptually transparent for speech. — [Liang et al.](https://web.stanford.edu/~bgirod/pdfs/LiangMM2003.pdf)
- No source retrieved states a tolerable playout-rate range for music. A search for a 10% figure found nothing. — search, see Gaps

### Inferences (all my reasoning, none sourced)
- A gap of 500-2000 ms: keep concealment to at most about 40-60 ms (what Opus PLC can do before it turns noise-like), then fade to silence over 5-20 ms (Relay uses 5 ms, which risks a click or audible thump on loud content; 10-20 ms is safer), hold silence, and on the first fresh packet fade in. Repeating or looping audio longer than about 100 ms sounds worse than silence for music (inference).
- On resume the receiver gets a burst of the late backlog at once. Playing it at normal speed adds the whole gap as lag. Options, ranked by lag: (1) drop everything older than the target and play the newest (lowest lag, one audible discontinuity; this is what Relay's hard skip does); (2) play the backlog with pitch-preserving catch-up (no discontinuity but takes seconds at 10-20% speed, and during a game the audio is out of sync for all of it); (3) grow the target to ride the burst (no discontinuity, but it needs a target of one burst, which exceeds the 150 ms limit).
- Where the burst is shorter than about 150-250 ms, raising the target quickly (within one packet) and lowering slowly is clearly better than skipping. Above that, option (1) with a fade at both ends is better than any speed-up, because the speed-up makes both the lag and the artefact last longer.
- The most important change is to skip only once per burst and not repeatedly. 8-19 dropouts per 10 s suggests the policy skips, refills to the target, then underruns again because the next burst arrives before the buffer refills. Hysteresis (after a skip, hold the target higher for 5-15 s as Roc's cooldowns do) addresses that directly.
- Speed or pitch variation heard by the user ("speeding up and slowing down") is consistent with interpolation trim running at its maximum (5000 ppm gives about 8.6 cents) plus skips; at that level it is more likely the skips and underrun refills than the trim itself (inference).

### Gaps
- No perceptual literature on time-scale modification or concealment for music (MUSHRA-style or otherwise), and none on game audio, was found.

## 4. Opus features

### Takeaway
In-band FEC (LBRR) is a SILK-layer feature and is useless for 160 kbit/s CELT music. DRED is a speech-oriented 1.5 feature that needs the libopus encoder with a special build, which MediaCodec does not expose. Opus PLC for CELT is good for a few frames only. For Relay the viable gains are 10 ms or 20 ms frames, OPUS_APPLICATION_AUDIO (already what AOSP uses), and application-level redundancy.

### Cited Findings
- OPUS_SET_INBAND_FEC applies "only" to the LPC (SILK) layer. Value 1 makes the encoder switch to SILK at high loss even at high bitrates; value 2 enables FEC without switching music to SILK (so a CELT-only stream carries no FEC). Default is off. OPUS_SET_PACKET_LOSS_PERC 0-100 trades quality for loss robustness. OPUS_SET_DRED_DURATION enables DRED in units of 10 ms frames. OPUS_SET_SIGNAL(MUSIC) biases toward MDCT (CELT). — [libopus 1.5 encoder CTLs](https://opus-codec.org/docs/opus_api-1.5/group__opus__encoderctls.html)
- RFC 6716: LBRR frames are reduced-bitrate encodings of previous SILK frames, at most one LBRR frame per channel per 10 or 20 ms frame; documented only under SILK. Algorithmic delay: SILK frames of 10-60 ms plus a 5 ms look-ahead; CELT frames 2.5-20 ms plus 2.5 ms look-ahead; hybrid adds 2.7 ms to the CELT path; total 5-65.2 ms. DTX sends one frame every 400 ms in silence; a zero-length frame signals DTX or loss. — [RFC 6716](https://datatracker.ietf.org/doc/html/rfc6716)
- DRED (Opus 1.5): each packet can carry up to 1 s of redundant audio; overhead about 12-32 kbit/s for the full second ("each 20 ms packet is effectively sent 50 times"); build with --enable-dred (also enables deep PLC, +2 MB); about 1% CPU at run time; not standardised, so 1.5 DRED is not compatible with the final version; data is only used after loss is observed so it needs jitter-buffer integration. The article discusses speech only. — [Opus 1.5 demo](https://opus-codec.org/demo/opus-1.5/)
- Deep PLC (Opus 1.5): a neural concealment that is decoder-only (no compatibility issue); needs decoder complexity 5 or higher; --enable-deep-plc (+1 MB); about 1% of a laptop core at high loss. Described for speech. — [Opus 1.5 demo](https://opus-codec.org/demo/opus-1.5/); release page defers to the demo for details: [libopus 1.5 release](https://opus-codec.org/release/stable/2024/03/04/libopus-1_5.html)

### Inferences
- In-band FEC does not work for Relay: at 160 kbit/s stereo with OPUS_APPLICATION_AUDIO it is CELT, FEC is SILK-only, and AOSP's encoder does not set it anyway. Confirmed for the API doc and RFC 6716; the claim "CELT has no FEC" is an inference from the doc wording, and agrees with my memory.
- DRED is not available to Relay as built: the encoder is MediaCodec, and a mixed-in DRED encoder would require shipping libopus 1.5+ (JNI or Rust) on the phone and a DRED-enabled decoder on the PC. The PC's `unsafe_libopus` is a transpilation of an older libopus; whether it has DRED is unknown (gap). The DRED training is on speech; its 12-32 kbit/s overhead and speech-trained models make it unlikely to help music and game sound (inference), and it only repairs loss, not lateness, unless the packet never arrives within the window.
- Opus CELT PLC (memory plus patent text above): a usable few frames (about 5 x 20 ms = 100 ms) before it goes noise-like; for music, treat 20-40 ms as acceptable and anything over 60 ms as needing a fade. Opus frame size options: 10 ms halves per-packet delay and doubles packet rate (100/s; the header overhead and Wi-Fi airtime per packet rise); 40 ms or 60 ms cut packet rate but each loss is longer and delay grows by the extra frame. For a bursty-late pattern frame size barely changes the outcome; the burst is 480-1995 ms.
- Application mode: OPUS_APPLICATION_AUDIO (not VOIP) is right for mixed music and game media (VOIP applies speech-oriented settings and may select SILK or hybrid at lower rates) and it is what AOSP uses. RESTRICTED_LOWDELAY removes the SILK/hybrid modes and the 2.5 ms look-ahead (algorithmic delay about 5 ms instead of 7.5 ms: memory, from the Opus docs) and is an option if the extra 2.5 ms matters; it requires libopus (not MediaCodec). Delay difference is a few ms, small against a 60-160 ms buffer.
- MediaCodec cannot set FEC, DTX, signal type or frame size, so FEC-style options need either an app-level redundancy layer (below) or a JNI or Rust libopus on Android. A JNI libopus gives full control (frame size, complexity, VBR constraints), at the cost of maintaining a native build; given the PC side already links a Rust libopus, a shared Rust encoder in pb-ffi would be consistent (inference).

### Gaps
- libopus CELT PLC source (celt_decoder.c) was not read; real fade length and quality in music were not verified. DTX behaviour on MediaCodec is "effectively off" only per the AOSP summary. Which libopus version `unsafe_libopus` transpiles was not checked.

## 5. Redundancy: RED, duplicates, XOR/RS FEC, NACK

### Takeaway
Redundancy repairs isolated loss, not late bursts. Relay's data shows 32 lost of 23,201 packets (0.14%), so FEC would fix almost nothing; the burst pattern is a 0.5-2 s outage in which all redundancy sent in that window is equally late. A NACK cannot make it either at a 60-160 ms target.

### Cited Findings
- Moonlight's audio FEC is 4 data + 2 parity Reed-Solomon packets per block (50% overhead) with a block-granular give-up and PLC placeholders. — [RtpAudioQueue.c](https://raw.githubusercontent.com/moonlight-stream/moonlight-common-c/master/src/RtpAudioQueue.c), [Wolf docs](https://games-on-whales.github.io/wolf/stable/protocols/rtp-opus.html)
- An IETF draft states Reed-Solomon FEC offers optimal protection against random and bursty losses (a general statement, within the block size). — [draft-galanos-fecframe-rtp-reedsolomon](https://potaroo.net/ietf/all-ids/draft-galanos-fecframe-rtp-reedsolomon-01.html)
- GStreamer's rtpjitterbuffer exposes `rtx-delay` (extra wait before requesting retransmission), showing the usual tradeoff that a retransmission waits for the gap to be noticed first. — [rtpjitterbuffer docs](https://GStreamer.freedesktop.org/documentation/rtpmanager/rtpjitterbuffer.html)
- RFC 2198 (RED) and FlexFEC RFC 8627: not fetched. What I know (memory): RED sends previous frame(s) inside the next packet at the cost of that frame's bitrate again; FlexFEC is XOR-based parity over a window of packets.

### Inferences
- Costs at 400 bytes/packet and 50 packets/s: sending the previous frame in each packet doubles the payload to about 800 B per datagram (320 kbit/s), which is still one Wi-Fi frame under the typical QUIC datagram MTU of about 1200 B but doubles airtime; 4+2 RS adds 50% (240 kbit/s) and adds a block delay of 4 frames (80 ms) for a recovery; XOR over 4 packets adds 25% and fixes one loss per block. The bursts here are 24-100 packets long, so none of these span them, and even a 1 s DRED window would only help once the link returns, by which time the playout point is long past (inference).
- Extra airtime on a crowded 2.4 GHz channel can worsen the bursts (more retries, more contention), so redundancy may be net negative for this failure mode (inference; I found no measurement).
- NACK: a retransmission takes at least one RTT plus the detection delay (rtx-delay). With Wi-Fi bursts of hundreds of ms it cannot arrive within a 60-160 ms playout window; it only helps for single losses with an RTT of about 5-20 ms and then adds one RTT to the buffer need. Not useful here (inference).
- A better use of the spare bits is lower bitrate: if a 160 kbit/s stream with 64-96 kbit/s would be transparent for most games (inference), lowering it shortens airtime per burst and helps the link recover. Check for quality.

### Gaps
- No RFC 2198 / RFC 8627 text retrieved; no airtime measurements; no data on whether Relay's loss events are correlated with the bursts.

## 6. Sender-side freshness

### Takeaway
Relay already drops stale audio at the sender (about 100 ms datagram buffer, 4 KiB Opus backlog of about 200 ms), which is the right idea. The pattern in the brief (bursts of arrivals after gaps) suggests the stale data is held in the radio or driver and the phone's own queue, below what the app controls.

### Cited Findings
- Relay's sender caps unsent QUIC data and skips packets beyond about 200 ms of Opus, so a backlog is not built. — [audio.rs](D:\Desktop\Project\phone bridge audio\crates\pb-transport\src\audio.rs) lines 22-24 and 52-55
- Snapcast and Roc both drop or skip stale chunks on the receiver (Snapcast drops anything older than 5 s + buffer; Roc has stale_tolerance and a queue-stall mode). — [Snapcast](https://raw.githubusercontent.com/badaix/snapcast/master/client/stream.cpp), [Roc](https://raw.githubusercontent.com/roc-streaming/roc-toolkit/develop/src/internal_modules/roc_audio/latency_config.h)
- Moonlight emits zero-length placeholders and moves on rather than waiting; it does not retransmit audio. — [RtpAudioQueue.c](https://raw.githubusercontent.com/moonlight-stream/moonlight-common-c/master/src/RtpAudioQueue.c)

### Inferences
- Because every datagram carries the sender clock (ms), the receiver can compute per-packet arrival delay relative to the minimum seen (the "relative arrival delay" scheme NetEq's alternative mode uses). This lets it identify a post-gap backlog (a run of packets with falling relative delay) and drop all but the newest few at once, rather than playing them or skipping later when the queue is over target. Cost: no extra delay. This is a receiver-side policy that costs nothing in bits.
- A 200 ms sender backlog is itself a late-burst source: after an outage of 1 s the phone will send the 200 ms backlog in one burst, and the receiver sees 10 packets arriving at once. Reducing the backlog to 40-60 ms (2-3 packets) means a recovering link sends only fresh audio. QUIC's own congestion window and pacing on a lossy Wi-Fi link may hold datagrams back as well (inference: the datagram buffer is behind congestion control).
- Whether the late packets arrive from the sender's backlog or from the Wi-Fi retry queue was not determined from available logs.

### Gaps
- No evidence on what the phone's Wi-Fi driver queue holds during a stall (needs capture on the phone). No source on real-time systems' sender-side freshness beyond the above.

## Suggested starting parameters (all inferred from the above, not tested)

| Item | Start value | Delay cost | Basis |
|---|---|---|---|
| Target delay | 60 ms floor, grow within 1 packet to cover observed 95th percentile of relative arrival delay, cap 150 ms | the target itself | NetEq 0.95 quantile, 0.9993 forget; Roc asymmetry |
| Target decrease | only when current >= 1.7x estimate, then after a 5 s cooldown; after a skip or underrun hold the target for 5-15 s | none | Roc latency_config.h |
| Catch-up by interpolation | keep at or under 0.5% | none | Snapcast 0.05%, Roc 0.5% |
| Catch-up by WSOLA | optional later; 10-20 ms overlap, search window 2.5-20 ms, 1-3% speed; needs music testing | none, reduces lag at 1-3% per s | NetEq accelerate |
| Burst > target | PLC up to 20-40 ms, fade 10-20 ms, silence, then drop backlog older than target, fade in 10-20 ms | none (resets lag) | inference |
| Fade length | 10-20 ms, up from 5 ms | 0 | inference |
| Sender backlog | 40-60 ms of Opus (2-3 packets), down from about 200 ms | reduces bursts after recovery | inference |
| Opus | AUDIO app, 20 ms (or 10 ms with libopus), complexity 10, in-band FEC off, DTX off | 20 ms frame + 2.5 ms look-ahead + (SILK-less) | RFC 6716, AOSP |
| FEC/RED/NACK | not recommended for this pattern | +25-100% airtime | inference |
