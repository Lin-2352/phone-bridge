//! WASAPI audio output via `cpal`: plays raw i16-LE PCM frames streamed from
//! the phone over the Phone Bridge transport.
//!
//! Data path: a tokio task converts each chunk to the device's channel count
//! and sample rate and pushes it into a single-producer/single-consumer ring of
//! atomics. The cpal callback only reads the ring and converts samples: no
//! locks, no allocation.
//!
//! The queue is the delay a listener hears, so it stays as short as the network
//! allows: [`PlayoutTarget`] picks its length from the gaps between arrivals.
//! Playback runs half a percent faster or slower to hold the queue's peak there,
//! so a small excess or a device whose clock differs never causes a jump. Only when
//! the queue passes target + [`SLACK_MS`], after a real stall, is the oldest audio
//! skipped. Every break in the sound (running dry, restarting, skipping) is faded
//! over [`FADE_MS`], so it is a soft gap and never a click or buzz.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicI16, AtomicI32, AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;

use cpal::{
    traits::{DeviceTrait, HostTrait, StreamTrait},
    SampleFormat, Stream, StreamConfig,
};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::audio_conv::{i16_to_f32, remix, Resampler};
use crate::thread_priority;

/// The queue before the network has shown what it needs, in ms.
pub const START_TARGET_MS: usize = 120;
/// The shortest queue, in ms. Even a perfect link leaves gaps of about 40 ms: the
/// phone captures in 21 ms pieces, and the device takes 10 ms of sound at a time.
pub const MIN_TARGET_MS: usize = 60;
/// The longest queue, in ms: a longer Wi-Fi stall is heard as a short gap, rather
/// than as delay on everything, all the time.
pub const MAX_TARGET_MS: usize = 160;
/// How far past the target the queue may go before old audio is skipped, in ms.
/// Less than that is played off by a small change of speed.
pub const SLACK_MS: usize = 60;
/// Length of the fade over a break in the sound, in ms.
pub const FADE_MS: usize = 5;
const RING_MS: usize = 500;
/// How often the speed is reconsidered, in ms: long enough to see the queue's
/// peak between bursts of packets.
const STEER_MS: usize = 1000;
/// The queue's peak counts as on target within this, in ms: packet bursts move it
/// around, and that must not make the speed hunt.
const ON_TARGET_MS: i64 = 10;
/// Speed change for each ms the queue's peak is off target, in parts per million.
const TRIM_PPM_PER_MS: i64 = 1_000;
/// The most the speed changes: far more than any device's clock is off, and too
/// little to hear. 2% was heard as the sound being slowed down.
const MAX_TRIM_PPM: i64 = 5_000;
/// [`PlayoutTarget`] looks at this many stretches of [`TARGET_WINDOW_S`] seconds.
const TARGET_WINDOWS: usize = 15;
/// How often [`PlayoutTarget::note`] should be called, in seconds.
pub const TARGET_WINDOW_S: u64 = 2;
/// Stretches that may be worse than the queue covers: about one short gap in 15 s
/// at worst, in exchange for not delaying everything for the sake of a rare stall.
const TARGET_OUTLIERS: usize = 2;
/// Added to the gap to cover. Gaps come in runs, with the queue not back at its peak
/// when the next one starts, and the device takes 10 ms at a time. Measured with 20:
/// a dropout every 24 s from gaps of 60 to 90 ms, each costing as long a refill.
const TARGET_MARGIN_MS: usize = 40;

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
pub struct Stats {
    dropped_frames: AtomicU64,
    underruns: AtomicU64,
    /// The consumer's current target, in ms (for the log).
    target_ms: AtomicUsize,
    consumed_frames: AtomicU64,
    max_callback_gap_us: AtomicU64,
    starved: AtomicU64,
    /// Queue length in samples since the last look; MAX before the first.
    queue_low: AtomicUsize,
    queue_peak: AtomicUsize,
    /// The feeding thread's priority as Windows reports it.
    thread_priority: AtomicI32,
    /// The queue length the owner asks for, in ms.
    want_target_ms: AtomicUsize,
    /// Speed correction now, in parts per million (for the log).
    trim_ppm: AtomicI32,
    /// What the device itself holds, in sample frames (for the log).
    buffer_frames: AtomicUsize,
}

impl Default for Stats {
    fn default() -> Self {
        Self {
            dropped_frames: AtomicU64::new(0),
            underruns: AtomicU64::new(0),
            target_ms: AtomicUsize::new(0),
            consumed_frames: AtomicU64::new(0),
            max_callback_gap_us: AtomicU64::new(0),
            starved: AtomicU64::new(0),
            queue_low: AtomicUsize::new(usize::MAX),
            queue_peak: AtomicUsize::new(0),
            thread_priority: AtomicI32::new(0),
            want_target_ms: AtomicUsize::new(START_TARGET_MS),
            trim_ppm: AtomicI32::new(0),
            buffer_frames: AtomicUsize::new(0),
        }
    }
}

/// What the device did with the stream since the last look, for the log.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PlaybackWindow {
    /// Sound the device took. Under the time that passed means it stalled.
    pub consumed_ms: u64,
    pub max_callback_gap_ms: u64,
    /// Callbacks that found the device had played everything it was given.
    pub starved: u64,
    pub queue_low_ms: u64,
    pub queue_peak_ms: u64,
    /// The speed correction at the end of the window, in parts per million.
    pub trim_ppm: i32,
    /// The delay the device's own buffer adds at most, in ms.
    pub device_buffer_ms: u64,
}

impl Stats {
    fn note_queue(&self, samples: usize) {
        self.queue_low.fetch_min(samples, Ordering::Relaxed);
        self.queue_peak.fetch_max(samples, Ordering::Relaxed);
    }

    /// The queue length to aim for, in ms. Takes hold within [`STEER_MS`], kept
    /// within [`MIN_TARGET_MS`] and [`MAX_TARGET_MS`].
    pub fn set_target_ms(&self, ms: usize) {
        self.want_target_ms.store(ms, Ordering::Relaxed);
    }

    /// The numbers since the last call, which restarts them.
    fn take_window(&self, rate: u32, channels: usize) -> PlaybackWindow {
        let ms = |samples: usize| frames_to_ms((samples / channels.max(1)) as u64, rate);
        let low = self.queue_low.swap(usize::MAX, Ordering::Relaxed);
        PlaybackWindow {
            consumed_ms: frames_to_ms(self.consumed_frames.swap(0, Ordering::Relaxed), rate),
            max_callback_gap_ms: self.max_callback_gap_us.swap(0, Ordering::Relaxed) / 1000,
            starved: self.starved.swap(0, Ordering::Relaxed),
            queue_low_ms: if low == usize::MAX { 0 } else { ms(low) },
            queue_peak_ms: ms(self.queue_peak.swap(0, Ordering::Relaxed)),
            trim_ppm: self.trim_ppm.load(Ordering::Relaxed),
            device_buffer_ms: frames_to_ms(self.buffer_frames.load(Ordering::Relaxed) as u64, rate),
        }
    }
}

/// How long the queue should be, from the gaps between arrivals lately: during a
/// gap the queue is all that plays. Gaps that keep coming get covered; a rare stall
/// doesn't raise the delay on everything, and when the network calms down the target
/// follows within 30 s.
#[derive(Default)]
pub struct PlayoutTarget {
    worst: VecDeque<u32>,
}

impl PlayoutTarget {
    /// The longest gap in the last [`TARGET_WINDOW_S`] seconds, in ms.
    /// Returns the queue length to aim for now, in ms.
    pub fn note(&mut self, worst_gap_ms: u32) -> usize {
        if self.worst.len() == TARGET_WINDOWS {
            self.worst.pop_front();
        }
        self.worst.push_back(worst_gap_ms);
        if self.worst.len() <= TARGET_OUTLIERS {
            return START_TARGET_MS;
        }
        let mut sorted: Vec<u32> = self.worst.iter().copied().collect();
        sorted.sort_unstable_by(|a, b| b.cmp(a));
        (sorted[TARGET_OUTLIERS] as usize + TARGET_MARGIN_MS).clamp(MIN_TARGET_MS, MAX_TARGET_MS)
    }
}

/// Watches the device's callbacks. Lives in the callback: atomics only.
#[derive(Default)]
struct DeviceWatch {
    last_us: Option<u64>,
    buffer_frames: usize,
}

impl DeviceWatch {
    fn on_callback(&mut self, now_us: u64, frames: usize, stats: &Stats) {
        stats
            .consumed_frames
            .fetch_add(frames as u64, Ordering::Relaxed);
        if let Some(last) = self.last_us {
            stats
                .max_callback_gap_us
                .fetch_max(now_us.saturating_sub(last), Ordering::Relaxed);
            // The whole buffer is free, so the device has played all of it.
            if frames >= self.buffer_frames {
                stats.starved.fetch_add(1, Ordering::Relaxed);
            }
        }
        self.buffer_frames = self.buffer_frames.max(frames);
        stats
            .buffer_frames
            .store(self.buffer_frames, Ordering::Relaxed);
        self.last_us = Some(now_us);
    }
}

/// What runs at the start of every callback, whatever the sample format.
struct CallbackStart {
    began: std::time::Instant,
    watch: DeviceWatch,
    stats: Arc<Stats>,
    channels: usize,
    first: bool,
}

impl CallbackStart {
    fn new(stats: Arc<Stats>, channels: usize) -> Self {
        Self {
            began: std::time::Instant::now(),
            watch: DeviceWatch::default(),
            stats,
            channels,
            first: true,
        }
    }

    fn run(&mut self, samples: usize) {
        if self.first {
            self.first = false;
            thread_priority::raise_this_thread_for_audio();
            self.stats
                .thread_priority
                .store(thread_priority::this_thread(), Ordering::Relaxed);
        }
        let now_us = self.began.elapsed().as_micros() as u64;
        self.watch
            .on_callback(now_us, samples / self.channels, &self.stats);
    }
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
    pub min_target: usize,
    pub max_target: usize,
    pub slack: usize,
    pub capacity: usize,
    /// Sample frames, not samples.
    pub fade_frames: usize,
    pub steer_every: usize,
    rate: usize,
    channels: usize,
}

impl Limits {
    pub fn new(rate: u32, channels: usize) -> Self {
        let per_ms = |ms: usize| (rate as usize * ms / 1000) * channels;
        Self {
            rate: rate as usize,
            channels,
            start_target: per_ms(START_TARGET_MS),
            min_target: per_ms(MIN_TARGET_MS),
            max_target: per_ms(MAX_TARGET_MS),
            slack: per_ms(SLACK_MS),
            capacity: per_ms(RING_MS),
            fade_frames: (rate as usize * FADE_MS / 1000).max(1),
            steer_every: per_ms(STEER_MS),
        }
    }

    /// Whole sample frames, as samples, for `ms` of sound.
    fn samples(&self, ms: usize) -> usize {
        (self.rate * ms / 1000) * self.channels
    }

    /// Ms of sound in `samples`, which may be negative.
    fn ms(&self, samples: i64) -> i64 {
        samples * 1000 / (self.rate * self.channels).max(1) as i64
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
    /// The queue's highest point since the speed was last reconsidered.
    peak: usize,
    /// Samples to go until the speed is reconsidered.
    steer_in: usize,
    /// How much faster than normal to read the queue, in parts per million.
    trim_ppm: i64,
    /// Where the read position is between two sample frames, in 1/2^32 of one.
    frac: u32,
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
            peak: 0,
            steer_in: limits.steer_every,
            trim_ppm: 0,
            frac: 0,
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

    /// Takes up the target the owner asks for, and sets the speed that brings the
    /// queue's peak to it. Never mid-way through a callback's sound.
    fn steer(&mut self) {
        self.steer_in = self.limits.steer_every;
        let want = self
            .stats
            .want_target_ms
            .load(Ordering::Relaxed)
            .clamp(MIN_TARGET_MS, MAX_TARGET_MS);
        self.target = self.limits.samples(want);
        self.stats.target_ms.store(want, Ordering::Relaxed);
        let off_ms = self.limits.ms(self.peak as i64 - self.target as i64);
        self.peak = 0;
        self.trim_ppm = if !self.playing || off_ms.abs() <= ON_TARGET_MS {
            0
        } else {
            (off_ms * TRIM_PPM_PER_MS).clamp(-MAX_TRIM_PPM, MAX_TRIM_PPM)
        };
        self.stats
            .trim_ppm
            .store(self.trim_ppm as i32, Ordering::Relaxed);
    }

    /// How far to move through the queue for each frame played, in 1/2^32 of a frame.
    fn read_step(&self) -> u64 {
        const ONE: i64 = 1 << 32;
        let step = match (self.trim_ppm, self.frac) {
            (0, 0) => ONE,
            // No more correction wanted: on to the next whole frame, then plain copying.
            (0, _) => ONE + ONE / 1000,
            (trim, _) => ONE + ONE * trim / 1_000_000,
        };
        step as u64
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
        self.stats.note_queue(fill);
        if fill > self.target + self.limits.slack {
            let skip = fill - self.target;
            tail += skip;
            fill -= skip;
            self.shared.tail.store(tail, Ordering::Release);
            self.stats
                .dropped_frames
                .fetch_add((skip / self.channels) as u64, Ordering::Relaxed);
            self.frac = 0;
            self.start_fade_in();
        }
        self.peak = self.peak.max(fill);
        self.steer_in = self.steer_in.saturating_sub(out.len());
        if self.steer_in == 0 {
            self.steer();
        }
        if !self.playing {
            if fill < self.target {
                self.fill_silence(out, &conv);
                return;
            }
            self.playing = true;
            self.frac = 0;
            self.trim_ppm = 0;
            self.peak = fill;
            self.steer_in = self.limits.steer_every;
            self.start_fade_in();
        }
        let channels = self.channels;
        let wanted = out.len() / channels;
        let queued = fill / channels;
        let cap = self.shared.buf.len();
        let total = self.limits.fade_frames as i32 + 1;
        let mut read = 0;
        let mut made = 0;
        while made < wanted {
            let frac = self.frac;
            // Between two frames both must be there; on a frame, only that one.
            if read + usize::from(frac != 0) >= queued {
                break;
            }
            let at = tail + read * channels;
            for c in 0..channels {
                let a = i64::from(self.shared.buf[(at + c) % cap].load(Ordering::Relaxed));
                let s = if frac == 0 {
                    a
                } else {
                    let b = i64::from(
                        self.shared.buf[(at + channels + c) % cap].load(Ordering::Relaxed),
                    );
                    a + (((b - a) * i64::from(frac)) >> 32)
                } as i32;
                let v = if self.fade_in > 0 {
                    let left = self.fade_in as i32;
                    (i32::from(self.from[c]) * left + s * (total - left)) / total
                } else {
                    s
                } as i16;
                out[made * channels + c] = conv(v);
                self.last[c] = v;
            }
            self.fade_in = self.fade_in.saturating_sub(1);
            made += 1;
            let next = u64::from(frac) + self.read_step();
            let whole = (next >> 32) as usize;
            read += whole;
            // Passing a frame with no correction wanted is where the fraction ends.
            self.frac = if self.trim_ppm == 0 && whole >= 2 {
                0
            } else {
                next as u32
            };
        }
        self.shared
            .tail
            .store(tail + read * channels, Ordering::Release);
        if made < wanted {
            self.playing = false;
            self.frac = 0;
            self.trim_ppm = 0;
            self.stats.underruns.fetch_add(1, Ordering::Relaxed);
            self.from.copy_from_slice(&self.last);
            self.fade_in = 0;
            self.fade_out = self.limits.fade_frames;
            self.fill_silence(&mut out[made * channels..], &conv);
        }
    }
}

/// A running audio output stream. Drop (or `stop`) to stop playback.
pub struct AudioOut {
    /// Device sample rate: converts the dropped-frame count to time.
    rate: u32,
    channels: usize,
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
        let mut start_of_callback = CallbackStart::new(stats.clone(), dev_ch);
        let built = match sample_format {
            SampleFormat::F32 => device.build_output_stream(
                &config,
                move |data: &mut [f32], _| {
                    start_of_callback.run(data.len());
                    consumer.render(data, i16_to_f32, 0.0);
                },
                err_cb,
                None,
            ),
            _ => device.build_output_stream(
                &config,
                move |data: &mut [i16], _| {
                    start_of_callback.run(data.len());
                    consumer.render(data, |s| s, 0);
                },
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
            channels: dev_ch,
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

    /// What the device did since the last call, for the log.
    pub fn take_window(&self) -> PlaybackWindow {
        self.stats.take_window(self.rate, self.channels)
    }

    /// The feeding thread's priority as Windows reports it. 0 is normal.
    pub fn thread_priority(&self) -> i32 {
        self.stats.thread_priority.load(Ordering::Relaxed)
    }

    /// The queue length to aim for, in ms; see [`PlayoutTarget`].
    pub fn set_target_ms(&self, ms: usize) {
        self.stats.set_target_ms(ms);
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

    /// 1 kHz stereo, so 2 samples a ms: target 240 samples, skip above 360,
    /// capacity 1000, fade 5 frames.
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
        assert_eq!(l.start_target, 11520);
        assert_eq!(l.min_target, 5760);
        assert_eq!(l.max_target, 15360);
        assert_eq!(l.slack, 5760);
        assert_eq!(l.capacity, 48000);
        assert_eq!(l.fade_frames, 240);
        assert!(
            l.max_target + l.slack < l.capacity,
            "the ring holds the worst queue"
        );
    }

    #[test]
    fn a_callback_that_finds_the_whole_buffer_free_counts_as_starved() {
        let stats = Stats::default();
        let mut watch = DeviceWatch::default();
        watch.on_callback(0, 1000, &stats); // the first fill is the whole buffer
        watch.on_callback(10_000, 480, &stats);
        watch.on_callback(20_000, 480, &stats);
        watch.on_callback(60_000, 1000, &stats); // 40 ms late: the device ran dry
        let w = stats.take_window(48_000, 2);
        assert_eq!(w.starved, 1);
        assert_eq!(w.max_callback_gap_ms, 40);
        assert_eq!(w.consumed_ms, 2960 * 1000 / 48_000);
        assert_eq!(w.device_buffer_ms, 20, "the first fill is the whole buffer");
        let next = stats.take_window(48_000, 2);
        assert_eq!(
            (next.starved, next.max_callback_gap_ms, next.consumed_ms),
            (0, 0, 0)
        );
    }

    #[test]
    fn a_window_reports_the_lowest_and_highest_queue() {
        let (mut p, mut c, stats) = small(); // 2 samples a millisecond
        p.push(&[1; 320]);
        render_i16(&mut c, 20); // sees 320
        render_i16(&mut c, 100); // sees 300
        let w = stats.take_window(1000, 2);
        assert_eq!((w.queue_low_ms, w.queue_peak_ms), (150, 160));
    }

    #[test]
    fn nothing_plays_until_the_target_is_full_and_then_it_fades_in() {
        let (mut p, mut c, stats) = small();
        p.push(&[50; 100]);
        assert_eq!(render_i16(&mut c, 50), vec![0; 50]);
        p.push(&[50; 140]); // now 240 = target
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
            let n = if i == 0 { 240 } else { 100 };
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
    fn running_dry_fades_to_silence_and_counts_once() {
        let (mut p, mut c, stats) = small();
        p.push(&[900; 240]);
        let out = render_i16(&mut c, 340);
        assert_eq!(&out[10..240], &[900; 230][..]);
        assert!(out[242] < 900 && out[242] > 0, "fades, got {}", out[242]);
        assert_eq!(&out[250..], &[0; 90][..], "silent after the fade");
        assert_eq!(stats.underruns.load(Ordering::Relaxed), 1);
        // Below the target: silence, and no second underrun counted.
        p.push(&[900; 200]);
        assert_eq!(render_i16(&mut c, 50), vec![0; 50]);
        assert_eq!(stats.underruns.load(Ordering::Relaxed), 1);
        p.push(&[900; 40]); // now 240 = the target
        assert!(render_i16(&mut c, 20)[19] == 900, "plays at the target");
    }

    #[test]
    fn a_dropout_does_not_raise_the_target() {
        let (mut p, mut c, stats) = small();
        p.push(&[900; 240]);
        render_i16(&mut c, 340);
        assert_eq!(stats.underruns.load(Ordering::Relaxed), 1);
        assert_eq!(stats.target_ms.load(Ordering::Relaxed), START_TARGET_MS);
    }

    #[test]
    fn no_break_in_the_sound_is_a_click() {
        // From full level to dry and back: each step stays well below the jump
        // a hard cut would make.
        let (mut p, mut c, _) = small();
        let mut heard = Vec::new();
        p.push(&[20_000; 240]);
        heard.extend(render_i16(&mut c, 320));
        p.push(&[20_000; 300]);
        heard.extend(render_i16(&mut c, 200));
        let worst = heard
            .windows(2)
            .map(|w| (i32::from(w[1]) - i32::from(w[0])).abs())
            .max()
            .unwrap();
        assert!(worst <= 5_000, "a step of {worst}");
    }

    #[test]
    fn a_new_target_is_kept_within_its_limits() {
        let (mut p, mut c, stats) = small();
        p.push(&[1; 240]);
        stats.set_target_ms(10_000);
        for _ in 0..120 {
            p.push(&[1; 20]);
            render_i16(&mut c, 20);
        }
        assert_eq!(stats.target_ms.load(Ordering::Relaxed), MAX_TARGET_MS);
        stats.set_target_ms(1);
        for _ in 0..120 {
            p.push(&[1; 20]);
            render_i16(&mut c, 20);
        }
        assert_eq!(stats.target_ms.load(Ordering::Relaxed), MIN_TARGET_MS);
    }

    /// `ticks` of 10 ms: a 20 ms chunk every second tick, the device taking 10 ms
    /// each tick, except that each 500th tick it takes none (`slow`) or twice (fast).
    fn run_with_device_slightly_off(
        p: &mut RingProducer,
        c: &mut RingConsumer,
        ticks: usize,
        slow: bool,
    ) {
        for tick in 0..ticks {
            if tick % 2 == 0 {
                p.push(&[5; 40]);
            }
            match (tick % 500 == 499, slow) {
                (true, true) => {}
                (true, false) => {
                    render_i16(c, 40);
                }
                (false, _) => {
                    render_i16(c, 20);
                }
            }
        }
    }

    #[test]
    fn a_device_running_slightly_slow_never_skips() {
        let (mut p, mut c, stats) = small();
        p.push(&[5; 240]);
        run_with_device_slightly_off(&mut p, &mut c, 60_000, true);
        assert_eq!(stats.dropped_frames.load(Ordering::Relaxed), 0);
        assert_eq!(stats.underruns.load(Ordering::Relaxed), 0);
        stats.take_window(1000, 2);
        render_i16(&mut c, 2);
        let peak = stats.take_window(1000, 2).queue_peak_ms;
        assert!(peak <= 140, "the queue crept up to {peak} ms");
    }

    #[test]
    fn a_device_running_slightly_fast_never_runs_dry() {
        let (mut p, mut c, stats) = small();
        p.push(&[5; 240]);
        run_with_device_slightly_off(&mut p, &mut c, 60_000, false);
        assert_eq!(stats.underruns.load(Ordering::Relaxed), 0);
        assert_eq!(stats.dropped_frames.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn bursts_of_100_ms_keep_the_peak_at_the_target() {
        // Peaks 40 ms above the target, within the slack, so only a change of
        // speed can bring it down. Sound arrives 100 ms at a time.
        let (mut p, mut c, stats) = small();
        p.push(&[5; 120]);
        for tick in 0..6_000 {
            if tick % 10 == 0 {
                p.push(&[5; 200]);
            }
            render_i16(&mut c, 20);
            if tick == 5_000 {
                stats.take_window(1000, 2);
                stats.dropped_frames.store(0, Ordering::Relaxed);
            }
        }
        let w = stats.take_window(1000, 2);
        assert!(w.queue_peak_ms <= 130, "peak {} ms", w.queue_peak_ms);
        assert!(w.queue_low_ms >= 10, "low {} ms", w.queue_low_ms);
        assert_eq!(stats.dropped_frames.load(Ordering::Relaxed), 0);
        assert_eq!(stats.underruns.load(Ordering::Relaxed), 0);
    }

    /// A sine with a 200 ms period at 1 kHz: its largest step between samples is
    /// about 314, so anything far above that is a jump.
    fn sine(frame: usize) -> i16 {
        ((frame as f64 / 200.0 * std::f64::consts::TAU).sin() * 10_000.0) as i16
    }

    #[test]
    fn trim_keeps_a_sine_continuous_while_the_queue_drains_to_a_lower_target() {
        let (mut p, mut c, stats) = small();
        let mut frame = 0;
        let mut feed = |p: &mut RingProducer, frames: usize| {
            let chunk: Vec<i16> = (0..frames)
                .flat_map(|_| {
                    frame += 1;
                    [sine(frame), sine(frame)]
                })
                .collect();
            p.push(&chunk);
        };
        feed(&mut p, 120);
        render_i16(&mut c, 40); // past the fade-in
        stats.set_target_ms(80);
        let mut heard = Vec::new();
        for _ in 0..2_000 {
            feed(&mut p, 10);
            heard.extend(render_i16(&mut c, 20));
        }
        let worst = heard
            .chunks(2)
            .map(|f| f[0])
            .collect::<Vec<_>>()
            .windows(2)
            .map(|w| (i32::from(w[1]) - i32::from(w[0])).abs())
            .max()
            .unwrap();
        assert!(worst <= 350, "a step of {worst}");
        assert_eq!(stats.dropped_frames.load(Ordering::Relaxed), 0, "no skip");
        stats.take_window(1000, 2);
        render_i16(&mut c, 2);
        let peak = stats.take_window(1000, 2).queue_peak_ms;
        assert!(peak <= 95, "still at {peak} ms");
    }

    #[test]
    fn the_speed_never_moves_far_enough_to_hear() {
        // Far over the target, but within the slack: at most half a percent faster.
        let (mut p, mut c, stats) = small();
        p.push(&[5; 300]);
        for _ in 0..200 {
            p.push(&[5; 20]);
            render_i16(&mut c, 20);
            assert!(c.trim_ppm.abs() <= 5_000, "trim {}", c.trim_ppm);
        }
        assert_ne!(c.trim_ppm, 0, "and it is correcting");
        assert_eq!(i64::from(stats.take_window(1000, 2).trim_ppm), c.trim_ppm);
    }

    #[test]
    fn a_queue_close_to_its_target_is_left_alone() {
        // Bursts move the peak around a little: that must not make the speed hunt.
        let (mut p, mut c, _) = small();
        p.push(&[5; 240]);
        for _ in 0..400 {
            p.push(&[5; 20]);
            render_i16(&mut c, 20);
            assert_eq!(c.trim_ppm, 0);
        }
    }

    #[test]
    fn when_no_correction_is_needed_samples_are_copied_exactly() {
        // Off target for a while, then on it: the original samples come back.
        let (mut p, mut c, stats) = small();
        p.push(&[5; 300]);
        for _ in 0..1_500 {
            p.push(&[5; 20]);
            render_i16(&mut c, 20);
        }
        let mut next = 99i16;
        let mut got = Vec::new();
        for _ in 0..200 {
            let chunk: Vec<i16> = (0..20)
                .map(|_| {
                    next += 1;
                    next
                })
                .collect();
            p.push(&chunk);
            got.extend(render_i16(&mut c, 20));
        }
        let played: Vec<i16> = got.into_iter().filter(|s| *s >= 100).collect();
        assert!(played.len() > 1000, "only {} samples", played.len());
        let bad = played[2..].windows(2).position(|w| w[1] != w[0] + 1);
        assert_eq!(
            bad,
            None,
            "interpolation left on: {:?}, frac {} trim {}",
            bad.map(|i| &played[i..(i + 8).min(played.len())]),
            c.frac,
            c.trim_ppm
        );
        assert_eq!(stats.dropped_frames.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn the_first_windows_use_the_start_target() {
        let mut t = PlayoutTarget::default();
        assert_eq!(t.note(0), START_TARGET_MS);
        assert_eq!(t.note(0), START_TARGET_MS);
    }

    #[test]
    fn repeated_gaps_set_the_target() {
        let mut t = PlayoutTarget::default();
        for _ in 0..5 {
            t.note(20);
        }
        for _ in 0..2 {
            t.note(90);
        }
        assert_eq!(t.note(90), 130, "a 90 ms gap three times, plus the margin");
    }

    #[test]
    fn one_stall_does_not_set_the_target() {
        let mut t = PlayoutTarget::default();
        for _ in 0..10 {
            t.note(50);
        }
        assert_eq!(t.note(400), 90, "50 ms gaps as a rule, plus the margin");
        assert_eq!(t.note(350), 90, "nor does a second one");
    }

    #[test]
    fn the_target_falls_within_30_s() {
        let mut t = PlayoutTarget::default();
        for _ in 0..15 {
            t.note(120);
        }
        let mut last = 0;
        for _ in 0..13 {
            last = t.note(50);
        }
        assert_eq!(last, 90, "26 s later only two old windows remain");
    }

    #[test]
    fn the_target_stays_within_its_limits() {
        let mut t = PlayoutTarget::default();
        let mut last = 0;
        for _ in 0..15 {
            last = t.note(5_000);
        }
        assert_eq!(last, MAX_TARGET_MS);
        for _ in 0..15 {
            last = t.note(0);
        }
        assert_eq!(last, MIN_TARGET_MS);
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
        p.push(&[1; 240]);
        render_i16(&mut c, 20);
        p.push(&[1; 130]); // 350, under the 360 limit
        render_i16(&mut c, 2);
        assert_eq!(stats.dropped_frames.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn past_the_slack_the_oldest_audio_is_skipped_down_to_the_target() {
        let (mut p, mut c, stats) = small();
        p.push(&[1; 240]);
        render_i16(&mut c, 20);
        p.push(&[2; 200]); // 420 > 360: skip down to 240
        let out = render_i16(&mut c, 60);
        assert_eq!(&out[..40], &[1; 40][..], "the newest of the old audio");
        assert_eq!(&out[40..], &[2; 20][..], "all of the new audio");
        assert_eq!(stats.dropped_frames.load(Ordering::Relaxed), 90);
    }

    #[test]
    fn a_long_stalls_backlog_plays_the_newest_sound() {
        let (mut p, mut c, stats) = small();
        p.push(&[1_000; 240]);
        render_i16(&mut c, 100);
        p.push(&[2_000; 800]); // a stall's backlog at once
        let out = render_i16(&mut c, 200);
        assert_eq!(&out[10..], &[2_000; 190][..], "nothing stale is played");
        assert!(out[0] > 1_000 && out[0] < 2_000, "the jump is blended");
        assert!(stats.dropped_frames.load(Ordering::Relaxed) > 0);
    }

    #[test]
    fn after_a_stall_the_newest_sound_plays_within_the_slack() {
        let (mut p, mut c, stats) = small();
        p.push(&[1_000; 240]);
        render_i16(&mut c, 100);
        p.push(&[2_000; 800]);
        render_i16(&mut c, 2);
        stats.take_window(1000, 2);
        render_i16(&mut c, 2);
        let peak = stats.take_window(1000, 2).queue_peak_ms;
        assert!(peak <= 120, "still {peak} ms behind");
    }

    #[test]
    fn after_running_dry_a_backlog_plays_from_its_newest_sound() {
        let (mut p, mut c, _) = small();
        p.push(&[1_000; 240]);
        render_i16(&mut c, 300); // ran dry
        p.push(&[2_000; 500]);
        p.push(&[3_000; 300]); // the stall ends with all of it at once
        let out = render_i16(&mut c, 40);
        assert_eq!(&out[10..], &[3_000; 30][..], "nothing stale is played");
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
        p.push(&[16384; 240]);
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
