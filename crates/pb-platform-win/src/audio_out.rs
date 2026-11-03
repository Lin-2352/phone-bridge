//! WASAPI audio output via `cpal`: plays raw i16-LE PCM frames streamed from
//! the phone over the Phone Bridge transport.
//!
//! Data path: a tokio task converts each chunk to the device's channel count
//! and sample rate and pushes it into a single-producer/single-consumer ring of
//! atomics. The cpal callback only reads the ring and converts samples: no
//! locks, no allocation.
//!
//! The queue follows the network. It starts at [`START_TARGET_MS`]; running dry
//! raises the target (up to [`MAX_TARGET_MS`]) and a clean minute lowers it again.
//! Only when the queue passes target + [`SLACK_MS`] is the oldest audio skipped,
//! so the burst that follows a Wi-Fi gap refills the queue instead of causing a
//! jump. Every break in the sound (running dry, restarting, skipping) is faded
//! over [`FADE_MS`], so it is a soft gap and never a click or buzz.

use std::sync::atomic::{AtomicI16, AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;

use cpal::{
    traits::{DeviceTrait, HostTrait, StreamTrait},
    SampleFormat, Stream, StreamConfig,
};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::audio_conv::{i16_to_f32, remix, Resampler};

/// Audio buffered before playback starts, and the lowest the target goes, in ms.
pub const START_TARGET_MS: usize = 160;
/// The most delay the target may add when the network is at its worst, in ms.
pub const MAX_TARGET_MS: usize = 260;
/// How far past the target the queue may go before old audio is skipped, in ms.
pub const SLACK_MS: usize = 80;
/// Added to the target each time the queue runs dry, in ms.
pub const GROW_MS: usize = 40;
/// Taken off the target after each clean [`SHRINK_AFTER_S`], in ms.
pub const SHRINK_MS: usize = 20;
pub const SHRINK_AFTER_S: usize = 60;
/// Length of the fade over a break in the sound, in ms.
pub const FADE_MS: usize = 5;
const RING_MS: usize = 500;

/// Typed errors for audio output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AudioOutError {
    /// There is no default output device.
    NoDevice,
    /// The device's native sample format is neither F32 nor I16.
    UnsupportedFormat(String),
    /// Querying the device configuration failed.
    Config(String),
    /// Building or starting the stream failed.
    Stream(String),
    /// The requested source format is not mono/stereo or has a zero rate.
    BadSource,
}

impl std::fmt::Display for AudioOutError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoDevice => write!(f, "no default audio output device"),
            Self::UnsupportedFormat(s) => {
                write!(
                    f,
                    "the output device uses an unsupported sample format ({s})"
                )
            }
            Self::Config(s) => write!(f, "audio output configuration error: {s}"),
            Self::Stream(s) => write!(f, "audio output stream error: {s}"),
            Self::BadSource => write!(f, "unsupported source audio format"),
        }
    }
}

impl std::error::Error for AudioOutError {}

/// Milliseconds of sound in `frames` sample frames at `rate` Hz.
pub fn frames_to_ms(frames: u64, rate: u32) -> u64 {
    if rate == 0 {
        0
    } else {
        frames * 1000 / u64::from(rate)
    }
}

/// Counters shared between the callback, the producer and the owner.
#[derive(Default)]
pub struct Stats {
    dropped_frames: AtomicU64,
    underruns: AtomicU64,
    /// The consumer's current target, in ms (for the log).
    target_ms: AtomicUsize,
}

struct Shared {
    buf: Box<[AtomicI16]>,
    /// Total samples ever written / read (monotonic).
    head: AtomicUsize,
    tail: AtomicUsize,
}

/// Queue limits in interleaved samples.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    pub start_target: usize,
    pub max_target: usize,
    pub slack: usize,
    pub grow: usize,
    pub shrink: usize,
    pub shrink_after: usize,
    pub capacity: usize,
    /// Sample frames, not samples.
    pub fade_frames: usize,
}

impl Limits {
    pub fn new(rate: u32, channels: usize) -> Self {
        let per_ms = |ms: usize| (rate as usize * ms / 1000) * channels;
        Self {
            start_target: per_ms(START_TARGET_MS),
            max_target: per_ms(MAX_TARGET_MS),
            slack: per_ms(SLACK_MS),
            grow: per_ms(GROW_MS),
            shrink: per_ms(SHRINK_MS),
            shrink_after: rate as usize * channels * SHRINK_AFTER_S,
            capacity: per_ms(RING_MS),
            fade_frames: (rate as usize * FADE_MS / 1000).max(1),
        }
    }
}

/// Write side of the ring (not used from the cpal callback).
pub struct RingProducer {
    shared: Arc<Shared>,
    channels: usize,
    stats: Arc<Stats>,
}

/// Read side of the ring; lives inside the cpal callback. Only it moves `tail`.
pub struct RingConsumer {
    shared: Arc<Shared>,
    limits: Limits,
    channels: usize,
    playing: bool,
    stats: Arc<Stats>,
    target: usize,
    /// Samples played since the queue last ran dry.
    clean: usize,
    /// The last value sent to the device, per channel.
    last: Vec<i16>,
    /// What the current fade starts from, per channel.
    from: Vec<i16>,
    /// Frames left of the fade from `from` to the incoming sound.
    fade_in: usize,
    /// Frames left of the fade from `from` to silence.
    fade_out: usize,
}

/// Create a ring for a device running at `rate` with `channels`.
pub fn ring(rate: u32, channels: usize) -> (RingProducer, RingConsumer, Arc<Stats>) {
    let limits = Limits::new(rate, channels);
    let shared = Arc::new(Shared {
        buf: (0..limits.capacity).map(|_| AtomicI16::new(0)).collect(),
        head: AtomicUsize::new(0),
        tail: AtomicUsize::new(0),
    });
    let stats = Arc::new(Stats::default());
    stats.target_ms.store(START_TARGET_MS, Ordering::Relaxed);
    (
        RingProducer {
            shared: shared.clone(),
            channels,
            stats: stats.clone(),
        },
        RingConsumer {
            shared,
            limits,
            channels,
            playing: false,
            stats: stats.clone(),
            target: limits.start_target,
            clean: 0,
            last: vec![0; channels],
            from: vec![0; channels],
            fade_in: 0,
            fade_out: 0,
        },
        stats,
    )
}

impl RingProducer {
    /// Queue interleaved device-format samples. Returns samples stored (fewer
    /// than given only when the consumer has stopped reading and the ring is
    /// full). Keeping the delay bounded is the consumer's job.
    pub fn push(&mut self, samples: &[i16]) -> usize {
        let sh = &self.shared;
        let head = sh.head.load(Ordering::Relaxed);
        let fill = head - sh.tail.load(Ordering::Acquire);
        let cap = sh.buf.len();
        let n = samples.len().min(cap - fill) / self.channels * self.channels;
        if n < samples.len() {
            let lost = (samples.len() - n) / self.channels;
            self.stats
                .dropped_frames
                .fetch_add(lost as u64, Ordering::Relaxed);
        }
        for (k, &s) in samples[..n].iter().enumerate() {
            sh.buf[(head + k) % cap].store(s, Ordering::Relaxed);
        }
        sh.head.store(head + n, Ordering::Release);
        n
    }
}

impl RingConsumer {
    fn start_fade_in(&mut self) {
        self.from.copy_from_slice(&self.last);
        self.fade_in = self.limits.fade_frames;
        self.fade_out = 0;
    }

    fn set_target(&mut self, samples: usize) {
        self.target = samples;
        let per_ms = (self.limits.start_target / START_TARGET_MS).max(1);
        self.stats
            .target_ms
            .store(samples / per_ms, Ordering::Relaxed);
    }

    /// Silence, reached by a fade from the last value sent.
    fn fill_silence<T: Copy>(&mut self, out: &mut [T], conv: &impl Fn(i16) -> T) {
        let total = self.limits.fade_frames as i32;
        for frame in out.chunks_mut(self.channels) {
            let left = self.fade_out as i32;
            for (c, o) in frame.iter_mut().enumerate() {
                let v = (i32::from(self.from[c]) * left / total) as i16;
                *o = conv(v);
                self.last[c] = v;
            }
            self.fade_out = self.fade_out.saturating_sub(1);
        }
    }

    /// Fill `out` from the ring, converting each sample with `conv`. Real-time
    /// safe: no locks, no allocation.
    pub fn render<T: Copy>(&mut self, out: &mut [T], conv: impl Fn(i16) -> T, _silence: T) {
        let mut tail = self.shared.tail.load(Ordering::Relaxed);
        let mut fill = self.shared.head.load(Ordering::Acquire) - tail;
        if self.playing && fill > self.target + self.limits.slack {
            let skip = fill - self.target;
            tail += skip;
            fill -= skip;
            self.shared.tail.store(tail, Ordering::Release);
            self.stats
                .dropped_frames
                .fetch_add((skip / self.channels) as u64, Ordering::Relaxed);
            self.start_fade_in();
        }
        if !self.playing {
            if fill < self.target {
                self.fill_silence(out, &conv);
                return;
            }
            self.playing = true;
            self.start_fade_in();
        }
        let n = fill.min(out.len()) / self.channels * self.channels;
        let cap = self.shared.buf.len();
        let total = self.limits.fade_frames as i32 + 1;
        for (f, frame) in out[..n].chunks_mut(self.channels).enumerate() {
            let at = tail + f * self.channels;
            for (c, o) in frame.iter_mut().enumerate() {
                let s = i32::from(self.shared.buf[(at + c) % cap].load(Ordering::Relaxed));
                let v = if self.fade_in > 0 {
                    let left = self.fade_in as i32;
                    (i32::from(self.from[c]) * left + s * (total - left)) / total
                } else {
                    s
                } as i16;
                *o = conv(v);
                self.last[c] = v;
            }
            self.fade_in = self.fade_in.saturating_sub(1);
        }
        self.shared.tail.store(tail + n, Ordering::Release);
        if n < out.len() {
            self.playing = false;
            self.clean = 0;
            self.stats.underruns.fetch_add(1, Ordering::Relaxed);
            self.set_target((self.target + self.limits.grow).min(self.limits.max_target));
            self.from.copy_from_slice(&self.last);
            self.fade_in = 0;
            self.fade_out = self.limits.fade_frames;
            self.fill_silence(&mut out[n..], &conv);
        } else {
            self.clean += n;
            if self.clean >= self.limits.shrink_after {
                self.clean = 0;
                let lower = self
                    .target
                    .saturating_sub(self.limits.shrink)
                    .max(self.limits.start_target);
                self.set_target(lower);
            }
        }
    }
}

/// A running audio output stream. Drop (or `stop`) to stop playback.
pub struct AudioOut {
    /// Device sample rate: converts the dropped-frame count to time.
    rate: u32,
    device: String,
    stream: Option<Stream>,
    task: JoinHandle<()>,
    stats: Arc<Stats>,
}

impl AudioOut {
    /// Start playing. `pcm_rx` receives raw i16-LE interleaved PCM chunks;
    /// `sample_rate` (44100/48000) and `channels` (1/2) describe them (from
    /// the accepted audio session). Must be called inside a tokio runtime.
    pub fn start(
        sample_rate: u32,
        channels: u32,
        mut pcm_rx: mpsc::Receiver<Vec<u8>>,
    ) -> Result<Self, AudioOutError> {
        if sample_rate == 0 || !(1..=2).contains(&channels) {
            return Err(AudioOutError::BadSource);
        }
        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .ok_or(AudioOutError::NoDevice)?;
        let default_config = device
            .default_output_config()
            .map_err(|e| AudioOutError::Config(e.to_string()))?;
        let sample_format = default_config.sample_format();
        if !matches!(sample_format, SampleFormat::F32 | SampleFormat::I16) {
            return Err(AudioOutError::UnsupportedFormat(format!(
                "{sample_format:?}"
            )));
        }
        let config: StreamConfig = default_config.into();
        let dev_rate = config.sample_rate.0;
        let dev_ch = usize::from(config.channels);

        let (mut producer, mut consumer, stats) = ring(dev_rate, dev_ch);
        let src_ch = channels as usize;
        let mut resampler = Resampler::new(sample_rate, dev_rate, dev_ch);
        let task = tokio::spawn(async move {
            while let Some(chunk) = pcm_rx.recv().await {
                let pcm: Vec<i16> = chunk
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|b| i16::from_le_bytes(*b))
                    .collect();
                let remixed = remix(&pcm, src_ch, dev_ch);
                producer.push(&resampler.process(&remixed));
            }
        });

        let err_cb = |err| eprintln!("[pb-audio] stream error: {err}");
        let built = match sample_format {
            SampleFormat::F32 => device.build_output_stream(
                &config,
                move |data: &mut [f32], _| consumer.render(data, i16_to_f32, 0.0),
                err_cb,
                None,
            ),
            _ => device.build_output_stream(
                &config,
                move |data: &mut [i16], _| consumer.render(data, |s| s, 0),
                err_cb,
                None,
            ),
        };
        let stream = match built
            .map_err(|e| e.to_string())
            .and_then(|s| s.play().map(|()| s).map_err(|e| e.to_string()))
        {
            Ok(s) => s,
            Err(e) => {
                task.abort();
                return Err(AudioOutError::Stream(e));
            }
        };
        Ok(Self {
            rate: dev_rate,
            device: format!("{dev_rate} Hz, {dev_ch} ch, {sample_format:?}"),
            stream: Some(stream),
            task,
            stats,
        })
    }

    /// Audio discarded because the queue exceeded the maximum latency, in
    /// milliseconds of sound (the counter itself counts sample frames, one
    /// per channel group, not network chunks).
    pub fn dropped_ms(&self) -> u64 {
        frames_to_ms(self.dropped_frames(), self.rate)
    }

    /// The queue length playback aims for now, in ms: the delay this end adds.
    pub fn target_ms(&self) -> usize {
        self.stats.target_ms.load(Ordering::Relaxed)
    }

    /// The device's rate, channels and sample format, for the log.
    pub fn device(&self) -> &str {
        &self.device
    }

    /// Sample frames (one sample per channel) dropped because the queue
    /// exceeded the maximum latency.
    pub fn dropped_frames(&self) -> u64 {
        self.stats.dropped_frames.load(Ordering::Relaxed)
    }

    /// Times the device ran out of audio (silence was played).
    pub fn underruns(&self) -> u64 {
        self.stats.underruns.load(Ordering::Relaxed)
    }

    /// Stop playback and the feeding task (same as dropping).
    pub fn stop(self) {}
}

impl Drop for AudioOut {
    fn drop(&mut self) {
        // Stream first so the callback stops, then the producer task.
        self.stream.take();
        self.task.abort();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 1 kHz stereo: target 320 samples, skip above 480, capacity 1000, fade 5 frames.
    fn small() -> (RingProducer, RingConsumer, Arc<Stats>) {
        ring(1000, 2)
    }

    fn render_i16(c: &mut RingConsumer, n: usize) -> Vec<i16> {
        let mut out = vec![7i16; n];
        c.render(&mut out, |s| s, 0);
        out
    }

    #[test]
    fn limits_follow_the_documented_latencies() {
        let l = Limits::new(48000, 2);
        assert_eq!(l.start_target, 15360);
        assert_eq!(l.max_target, 24960);
        assert_eq!(l.slack, 7680);
        assert_eq!(l.capacity, 48000);
        assert_eq!(l.fade_frames, 240);
        assert!(
            l.max_target + l.slack < l.capacity,
            "the ring holds the worst queue"
        );
    }

    #[test]
    fn nothing_plays_until_the_target_is_full_and_then_it_fades_in() {
        let (mut p, mut c, stats) = small();
        p.push(&[50; 100]);
        assert_eq!(render_i16(&mut c, 50), vec![0; 50]);
        p.push(&[50; 220]); // now 320 = target
        let out = render_i16(&mut c, 20);
        assert!(out[0] > 0 && out[0] < 50, "starts faded, got {}", out[0]);
        assert_eq!(&out[10..], &[50; 10][..], "full level after the fade");
        assert_eq!(stats.underruns.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn samples_come_out_in_order_across_the_wrap_point() {
        let (mut p, mut c, _) = small();
        let mut next = 0i16;
        let mut got = Vec::new();
        for i in 0..20 {
            // Prime to the target, then keep it there (never overflows).
            let n = if i == 0 { 320 } else { 100 };
            let chunk: Vec<i16> = (0..n)
                .map(|_| {
                    next += 1;
                    next
                })
                .collect();
            p.push(&chunk);
            got.extend(render_i16(&mut c, 100));
        }
        let played = &got[10..]; // past the fade-in
        assert!(played.len() > 1000, "wrapped the 1000-sample ring");
        assert!(played.windows(2).all(|w| w[1] == w[0] + 1));
    }

    #[test]
    fn running_dry_fades_to_silence_counts_once_and_raises_the_target() {
        let (mut p, mut c, stats) = small();
        p.push(&[900; 320]);
        let out = render_i16(&mut c, 420);
        assert_eq!(&out[10..320], &[900; 310][..]);
        assert!(out[322] < 900 && out[322] > 0, "fades, got {}", out[322]);
        assert_eq!(&out[330..], &[0; 90][..], "silent after the fade");
        assert_eq!(stats.underruns.load(Ordering::Relaxed), 1);
        assert_eq!(stats.target_ms.load(Ordering::Relaxed), 200);
        // Below the new target: silence, and no second underrun counted.
        p.push(&[900; 300]);
        assert_eq!(render_i16(&mut c, 50), vec![0; 50]);
        assert_eq!(stats.underruns.load(Ordering::Relaxed), 1);
        p.push(&[900; 100]); // now 400 = 200 ms
        assert!(render_i16(&mut c, 20)[19] == 900, "plays at the new target");
    }

    #[test]
    fn no_break_in_the_sound_is_a_click() {
        // From full level to dry and back: each step stays well below the jump
        // a hard cut would make.
        let (mut p, mut c, _) = small();
        let mut heard = Vec::new();
        p.push(&[20_000; 320]);
        heard.extend(render_i16(&mut c, 400));
        p.push(&[20_000; 400]);
        heard.extend(render_i16(&mut c, 200));
        let worst = heard
            .windows(2)
            .map(|w| (i32::from(w[1]) - i32::from(w[0])).abs())
            .max()
            .unwrap();
        assert!(worst <= 5_000, "a step of {worst}");
    }

    #[test]
    fn the_target_stops_at_its_limit_and_comes_back_after_clean_minutes() {
        let (mut p, mut c, stats) = small();
        for _ in 0..6 {
            let want = stats.target_ms.load(Ordering::Relaxed) * 2;
            p.push(&vec![9; want]);
            render_i16(&mut c, want + 10);
        }
        assert_eq!(stats.target_ms.load(Ordering::Relaxed), MAX_TARGET_MS);
        p.push(&[9; 520]);
        render_i16(&mut c, 2);
        for _ in 0..600 {
            p.push(&[9; 200]);
            render_i16(&mut c, 200);
        }
        assert_eq!(
            stats.target_ms.load(Ordering::Relaxed),
            MAX_TARGET_MS - SHRINK_MS
        );
    }

    #[test]
    fn the_dropped_counter_is_sample_frames_and_converts_to_time() {
        // The log said 157440 "chunks" in 196 s: that is sample frames, 3.28 s of sound.
        assert_eq!(frames_to_ms(157_440, 48_000), 3280);
        assert_eq!(frames_to_ms(44_100, 44_100), 1000);
        assert_eq!(frames_to_ms(5, 0), 0);
    }

    #[test]
    fn the_burst_after_a_gap_is_kept_inside_the_slack() {
        let (mut p, mut c, stats) = small();
        p.push(&[1; 320]);
        render_i16(&mut c, 20);
        p.push(&[1; 150]); // 450, under the 480 limit
        render_i16(&mut c, 2);
        assert_eq!(stats.dropped_frames.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn past_the_slack_the_oldest_audio_is_skipped_down_to_the_target() {
        let (mut p, mut c, stats) = small();
        p.push(&[1; 320]);
        render_i16(&mut c, 20);
        p.push(&[2; 300]); // 600 > 480: skip down to 320
        let out = render_i16(&mut c, 40);
        assert_eq!(&out[..20], &[1; 20][..], "the newest of the old audio");
        assert_eq!(&out[20..], &[2; 20][..], "all of the new audio");
        assert_eq!(stats.dropped_frames.load(Ordering::Relaxed), 140);
    }

    #[test]
    fn a_long_stalls_backlog_plays_the_newest_sound() {
        let (mut p, mut c, stats) = small();
        p.push(&[1_000; 320]);
        render_i16(&mut c, 100);
        p.push(&[2_000; 800]); // a stall's backlog at once
        let out = render_i16(&mut c, 200);
        assert_eq!(&out[10..], &[2_000; 190][..], "nothing stale is played");
        assert!(out[0] > 1_000 && out[0] < 2_000, "the jump is blended");
        assert!(stats.dropped_frames.load(Ordering::Relaxed) > 0);
    }

    #[test]
    fn a_consumer_that_stopped_reading_cannot_overrun_the_ring() {
        let (mut p, _c, stats) = small();
        assert_eq!(p.push(&[1; 2000]), 1000, "capacity is 1000 samples");
        assert_eq!(p.push(&[1; 10]), 0);
        assert_eq!(stats.dropped_frames.load(Ordering::Relaxed), 505);
    }

    #[test]
    fn f32_rendering_converts_samples() {
        let (mut p, mut c, _) = small();
        p.push(&[16384; 320]);
        let mut out = vec![9.0f32; 20];
        c.render(&mut out, i16_to_f32, 0.0);
        assert_eq!(&out[10..], &[0.5; 10][..]);
    }

    #[tokio::test]
    async fn source_format_errors_are_typed() {
        let (_tx, rx) = mpsc::channel(1);
        assert_eq!(
            AudioOut::start(48000, 3, rx).err(),
            Some(AudioOutError::BadSource)
        );
        let msg = AudioOutError::UnsupportedFormat("U8".into()).to_string();
        assert!(msg.contains("unsupported sample format"));
    }

    #[tokio::test]
    async fn starts_plays_silence_and_stops_cleanly() {
        let (tx, rx) = mpsc::channel(16);
        let silence = vec![0u8; 1920]; // 10 ms of 48 kHz stereo i16
        match AudioOut::start(48000, 2, rx) {
            Ok(ao) => {
                for _ in 0..15 {
                    tx.send(silence.clone()).await.unwrap();
                }
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                ao.stop();
                // The producer task is gone, so the channel closes.
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                assert!(tx.send(silence).await.is_err());
            }
            Err(e) => {
                // No usable device (headless CI): the error must be typed.
                assert!(matches!(
                    e,
                    AudioOutError::NoDevice
                        | AudioOutError::Config(_)
                        | AudioOutError::UnsupportedFormat(_)
                        | AudioOutError::Stream(_)
                ));
            }
        }
    }
}
