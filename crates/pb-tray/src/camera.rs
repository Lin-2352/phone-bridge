//! The phone camera on the PC: records from the phone are decoded on the GPU,
//! fitted into the fixed 16:9 NV12 frame, and written into the frames file
//! that the media source (inside Windows' Frame Server) reads. The virtual
//! camera device exists only while a session runs (or for the 30 s the tray
//! waits to reconnect).
#![allow(unsafe_code)]

use std::{
    sync::{
        atomic::{AtomicBool, AtomicU32, Ordering},
        mpsc, Arc, Mutex,
    },
    time::Duration,
};

use pb_camera_frames::Ring;
use pb_camera_source::{file::Mapped, pump::TYPES};
use pb_transport::camera::{CameraRemote, CameraView};
use pb_video::{
    record::{Config, Record, FLAG_CONFIG},
    Codec,
};
use windows::Win32::Foundation::HWND;

use crate::{
    camera_device::{CameraDevice, MfVirtualCamera},
    camera_fit::Fitter,
    camera_register::frames_path,
    screen_decode::Decoder,
    screen_render::device_for_window,
};

pub struct CameraOpts {
    pub log: Box<dyn Fn(&str) + Send + Sync>,
    /// The session ended on its own (not by `stop`): why, for the user.
    pub on_ended: Box<dyn Fn(String) + Send + Sync>,
}

struct Active {
    remote: Arc<CameraRemote>,
    stop: Arc<AtomicBool>,
}

static ACTIVE: Mutex<Option<Active>> = Mutex::new(None);
static DEVICE: Mutex<Option<CameraDevice<MfVirtualCamera>>> = Mutex::new(None);
/// Frames per second written into the frames file (0 when idle).
static FPS: AtomicU32 = AtomicU32::new(0);

pub fn fps() -> u32 {
    FPS.load(Ordering::Relaxed)
}

/// The picture being written is (nearly) black: the phone's lens is covered
/// or the camera delivers nothing but black.
static DARK: AtomicBool = AtomicBool::new(false);

/// The phone has been asked for its camera and has not answered yet.
static PENDING: AtomicBool = AtomicBool::new(false);

pub fn pending() -> bool {
    PENDING.load(Ordering::Acquire)
}

/// Marks a request as open for as long as it lives, so every way out
/// (answer, refusal, timeout, cancel) clears it.
pub struct PendingGuard(());

impl PendingGuard {
    pub fn new() -> Self {
        PENDING.store(true, Ordering::Release);
        Self(())
    }
}

impl Drop for PendingGuard {
    fn drop(&mut self) {
        PENDING.store(false, Ordering::Release);
    }
}

/// Why the last start failed (cleared when a start works).
static LAST_ERROR: Mutex<Option<String>> = Mutex::new(None);

pub fn last_error() -> Option<String> {
    lock(&LAST_ERROR).clone()
}

pub fn set_last_error(e: Option<String>) {
    *lock(&LAST_ERROR) = e;
}

pub fn picture_dark() -> bool {
    DARK.load(Ordering::Relaxed)
}

/// The lens in use (0 back, 1 front), for "Switch lens".
static LENS: AtomicU32 = AtomicU32::new(0);

fn lock<T>(m: &'static Mutex<T>) -> std::sync::MutexGuard<'static, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

/// The clock the source reads: 100 ns units since 1601 (MFGetSystemTime),
/// in milliseconds.
fn now_ms() -> u64 {
    let unix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64);
    unix + 11_644_473_600_000
}

/// Forget any picture a killed tray left in the frames file.
pub fn clear_frames() {
    if let Ok(mut m) = Mapped::open(&frames_path(), true) {
        if let Some(mut ring) = Ring::open(m.words()) {
            ring.zero();
        }
    }
}

/// A session or the reconnect wait is under way (the device exists).
pub fn is_running() -> bool {
    lock(&ACTIVE).is_some() || lock(&DEVICE).is_some()
}

/// The user asked for the other lens.
pub fn switch_lens() {
    if let Some(a) = lock(&ACTIVE).as_ref() {
        let next = 1 - LENS.load(Ordering::Relaxed).min(1);
        LENS.store(next, Ordering::Relaxed);
        a.remote.switch_lens(next);
    }
}

pub fn lens() -> u32 {
    LENS.load(Ordering::Relaxed)
}

pub fn set_lens(lens: u32) {
    LENS.store(lens.min(1), Ordering::Relaxed);
}

/// End everything now: the session, then the virtual camera (Stop, Remove,
/// Shutdown). Safe to call twice.
pub fn stop() {
    if let Some(a) = lock(&ACTIVE).take() {
        a.stop.store(true, Ordering::Release);
        a.remote.close();
    }
    // Dropping the device runs its teardown.
    drop(lock(&DEVICE).take());
}

/// Begin a session whose records arrive in `view`. The virtual camera is
/// created on the first session and kept across reconnects.
pub fn start(
    view: CameraView,
    codec: Codec,
    opts: CameraOpts,
    rt: &tokio::runtime::Handle,
) -> Result<(), String> {
    let r = start_session(view, codec, opts, rt);
    set_last_error(r.clone().err());
    r
}

fn start_session(
    view: CameraView,
    codec: Codec,
    opts: CameraOpts,
    rt: &tokio::runtime::Handle,
) -> Result<(), String> {
    {
        let mut dev = lock(&DEVICE);
        if dev.is_none() {
            let cam = MfVirtualCamera::spawn()?;
            *dev = Some(CameraDevice::open(cam)?);
            (opts.log)("CAMERA virtual camera started");
        }
    }
    let (remote, mut records) = {
        let (remote, records) = view.into_parts();
        (Arc::new(remote), records)
    };
    let stop = Arc::new(AtomicBool::new(false));
    *lock(&ACTIVE) = Some(Active {
        remote: remote.clone(),
        stop: stop.clone(),
    });
    let (tx, rx) = mpsc::channel::<Record>();
    rt.spawn(async move {
        while let Some(r) = records.recv().await {
            if tx.send(r).is_err() {
                return;
            }
        }
    });
    let opts = Arc::new(opts);
    std::thread::spawn(move || {
        let result = pipe(&remote, &rx, &stop, codec, &opts);
        let user_stopped = stop.load(Ordering::Acquire);
        // Only clear what is still ours (a reconnect may have replaced it).
        {
            let mut a = lock(&ACTIVE);
            if a.as_ref().is_some_and(|x| Arc::ptr_eq(&x.stop, &stop)) {
                *a = None;
            }
        }
        if !user_stopped {
            (opts.on_ended)(match result {
                Ok(()) => "Phone camera stopped".into(),
                Err(e) => {
                    (opts.log)(&format!("CAMERA pipeline failed: {e}"));
                    set_last_error(Some(e));
                    "Can't show this phone's camera on this PC".into()
                }
            });
        }
    });
    Ok(())
}

/// Decode, fit, write: until the session ends.
fn pipe(
    remote: &CameraRemote,
    rx: &mpsc::Receiver<Record>,
    stop: &AtomicBool,
    codec: Codec,
    opts: &CameraOpts,
) -> Result<(), String> {
    let device = device_for_window(HWND::default())?;
    let mut decoder = Decoder::new(&device, codec)?;
    let mut fitter = Fitter::new(&device)?;
    let mut mapped = Mapped::open(&frames_path(), true)
        .map_err(|e| format!("the camera is not installed ({e})"))?;
    let mut ring = Ring::open(mapped.words()).ok_or("the frames file is not formatted")?;
    ring.zero();
    ring.beat(now_ms());
    let (mut video, mut rotation) = (
        (remote.accept.width, remote.accept.height),
        remote.accept.rotation,
    );
    let mut out_buf = Vec::new();
    let mut last_beat = std::time::Instant::now();
    let mut frames = 0u32;
    let mut since = std::time::Instant::now();
    let mut last_level = std::time::Instant::now();
    loop {
        if stop.load(Ordering::Acquire) {
            break;
        }
        match rx.recv_timeout(Duration::from_secs(1)) {
            Ok(r) => {
                if r.flags & FLAG_CONFIG != 0 {
                    if let Some(c) = Config::decode(&r.payload) {
                        video = (u32::from(c.width), u32::from(c.height));
                        rotation = u32::from(c.rotation);
                    }
                }
                let Some(f) = decoder.push(&r)? else {
                    continue;
                };
                let wanted = {
                    let w = ring.wanted();
                    if TYPES.contains(&w) {
                        w
                    } else {
                        TYPES[0]
                    }
                };
                let n = (wanted.0 * wanted.1 * 3 / 2) as usize;
                out_buf.resize(n, 0);
                fitter.fit_into(&f.texture, f.index, video, rotation, wanted, &mut out_buf)?;
                ring.write_frame(wanted.0, wanted.1, f.pts_us.max(0) as u64, |slot| {
                    slot[..n].copy_from_slice(&out_buf[..n]);
                });
                remote.feedback(f.pts_us);
                frames += 1;
                if last_level.elapsed() >= Duration::from_secs(1) {
                    let mean =
                        pb_camera_frames::mean_luma(&out_buf, wanted.0 as usize, wanted.1 as usize);
                    DARK.store(pb_camera_frames::is_dark(mean), Ordering::Relaxed);
                    (opts.log)(&format!("CAMERA picture level {mean}"));
                    last_level = std::time::Instant::now();
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
        if last_beat.elapsed() >= Duration::from_secs(1) {
            ring.beat(now_ms());
            last_beat = std::time::Instant::now();
        }
        if since.elapsed() >= Duration::from_secs(2) {
            let rate = frames * 1000 / since.elapsed().as_millis().max(1) as u32;
            FPS.store(rate, Ordering::Relaxed);
            (opts.log)(&format!("CAMERA stats {rate}/s written"));
            frames = 0;
            since = std::time::Instant::now();
        }
    }
    // Nothing of the picture stays in the file.
    ring.zero();
    FPS.store(0, Ordering::Relaxed);
    DARK.store(false, Ordering::Relaxed);
    Ok(())
}
