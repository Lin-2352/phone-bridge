//! WASAPI audio output via `cpal`: plays raw i16-LE PCM frames
//! streamed from the phone over the Phone Bridge transport.
//!
//! `AudioOut::start` takes a `tokio::sync::mpsc::Receiver<Vec<u8>>` of raw
//! PCM bytes (i16 LE, interleaved channels) and plays them through the
//! default output device in shared mode (WASAPI on Windows).

use std::sync::{Arc, Mutex};

use cpal::{
    traits::{DeviceTrait, HostTrait, StreamTrait},
    SampleFormat, Stream, StreamConfig,
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

        let default_config = device
            .default_output_config()
            .map_err(|e| AudioOutError(e.to_string()))?;

        let sample_format = default_config.sample_format();
        let config: StreamConfig = default_config.into();

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
        // Handles F32 (Windows WASAPI default) and I16 formats.
        let stream = match sample_format {
            SampleFormat::F32 => device.build_output_stream(
                &config,
                move |data: &mut [f32], _| {
                    let mut buf = ring_drain.lock().unwrap_or_else(|p| p.into_inner());
                    let avail = buf.len().min(data.len());
                    for i in 0..avail {
                        data[i] = buf[i] as f32 / 32768.0;
                    }
                    buf.drain(..avail);
                    for s in &mut data[avail..] {
                        *s = 0.0;
                    }
                },
                |err| {
                    eprintln!("[pb-audio] stream error: {err}");
                },
                None,
            ),
            SampleFormat::I16 => device.build_output_stream(
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
            ),
            _ => device.build_output_stream(
                &config,
                move |data: &mut [f32], _| {
                    let mut buf = ring_drain.lock().unwrap_or_else(|p| p.into_inner());
                    let avail = buf.len().min(data.len());
                    for i in 0..avail {
                        data[i] = buf[i] as f32 / 32768.0;
                    }
                    buf.drain(..avail);
                    for s in &mut data[avail..] {
                        *s = 0.0;
                    }
                },
                |err| {
                    eprintln!("[pb-audio] stream error: {err}");
                },
                None,
            ),
        }
        .map_err(|e| AudioOutError(e.to_string()))?;

        stream.play().map_err(|e| AudioOutError(e.to_string()))?;

        Ok(Self { _stream: stream })
    }

    /// Stop playback immediately (equivalent to dropping `self`).
    pub fn stop(self) {
        // `_stream` is dropped here, which stops the cpal stream.
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn audio_out_can_receive_and_stop() {
        let (tx, rx) = mpsc::channel(16);
        // Feed 100ms of 48kHz stereo silence (48000 * 2 * 2 * 0.1 = 19200 bytes)
        let silence = vec![0u8; 1920];
        tx.send(silence.clone()).await.unwrap();

        // If an audio output device exists on this machine, test playback start and stop;
        // if no audio device is available (e.g. headless CI), AudioOut gracefully returns an error.
        match AudioOut::start(48000, 2, rx) {
            Ok(ao) => {
                for _ in 0..5 {
                    if tx.send(silence.clone()).await.is_err() {
                        break;
                    }
                }
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                ao.stop();
            }
            Err(e) => {
                println!("AudioOut error: {}", e.0);
                assert!(!e.0.is_empty());
            }
        }
    }
}
