# Existing wireless audio and game-streaming systems on crowded 2.4 GHz Wi-Fi

Research date 2026-11-05. About 20 tool calls (search plus fetch). Coverage is uneven: several systems (Sonos internals, Chromecast Audio, WO Mic, Parsec, Miracast measurements, SoundWire and AudioRelay independent tests, Moonlight packet timing) returned no primary source. Those are listed under Gaps rather than filled from memory. Search snippets were summarised by a small model, so figures are as reported by that tool; spot-check the linked pages before quoting them.

## 1. Per-system: transport, buffer, real delay, loss recovery, 2.4 GHz reports

### Takeaway
The only hard, sourced numbers I found: AirPlay 1 sources set about 2.0-2.25 s of latency and AirPlay 2 about 0.5 s; Snapcast defaults to TCP with sub-0.2 ms inter-client sync (buffer is large by design); Roc Toolkit is the one open system that exposes low target latency (default 200 ms, 25 ms reported on stable Ethernet) with Reed-Solomon/LDPC FEC; Sunshine/Wolf audio uses Opus over RTP with Reed-Solomon FEC of 2 parity packets per 4 data packets; AudioRelay uses UDP with a user-set maximum buffer, with 30-50 ms advised and about 100-150 ms seen on unstable Wi-Fi. Valve officially advises 5 GHz over congested 2.4 GHz.

### Cited Findings
**AirPlay 1 / AirPlay 2 (and shairport-sync as receiver)**
- Classic AirPlay sources set a latency of about 2.0 to 2.25 s; AirPlay 2 can use shorter latencies, about 0.5 s. The latency is the delay between a sample's timestamp and when the output plays it, and the source sets it during negotiation. (vendor/maintainer documentation, shairport-sync README, undated, current) — [shairport-sync](https://github.com/mikebrady/shairport-sync)
- Sync is done with timing feedback and resampling ("interpolation": adding/removing frames); AirPlay 2 timing uses PTP via NQPTP, classic AirPlay uses an NTP variant. — [shairport-sync](https://github.com/mikebrady/shairport-sync)
- RAOP is UDP with sequence numbers; receivers detect gaps and ask for resends. Resend request uses payload type 0x56 on the control port, not the stream port. The sender keeps a backlog of recently sent packets (reverse-engineered notes; no duration given; one PulseAudio patch used a 1000-packet circular buffer). The receive buffer can wait for a late resend. A mode that skips resends exists. (reverse-engineered, not Apple documentation) — [Airtunes2 notes](https://git.zx2c4.com/Airtunes2/tree/airtunes2.rst); [PulseAudio bug 42804](https://lists.freedesktop.org/archives/pulseaudio-bugs/2013-September/006803.html); [shairplay raop_buffer.c](https://grok.dragonflybsd.org/xref/dports/audio/shairplay/shairplay-498bc5b/src/lib/raop_buffer.c)
- Wi-Fi symptoms on the receiver side: audio "may seem to pause or drop for several seconds"; iOS may disconnect. Causes listed: slow Wi-Fi, a Wi-Fi adapter in power-save mode, limited host resources. Fixes: disable Wi-Fi power save; raise `audio_backend_buffer_desired_length` (example 19845 frames, triple the ALSA default, fixed a Pi Zero on a busy network). (maintainer troubleshooting guide) — [shairport-sync TROUBLESHOOTING](https://github.com/mikebrady/shairport-sync/blob/master/TROUBLESHOOTING.md)

**Sonos / SonosNet**
- Official: SonosNet channel can be set to 1, 6 or 11 (non-overlapping 2.4 GHz channels). — [Sonos support](https://support.sonos.com/en/article/change-your-sonos-system-s-wireless-channel)
- Anecdote (forum): users recommend keeping SonosNet off the router's channel, mention Bluetooth/Zigbee/cameras as interferers, and report the error "network connection speed insufficient to maintain playback buffer" even after changing channels. Shows Sonos's reliability model is a large playback buffer that runs dry. — [Sonos community](https://en.community.sonos.com/controllers-and-music-services-229131/error-network-connection-speed-insufficient-to-maintain-playback-buffer-6925685)
- No Sonos buffer size or retry detail found.

**Snapcast**
- Transport TCP; encoded chunks sent over TCP to clients. Codecs PCM, FLAC (default), Vorbis, Opus ("lossy low-latency"). Typical inter-client deviation below 0.2 ms, corrected by dropping or duplicating single samples. Client ALSA buffer_time default 80 ms, PulseAudio 100 ms (that is the local device buffer, not the network buffer). (project README) — [Snapcast](https://github.com/badaix/snapcast)
- Snapcast optimises synchronisation across rooms, not delay: it needs a deliberately large server-side buffer. The README gives no Wi-Fi figures. The 1000 ms default buffer is not on the README page (I could not confirm it here).

**Roc Toolkit**
- Tunables in the PulseAudio module docs: target latency default 200 ms, packet length 5 ms, FEC encoding RS8M or LDPC with source/repair packets per block defaults 18 and 10; latency profiles default/intact/responsive/gradual. Reed-Solomon and LDPC-Staircase are implemented; dynamic FEC block-size and code-rate adjustment is on the roadmap but unfinished (needs RTCP). (project docs and roadmap via search result) — [Roc roadmap](https://sources.debian.org/src/roc-toolkit/0.4.0%2Bdfsg-6/docs/sphinx/development/roadmap.rst); [Roc 0.3 article](http://gavv.net/articles/roc-0.3/)
- Measured by the maintainer: streaming latency on Ethernet 7 ms, plus 12 ms for two USB sound cards (6 ms in, 6 ms out). (author, Roc 0.3 article) — [Roc 0.3](http://gavv.net/articles/roc-0.3/)
- "gradual" profile tolerates high jitter but can oscillate in latency and limits minimum latency; "responsive" is for low-jitter links; the profile is chosen from the requested latency; adaptive jitter buffer size is in progress, not released. Resampling-based clock sync adjusts only about 5-20 samples per second. — [Roc 0.3](http://gavv.net/articles/roc-0.3/)
- Anecdote (GitHub issue): on a stable Ethernet link about 25 ms target latency suits; jitter spikes triggered latency adjustments. — [roc-toolkit issue 688](https://github.com/roc-streaming/roc-toolkit/issues/688)
- No Wi-Fi measurements found.

**SoundWire (Georgie Labs)**
- Vendor claim: "low-latency live sound over WiFi"; also works over cellular/WAN. No latency figure, no transport statement, no buffer figure on the page. — [georgielabs.net](https://georgielabs.net/)
- The changelog mentions a "disable fast Android audio path" setting (low-latency path can disable the equalizer). — [APKMirror changelog](https://www.apkmirror.com/?p=4538480)
- No independent measurement found.

**AudioRelay**
- Developer confirmed UDP transport; out-of-order packets were being dropped; one user's Wi-Fi test swung between 0 and 150 ms. (developer reply, forum, anecdotal) — [AudioRelay community](https://community.audiorelay.net/t/constant-hiccups-and-glitches/101)
- FAQ: if buffer is near or above 100 ms, lower "Maximum buffer" to 30-50 ms; 10 ms probably too low to be stable; on Android pick OpenSL ES and the Low buffer setting; the Android AudioTrack's internal buffer sets a minimum latency and a smaller custom value is raised. Uses the device "fast path"; on some phones (Xiaomi) that loses quality. (vendor FAQ) — [AudioRelay FAQ](https://docs.audiorelay.net/faq)
- Forum reports range about 4 ms average in good conditions to roughly 100-150 ms when unstable (anecdotal, summarised by the search tool, not individually verified). — [AudioRelay community: max buffer](https://community.audiorelay.net/t/max-buffer-wont-go-lower-than-120ms/359)

**Moonlight / Sunshine (and Wolf, a compatible host)**
- Wolf's documentation of the Moonlight audio protocol: Opus over RTP, AES-128-CBC encrypted, every 4 audio RTP packets followed by 2 Reed-Solomon FEC parity packets (about 33 percent overhead on top of the audio), frame size negotiated over RTSP. — [Wolf RTP/Opus](https://games-on-whales.github.io/wolf/stable/protocols/rtp-opus.html)
- Sunshine API docs show an `audio_fec_packet_t` and an Opus stream config (bitrate, channels, rate) but no FEC ratio or timing. — [Sunshine docs](https://docs.lizardbyte.dev/projects/sunshine/latest/structstream_1_1audio__fec__packet__t.html)
- moonlight-python (built on moonlight-common-c) positions audio by frame index so lost packets do not shift the timeline. — [moonlight-python releases](https://github.com/StanfordFSL/moonlight-python/releases)
- Not confirmed: 5 ms frame size, whether Sunshine's FEC matches Wolf's.

**Steam Link / Remote Play**
- Valve: choose 5 GHz over "highly congested" 2.4 GHz; disable AP/client isolation. 2021 build notes: "greatly improved audio latency" (not tied to band). — [Steam Support](https://help.steampowered.com/en/faqs/view/3E3D-BE6B-787D-A5D2)
- Anecdote: opposing user reports (5 GHz "crazy smooth" vs 2.4 GHz; another user fine on 2.4 GHz). — [Steam forum](https://steamcommunity.com/app/353380/discussions/0/4328520278444297841); [Slashdot 2018](https://games.slashdot.org/story/18/05/17/2232232/with-steam-link-app-your-smartphone-can-be-an-imperfect-gaming-monitor)

**Miracast / Wi-Fi Display**
- Industry article: typically tens of ms to 100 ms or more at best; specification caps latency at 250 ms at highest resolution and frame rate. AirServer (receiver vendor) says about 180 ms in good conditions. A sink patent reports input lag up to 68 ms on some sinks and that x264 frame buffering tunes encoder latency. The PCR is sent about every 100 ms in the transport stream. — [DisplayDaily](https://displaydaily.com/miracast-plans-to-build-on-wide-acceptance/); [AirServer](https://support.airserver.com/support/solutions/articles/43000531772-what-is-the-miracast-screen-mirroring-protocol-); [US9497506B2](https://patents.google.com/patent/US9497506)
- No 2.4 GHz measurement found.

### Inferences
- The source-sets-latency design in AirPlay (about 2 s classic, about 0.5 s AirPlay 2) means retransmit and jitter absorption are bought with delay. Reported figures match a "buffer deeply" design, not a low-delay one.
- AudioRelay is the closest peer to Relay (UDP, Android, ordinary APIs). Its own docs advise 30-50 ms but forum reports of 100-150 ms spikes on unstable Wi-Fi are consistent with what Relay measured on 2.4 GHz; I found no evidence that it beats Relay's gap behaviour.
- Roc and Sunshine/Wolf are the two systems with documented FEC; Roc's default (18 source + 10 repair per 5 ms packet) implies a block of 28 packets spanning about 140 ms, so FEC itself needs a delay of that order (arithmetic from the documented defaults, not a documented claim).

### Gaps
- Sonos buffer length and SonosNet protocol detail; Chromecast Audio (Cast) buffer and transport; WO Mic; Parsec audio pipeline and 2.4 GHz guidance; AirPlay 2 transport/buffer from a primary source; Miracast 2.4 GHz measurement; independent latency tests of SoundWire and AudioRelay; Moonlight/Sunshine audio frame size. No source found for any of these in this pass.

## 2. Bluetooth (A2DP SBC / aptX LL / LDAC, LE Audio) on the same band

### Takeaway
Bluetooth accepts either high delay (SBC about 170-250 ms) or a specialised low-delay codec (aptX LL about 31-40 ms). I found no sourced material on how it stays reliable on 2.4 GHz (hopping, retransmit); that is a gap.

### Cited Findings
- SBC measured about 173 ms and 248 ms on the same headphones in two tests; another test 181 ms; a 2015 Japanese technical paper attributes about 50 ms of the delay to codec frame-based encoding. Standard aptX measured 112-166 ms. aptX Low Latency measured about 31-34 ms; vendor figure about 40 ms with both ends supporting it; codec-only delay 1.9 ms in the paper. (RTINGS tests, lab-style but consumer reviewer; JAS paper) — [RTINGS](https://www.rtings.com/headphones/learn/latency); [JAS 2015 paper](https://www.jas-audio.or.jp/journal-pdf/2015/01/201501_020-026.pdf)
- LDAC: no reliable wireless latency measurement found; RTINGS reports it has no reliable way to measure it; LDAC falls back to lower bitrates when signal drops (secondary source; one forum post claims about 900 ms, unverified). — [RTINGS](https://www.rtings.com/headphones/learn/latency)

### Inferences
- aptX LL shows that about 30-40 ms over 2.4 GHz is achievable on a dedicated, scheduled link, but at the cost of small frames and a purpose-built radio link, not shared access-point Wi-Fi.

### Gaps
- AFH, retransmission, LE Audio (isochronous channels, presentation delay) not covered; would need Bluetooth Core spec or Bluetooth SIG sources.

## 3. Was the old low-delay claim true? Which systems buffer for seconds?

### Takeaway
Sourced evidence supports the honest version: classic AirPlay and similar audio-only systems were reliable on 2.4 GHz because they buffered about 2 s and the sender set that delay; the picture was never in sync with them. True low-delay (tens of ms) examples I found are wired or stable links (Roc 7 ms on Ethernet) or specialised radios (aptX LL).

### Cited Findings
- AirPlay classic about 2.0-2.25 s, AirPlay 2 about 0.5 s — [shairport-sync](https://github.com/mikebrady/shairport-sync)
- Sonos errors reference running out of playback buffer on congested 2.4 GHz (anecdote) — [Sonos community](https://en.community.sonos.com/controllers-and-music-services-229131/error-network-connection-speed-insufficient-to-maintain-playback-buffer-6925685)
- Miracast spec cap 250 ms; typical 100 ms or more at best — [DisplayDaily](https://displaydaily.com/miracast-plans-to-build-on-wide-acceptance/)
- Valve advises 5 GHz over congested 2.4 GHz for streaming — [Steam Support](https://help.steampowered.com/en/faqs/view/3E3D-BE6B-787D-A5D2)

### Inferences
- The user's belief is accurate for "listened to wireless audio without dropouts on 2.4 GHz" but not for "without delay": the delay was hidden by buffering (audio-only, video not synced, or a player that delays the video to match). Relay cannot do this because the video plays on the phone.
- Relay's 480-1995 ms gaps are comparable to or larger than the whole AirPlay 2 buffer (about 500 ms), so a buffer within Relay's 40-160 ms target cannot absorb them. Reaching 2.4 GHz reliability at low delay therefore needs reduction of the gaps (air-time use, retries, power-save), not only a bigger queue.

### Gaps
- No independent measurements comparing old systems on 2.4 GHz specifically; the claim about Chromecast Audio and Sonos delays was not found from primary sources.

## 4. Closest match to Relay; SoundWire/AudioRelay claims vs measured

### Takeaway
AudioRelay and SoundWire (Android, no root, ordinary Wi-Fi) are the closest. SoundWire gives no figure; AudioRelay advises 30-50 ms buffers; independent measurements were not found.

### Cited Findings
- AudioRelay: UDP; AudioTrack internal buffer sets a floor on latency; "fast path" use; 30-50 ms advice — [AudioRelay FAQ](https://docs.audiorelay.net/faq); [AudioRelay community](https://community.audiorelay.net/t/constant-hiccups-and-glitches/101)
- SoundWire: no figure — [georgielabs.net](https://georgielabs.net/)
- Android-level audio path latency has been measured academically (Zaporowski, Blaszke, Weber, "Measurement of Latency in the Android Audio Path", AES 2018, paper 413); not Wi-Fi streaming. — [AES e-library](https://aes2.org/publications/elibrary-page/?id=19526)

### Inferences
- Relay's design (UDP-like datagrams, Opus 20 ms) is the same class as AudioRelay; the differentiator would be loss handling, which the sourced docs for AudioRelay do not show (it discards out-of-order packets).

### Gaps
- Independent latency tests of SoundWire/AudioRelay; whether either captures only media sound; their actual 2.4 GHz behaviour.

## 5. Recurring techniques on bad Wi-Fi

### Takeaway
Sourced recurring tricks: FEC (Reed-Solomon, 4+2 in Moonlight/Wolf; 18+10 in Roc), small packets (Roc 5 ms), sender-set or user-set jitter buffer, resend requests (RAOP), resampling clock drift correction instead of dropping, disabling Wi-Fi power-save, 5 GHz or wired where possible, non-overlapping channels.

### Cited Findings
- Moonlight/Wolf: 4 data + 2 parity Reed-Solomon packets — [Wolf](https://games-on-whales.github.io/wolf/stable/protocols/rtp-opus.html)
- Roc: packet length 5 ms, RS8M/LDPC with 18 source + 10 repair; adaptive FEC and adaptive jitter buffer still unfinished — [Roc docs/roadmap](https://sources.debian.org/src/roc-toolkit/0.4.0%2Bdfsg-6/docs/sphinx/development/roadmap.rst)
- Roc and Snapcast fix drift by tiny resampling or single-sample add/drop (5-20 samples per second in Roc), not by skipping chunks — [Roc 0.3](http://gavv.net/articles/roc-0.3/); [Snapcast](https://github.com/badaix/snapcast)
- Moonlight client positions audio by frame index so a lost packet leaves a gap without shifting the timeline — [moonlight-python](https://github.com/StanfordFSL/moonlight-python/releases)
- Opus in-band FEC carries a lower-bitrate copy of the previous frame; helps only if the next packet arrives, so bursts defeat it; secondary source (blog, unverified quality claim "degrades above about 5% loss"). Mozilla audio-FEC experiments (blog): FEC made little difference at low loss, more at high loss. — [omr.it.com](https://omr.it.com/blog/packet-loss-concealment-webrtc-audio-8-percent-drop/); [Mozilla WebRTC blog](https://blog.mozilla.org/webrtc/audio-fec-experiments/)
- Receiver Wi-Fi power-save off — [shairport-sync TROUBLESHOOTING](https://github.com/mikebrady/shairport-sync/blob/master/TROUBLESHOOTING.md)
- Opus RFC 6716 covers loss resilience and concealment — [RFC 6716](https://rfc-editor.org/rfc/rfc6716)

### Inferences
- Burst loss is the key. Relay's observed gaps of 0.5-2 s are bursts of the "stall" type (consistent with retries, power save or airtime contention), which FEC at 33-100 percent overhead cannot cover unless the interleave spans the stall, which needs a buffer of the same length. FEC also adds airtime, and Relay's phone is not bandwidth-limited (0.2-3.8 Mbit/s total), so the problem is access, not throughput.
- Candidate copies, in order of evidence strength: (a) small parity group such as 4+2 (Moonlight) for random loss; (b) Opus in-band FEC/PLC for single-frame loss; (c) sender or receiver resampling to hold queue depth instead of skipping audio (Roc, shairport-sync, Snapcast); (d) Wi-Fi power-save and QoS (WMM) settings on the phone socket (not sourced in this pass); (e) a user-visible latency setting (AudioRelay's Maximum buffer).

### Gaps
- Pacing, dedicated audio priority/DSCP, and dynamic bitrate: no sourced systems found in this pass. Sunshine FEC percentage for video, and Parsec details, not found.
