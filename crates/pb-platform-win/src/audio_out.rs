//! WASAPI audio output via `cpal`: plays raw i16-LE PCM frames
//! streamed from the phone over the Phone Bridge transport.
//!
//! `AudioOut::start` takes a `tokio::sync::mpsc::Receiver<Vec<u8>>` of raw
//! PCM bytes (i16 LE, interleaved channels) and plays them through the
//! default output device in shared mode (WASAPI on Windows).

use std::sync::{Arc, Mutex};

use cpal::{
    traits::{DeviceTrait, HostTrait, StreamTrait},
    BufferSize, SampleRate, Stream, StreamConfig,
};
use tokio::sync::mpsc;

/// A running audio output stream. Drop to stop playback.
pub struct AudioOut {
    /// Kept alive so the cpal callback keeps running.
    _stream: Stream,
}

/// Error type for audio-output operations.
#[derive(Debug)]
pub struct AudioOutError(pub String);

impl std::fmt::Display for AudioOutError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "audio output error: {}", self.0)
    }
}

impl std::error::Error for AudioOutError {}

fn e(s: impl Into<String>) -> AudioOutError {
    AudioOutError(s.into())
}

impl AudioOut {
    /// Start playing audio.
    ///
    /// `pcm_rx` receives raw i16-LE PCM chunks (bytes, interleaved channels).
    /// `sample_rate` and `channels` must match what the phone sends (from
    /// [`AudioAccept`]).
    pub fn start(
        sample_rate: u32,
        channels: u32,
        mut pcm_rx: mpsc::Receiver<Vec<u8>>,
    ) -> Result<Self, AudioOutError> {
        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .ok_or_else(|| e("no default audio output device"))?;

        let config = StreamConfig {
            channels: channels as u16,
            sample_rate: SampleRate(sample_rate),
            buffer_size: BufferSize::Default,
        };

        // Shared ring buffer: the async receiver task fills it, the cpal
        // callback drains it.
        let ring: Arc<Mutex<Vec<i16>>> = Arc::new(Mutex::new(Vec::new()));
        let ring_fill = ring.clone();
        let ring_drain = ring.clone();

        // Bridge async → sync via a std mpsc channel so the fill thread can
        // block without touching the tokio runtime.
        let (std_tx, std_rx) = std::sync::mpsc::channel::<Vec<u8>>();

        // Fill thread: receives raw byte chunks and pushes i16 samples into
        // the ring buffer.
        std::thread::Builder::new()
            .name("pb-audio-fill".into())
            .spawn(move || {
                while let Ok(chunk) = std_rx.recv() {
                    let samples: Vec<i16> = chunk
                        .chunks_exact(2)
                        .map(|b| i16::from_le_bytes([b[0], b[1]]))
                        .collect();
                    if let Ok(mut buf) = ring_fill.lock() {
                        buf.extend_from_slice(&samples);
                        // Cap to 2 seconds of audio to prevent runaway growth.
                        let max = (sample_rate * channels * 2) as usize;
                        if buf.len() > max {
                            let drain = buf.len() - max;
                            buf.drain(0..drain);
                        }
                    }
                }
            })
            .map_err(|e| AudioOutError(e.to_string()))?;

        // Tokio task: forwards chunks from the async mpsc into the std mpsc.
        let std_tx_clone = std_tx.clone();
        tokio::spawn(async move {
            while let Some(chunk) = pcm_rx.recv().await {
                if std_tx_clone.send(chunk).is_err() {
                    break;
                }
            }
        });

        // cpal output callback: drains the ring buffer into the device buffer,
        // outputting silence for any samples not yet available.
        let stream = device
            .build_output_stream(
                &config,
                move |data: &mut [i16], _| {
                    let mut buf = ring_drain.lock().unwrap_or_else(|p| p.into_inner());
                    let avail = buf.len().min(data.len());
                    data[..avail].copy_from_slice(&buf[..avail]);
                    buf.drain(..avail);
                    for s in &mut data[avail..] {
                        *s = 0;
                    }
                },
                |err| {
                    eprintln!("[pb-audio] stream error: {err}");
                },
                None,
            )
            .map_err(|e| AudioOutError(e.to_string()))?;

        stream.play().map_err(|e| AudioOutError(e.to_string()))?;

        Ok(Self { _stream: stream })
    }

    /// Stop playback immediately (equivalent to dropping `self`).
    pub fn stop(self) {
        // `_stream` is dropped here, which stops the cpal stream.
    }
}
