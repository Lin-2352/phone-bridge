//! Opens "Phone Bridge Camera" the way an app does (Media Foundation source
//! reader through Windows' Frame Server) and keeps the latest small picture
//! and the frames per second it measured. If this works, apps can see the
//! camera; if it does not, the window says so.
#![allow(unsafe_code)]

use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

use windows::Win32::{
    Media::MediaFoundation::{
        IMFActivate, IMFMediaSource, MFCreateAttributes, MFCreateMediaType,
        MFCreateSourceReaderFromMediaSource, MFMediaType_Video, MFShutdown, MFStartup,
        MFVideoFormat_RGB32, MFSTARTUP_FULL, MF_MT_DEFAULT_STRIDE, MF_MT_FRAME_SIZE,
        MF_MT_MAJOR_TYPE, MF_MT_SUBTYPE, MF_SOURCE_READER_ENABLE_VIDEO_PROCESSING,
        MF_SOURCE_READER_FIRST_VIDEO_STREAM, MF_VERSION,
    },
    System::Com::{CoInitializeEx, CoUninitialize, COINIT_MULTITHREADED},
};

use crate::camera_device::{enumerate_cameras, is_our_camera};

/// The preview is at most this wide (the camera sends 1920).
const PREVIEW_W: usize = 480;

/// The newest picture (top-down BGRA) and what has been measured so far.
#[derive(Default, Clone)]
pub struct Snapshot {
    pub width: u32,
    pub height: u32,
    pub bgra: Vec<u8>,
    /// `None` until a second has passed.
    pub fps: Option<u32>,
}

pub struct Preview {
    stop: Arc<AtomicBool>,
    shared: Arc<Mutex<Snapshot>>,
    thread: Option<JoinHandle<()>>,
}

impl Preview {
    /// Start reading. `on_update` runs after every new picture (to repaint).
    pub fn start(on_update: Box<dyn Fn() + Send>) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let shared = Arc::new(Mutex::new(Snapshot::default()));
        let (s, sh) = (stop.clone(), shared.clone());
        let thread = std::thread::spawn(move || {
            // SAFETY: COM and Media Foundation on this worker thread, undone below.
            unsafe {
                let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
                if MFStartup(MF_VERSION, MFSTARTUP_FULL).is_ok() {
                    let r = read_loop(&s, &sh, &*on_update);
                    if r.is_err() {
                        set_fps(&sh, 0);
                        on_update();
                    }
                    let _ = MFShutdown();
                }
                CoUninitialize();
            }
        });
        Self {
            stop,
            shared,
            thread: Some(thread),
        }
    }

    pub fn snapshot(&self) -> Snapshot {
        self.shared.lock().map(|g| g.clone()).unwrap_or_default()
    }

    /// Stop reading: the camera is closed again (Frame Server stops the source).
    pub fn stop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for Preview {
    fn drop(&mut self) {
        self.stop();
    }
}

fn set_fps(shared: &Mutex<Snapshot>, fps: u32) {
    if let Ok(mut g) = shared.lock() {
        g.fps = Some(fps);
    }
}

/// Open the camera by name and read until stopped. Err: it could not be
/// opened or read (the window shows "listed, but no pictures").
unsafe fn read_loop(
    stop: &AtomicBool,
    shared: &Mutex<Snapshot>,
    on_update: &dyn Fn(),
) -> Result<(), String> {
    let act: IMFActivate = enumerate_cameras()
        .into_iter()
        .find(|(n, _)| is_our_camera(n))
        .map(|(_, a)| a)
        .ok_or("not listed")?;
    let source: IMFMediaSource = act.ActivateObject().map_err(|e| format!("open: {e}"))?;
    let mut attrs = None;
    MFCreateAttributes(&mut attrs, 1).map_err(|e| e.to_string())?;
    let attrs = attrs.ok_or("no attributes")?;
    attrs
        .SetUINT32(&MF_SOURCE_READER_ENABLE_VIDEO_PROCESSING, 1)
        .map_err(|e| e.to_string())?;
    let reader =
        MFCreateSourceReaderFromMediaSource(&source, &attrs).map_err(|e| format!("reader: {e}"))?;
    let stream = MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32;
    let rgb = MFCreateMediaType().map_err(|e| e.to_string())?;
    rgb.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)
        .map_err(|e| e.to_string())?;
    rgb.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_RGB32)
        .map_err(|e| e.to_string())?;
    reader
        .SetCurrentMediaType(stream, None, &rgb)
        .map_err(|e| format!("rgb: {e}"))?;
    let cur = reader
        .GetCurrentMediaType(stream)
        .map_err(|e| e.to_string())?;
    let size = cur
        .GetUINT64(&MF_MT_FRAME_SIZE)
        .map_err(|e| e.to_string())?;
    let (w, h) = ((size >> 32) as usize, size as u32 as usize);
    // RGB32 is usually bottom-up (negative stride): flip while copying.
    let stride = cur.GetUINT32(&MF_MT_DEFAULT_STRIDE).unwrap_or(0) as i32;
    let bottom_up = stride < 0;
    let step = (w / PREVIEW_W).max(1);
    let (pw, ph) = (w / step, h / step);
    let mut frames = 0u32;
    let mut since = Instant::now();
    while !stop.load(Ordering::Acquire) {
        let mut flags = 0u32;
        let mut sample = None;
        reader
            .ReadSample(stream, 0, None, Some(&mut flags), None, Some(&mut sample))
            .map_err(|e| format!("read: {e}"))?;
        if flags & 3 != 0 {
            return Err("the camera ended".into());
        }
        let Some(sample) = sample else { continue };
        let buf = sample
            .ConvertToContiguousBuffer()
            .map_err(|e| e.to_string())?;
        let (mut ptr, mut len) = (std::ptr::null_mut(), 0u32);
        buf.Lock(&mut ptr, None, Some(&mut len))
            .map_err(|e| e.to_string())?;
        if len as usize >= w * h * 4 {
            let src = std::slice::from_raw_parts(ptr, w * h * 4);
            let mut out = vec![0u8; pw * ph * 4];
            for y in 0..ph {
                let sy = if bottom_up {
                    h - 1 - y * step
                } else {
                    y * step
                };
                for x in 0..pw {
                    let so = (sy * w + x * step) * 4;
                    let d = (y * pw + x) * 4;
                    out[d..d + 4].copy_from_slice(&src[so..so + 4]);
                }
            }
            if let Ok(mut g) = shared.lock() {
                g.width = pw as u32;
                g.height = ph as u32;
                g.bgra = out;
            }
        }
        let _ = buf.Unlock();
        frames += 1;
        if since.elapsed() >= Duration::from_secs(1) {
            set_fps(
                shared,
                (u64::from(frames) * 1000 / since.elapsed().as_millis().max(1) as u64) as u32,
            );
            frames = 0;
            since = Instant::now();
        }
        on_update();
    }
    let _ = source.Shutdown();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Needs the camera streaming from the phone (`pb-tray --start-camera`):
    /// run by hand with `cargo test -p pb-tray camera_preview -- --ignored`.
    #[test]
    #[ignore]
    fn the_virtual_camera_gives_pictures_like_an_app_would() {
        let mut p = Preview::start(Box::new(|| {}));
        std::thread::sleep(Duration::from_secs(4));
        let s = p.snapshot();
        p.stop();
        assert!(s.fps.is_some_and(|f| f >= 10), "fps {:?}", s.fps);
        assert!(s.width > 0 && !s.bgra.is_empty());
    }
}
