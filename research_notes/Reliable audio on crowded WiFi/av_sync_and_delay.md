# A/V sync and audio delay for Relay (phone audio to PC speakers)

Method note: about 22 tool calls. Several developer.android.com and source.android.com pages returned 404, so some Android claims rest on AOSP mirrors or are unverified (flagged below). Search-tool summaries were used for ITU/ATSC/EBU numbers; the primary PDFs were listed but not read in full. Items marked "UNVERIFIED (background knowledge)" have no source from this session and must not be presented as cited facts.

## 1. Perceptual limits: when is audio lag noticed or unacceptable?

### Takeaway
Standards for audio that lags the picture: detectable around 45 to 125 ms depending on the standard, unacceptable around 60 to 185 ms. Broadcast engineering (ATSC, EBU) is much stricter than the ITU subjective-test limit. Games and music need far less than video: interactive tolerances start near 30 to 50 ms. A path of 200 to 280 ms is past every video-standard limit except ITU's "acceptable" edge (185 ms) and is well past any game or music-performance limit. Target below about 100 ms total for video and below about 50 ms for games; this is not reachable via a phone capture path, so see section 4.

### Cited Findings
Standards (sign convention: "lag" means sound arrives after the picture)
- ITU-R BT.1359-1 (1998): detectability threshold about +45 ms audio early and -125 ms audio late. Acceptability threshold about +90 ms audio early and -185 ms audio late. Averages from subjective tests. Viewers tolerate audio lag better than audio lead. — [ITU-R BT.1359-1 PDF](https://www.itu.int/dms_pubrec/itu-r/rec/bt/R-REC-BT.1359-1-199811-I!!PDF-E.pdf) (figures via search summary; also [Fora Soft explainer](https://www.forasoft.com/learn/audio-for-video/articles-audio/lip-sync-itu-r-bt-1359-tolerance-windows) as a secondary source)
- ATSC IS-191: sound should never lead the video by more than 15 ms and never lag by more than 45 ms, measured at DTV encoder inputs. Note the user's brief had the signs reversed. — [TV Technology, ATSC Recommends AV Sync Rules](https://tvtechnology.com/opinions/atsc-recommends-av-sync-rules)
- EBU R37 (2007): +40 ms (audio early) and -60 ms (audio late) at the output feeding broadcast transmitters. Again the brief had the order reversed. — [EBU R37 page](https://tech.ebu.ch/publications/r037); figures via search summary, not confirmed in the PDF
- Standards are broadcast-chain limits for a whole programme, not a user "feel" threshold. ATSC and EBU are deliberately tight so the chain's errors can add up. (Interpretation, no direct source.)

Research on audio-visual timing
- Speech: natural speech integration window about 40 ms audio lead to 200 ms audio lag. — [Frontiers/PMC4117430 "No, there is no 150 ms lead..."](https://www.ncbi.nlm.nih.gov/pmc/articles/PMC4117430/)
- Narrator video: window about 131 ms audio lead to 258 ms audio lag (Dixon and Spitz 1980, as cited in the same paper). — [PMC4117430](https://www.ncbi.nlm.nih.gov/pmc/articles/PMC4117430/)
- Plosive-vowel syllables: synchrony is precise, between 20 ms audio lead and 70 ms audio lag. — [PLoS Comput Biol 2014 via DOAJ](https://doaj.org/article/0b8509061aed474a8f1bd35e8142a614)
- Action-oriented content showed more consistent, sensitive timing than speech or music; thresholds vary by stimulus (111 ms audio-lead for a static disc and noise burst vs 79 ms for a moving disc and descending tone). — [Frontiers Psychol 2015;6:736](https://www.frontiersin.org/journals/psychology/articles/10.3389/fpsyg.2015.00736/full)

Games and interaction
- A study with 0, 40, 270 and 500 ms of added audio delay in a game found delay raised tension and lowered positive feelings, worst at 500 ms, with the biggest effect on highly skilled players. — [Regensburg, game auditory latency study](https://epub.uni-regensburg.de/59003/3/3678299.3678331.pdf) (via search summary)
- Latency JND for musicians (37 participants): mean about 49 ms at 0 ms base delay, 27 ms at 64 ms base, 77 ms at 512 ms base. Higher musical sophistication gave lower thresholds. — [JND Audio Latency, Regensburg](https://epub.uni-regensburg.de/59003/1/JND_Audio_Latency.pdf)
- AR/avatar study (ISMAR 2022): minimum noticeable audio-visual delay between 160 and 320 ms; avatar-sound delay less tolerable at 320 ms but never completely intolerable up to 1200 ms. — [ISMAR 2022](https://computer.org/csdl/proceedings-article/ismar/2022/532500a026/1JrRe0qKUZG)
- Active rhythmic performance widened the window of subjective synchrony by up to 59 ms (so interaction makes people more tolerant of offset, not less, in that VR study). — [Uni Weimar](https://www.uni-weimar.de/de/medien/professuren/medieninformatik/vr/research/hci/rhythmic-interaction-influences-synchrony-perception-in-vr/)
- Ensemble clapping: timing degraded from about 30 ms, sharply above 100 ms (Schuett, as summarised in the search result). — secondary, no primary link retrieved

Music alone
- No visual reference means no sync error; delay only matters for latency in playing along. For listening only, a constant delay is not perceptible, only the start gap and any interaction (pause, volume) lag. (Interpretation.) The musician JND above applies to playing along, not listening.

### Inferences
- Clash Royale-style tap-then-sound feedback is an action stimulus where the player's own tap is the visual reference. The relevant bar is closer to the game and interaction numbers (about 30 to 100 ms) than to the video limit, but tolerance grows for non-musical, non-rhythmic play.
- For video (YouTube), a realistic aim is to stay inside ITU's -125 ms detectability, i.e. under about 125 ms. At about 250 ms Relay is past ITU's -185 ms "unacceptable" mark.

### Gaps
- No peer-reviewed study of mobile rhythm games (e.g. Cytus, Arcaea) found; only a vendor claim that 10 to 20 ms is "very good" for rhythm games. Not quoted as fact.
- Primary ITU, ATSC, EBU documents were not read in full; numbers come from search summaries and one secondary article. The 2025/2026 in-force status of BT.1359 is unconfirmed.
- Younkin and Corriveau, Steinmetz 1996 and Blauert/Reeves and Voelker were searched but not found.

## 2. How apps stay in sync with Bluetooth (getTimestamp, getLatency, AVDTP delay reporting)

### Takeaway
Players treat audio playback position as the master clock and delay or drop video frames to match when audio is actually heard. The delay figure comes from AudioTrack's timestamp and a latency number that Android derives, for A2DP, from the Bluetooth stack (including AVDTP delay reports when enabled). This only works when the app is the one playing the audio, so it applies to media players and some games, not to a downstream sink like Relay.

### Cited Findings
- Amazon's Fire TV guide treats the audio playback position as the primary time reference; the video pipeline outputs frames matching the latest rendered audio frame. — [Amazon AV synchronization](https://developer.amazon.com/docs/fire-tv/audio-video-synchronization.html)
- `AudioTrack.getLatency()` is a hidden method, needing reflection. ExoPlayer's automatic audio delay handling (AudioTrackPositionTracker) is what makes Bluetooth sync work for ExoPlayer apps (such as YouTube, per a Stack Overflow thread). Treat the YouTube claim as secondary. — [Amazon AV sync](https://developer.amazon.com/docs/fire-tv/audio-video-synchronization.html); [VLC issue 1487](https://code.videolan.org/videolan/vlc-android/-/issues/1487)
- OS latency figures were accurate on a test device: under about 50 ms for internal speakers and 200 to 300 ms for Bluetooth headsets, and MediaPlayer and ExoPlayer apps sync well. VLC's `getTimestamp`-based compensation was gated behind a build flag and misbehaved over Bluetooth. — [VLC issue 1487](https://code.videolan.org/videolan/vlc-android/-/issues/1487)
- AOSP added AVDTP 1.3 delay reports. The stack passes the delay value and bytes sent to the audio HAL, the HAL reads a presentation position, and it falls back to the old estimate on failure. Delay reporting was gated by `persist.bluetooth.enabledelayreports` (default false in that commit); delay is stored per peer and the active device's value applied. — [AOSP commit 82fc8edc](https://android.googlesource.com/platform/packages/modules/Bluetooth/+/82fc8edc52%5E%21/); [e/OS commit 40d2ec05](https://gitlab.e.foundation/e/os/android_packages_modules_bluetooth/-/commit/40d2ec05ca775296f0b085f44eb8407dc16bd904); [commit 3e8c2b36](https://gitlab.e.foundation/e/os/android_packages_modules_bluetooth/-/commit/3e8c2b36db568219a5c678f0527d92725d0db865); [commit b8196c5f](https://gitlab.e.foundation/e/os/android_packages_modules_bluetooth/-/commit/b8196c5fcaa95828a380f8cf7b55520de132349e)
- Android CDD 5.6: recommended continuous input latency 30 ms or less for devices with a microphone; round-trip is input plus output plus one buffer period. — [CDD 5.6 (android-9.0.0_r30)](https://android.googlesource.com/platform/compatibility/cdd/+/refs/tags/android-9.0.0_r30/5_multimedia/5_6_audio-latency.md)
- UNVERIFIED (background knowledge): `AudioTrack.getTimestamp()` is public and returns a frame position and nanoTime pair at which the frame was presented; ExoPlayer combines it with `getLatency` to compute presentation latency; any delay up to a few hundred ms (Bluetooth) is hidden by delaying video by the same amount. Games that use a video-sync clock do not do this; most games play sound as soon as an event happens and cannot hide delay (their audio runs "now" and nothing can be delayed to match it).

### Inferences
- A player can hide an output delay of several hundred ms, but only because it controls the video. A game cannot hide delay in the same way, because its picture is real-time input-driven. Bluetooth "okay" sync with games on this phone is probably because games get the phone's output latency anyway; large A2DP delay still shows in games (UNVERIFIED).
- Relay appears to Android as no delay at all (phone silent, output route unchanged), so compensation does not occur by design.

### Gaps
- Primary ExoPlayer/AudioTrack source (`AudioTrackPositionTracker`, `DefaultAudioSink`) not retrieved. Android's mandated latency-reporting behaviour (public docs) not retrieved: developer.android.com av-sync URLs 404ed.
- Exact maximum delay ExoPlayer hides: not found.

## 3. Can an unprivileged Android app tell the system or other apps "this output has N ms of extra delay"?

### Takeaway
No reachable mechanism was found. Every route that would feed a latency number to other apps (virtual audio devices, audio routing, effects on session 0, A2DP sink/delay reporting) is either system-privileged or not an exposed API. Only MediaRouter2 remote playback hides delay, and only for the app that owns playback (not phone-wide capture).

### Cited Findings
- VirtualDeviceManager: `CREATE_VIRTUAL_DEVICE` is held only by system apps with specific roles; devices need a Companion Device Manager association; audio capture/injection needs `MODIFY_AUDIO_ROUTING`. Blocked for ordinary apps. — [AOSP VirtualDeviceManager.java (aicp mirror)](https://git.aicp-rom.com/frameworks_base.git/plain/core/java/android/companion/virtual/VirtualDeviceManager.java); [e/OS commit b86bdb49](https://gitlab.e.foundation/e/os/android_frameworks_base/-/commit/b86bdb49da72fc918c620115bf256f4104939ddc)
- `MediaRecorder.AudioSource.REMOTE_SUBMIX` needs `CAPTURE_AUDIO_OUTPUT`, system-only. Relay uses the unprivileged AudioPlaybackCapture instead. — [AudioSource docs](https://Developer.android.com/reference/kotlin/android/media/MediaRecorder.AudioSource)
- MediaRouter2: a "live audio route" is transparent to the app; a "remote playback route" operates independently of the local device and the app controls playback through the routing controller. No latency field is mentioned in the pages retrieved. — [MediaRoute2Info reference](https://developer.android.com/reference/android/media/MediaRoute2Info); [Media routing guide](https://developer.android.com/media/routing)
- AudioPlaybackCapture only captures USAGE_UNKNOWN, USAGE_GAME and USAGE_MEDIA, honours the stricter of app-level and player-level capture policy, and requires the manifest opt-in (`allowAudioPlaybackCapture`). — [AudioPlaybackCaptureConfiguration reference](https://developer.android.com/reference/kotlin/android/media/AudioPlaybackCaptureConfiguration)
- A forum post citing the Android docs says AudioPlaybackCapture does not affect the latency of the captured app (it is a tap, not a reroute). — [B4X thread](https://b4x.com/android/forum/threads/capture-the-output-audio-with-audiorecord.163075) (weak source)
- Relay's own README: the phone's volume bar sets the PC level, but a game on the low-latency output gets a cast-style bar because raising real volume would unmute the phone; "Audio is not instant... a game's sounds can feel a little behind". — `D:\Desktop\Project\phone bridge audio\README.md` lines 117 and 121
- UNVERIFIED (background knowledge): audio effects on session 0 (global mix) need `MODIFY_AUDIO_SETTINGS` and were deprecated, and effects do not carry a latency value to other apps; A2DP sink role and USB audio accessory role are system or accessory-mode only and also do not report delay to apps; AudioManager has no public "set output latency" call; `getOutputLatency` was never a public API.
- For comparison only (Bluetooth, rejected for Relay): A2DP gives automatic compensation via AVDTP delay reports and presentation position, at 200 to 300 ms of real delay hidden in players (VLC issue above).

### Inferences
- Because the sink is not an Android audio device, there is nowhere to attach a delay. The only compensation Relay can do itself is on the PC side: it cannot delay the picture on the phone.
- A Cast/Miracast-style receiver role on the phone would be the wrong direction (phone is the source).

### Gaps
- No primary source on session-0 effects, `AudioManager` latency APIs, A2DP sink role or USB accessory role; stated only as unverified background. MediaRouter2 pages were skimmed via summary only.

## 4. How Miracast, Chromecast, AirPlay and Steam Link keep sound and picture together; could Relay move the picture too?

### Takeaway
All of them ship picture and sound together and sync them at the receiver with timestamps, accepting end-to-end delays of 150 to 400 ms for video watching. That is the design that makes delay not matter, and it only works when both streams are on the receiving device. Relay's screen mirror gives that option in principle: play the picture on the PC and delay the picture or sound there until they line up.

### Cited Findings
- Typical latency in good conditions from a vendor comparison: AirPlay about 150 ms, Miracast about 180 ms, Google Cast about 400 ms (AirServer-related, not independent). — [KTC Play comparison](https://us.ktcplay.com/blogs/product-comparisons/miracast-vs-airplay-vs-chromecast-latency)
- Miracast certification mandates no maximum latency; it can drift out of sync. Chromecast app casting avoids mirroring lag by handing playback to the receiver. — [AirServer support](https://support.airserver.com/support/solutions/articles/43000789665-airplay-vs-miracast-vs-google-cast-what-s-the-difference-); [Wikipedia Miracast](https://ipfs.ssi.eecc.de/ipns/en.wikipedia-on-ipfs.org/wiki/Miracast)
- General guidance: under 30 ms excellent, over 100 ms sluggish (vendor blog). — [VCOM](https://www.vcom.hk/ru/blogs/blog/why-does-screen-casting-lag-causes-fixes-amp-best-wireless-hdmi-solutions)

### Inferences
- If Relay's screen mirror video and audio were stamped with the same capture clock and presented together on the PC, the video could be held back by the audio's extra delay (as players do for Bluetooth), giving lip-sync even at 250 ms. The cost is that touch input from the PC would feel that much slower, and the phone's own screen still shows the real picture. So this helps watching on the PC but not playing on the phone.
- Whether Relay's mirror stream timestamps video and audio against a shared clock is not checked here.

### Gaps
- Steam Link audio/video sync behaviour and Relay's own screen-mirroring sync (code, docs): not researched (README grep covered audio notes only).
- No independent measurements for Miracast/AirPlay/Cast delays; the figures are vendor or blog numbers.

## 5. Delay Android adds before Relay sees the sound (capture path)

### Takeaway
No measured number for AudioPlaybackCapture exists in sources I could reach. Code analysis suggests the capture is bounded by AudioFlinger mixing plus the capture record buffer; Relay's current capture settings use the plain minimum-size AudioRecord with no low-latency flag. Unprivileged tuning is limited to AudioRecord buffer size and read chunk size.

### Cited Findings
- The remote submix HAL's output latency getter is computed as pipe depth in frames times 1000 divided by the sample rate. The latest source defines the pipe depth as 1024*8 frames (about 170 ms at 48 kHz, my arithmetic); Android 8.0 used 1024*4 (about 85 ms). That is a reported latency from a fixed buffer, not a measurement, and AudioPlaybackCapture may not use this HAL path the same way. — [audio_remote_submix audio_hw.cpp (main)](https://android.googlesource.com/platform/hardware/libhardware/+/main/modules/audio_remote_submix/audio_hw.cpp); [android-8.0.0_r15](https://android.googlesource.com/platform/hardware/libhardware.git/+/android-8.0.0_r15/modules/audio_remote_submix/audio_hw.cpp)
- A comment in that file notes "3 * 5ms = 15ms < 1024 frames * 1000 / 48000 = 21.333ms" for read timing. A later AOSP change aligns time in the transfer with the duration of audio in the submix I/O. — same [audio_hw.cpp](https://android.googlesource.com/platform/hardware/libhardware/+/main/modules/audio_remote_submix/audio_hw.cpp); [OmniROM change](https://gerrit.omnirom.org/plugins/gitiles/android_hardware_interfaces/+/06085453774ad21db13f27de50140c3cac56c92c)
- AOSP's latency contributors: application processing, number of buffers, frames per buffer, extra processing such as a DSP. Buffers are usually an effect of avoiding underruns, not a cause. Scheduling latency over 2 ms causes audio problems; audio threads use SCHED_FIFO priority 2 or 3. — [AOSP audio latency contributors](https://source.android.com/docs/core/audio/latency/contrib)
- AAudio exclusive MMAP mode bypasses the mixer and has much lower latency than shared mode; the legacy AudioFlinger path is used if MMAP is unavailable. That is for apps' own streams, not playback capture. — [AAudio and MMAP](https://source.android.com/docs/core/audio/aaudio)
- Relay's current capture: `AudioRecord.getMinBufferSize` doubled, at least 4096 bytes; reads in 20 ms chunks; no PERFORMANCE_MODE flag. — `D:\Desktop\Project\phone bridge\android\app\src\main\java\dev\phonebridge\app\AudioStreamer.kt` lines 49 to 110
- Latency measurement tool support: OboeTester round-trip latency and CTS Verifier use a loopback and correlation. — [AOSP latency measure](https://source.android.com/docs/core/audio/latency/measure)
- UNVERIFIED (background knowledge): output streams for most media apps use a deep-buffer output with multi-ten-ms periods (commonly about 20 ms periods, 4 to 8 periods) while games usually use the fast/low-latency track (about 5 to 10 ms), which explains why game audio feels different when captured; `AudioRecord.setPreferredDevice` and `PERFORMANCE_MODE_LOW_LATENCY` for record apply to mic input, and their effect on playback-capture records is not confirmed; MediaTek-specific paths not found.

### Inferences
- Using the minimum AudioRecord buffer and short reads is already close to what an app can do. Gains would come from the app side of Relay (Opus frame size 20 ms to 10 ms or 5 ms, jitter queue 110 ms to lower), not from Android.

### Gaps
- No measured AudioPlaybackCapture-only delay, no MediaTek data, no Android 13 specific behaviour, no Superpowered figures (search returned none).

## 6. WASAPI shared vs exclusive, event mode, IAudioClient3, cpal behaviour

### Takeaway
cpal 0.15.3 opens WASAPI in shared, event-driven mode with the plain IAudioClient and, for BufferSize::Default, passes duration 0, so the engine's default period (typically 10 ms) applies. IAudioClient3 can go lower (about 2.67 ms at 48 kHz on good drivers) but cpal does not use it. On the PC side this is a small cost (about 10 to 20 ms) next to the 110 ms queue.

### Cited Findings
- Shared mode through original IAudioClient is stuck near 10 ms; exclusive mode allows lower; IAudioClient3 (Windows 10+) offers `InitializeSharedAudioStream` with a chosen periodicity, as low as 2.67 ms (128 frames at 48 kHz) on a fixed driver; minimum is driver-dependent and may report 10 ms. — [Microsoft IAudioClient3](https://learn.microsoft.com/windows/win32/api/audioclient/nn-audioclient-iaudioclient3); [JUCE issue 560](https://github.com/juce-framework/JUCE/issues/560); [Stack Overflow via kiwix](https://kiwix.lema.org/stackoverflow.com_en_all_2023-11/questions/37468283/how-to-get-below-10ms-latency-using-wasapi-shared-mode)
- cubeb notes the IAudioClient3 minimum can exceed IAudioClient::GetDevicePeriod, and whether a low-latency session forces other sessions to the same latency is unproven. — [cubeb_wasapi.cpp](https://repo.retrodeck.net/Xargon/Duckstation/raw/commit/9f5cddc94f2ff8d631099ae81758d8b5b23177d4/dep/cubeb/src/cubeb_wasapi.cpp)
- cpal 0.15.3 WASAPI output: `AUDCLNT_SHAREMODE_SHARED` with `AUDCLNT_STREAMFLAGS_EVENTCALLBACK`; `BufferSize::Default` converts to duration 0 and `Fixed(frames)` converts to frames times 10,000,000 divided by sample rate (100 ns units); no IAudioClient3 call found in the wasapi module. — `D:\Rust\cargo\registry\src\index.crates.io-1949cf8c6b5b557f\cpal-0.15.3\src\host\wasapi\device.rs` lines 566 to 706 and 1012 to 1015
- Relay's PC output sits in `crates\pb-platform-win\src\audio_out.rs` using cpal with a shared ring buffer. A 96 kHz device with Relay's 48 kHz stream: cpal here passes the format and checks `is_format_supported` without `AUTOCONVERTPCM`, so a mismatch fails or needs Relay's own conversion; the recent commit "adapt AudioOut to device native sample format" covers format, not rate. — device.rs lines 684 to 706 above; git log in the task context. (Whether Relay resamples was not checked.)

### Inferences
- A fixed small buffer through cpal (`BufferSize::Fixed(480)` or so) could cut the engine period to the driver's minimum if the driver honours it in shared mode; unverified on this PC. Swapping to IAudioClient3 would save at most about 7 ms. Not a priority.

### Gaps
- Microsoft's pages on exclusive mode period limits were not fetched; no measured shared-mode latency on the user's PC; the 96 kHz behaviour is a code-reading inference.

## 7. Best way to measure true end-to-end delay without special hardware

### Takeaway
Play a known click or chirp from a phone app and record phone speaker and PC speaker in one stereo recording with a single clock: the time difference between the two arrivals is Relay's added delay. No source describes this exact setup; the design below is an inference from AOSP's correlation-based loopback method.

### Cited Findings
- AOSP's own method: play a known test signal, record it, and find the time offset by normalised correlation. — [AOSP latency measure](https://source.android.com/docs/core/audio/latency/measure)

### Inferences
- Setup A (best, needs no PC mic consent beyond a test): phone plays a click track (several clicks 1 s apart, a short chirp is easier to correlate). Place a third device (a second phone or a laptop recorder) so it hears both the phone speaker and the PC speakers. Use an audio editor or a small script to cross-correlate the two arrivals. The delay is the offset. Place both sources equally far away (1 m is about 3 ms).
- Setup B (one device only): use the PC's microphone to record both. This needs the phone speaker audible, which conflicts with Relay muting the phone during a share. Run the test with the share's mute option off, or use a fixed acoustic click such as a hand clap and a PC-side timestamp.
- Setup C (software only, no acoustic path): put a unique click in the test stream, timestamp it on the phone at AudioTrack write plus `getTimestamp`, and timestamp it on the PC at playback, with the clocks aligned by Relay's existing connection round-trip. Measures path delay but not the app's output buffer and the PC speaker path; phone-to-PC clock offset must be known.
- Report minimum and median over at least 20 clicks, in both good and 2.4 GHz Wi-Fi conditions; the 110 ms queue and any drift show up as a trend.
- The user must explicitly approve playing sound on the PC, per the brief; the test fits inside that approval as long as only the test tone plays.

### Gaps
- No existing Relay tool or measurement was found; no source for a clap-test accuracy figure (typically phone-to-phone tests resolve to a few ms, UNVERIFIED).

## Delay budget sketch (arithmetic from cited and code numbers; not measured)
- App output (deep buffer, typical 20 to 40 ms, UNVERIFIED) + capture/mixer/submix (unknown, up to tens of ms) + Opus 20 ms frame + 20 ms read chunk + network 5 to 100 ms (Wi-Fi dependent) + queue about 110 ms + WASAPI shared about 10 to 20 ms = about 200 to 300 ms, consistent with the earlier 200 to 280 ms estimate. The queue is the largest single term and the only one Relay fully controls.
