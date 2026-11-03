//! WASAPI loopback capture of the default output device via `cpal`.
//!
//! cpal records an output device as loopback when an input stream is built on
//! it. Audio is converted to 48 kHz stereo i16 LE and delivered in 20 ms
//! frames (3840 bytes) to a callback, which matches
//! `pb_transport::audio::AudioSession::send_frame(Vec<u8>)`. Capture runs on
//! its own thread (cpal streams are not `Send`), starts only when asked, and
//! is joined by `stop`/drop. A default-device change or device removal stops
//! the capture and reports an error; restarting is the caller's job.
//!
//! Note: WASAPI delivers no loopback packets while nothing is playing, so
//! frames arrive only while the PC produces sound.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use cpal::{
    traits::{DeviceTrait, HostTrait, StreamTrait},
    SampleFormat, StreamConfig, StreamError,
};

use crate::audio_conv::FrameBuilder;

const POLL: Duration = Duration::from_millis(50);
const DEVICE_CHECK: Duration = Duration::from_millis(500);

/// Typed capture errors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AudioCaptureError {
    /// There is no default output device to capture.
    NoDevice,
    /// The device's native format is neither F32 nor I16.
    FormatUnsupported(String),
    /// The device was unplugged or disabled while capturing.
    DeviceRemoved,
    /// The default output device changed; the capture stopped.
    DefaultDeviceChanged,
    /// Any other stream or configuration failure.
    Stream(String),
}

impl std::fmt::Display for AudioCaptureError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoDevice => write!(f, "no default audio output device to capture"),
            Self::FormatUnsupported(s) => {
                write!(
                    f,
                    "the output device uses an unsupported sample format ({s})"
                )
            }
            Self::DeviceRemoved => write!(f, "the audio device was removed"),
            Self::DefaultDeviceChanged => write!(f, "the default audio device changed"),
            Self::Stream(s) => write!(f, "audio capture error: {s}"),
        }
    }
}

impl std::error::Error for AudioCaptureError {}

/// Map a cpal runtime stream error.
pub fn map_stream_error(err: &StreamError) -> AudioCaptureError {
    match err {
        StreamError::DeviceNotAvailable => AudioCaptureError::DeviceRemoved,
        other => AudioCaptureError::Stream(other.to_string()),
    }
}

/// A running loopback capture. `stop` (or drop) joins the capture thread.
pub struct AudioCapture {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl AudioCapture {
    /// Start capturing the default output device.
    ///
    /// `on_frame` gets each 20 ms 48 kHz stereo i16 LE frame, from the cpal
    /// callback thread (it must not block; `send_frame` never does).
    /// `on_error` is called at most once, from the capture thread, if the
    /// capture ends by itself (device removed/changed, stream failure); it is
    /// not called after `stop`. Setup errors are returned directly.
    pub fn start(
        on_frame: impl FnMut(Vec<u8>) + Send + 'static,
        on_error: impl FnOnce(AudioCaptureError) + Send + 'static,
    ) -> Result<Self, AudioCaptureError> {
        let stop = Arc::new(AtomicBool::new(false));
        let (ready_tx, ready_rx) = mpsc::channel::<Result<(), AudioCaptureError>>();
        let stop_t = stop.clone();
        let thread = std::thread::Builder::new()
            .name("pb-audio-capture".into())
            .spawn(move || run(on_frame, on_error, stop_t, ready_tx))
            .map_err(|e| AudioCaptureError::Stream(e.to_string()))?;
        match ready_rx.recv() {
            Ok(Ok(())) => Ok(Self {
                stop,
                thread: Some(thread),
            }),
            Ok(Err(e)) => {
                let _ = thread.join();
                Err(e)
            }
            Err(_) => {
                let _ = thread.join();
                Err(AudioCaptureError::Stream("capture thread died".into()))
            }
        }
    }

    /// Stop capturing and join the thread.
    pub fn stop(self) {}
}

impl Drop for AudioCapture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

fn default_name(host: &cpal::Host) -> Option<String> {
    host.default_output_device().and_then(|d| d.name().ok())
}

fn run(
    mut on_frame: impl FnMut(Vec<u8>) + Send + 'static,
    on_error: impl FnOnce(AudioCaptureError),
    stop: Arc<AtomicBool>,
    ready: mpsc::Sender<Result<(), AudioCaptureError>>,
) {
    let failure: Arc<Mutex<Option<AudioCaptureError>>> = Arc::new(Mutex::new(None));
    let setup = || -> Result<(cpal::Stream, Option<String>), AudioCaptureError> {
        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .ok_or(AudioCaptureError::NoDevice)?;
        let name = device.name().ok();
        let cfg = device
            .default_output_config()
            .map_err(|e| AudioCaptureError::Stream(e.to_string()))?;
        let format = cfg.sample_format();
        let config: StreamConfig = cfg.into();
        let mut fb = FrameBuilder::new(config.sample_rate.0, usize::from(config.channels));
        let fail = failure.clone();
        let err_cb = move |e: StreamError| {
            if let Ok(mut f) = fail.lock() {
                f.get_or_insert(map_stream_error(&e));
            }
        };
        let stream = match format {
            SampleFormat::F32 => device.build_input_stream(
                &config,
                move |d: &[f32], _| fb.push_f32(d, &mut on_frame),
                err_cb,
                None,
            ),
            SampleFormat::I16 => device.build_input_stream(
                &config,
                move |d: &[i16], _| fb.push_i16(d, &mut on_frame),
                err_cb,
                None,
            ),
            other => return Err(AudioCaptureError::FormatUnsupported(format!("{other:?}"))),
        }
        .map_err(|e| AudioCaptureError::Stream(e.to_string()))?;
        stream
            .play()
            .map_err(|e| AudioCaptureError::Stream(e.to_string()))?;
        Ok((stream, name))
    };
    let (stream, name) = match setup() {
        Ok(v) => {
            let _ = ready.send(Ok(()));
            v
        }
        Err(e) => {
            let _ = ready.send(Err(e));
            return;
        }
    };

    let host = cpal::default_host();
    let mut last_check = Instant::now();
    let ended = loop {
        if stop.load(Ordering::SeqCst) {
            break None;
        }
        if let Some(e) = failure.lock().ok().and_then(|mut f| f.take()) {
            break Some(e);
        }
        if last_check.elapsed() >= DEVICE_CHECK {
            last_check = Instant::now();
            match default_name(&host) {
                None => break Some(AudioCaptureError::DeviceRemoved),
                Some(n) if Some(&n) != name.as_ref() => {
                    break Some(AudioCaptureError::DefaultDeviceChanged)
                }
                _ => {}
            }
        }
        std::thread::sleep(POLL);
    };
    drop(stream);
    if let Some(e) = ended {
        on_error(e);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    #[test]
    fn device_unavailable_maps_to_device_removed() {
        assert_eq!(
            map_stream_error(&StreamError::DeviceNotAvailable),
            AudioCaptureError::DeviceRemoved
        );
        assert!(matches!(
            map_stream_error(&StreamError::BackendSpecific {
                err: cpal::BackendSpecificError {
                    description: "x".into()
                }
            }),
            AudioCaptureError::Stream(_)
        ));
    }

    #[test]
    fn errors_have_readable_messages() {
        assert!(AudioCaptureError::NoDevice
            .to_string()
            .contains("no default"));
        assert!(AudioCaptureError::FormatUnsupported("U8".into())
            .to_string()
            .contains("unsupported sample format"));
    }

    /// Reads the system output only; plays nothing. Needs a default output
    /// device and something playing (WASAPI sends no packets in silence).
    #[test]
    #[ignore = "needs an audio device with sound playing; run with --ignored"]
    fn loopback_delivers_20ms_frames_and_stops_cleanly() {
        let frames = Arc::new(AtomicUsize::new(0));
        let bad = Arc::new(AtomicUsize::new(0));
        let (f, b) = (frames.clone(), bad.clone());
        let cap = AudioCapture::start(
            move |x| {
                if x.len() != crate::audio_conv::FRAME_BYTES {
                    b.fetch_add(1, Ordering::SeqCst);
                }
                f.fetch_add(1, Ordering::SeqCst);
            },
            |e| panic!("capture ended: {e}"),
        )
        .expect("start loopback");
        std::thread::sleep(Duration::from_secs(2));
        let t = Instant::now();
        cap.stop();
        assert!(t.elapsed() < Duration::from_secs(1), "stop joins promptly");
        let n = frames.load(Ordering::SeqCst);
        println!("loopback frames in 2 s: {n}");
        assert!(n > 0, "no frames arrived (is anything playing?)");
        assert_eq!(bad.load(Ordering::SeqCst), 0);
    }
}
