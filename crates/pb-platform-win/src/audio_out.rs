//! WASAPI audio output via `cpal`: plays raw i16-LE PCM frames streamed from
//! the phone over the Phone Bridge transport.
//!
//! Data path: a tokio task converts each chunk to the device's channel count
//! and sample rate and pushes it into a single-producer/single-consumer ring of
//! atomics. The cpal callback only reads the ring and converts samples: no
//! locks, no allocation. It plays nothing until 100 ms are buffered, keeps the
//! queue between 100 and 150 ms (excess incoming audio is dropped and counted),
//! and outputs silence on underrun (counted, then re-prebuffers).

use std::sync::atomic::{AtomicI16, AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;

use cpal::{
    traits::{DeviceTrait, HostTrait, StreamTrait},
    SampleFormat, Stream, StreamConfig,
};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::audio_conv::{i16_to_f32, remix, Resampler};

/// Audio that must be buffered before playback starts, in ms.
pub const PREBUFFER_MS: usize = 100;
/// When the queue would exceed this, incoming audio is dropped, in ms.
pub const MAX_LATENCY_MS: usize = 150;
/// Queue length after a drop (the target), in ms.
pub const TARGET_MS: usize = 100;
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

/// Counters shared between the callback, the producer and the owner.
#[derive(Default)]
pub struct Stats {
    dropped_frames: AtomicU64,
    underruns: AtomicU64,
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
    pub prebuffer: usize,
    pub target: usize,
    pub max: usize,
    pub capacity: usize,
}

impl Limits {
    pub fn new(rate: u32, channels: usize) -> Self {
        let per_ms = |ms: usize| (rate as usize * ms / 1000) * channels;
        Self {
            prebuffer: per_ms(PREBUFFER_MS),
            target: per_ms(TARGET_MS),
            max: per_ms(MAX_LATENCY_MS),
            capacity: per_ms(RING_MS),
        }
    }
}

/// Write side of the ring (not used from the cpal callback).
pub struct RingProducer {
    shared: Arc<Shared>,
    limits: Limits,
    channels: usize,
    stats: Arc<Stats>,
}

/// Read side of the ring; lives inside the cpal callback.
pub struct RingConsumer {
    shared: Arc<Shared>,
    limits: Limits,
    playing: bool,
    stats: Arc<Stats>,
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
    (
        RingProducer {
            shared: shared.clone(),
            limits,
            channels,
            stats: stats.clone(),
        },
        RingConsumer {
            shared,
            limits,
            playing: false,
            stats: stats.clone(),
        },
        stats,
    )
}

impl RingProducer {
    /// Queue interleaved device-format samples. If they would push the queue
    /// past the maximum latency, only enough is kept to reach the target and
    /// the rest is dropped (counted in frames). Returns samples stored.
    pub fn push(&mut self, samples: &[i16]) -> usize {
        let sh = &self.shared;
        let head = sh.head.load(Ordering::Relaxed);
        let fill = head - sh.tail.load(Ordering::Acquire);
        let mut n = samples.len();
        if fill + n > self.limits.max {
            let room = self.limits.target.saturating_sub(fill);
            let keep = room.min(n) / self.channels * self.channels;
            let dropped = (n - keep) / self.channels;
            self.stats
                .dropped_frames
                .fetch_add(dropped as u64, Ordering::Relaxed);
            n = keep;
        }
        let cap = sh.buf.len();
        for (k, &s) in samples[..n].iter().enumerate() {
            sh.buf[(head + k) % cap].store(s, Ordering::Relaxed);
        }
        sh.head.store(head + n, Ordering::Release);
        n
    }
}

impl RingConsumer {
    /// Fill `out` from the ring, converting each sample with `conv`. Real-time
    /// safe: no locks, no allocation.
    pub fn render<T: Copy>(&mut self, out: &mut [T], conv: impl Fn(i16) -> T, silence: T) {
        let sh = &self.shared;
        let tail = sh.tail.load(Ordering::Relaxed);
        let fill = sh.head.load(Ordering::Acquire) - tail;
        if !self.playing {
            if fill < self.limits.prebuffer {
                out.fill(silence);
                return;
            }
            self.playing = true;
        }
        let n = fill.min(out.len());
        let cap = sh.buf.len();
        for (k, o) in out[..n].iter_mut().enumerate() {
            *o = conv(sh.buf[(tail + k) % cap].load(Ordering::Relaxed));
        }
        sh.tail.store(tail + n, Ordering::Release);
        if n < out.len() {
            out[n..].fill(silence);
            self.playing = false;
            self.stats.underruns.fetch_add(1, Ordering::Relaxed);
        }
    }
}

/// A running audio output stream. Drop (or `stop`) to stop playback.
pub struct AudioOut {
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
            stream: Some(stream),
            task,
            stats,
        })
    }

    /// Audio frames dropped because the queue exceeded the maximum latency.
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

    /// 1 kHz stereo: prebuffer 200 samples, target 200, max 300, capacity 1000.
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
        assert_eq!(l.prebuffer, 9600);
        assert_eq!(l.target, 9600);
        assert_eq!(l.max, 14400);
        assert_eq!(l.capacity, 48000);
    }

    #[test]
    fn nothing_plays_until_the_prebuffer_is_full() {
        let (mut p, mut c, stats) = small();
        p.push(&[5; 100]);
        assert_eq!(render_i16(&mut c, 50), vec![0; 50]);
        p.push(&[5; 100]); // now 200 = prebuffer
        assert_eq!(render_i16(&mut c, 4), vec![5; 4]);
        assert_eq!(stats.underruns.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn samples_come_out_in_order_across_the_wrap_point() {
        let (mut p, mut c, _) = small();
        let mut next = 0i16;
        let mut got = Vec::new();
        for i in 0..20 {
            // Prime with the prebuffer, then keep 200 queued (never overflows).
            let n = if i == 0 { 200 } else { 100 };
            let chunk: Vec<i16> = (0..n)
                .map(|_| {
                    next += 1;
                    next
                })
                .collect();
            p.push(&chunk);
            got.extend(render_i16(&mut c, 100));
        }
        let played: Vec<i16> = got.into_iter().filter(|&s| s != 0).collect();
        assert!(played.len() > 1000, "wrapped the 1000-sample ring");
        assert!(played.windows(2).all(|w| w[1] == w[0] + 1));
    }

    #[test]
    fn underrun_outputs_silence_counts_once_and_reprebuffers() {
        let (mut p, mut c, stats) = small();
        p.push(&[9; 200]);
        let out = render_i16(&mut c, 300);
        assert_eq!(&out[..200], &[9; 200][..]);
        assert_eq!(&out[200..], &[0; 100][..]);
        assert_eq!(stats.underruns.load(Ordering::Relaxed), 1);
        // Below the prebuffer again: silence, and no second underrun counted.
        p.push(&[9; 100]);
        assert_eq!(render_i16(&mut c, 50), vec![0; 50]);
        assert_eq!(stats.underruns.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn overflow_drops_incoming_audio_down_to_the_target_and_counts_frames() {
        let (mut p, mut c, stats) = small();
        assert_eq!(p.push(&[1; 250]), 250); // within max 300
                                            // 250 + 100 > 300: keep up to target 200 -> room for 0, drop 100 samples = 50 frames.
        assert_eq!(p.push(&[2; 100]), 0);
        assert_eq!(stats.dropped_frames.load(Ordering::Relaxed), 50);
        // Drain below target, then a big push keeps only up to the target.
        render_i16(&mut c, 250);
        assert_eq!(p.push(&[3; 400]), 200);
        assert_eq!(stats.dropped_frames.load(Ordering::Relaxed), 50 + 100);
    }

    #[test]
    fn f32_rendering_converts_samples() {
        let (mut p, mut c, _) = small();
        p.push(&[16384; 200]);
        let mut out = vec![9.0f32; 4];
        c.render(&mut out, i16_to_f32, 0.0);
        assert_eq!(out, vec![0.5; 4]);
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
