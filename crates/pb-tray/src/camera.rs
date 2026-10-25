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
/// A count, not a flag: a Cancel followed at once by a new Start must not be
/// cleared by the first request unwinding.
static PENDING: AtomicU32 = AtomicU32::new(0);

pub fn pending() -> bool {
    PENDING.load(Ordering::Acquire) > 0
}

/// Marks a request as open for as long as it lives, so every way out
/// (answer, refusal, timeout, cancel) clears it.
pub struct PendingGuard(());

impl PendingGuard {
    pub fn new() -> Self {
        PENDING.fetch_add(1, Ordering::AcqRel);
        Self(())
    }
}

impl Drop for PendingGuard {
    fn drop(&mut self) {
        PENDING.fetch_sub(1, Ordering::AcqRel);
    }
}

static INSTALLING: AtomicBool = AtomicBool::new(false);

/// Held while the elevated installer runs, so Repair and a first start
/// cannot raise two permission prompts at once.
pub struct InstallGuard(());

impl InstallGuard {
    pub fn try_new() -> Option<Self> {
        (!INSTALLING.swap(true, Ordering::AcqRel)).then_some(Self(()))
    }
}

impl Drop for InstallGuard {
    fn drop(&mut self) {
        INSTALLING.store(false, Ordering::Release);
    }
}

/// Why the last start failed (cleared when a start works).
static LAST_ERROR: Mutex<Option<String>> = Mutex::new(None);

pub fn last_error() -> Option<String> {
    lock(&LAST_ERROR).clone()
}

/// The last error came from decoding or drawing the picture, not from setup.
static PLAYBACK_ERROR: AtomicBool = AtomicBool::new(false);

pub fn playback_error() -> bool {
    PLAYBACK_ERROR.load(Ordering::Acquire)
}

pub fn set_last_error(e: Option<String>) {
    set_last_error_kind(e, false);
}

/// The error and its kind change together (under the one lock), so a setup
/// error written from another task is never flagged as a playback one.
pub fn set_last_error_kind(e: Option<String>, playback: bool) {
    let mut last = lock(&LAST_ERROR);
    PLAYBACK_ERROR.store(playback, Ordering::Release);
    *last = e;
}

/// A pipeline error that Repair cannot fix: not the frames file the
/// installer makes (missing, unreadable or wrongly formatted).
fn is_playback_error(e: &str) -> bool {
    !(e.starts_with("the camera is not installed") || e.starts_with("the frames file"))
}

/// Run `f` (a blocking install) while holding `guard`: the guard goes when
/// the job ends, not when the task that started it is aborted.
pub fn holding<T>(guard: InstallGuard, f: impl FnOnce() -> T) -> T {
    let _guard = guard;
    f()
}

pub fn picture_dark() -> bool {
    DARK.load(Ordering::Relaxed)
}

/// The lens in use (0 back, 1 front), as the phone last reported it (or as
/// asked since): "Switch lens" goes to the other one, a new request asks for it.
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
        let next = other_lens();
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

/// Where "Switch lens" goes from the lens in use.
fn other_lens() -> u32 {
    1 - lens().min(1)
}

/// Follow the lens the phone reports (a switch from its notification, or
/// one the PC asked for) until the session ends.
async fn follow_lens(mut updates: tokio::sync::watch::Receiver<Option<u8>>) {
    while updates.changed().await.is_ok() {
        let reported = *updates.borrow_and_update();
        if let Some(l) = reported {
            set_lens(l.into());
        }
    }
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
    rt.spawn(follow_lens(remote.lens_updates()));
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
                    let playback = is_playback_error(&e);
                    set_last_error_kind(Some(e), playback);
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

#[cfg(test)]
mod tests {
    use super::*;

    async fn until(want: u32) {
        let t0 = std::time::Instant::now();
        while lens() != want && t0.elapsed() < Duration::from_secs(5) {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }

    #[test]
    fn setup_errors_in_the_pipeline_are_not_playback_errors() {
        // Repair can fix these (frames file unreadable or wrong); it cannot fix a decoder.
        assert!(!is_playback_error(
            "the camera is not installed (access denied)"
        ));
        assert!(!is_playback_error("the frames file is not formatted"));
        assert!(is_playback_error("decoder: device removed"));
    }

    /// The only test that touches `LAST_ERROR` and `PLAYBACK_ERROR`.
    #[test]
    fn the_error_and_its_kind_are_set_together() {
        set_last_error_kind(Some("decoder: device removed".into()), true);
        assert!(playback_error());
        set_last_error(Some("could not install".into()));
        assert!(!playback_error(), "a setup error replaces the kind too");
        assert_eq!(last_error().as_deref(), Some("could not install"));
        set_last_error(None);
    }

    #[test]
    fn the_install_guard_is_held_while_the_job_runs() {
        let g = InstallGuard::try_new().expect("free");
        let seen = holding(g, || InstallGuard::try_new().is_none());
        assert!(seen, "still held inside the job");
        assert!(
            InstallGuard::try_new().is_some(),
            "released when the job ends"
        );
    }

    /// The only test that touches `INSTALLING` (a process-wide static).
    #[test]
    fn a_second_install_is_refused_while_one_runs() {
        let first = InstallGuard::try_new().expect("nothing is installing");
        assert!(
            InstallGuard::try_new().is_none(),
            "one UAC prompt at a time"
        );
        drop(first);
        assert!(InstallGuard::try_new().is_some(), "free again afterwards");
    }

    /// The only test that touches `PENDING` (a process-wide static).
    #[test]
    fn a_quick_cancel_then_start_keeps_pending() {
        assert!(!pending());
        let first = PendingGuard::new();
        assert!(pending());
        // Cancel then Start again before the first request has unwound.
        let second = PendingGuard::new();
        drop(first);
        assert!(pending(), "the new request is still open");
        drop(second);
        assert!(!pending(), "pending clears when all guards are dropped");
    }

    /// The only test that touches `LENS` (a process-wide static).
    #[tokio::test]
    async fn lens_follows_the_phone() {
        // A fake session: the phone's lens reports, as the transport hands them.
        let (phone, updates) = tokio::sync::watch::channel(None);
        set_lens(0);
        let follower = tokio::spawn(follow_lens(updates));
        // Switched on the phone (its notification button): the PC follows.
        phone.send_replace(Some(1));
        until(1).await;
        assert_eq!(lens(), 1, "the front lens the phone reported");
        // "Switch lens" toggles from there, and a new request asks for it.
        assert_eq!(other_lens(), 0);
        phone.send_replace(Some(0));
        until(0).await;
        assert_eq!(lens(), 0);
        assert_eq!(other_lens(), 1);
        // The session ends: the follower stops, the last lens is kept.
        drop(phone);
        tokio::time::timeout(Duration::from_secs(5), follower)
            .await
            .expect("the follower ends with the session")
            .unwrap();
        assert_eq!(lens(), 0);
    }
}
