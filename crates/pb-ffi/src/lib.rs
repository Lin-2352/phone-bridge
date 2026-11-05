//! UniFFI surface for the Android app: start the node, join a pairing code,
//! confirm the SAS, send clipboard text; events are pushed to a Kotlin listener.

use std::sync::{Arc, Mutex};

use pb_identity::FilePeerStore;
use pb_protocol::control::{
    ClipboardText, ControlMsg, PairingCode, MAX_CLIPBOARD_IMAGE, MAX_CLIPBOARD_TEXT,
};
use pb_transport::{Joining, Node, NodeConfig, Session, SharedStore};

mod applog;

uniffi::setup_scaffolding!();

mod audio;
pub use audio::{AudioSink, PbAudioEnd, PbAudioOut};

#[derive(Debug, uniffi::Error)]
pub enum PbError {
    Failed { reason: String },
}

impl std::fmt::Display for PbError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let Self::Failed { reason } = self;
        f.write_str(reason)
    }
}

impl std::error::Error for PbError {}

impl From<uniffi::UnexpectedUniFFICallbackError> for PbError {
    fn from(e: uniffi::UnexpectedUniFFICallbackError) -> Self {
        fail(e.reason)
    }
}

/// Implemented in Kotlin with an Android Keystore AES-GCM key, so the identity
/// seed is never stored in clear.
#[uniffi::export(with_foreign)]
pub trait KeyWrapper: Send + Sync {
    fn wrap(&self, plain: Vec<u8>) -> Result<Vec<u8>, PbError>;
    fn unwrap(&self, sealed: Vec<u8>) -> Result<Vec<u8>, PbError>;
}

struct ForeignWrapper(Arc<dyn KeyWrapper>);

impl pb_identity::vault::SeedWrapper for ForeignWrapper {
    fn wrap(&self, plain: &[u8]) -> std::io::Result<Vec<u8>> {
        self.0.wrap(plain.to_vec()).map_err(std::io::Error::other)
    }
    fn unwrap(&self, sealed: &[u8]) -> std::io::Result<Vec<u8>> {
        self.0
            .unwrap(sealed.to_vec())
            .map_err(std::io::Error::other)
    }
}

fn fail(e: impl std::fmt::Display) -> PbError {
    PbError::Failed {
        reason: e.to_string(),
    }
}

/// Something the UI should show.
#[derive(uniffi::Enum)]
pub enum PbEvent {
    Connected {
        peer_name: String,
    },
    Disconnected {
        peer_name: String,
        reason: String,
    },
    Clipboard {
        peer_name: String,
        text: String,
        sensitive: bool,
    },
    /// Universal Clipboard: text copied on the PC (only when sync is on).
    RemoteClipboard {
        peer_name: String,
        text: String,
        /// Marked private on the PC: keep it out of history, hide previews.
        sensitive: bool,
    },
    /// Universal Clipboard: a picture copied on the PC (only when image
    /// sharing is on). `path` is a PNG in the app's private incoming folder;
    /// Kotlin puts it on the clipboard and owns the file from here.
    RemoteImage {
        peer_name: String,
        path: String,
    },
    /// A file finished arriving; Kotlin moves it into Downloads/Phone Bridge.
    FileReceived {
        peer_name: String,
        path: String,
        name: String,
        size: u64,
    },
    /// A link or text shared from the PC.
    SharedText {
        peer_name: String,
        text: String,
        is_link: bool,
    },
    /// Asks the UI to accept or decline (only when auto-accept is off);
    /// answer with `PbNode::answer_offer(id, ..)`.
    IncomingOffer {
        id: u64,
        peer_name: String,
        name: String,
        size: u64,
    },
    TransferFailed {
        peer_name: String,
        reason: String,
    },
    /// A paired PC asks to control this phone and "allow control" is off:
    /// ask the user, then answer with `PbNode::answer_offer(id, ..)`.
    ControlRequest {
        id: u64,
        peer_name: String,
    },
    /// A paired PC asks to see this screen. Ask the user (unless `resume`:
    /// the capture from the same PC is still alive, answer at once), run
    /// Android's capture prompt, then call `PbNode::answer_screen(id, ..)`.
    ScreenRequest {
        id: u64,
        peer_name: String,
        /// Long side of the video; 0 = native.
        max_long_side: u32,
        sharp: bool,
        dim: bool,
        /// Codecs the PC decodes, best first (1 H.265 Main10, 2 H.265, 3 H.264).
        codecs: Vec<u8>,
        resume: bool,
    },
    /// A paired PC asks to use this phone's camera. Ask the user (unless
    /// `resume`), make sure the camera permission is held, open the camera,
    /// then call `PbNode::answer_camera(id, ..)`.
    CameraRequest {
        id: u64,
        peer_name: String,
        /// 0 back, 1 front.
        lens: u32,
        /// The 16:9 size wanted (1920x1080 or 1280x720).
        width: u32,
        height: u32,
        /// Codecs the PC decodes, best first (1 H.265 Main10, 2 H.265, 3 H.264).
        codecs: Vec<u8>,
        resume: bool,
    },
    /// A paired PC asks for this phone's audio (phone to PC). Ask the user
    /// (playback: Relay's own dialog unless the skip setting is on, then
    /// Android's projection dialog; microphone: always Relay's dialog and
    /// the RECORD_AUDIO grant), then call `PbNode::answer_audio_accept` with
    /// the format that will be sent, or `answer_audio_refuse`.
    AudioRequest {
        id: u64,
        peer_name: String,
        /// 0 playback (system audio), 1 microphone.
        source: u8,
        sample_rate: u32,
        channels: u32,
    },
    /// The question `id` is over without an answer from the app (the PC gave
    /// up, the link dropped, the time ran out or `stop_audio(0)`): close the dialog.
    AudioAskEnded {
        id: u64,
    },
    /// The PC has the stream. `direction` 0 phone to PC (start sending),
    /// 1 PC to phone (frames arrive on the `AudioSink`; start the player).
    AudioStarted {
        peer_name: String,
        direction: u8,
        source: u8,
        sample_rate: u32,
        channels: u32,
    },
    /// An audio session ended (also after a failed start of an accepted one).
    AudioEnded {
        direction: u8,
        cause: PbAudioEnd,
    },
    /// The phone itself refused a request without asking: `reason` is the wire
    /// code (2 unavailable, 3 busy, 4 bad parameters).
    AudioRefused {
        peer_name: String,
        direction: u8,
        source: u8,
        reason: u8,
    },
    /// A file transfer is under way (at most every 0.7 s, and once at the end).
    TransferProgress {
        peer_name: String,
        name: String,
        done: u64,
        total: u64,
        incoming: bool,
    },
    /// The question `id` (`IncomingOffer`, `ControlRequest`, `ScreenRequest`
    /// or `CameraRequest`) is over without an answer from the app: the PC
    /// gave up or went away, or the time ran out. Close its notification and
    /// window; an answer now would do nothing. (Audio has `AudioAskEnded`.)
    AskEnded {
        id: u64,
    },
}

/// Cancel handles of the transfers running now, by direction (true = incoming): the phone shows
/// one progress slot per direction, so its Cancel stops everything running that way.
static CANCELS: Mutex<Vec<(u64, bool, pb_transport::files::Cancel)>> = Mutex::new(Vec::new());
static CANCEL_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// A cancel handle for one transfer, registered until dropped.
struct CancelGuard {
    id: u64,
    cancel: pb_transport::files::Cancel,
}

impl CancelGuard {
    fn new(incoming: bool) -> Self {
        let cancel = pb_transport::files::Cancel::new();
        let id = CANCEL_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        CANCELS
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .push((id, incoming, cancel.clone()));
        Self { id, cancel }
    }
}

impl Drop for CancelGuard {
    fn drop(&mut self) {
        CANCELS
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .retain(|(id, _, _)| *id != self.id);
    }
}

/// Progress callback for a transfer, rate-limited so the UI is not flooded.
fn progress_events(
    events: Events,
    peer_name: String,
    name: String,
    incoming: bool,
) -> pb_transport::files::Progress {
    let last = Mutex::new(None::<std::time::Instant>);
    Arc::new(move |done, total| {
        let due = {
            let mut l = last.lock().unwrap_or_else(|p| p.into_inner());
            let now = std::time::Instant::now();
            let due = done >= total
                || l.is_none_or(|t| now.duration_since(t) >= std::time::Duration::from_millis(700));
            if due {
                *l = Some(now);
            }
            due
        };
        if due {
            push(
                &events,
                PbEvent::TransferProgress {
                    peer_name: peer_name.clone(),
                    name: name.clone(),
                    done,
                    total,
                    incoming,
                },
            );
        }
    })
}

#[derive(uniffi::Record)]
pub struct PbPeer {
    pub id: String,
    pub name: String,
    pub fingerprint: String,
}

/// Implemented in Kotlin (the foreground service); called from Rust threads.
///
/// Every callback returns `Result`: if the Kotlin side throws, UniFFI turns
/// it into `PbError` instead of a panic. A unit return would panic, and with
/// `panic = "abort"` that kills the app.
#[uniffi::export(with_foreign)]
pub trait EventListener: Send + Sync {
    fn on_event(&self, event: PbEvent) -> Result<(), PbError>;
}

type Events = Arc<dyn EventListener>;

/// PC -> phone control: implemented by the Kotlin accessibility service,
/// which exists only while the user has it enabled. Called on a Rust thread.
#[uniffi::export(with_foreign)]
pub trait InputListener: Send + Sync {
    /// `kind`: 0 move (a=dx, b=dy), 1 button (a=button), 2 wheel (a=dx, b=dy),
    /// 3 key (a=scan code), 4 character (a=UTF-16 unit).
    fn on_input(&self, kind: u8, a: i32, b: i32, pressed: bool) -> Result<(), PbError>;
    /// Control started (`active`) or ended, with the controlling device.
    fn on_control(&self, active: bool, peer_name: String) -> Result<(), PbError>;
}

/// A phone notification handed over by the Android listener.
#[derive(uniffi::Record)]
pub struct PbNotification {
    pub key: String,
    pub package: String,
    pub app_name: String,
    pub title: String,
    pub text: String,
    pub when_ms: i64,
    pub conversation: String,
    pub lines: Vec<String>,
    pub quiet: bool,
    pub private: bool,
    pub call: bool,
    pub actions: Vec<PbNotifyActionDesc>,
    /// The app icon as PNG; only needed the first time per app.
    pub icon_png: Option<Vec<u8>>,
    pub picture_jpeg: Option<Vec<u8>>,
}

#[derive(uniffi::Record)]
pub struct PbNotifyActionDesc {
    pub label: String,
    pub reply: bool,
}

#[derive(uniffi::Enum)]
pub enum PbNotifyActionKind {
    Dismiss,
    Button,
    Reply,
    Open,
    Silence,
}

/// Runs what a PC asked for on a notification (Kotlin). An error's reason
/// is shown to the user on the PC.
#[uniffi::export(with_foreign)]
pub trait NotifyHandler: Send + Sync {
    fn on_action(
        &self,
        key: String,
        kind: PbNotifyActionKind,
        index: u32,
        text: String,
    ) -> Result<(), PbError>;
}

/// The wire form of a notification (one-time code filled in) and its icon.
fn to_wire(n: PbNotification) -> (pb_protocol::control::Notification, Option<Vec<u8>>) {
    let otp =
        pb_transport::notify::find_otp(&format!("{} {}", n.title, n.text)).unwrap_or_default();
    let wire = pb_protocol::control::Notification {
        key: n.key,
        package: n.package,
        app_name: n.app_name,
        title: n.title,
        text: n.text,
        when_ms: n.when_ms,
        conversation: n.conversation,
        lines: n.lines,
        quiet: n.quiet,
        private: n.private,
        call: n.call,
        otp,
        actions: n
            .actions
            .into_iter()
            .map(|a| pb_protocol::control::NotifyActionDesc {
                label: a.label,
                reply: a.reply,
            })
            .collect(),
        icon_hash: Vec::new(),
        picture: n.picture_jpeg.unwrap_or_default(),
    };
    (wire, n.icon_png)
}

/// Send → shown-on-the-PC round trips since the last call (ms).
#[derive(uniffi::Record)]
pub struct PbConfirmStats {
    pub samples: u32,
    pub p50: u32,
    pub p95: u32,
}

/// The phone's answer to a screen request.
#[derive(uniffi::Record)]
pub struct PbScreenAccept {
    pub codec: u8,
    pub width: u32,
    pub height: u32,
    pub rotation: u16,
}

/// The phone's answer to a camera request.
#[derive(uniffi::Record)]
pub struct PbCameraAccept {
    pub codec: u8,
    pub width: u32,
    pub height: u32,
    pub rotation: u16,
    pub lens: u8,
}

/// What the encoder should do after a frame.
#[derive(uniffi::Record)]
pub struct PbAdvice {
    pub sent: bool,
    /// Ask the encoder for a key frame now.
    pub request_key: bool,
    /// New encoder bitrate (bits/s); 0 = unchanged.
    pub new_bitrate: u32,
}

fn advice(d: pb_video::Decision) -> PbAdvice {
    PbAdvice {
        sent: d.send,
        request_key: d.request_key,
        new_bitrate: d.new_bitrate.unwrap_or(0),
    }
}

/// The PC window's state and input during a screen session (Kotlin).
#[uniffi::export(with_foreign)]
pub trait ScreenListener: Send + Sync {
    fn on_state(&self, active: bool, dim: bool) -> Result<(), PbError>;
    /// The PC could not decode: switch to its next codec (`next_codec`).
    fn on_codec_failed(&self) -> Result<(), PbError>;
    /// phase: 0 hover, 1 down, 2 move, 3 up; x, y: 0..=65535.
    fn on_touch(&self, phase: u8, x: u16, y: u16) -> Result<(), PbError>;
    fn on_pinch(&self, out: bool, x: u16, y: u16) -> Result<(), PbError>;
    fn on_key(&self, scancode: u16, pressed: bool) -> Result<(), PbError>;
    fn on_char(&self, unit: u16) -> Result<(), PbError>;
    /// 1 back, 2 home, 4 recents.
    fn on_button(&self, button: u8) -> Result<(), PbError>;
    fn on_wheel(&self, dx: i16, dy: i16) -> Result<(), PbError>;
    /// The session ended. `resumable`: the link dropped; keep capturing
    /// (paused) for 30 s in case the same PC asks again.
    fn on_stopped(&self, resumable: bool) -> Result<(), PbError>;
}

/// The camera session's events for Kotlin.
#[uniffi::export(with_foreign)]
pub trait CameraListener: Send + Sync {
    /// The PC wants the other lens (0 back, 1 front).
    fn on_lens(&self, lens: u8) -> Result<(), PbError>;
    /// The session ended. `resumable`: the link was lost, keep the camera
    /// for 30 s; otherwise close it.
    fn on_stopped(&self, resumable: bool) -> Result<(), PbError>;
}

/// After a dropped link, a new request from the same PC within the grace
/// resumes without asking.
struct Grace {
    peer: pb_identity::PeerId,
    until: std::time::Instant,
}

impl Grace {
    fn new(peer: pb_identity::PeerId, now: std::time::Instant) -> Self {
        Self {
            peer,
            until: now + pb_transport::screen::RESUME_GRACE,
        }
    }

    fn admits(&self, peer: &pb_identity::PeerId, now: std::time::Instant) -> bool {
        self.peer == *peer && now < self.until
    }
}

/// Take the resume grace only if it belongs to this PC and hasn't run out.
fn take_grace(
    slot: &mut Option<Grace>,
    peer: &pb_identity::PeerId,
    now: std::time::Instant,
) -> bool {
    if slot.as_ref().is_some_and(|g| g.admits(peer, now)) {
        *slot = None;
        true
    } else {
        false
    }
}

/// The user pressed Stop while the link was down: nothing to resume, and
/// that PC's retries are declined quietly.
fn stop_during_grace(
    grace: &mut Option<Grace>,
    user_stop: &mut Option<Grace>,
    now: std::time::Instant,
) {
    if let Some(g) = grace.take() {
        *user_stop = Some(Grace::new(g.peer, now));
    }
}

/// A new request from the PC whose session is still running means that
/// session is stale: the PC restarted before the phone noticed the lost link.
fn supersedes(current: Option<&pb_identity::PeerId>, new: &pb_identity::PeerId) -> bool {
    current == Some(new)
}

/// Records kept while the session's streams open (config, first key frame).
const SCREEN_PENDING: usize = 32;

/// Send the records that waited for the streams; stop at the first refused
/// one (the frames after it refer to it). True if one was refused.
fn flush_pending(
    waiting: Vec<pb_video::Record>,
    mut send: impl FnMut(pb_video::Record) -> bool,
) -> bool {
    for r in waiting {
        if !send(r) {
            return true;
        }
    }
    false
}

/// What a phone-screen session leaves for its PC when it ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EndSlot {
    /// The user stopped it: that PC's requests are declined for a while.
    UserStop,
    /// The link dropped: that PC's next request resumes without asking.
    Grace,
}

/// `by_user`: Stop on the phone. `locked`: Android ended the projection as
/// the phone locked. `closed_by_pc`: the PC closed its window on purpose, so
/// its next request is a new share.
fn screen_end_record(by_user: bool, locked: bool, closed_by_pc: bool) -> Option<EndSlot> {
    if by_user {
        Some(EndSlot::UserStop)
    } else if locked || closed_by_pc {
        None
    } else {
        Some(EndSlot::Grace)
    }
}

/// One screen session, phone side: encoded frames go in here.
#[derive(uniffi::Object)]
pub struct PbScreen {
    sink: std::sync::OnceLock<pb_transport::screen::ScreenSink>,
    pending: Mutex<Vec<pb_video::Record>>,
    pacer: Mutex<pb_video::Pacer>,
    stopped: std::sync::atomic::AtomicBool,
    /// Android ended the projection because the phone locked.
    locked: std::sync::atomic::AtomicBool,
    /// A waiting frame was refused when the streams opened: the next
    /// advice asks the encoder for a key frame.
    want_key: std::sync::atomic::AtomicBool,
}

impl PbScreen {
    fn new(codec: u8) -> Self {
        let target = if codec == pb_video::Codec::H264 as u8 {
            16_000_000
        } else {
            12_000_000
        };
        Self {
            sink: std::sync::OnceLock::new(),
            pending: Mutex::new(Vec::new()),
            pacer: Mutex::new(pb_video::Pacer::new(target)),
            stopped: std::sync::atomic::AtomicBool::new(false),
            locked: std::sync::atomic::AtomicBool::new(false),
            want_key: std::sync::atomic::AtomicBool::new(false),
        }
    }

    /// The streams are open: send what waited.
    fn ready(&self, sink: pb_transport::screen::ScreenSink) {
        // Held until the sink is set: a frame sent meanwhile waits here and
        // goes out in order, never into a list nobody flushes again.
        let mut pending = self.pending.lock().unwrap_or_else(|p| p.into_inner());
        let refused = flush_pending(std::mem::take(&mut *pending), |r| {
            matches!(sink.send(r), pb_transport::screen::SendOutcome::Queued(_))
        });
        let _ = self.sink.set(sink);
        drop(pending);
        if refused {
            self.pacer
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .queue_full(std::time::Instant::now());
            self.want_key
                .store(true, std::sync::atomic::Ordering::Release);
        }
    }
}

#[uniffi::export]
impl PbScreen {
    /// An encoded unit (flags: 1 key, 2 codec config, 4 repeat).
    pub fn send_frame(&self, data: Vec<u8>, pts_us: i64, flags: u8) -> PbAdvice {
        let queued = self.sink.get().map_or(0, |s| s.queued_bytes());
        let d = self
            .pacer
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .on_frame(std::time::Instant::now(), pts_us, flags, queued);
        if d.send {
            let r = pb_video::Record {
                pts_us,
                flags,
                payload: data,
            };
            let refused = match self.sink.get() {
                Some(sink) => sink.send(r) == pb_transport::screen::SendOutcome::Full,
                None => {
                    let mut p = self.pending.lock().unwrap_or_else(|p| p.into_inner());
                    // `ready` may have set the sink while we waited for the lock.
                    if let Some(sink) = self.sink.get() {
                        sink.send(r) == pb_transport::screen::SendOutcome::Full
                    } else {
                        let full = p.len() >= SCREEN_PENDING;
                        if !full {
                            p.push(r);
                        }
                        full
                    }
                }
            };
            if refused {
                // What follows would lack this frame as its reference:
                // drop up to the next key frame instead.
                return advice(
                    self.pacer
                        .lock()
                        .unwrap_or_else(|p| p.into_inner())
                        .queue_full(std::time::Instant::now()),
                );
            }
        }
        let mut a = advice(d);
        if self
            .want_key
            .swap(false, std::sync::atomic::Ordering::AcqRel)
        {
            a.request_key = true;
        }
        a
    }

    /// Round trips measured since the last call (the stats log).
    pub fn take_confirm_stats(&self) -> PbConfirmStats {
        let mut v = self
            .pacer
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .take_confirm_ms();
        v.sort_unstable();
        let at = |q: f64| {
            v.get(((v.len() as f64 - 1.0) * q) as usize)
                .copied()
                .unwrap_or(0)
        };
        PbConfirmStats {
            samples: v.len() as u32,
            p50: at(0.5),
            p95: at(0.95),
        }
    }

    /// Something for the PC window to say ("Lower quality to save memory").
    pub fn notice(&self, text: String) {
        if let Some(s) = self.sink.get() {
            s.notice(&text);
        }
    }

    /// Camera sessions: the lens now in use (0 back, 1 front), after a
    /// switch and when streaming starts. Dropped before the streams open
    /// (`CameraAccept` carried the lens then).
    pub fn send_lens(&self, lens: u8) {
        if let Some(s) = self.sink.get() {
            s.send_lens(lens);
        }
    }

    /// Android stopped the projection because the phone locked: not the
    /// user's Stop, so that PC is not declined, and not a dropped link to
    /// resume; the PC is told why.
    pub fn stop_locked(&self) {
        self.locked
            .store(true, std::sync::atomic::Ordering::Release);
        if let Some(s) = self.sink.get() {
            s.notice(pb_protocol::SCREEN_LOCKED_NOTICE);
            s.close();
        }
    }

    /// The user stopped sharing on the phone.
    pub fn stop(&self) {
        self.stopped
            .store(true, std::sync::atomic::Ordering::Release);
        if let Some(s) = self.sink.get() {
            s.close();
        }
    }
}

/// Encoder size for the phone screen (see `pb_video::size::fit_size`):
/// returns [width, height].
#[uniffi::export]
pub fn fit_size(width: u32, height: u32, max_long: u32, align_w: u32, align_h: u32) -> Vec<u32> {
    let max_long = if max_long == 0 {
        width.max(height)
    } else {
        max_long
    };
    let (w, h) = pb_video::size::fit_size((width, height), max_long, (align_w, align_h));
    vec![w, h]
}

/// Open the structured log at `root` (phone caps: 5 MiB a day, 30 days,
/// `total_cap_bytes` in all; 0 keeps 30 MiB). False if the folder cannot be
/// created. Calling it again replaces the logger.
#[uniffi::export]
pub fn log_init(root: String, total_cap_bytes: u64) -> bool {
    applog::init(&root, total_cap_bytes)
}

/// Append one metadata line. `stream` is events, transfers, network, perf or
/// crash; `fields_json` a flat JSON object (nesting and values over 256
/// characters are rejected and the line is dropped). Never put clipboard
/// text, notification text or file names in it.
#[uniffi::export]
pub fn log_event(stream: String, kind: String, fields_json: String) {
    applog::event(&stream, &kind, &fields_json);
}

/// Delete every day folder (the root stays). False when not initialised or
/// something could not be deleted.
#[uniffi::export]
pub fn log_clear_all() -> bool {
    applog::clear_all()
}

/// Bytes the logs take on disk (0 when not initialised).
#[uniffi::export]
pub fn log_size_bytes() -> u64 {
    applog::size_bytes()
}

/// Off stops writing (what is held is flushed first); on resumes.
#[uniffi::export]
pub fn log_set_enabled(on: bool) {
    applog::set_enabled(on);
}

/// Day folder names (`YYYY-MM-DD`), oldest first.
#[uniffi::export]
pub fn log_days() -> Vec<String> {
    applog::days()
}

/// Write everything held to disk (call when the app goes to the background).
#[uniffi::export]
pub fn log_flush() {
    applog::flush();
}

/// The phone's current UTC offset in seconds (`TimeZone.getOffset / 1000`):
/// the day folder follows local time. Call after `log_init` and when the
/// time zone changes.
#[uniffi::export]
pub fn log_set_tz_offset_secs(secs: i32) {
    applog::set_tz_offset_secs(secs);
}

/// The first of the PC's codecs (best first) the phone can encode.
#[uniffi::export]
pub fn choose_codec(pc: Vec<u8>, phone: Vec<u8>) -> Option<u8> {
    let list = |v: Vec<u8>| -> Vec<pb_video::Codec> {
        v.into_iter()
            .filter_map(|c| pb_video::Codec::from_u32(c.into()))
            .collect()
    };
    pb_video::codec::choose(&list(pc), &list(phone)).map(|c| c as u8)
}

/// The PC's next codec after `failed`, if any.
#[uniffi::export]
pub fn next_codec(failed: u8, pc: Vec<u8>) -> Option<u8> {
    let pc: Vec<pb_video::Codec> = pc
        .into_iter()
        .filter_map(|c| pb_video::Codec::from_u32(c.into()))
        .collect();
    pb_video::codec::next_after(pb_video::Codec::from_u32(failed.into())?, &pc).map(|c| c as u8)
}

type ScreenAnswer = Option<(PbScreenAccept, Arc<PbScreen>)>;
type CameraAnswer = Option<(PbCameraAccept, Arc<PbScreen>)>;

/// How long a control request waits for the user on the phone.
const CONTROL_ASK_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

fn push(events: &Events, e: PbEvent) {
    // A failing listener must never take the node down.
    let _ = events.on_event(e);
}

#[derive(uniffi::Object)]
pub struct PbNode {
    node: Node,
    joining: tokio::sync::Mutex<Option<Joining>>,
    shared: Arc<Shared>,
    spawner: Mutex<Option<tokio::task::JoinHandle<()>>>,
}

/// State shared between the node object and the session pumps.
struct Shared {
    events: Events,
    incoming_dir: std::path::PathBuf,
    auto_accept: std::sync::atomic::AtomicBool,
    clipboard_sync: std::sync::atomic::AtomicBool,
    /// Apply pictures copied on the PC ("Share copied images").
    clipboard_images: std::sync::atomic::AtomicBool,
    pending: Mutex<std::collections::HashMap<u64, tokio::sync::oneshot::Sender<bool>>>,
    last_remote_clip: Mutex<[u8; 32]>,
    input: Mutex<Option<Arc<dyn InputListener>>>,
    /// Accept PC control without asking (otherwise each request asks).
    allow_control: std::sync::atomic::AtomicBool,
    /// Ends the running control session, if any.
    control: Mutex<Option<pb_transport::SessionCloser>>,
    /// A control request is being asked or a session runs: further requests
    /// are declined (one cursor, one Stop button, one closer).
    control_busy: std::sync::atomic::AtomicBool,
    /// Unix ms of the last control request refused because "Relay control"
    /// was not running (0: none since the node started).
    control_refused: RefusalStamp,
    /// Ids for questions to the user (offers, control requests), chosen here:
    /// the peer's own ids may repeat.
    next_ask: std::sync::atomic::AtomicU64,
    /// Phone notifications to paired PCs ("Notifications on PC").
    notify: pb_transport::notify::NotifyOutbox,
    notify_enabled: std::sync::atomic::AtomicBool,
    notify_handler: Mutex<Option<Arc<dyn NotifyHandler>>>,
    /// Kotlin calls in from its own threads; the outbox spawns tasks here.
    rt: tokio::runtime::Handle,
    screen_listener: Mutex<Option<Arc<dyn ScreenListener>>>,
    /// A screen request is being asked or a session runs.
    screen_busy: std::sync::atomic::AtomicBool,
    screen_pending:
        Mutex<std::collections::HashMap<u64, tokio::sync::oneshot::Sender<ScreenAnswer>>>,
    screen_grace: Mutex<Option<Grace>>,
    /// The PC of the request being asked or the session running, and a way
    /// to end it when that PC asks again.
    screen_current: Mutex<Option<(pb_identity::PeerId, pb_transport::SessionCloser)>>,
    camera_listener: Mutex<Option<Arc<dyn CameraListener>>>,
    camera_busy: std::sync::atomic::AtomicBool,
    camera_pending:
        Mutex<std::collections::HashMap<u64, tokio::sync::oneshot::Sender<CameraAnswer>>>,
    camera_grace: Mutex<Option<Grace>>,
    camera_user_stop: Mutex<Option<Grace>>,
    camera_current: Mutex<Option<(pb_identity::PeerId, pb_transport::SessionCloser)>>,
    /// The user stopped sharing: requests from that PC are declined quietly
    /// for a while (the PC retries after a dropped link).
    screen_user_stop: Mutex<Option<Grace>>,
    audio: Arc<audio::AudioHub>,
}

/// A picture from the PC is applied only when the user turned on clipboard
/// sync and, separately, sharing of copied images (both default off).
fn image_accepted(clipboard_sync: bool, clipboard_images: bool) -> bool {
    clipboard_sync && clipboard_images
}

/// What a PC control request meets before anyone is asked.
#[derive(Debug, PartialEq, Eq)]
enum ControlGate {
    /// Another request is being asked or a session runs.
    Busy,
    /// "Relay control" (the accessibility service) is not running.
    NoService,
    /// The user allows control without asking.
    Allowed,
    /// Ask the user (they may decline, or not answer in time).
    Ask,
}

fn control_gate(busy: bool, has_listener: bool, allow_control: bool) -> ControlGate {
    match () {
        _ if busy => ControlGate::Busy,
        _ if !has_listener => ControlGate::NoService,
        _ if allow_control => ControlGate::Allowed,
        _ => ControlGate::Ask,
    }
}

impl ControlGate {
    /// Only a refusal because the service was off is remembered for Home
    /// ("Control is off while a control request is waiting"); a busy phone or
    /// a user who declined is not something to fix.
    fn stamps_refusal(&self) -> bool {
        *self == Self::NoService
    }
}

/// When the last PC control request was refused because "Relay control" was
/// not running (unix ms, 0 for none). It must not outlive what it reports:
/// a served request or the service connecting clears it.
#[derive(Default)]
struct RefusalStamp(std::sync::atomic::AtomicU64);

impl RefusalStamp {
    /// A request met `gate`: only a refusal for the service being off counts.
    fn gate(&self, gate: &ControlGate, now_ms: u64) {
        if gate.stamps_refusal() {
            self.0.store(now_ms, std::sync::atomic::Ordering::Relaxed);
        }
    }

    /// A control session was accepted.
    fn served(&self) {
        self.0.store(0, std::sync::atomic::Ordering::Relaxed);
    }

    /// The input listener was registered (`true`) or cleared.
    fn listener(&self, registered: bool) {
        if registered {
            self.served();
        }
    }

    fn get(&self) -> u64 {
        self.0.load(std::sync::atomic::Ordering::Relaxed)
    }
}

/// Unix time in milliseconds (0 if the clock is before 1970).
fn unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

/// Whether the paired peer with this hex id was found reinstalled or reset
/// on the last attempt to reach it. Unknown or malformed ids: false.
fn changed_peer(node: &Node, peer_id: &str) -> bool {
    node.paired_peers()
        .into_iter()
        .find(|p| p.id.to_string() == peer_id)
        .is_some_and(|p| node.peer_changed(&p.id))
}

/// Name of a received picture inside the private incoming folder: safe
/// characters only, and the same content gets the same name.
fn image_file_name(png: &[u8]) -> String {
    use sha2::Digest;
    let h = sha2::Sha256::digest(png);
    let hex: String = h[..8].iter().map(|b| format!("{b:02x}")).collect();
    format!("clipboard-{hex}.png")
}

/// Folder inside incoming/ for received clipboard pictures. Its dot keeps it
/// apart from received files, whose names never start with one.
const IMAGE_DIR: &str = ".clipboard";

/// Drop the pending question `id`; if the app never answered it (its entry
/// is still there: answering removes it), tell the app it is over.
fn end_ask<T>(pending: &Mutex<std::collections::HashMap<u64, T>>, id: u64, events: &Events) {
    let unanswered = pending
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .remove(&id)
        .is_some();
    if unanswered {
        push(events, PbEvent::AskEnded { id });
    }
}

/// What a killed process left in incoming/. A whole received file was
/// already reported "saved" to the sender, so it is returned to be offered
/// to the app again. Leftovers go: unfinished `.name.partial-*` files and
/// clipboard pictures (dot names), and empty files (a name reserved just
/// before a rename that never happened; an empty file loses nothing).
fn recover_incoming(dir: &std::path::Path) -> Vec<(std::path::PathBuf, u64)> {
    let mut whole = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return whole;
    };
    for e in entries.flatten() {
        let path = e.path();
        let Ok(meta) = e.metadata() else { continue };
        let junk = e.file_name().to_string_lossy().starts_with('.');
        if meta.is_dir() {
            if junk {
                let _ = std::fs::remove_dir_all(&path);
            }
        } else if junk || meta.len() == 0 {
            let _ = std::fs::remove_file(&path);
        } else {
            whole.push((path, meta.len()));
        }
    }
    whole
}

/// Text the clipboard sync sends: not empty, and within what the wire takes
/// (a longer one would fail after a full connect, on every sync).
fn syncable(text: &str) -> bool {
    !text.is_empty() && text.len() <= MAX_CLIPBOARD_TEXT
}

/// A peer's codec list as Kotlin gets it: known codecs only (1 H.265
/// Main10, 2 H.265, 3 H.264); a cast would turn 257 into 1.
fn codec_list(codecs: &[u32]) -> Vec<u8> {
    codecs
        .iter()
        .filter(|c| pb_video::Codec::from_u32(**c).is_some())
        .filter_map(|c| u8::try_from(*c).ok())
        .collect()
}

/// A camera size the app supports: the 16:9 sizes the protocol names;
/// anything else asks for 1280x720.
fn camera_size(width: u32, height: u32) -> (u32, u32) {
    match (width, height) {
        (1920, 1080) | (1280, 720) => (width, height),
        _ => (1280, 720),
    }
}

/// A sender-chosen file name as the prompt and progress show it: the name
/// it will be saved under (no hidden or control characters, bounded).
fn shown_name(raw: &str) -> String {
    pb_transport::files::sanitize_file_name(raw)
}

fn clip_hash(text: &str) -> [u8; 32] {
    use sha2::Digest;
    sha2::Sha256::digest(text.as_bytes()).into()
}

/// Start the node. `data_dir` holds the identity seed and the paired-peer list.
#[uniffi::export(async_runtime = "tokio")]
pub async fn start_node(
    data_dir: String,
    device_name: String,
    port: u16,
    key_wrapper: Arc<dyn KeyWrapper>,
    listener: Arc<dyn EventListener>,
) -> Result<Arc<PbNode>, PbError> {
    let dir = std::path::PathBuf::from(data_dir);
    std::fs::create_dir_all(&dir).map_err(fail)?;
    let identity =
        pb_identity::vault::open_identity(&dir, &ForeignWrapper(key_wrapper)).map_err(fail)?;
    let store: SharedStore = Arc::new(Mutex::new(
        FilePeerStore::open(dir.join("peers.txt")).map_err(fail)?,
    ));
    let config = NodeConfig {
        device_name,
        port: (port != 0).then_some(port),
        discovery_port: Some(pb_transport::discovery::DISCOVERY_PORT),
        tuning: Default::default(),
    };
    // A process the system killed mid-transfer or mid-save left things in
    // incoming/: partial files go, whole files are offered again below.
    let recovered = recover_incoming(&dir.join("incoming"));
    // A taken port falls back to one the system picks: peers learn it from discovery and pairing.
    let (node, ev) = Node::bind_or_any(&identity, store, config)
        .await
        .map_err(fail)?;
    // The sender was told "saved": hand each whole file to the app again
    // (it moves it to the save folder). The sender's name was not kept.
    let sender = match node.paired_peers().as_slice() {
        [only] => only.name.clone(),
        _ => "A paired device".to_owned(),
    };
    for (path, size) in recovered {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        push(
            &listener,
            PbEvent::FileReceived {
                peer_name: sender.clone(),
                path: path.display().to_string(),
                name,
                size,
            },
        );
    }
    let shared = Arc::new(Shared {
        events: listener.clone(),
        incoming_dir: dir.join("incoming"),
        auto_accept: std::sync::atomic::AtomicBool::new(true),
        clipboard_sync: std::sync::atomic::AtomicBool::new(false),
        clipboard_images: std::sync::atomic::AtomicBool::new(false),
        pending: Mutex::default(),
        last_remote_clip: Mutex::new([0; 32]),
        input: Mutex::new(None),
        allow_control: std::sync::atomic::AtomicBool::new(false),
        control: Mutex::new(None),
        control_busy: std::sync::atomic::AtomicBool::new(false),
        control_refused: RefusalStamp::default(),
        next_ask: std::sync::atomic::AtomicU64::new(1),
        notify: pb_transport::notify::NotifyOutbox::new(node.clone()),
        notify_enabled: std::sync::atomic::AtomicBool::new(false),
        notify_handler: Mutex::new(None),
        rt: tokio::runtime::Handle::current(),
        screen_listener: Mutex::new(None),
        screen_busy: std::sync::atomic::AtomicBool::new(false),
        screen_pending: Mutex::default(),
        screen_grace: Mutex::new(None),
        screen_current: Mutex::new(None),
        camera_listener: Mutex::new(None),
        camera_busy: std::sync::atomic::AtomicBool::new(false),
        camera_pending: Mutex::default(),
        camera_grace: Mutex::new(None),
        camera_user_stop: Mutex::new(None),
        camera_current: Mutex::new(None),
        screen_user_stop: Mutex::new(None),
        audio: Arc::new(audio::AudioHub::new(
            listener.clone(),
            node.audio_slots().clone(),
        )),
    });
    // The phone only joins pairings; drop host-side requests.
    drop(ev.pairings);
    let mut sessions = ev.sessions;
    let sh = shared.clone();
    let spawner = tokio::spawn(async move {
        while let Some(s) = sessions.recv().await {
            tokio::spawn(pump(s, sh.clone()));
        }
    });
    Ok(Arc::new(PbNode {
        node,
        joining: tokio::sync::Mutex::new(None),
        shared,
        spawner: Mutex::new(Some(spawner)),
    }))
}

#[cfg(target_os = "android")]
fn fd_to_file(fd: i32) -> Result<std::fs::File, PbError> {
    android_init::file_from_fd(fd)
}

#[cfg(not(target_os = "android"))]
fn fd_to_file(_fd: i32) -> Result<std::fs::File, PbError> {
    Err(fail("file descriptors are only accepted on Android"))
}

async fn pump(mut s: Session, sh: Arc<Shared>) {
    use pb_transport::files::{receive_offer_with, Received, DEFAULT_MAX_FILE};
    let name = s.peer_name.clone();
    applog::session_open();
    push(
        &sh.events,
        PbEvent::Connected {
            peer_name: name.clone(),
        },
    );
    let mut actions =
        pb_transport::notify::RateWindow::new(pb_transport::notify::MAX_ACTIONS_PER_SEC);
    let reason = loop {
        match s.recv().await {
            Ok(Some(ControlMsg::ScreenStart(start))) => {
                break screen_session(s, start, sh.clone(), name.clone()).await;
            }
            Ok(Some(ControlMsg::AudioStart(start))) => {
                let audio = sh.audio.clone();
                break audio.serve(s, start, name.clone()).await;
            }
            Ok(Some(ControlMsg::CameraStart(start))) => {
                break camera_session(s, start, sh.clone(), name.clone()).await;
            }
            Ok(Some(ControlMsg::NotifySyncRequest(_))) => {
                // Off: an empty list, so the PC clears what it still shows.
                let answered = if sh.notify_enabled.load(std::sync::atomic::Ordering::Relaxed) {
                    sh.notify.answer_sync_request(&s).await
                } else {
                    s.send(ControlMsg::NotifySync(Default::default())).await
                };
                if let Err(e) = answered {
                    break e.to_string();
                }
                s.finish().await;
                break "notifications synced".to_owned();
            }
            Ok(Some(ControlMsg::NotifyAction(a))) => {
                if !actions.hit() {
                    s.closer().close(b"action flood");
                    break "action flood".to_owned();
                }
                let id = a.id;
                let (ok, reason) = match run_notify_action(&sh, a).await {
                    Ok(()) => (true, String::new()),
                    Err(e) => (false, e.to_string()),
                };
                let result = pb_protocol::control::NotifyActionResult { id, ok, reason };
                if let Err(e) = s.send(ControlMsg::NotifyActionResult(result)).await {
                    break e.to_string();
                }
            }
            Ok(Some(ControlMsg::ClipboardText(c))) => push(
                &sh.events,
                PbEvent::Clipboard {
                    peer_name: name.clone(),
                    text: c.text,
                    sensitive: c.sensitive,
                },
            ),
            Ok(Some(ControlMsg::ClipboardUpdate(u))) => {
                if sh.clipboard_sync.load(std::sync::atomic::Ordering::Relaxed) {
                    *sh.last_remote_clip
                        .lock()
                        .unwrap_or_else(|p| p.into_inner()) = clip_hash(&u.text);
                    push(
                        &sh.events,
                        PbEvent::RemoteClipboard {
                            peer_name: name.clone(),
                            text: u.text,
                            sensitive: u.sensitive,
                        },
                    );
                }
            }
            Ok(Some(ControlMsg::InputStart(start))) => {
                // Possible only while the accessibility service is running;
                // then automatic if the user allows control, else ask.
                let listener = sh.input.lock().ok().and_then(|g| g.clone());
                let busy = sh
                    .control_busy
                    .swap(true, std::sync::atomic::Ordering::AcqRel);
                let gate = control_gate(
                    busy,
                    listener.is_some(),
                    sh.allow_control.load(std::sync::atomic::Ordering::Relaxed),
                );
                // Still refused without an event; Home reads the time.
                sh.control_refused.gate(&gate, unix_ms());
                let accept = match gate {
                    ControlGate::Busy | ControlGate::NoService => false,
                    ControlGate::Allowed => true,
                    ControlGate::Ask => {
                        let ask = sh
                            .next_ask
                            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        let (tx, rx) = tokio::sync::oneshot::channel();
                        sh.pending
                            .lock()
                            .unwrap_or_else(|p| p.into_inner())
                            .insert(ask, tx);
                        push(
                            &sh.events,
                            PbEvent::ControlRequest {
                                id: ask,
                                peer_name: name.clone(),
                            },
                        );
                        // No answer in time, or the PC gave up: no.
                        let answer = tokio::select! {
                            r = tokio::time::timeout(CONTROL_ASK_TIMEOUT, rx) => {
                                r.ok().and_then(|r| r.ok()).unwrap_or(false)
                            }
                            () = s.closed() => false,
                        };
                        end_ask(&sh.pending, ask, &sh.events);
                        answer
                    }
                };
                if accept {
                    sh.control_refused.served();
                }
                let listener = listener.filter(|_| accept);
                if let Some(l) = &listener {
                    *sh.control.lock().unwrap_or_else(|p| p.into_inner()) = Some(s.closer());
                    let _ = l.on_control(true, name.clone());
                }
                let l2 = listener.clone();
                let _ = pb_transport::input::receive_input(&mut s, start, accept, move |e| {
                    use pb_protocol::InputEvent as E;
                    let Some(l) = &l2 else { return };
                    match e {
                        E::MouseMove { dx, dy } => {
                            let _ = l.on_input(0, dx.into(), dy.into(), false);
                        }
                        E::MouseButton { button, pressed } => {
                            let _ = l.on_input(1, button.into(), 0, pressed);
                        }
                        E::Wheel { dx, dy } => {
                            let _ = l.on_input(2, dx.into(), dy.into(), false);
                        }
                        E::Key { scancode, pressed } => {
                            let _ = l.on_input(3, scancode.into(), 0, pressed);
                        }
                        E::Char { unit } => {
                            let _ = l.on_input(4, unit.into(), 0, false);
                        }
                        // Absolute input belongs to screen sessions only.
                        E::Touch { .. } | E::Pinch { .. } => {}
                    }
                })
                .await;
                if let Some(l) = &listener {
                    sh.control.lock().unwrap_or_else(|p| p.into_inner()).take();
                    let _ = l.on_control(false, name.clone());
                }
                if !busy {
                    sh.control_busy
                        .store(false, std::sync::atomic::Ordering::Release);
                }
                break "input done".to_owned();
            }
            Ok(Some(ControlMsg::Offer(offer))) => {
                let is_image = offer.kind == pb_protocol::control::OfferKind::ClipboardImage as i32;
                let accept = if is_image {
                    // Governed by the clipboard switches, never by the file prompt.
                    image_accepted(
                        sh.clipboard_sync.load(std::sync::atomic::Ordering::Relaxed),
                        sh.clipboard_images
                            .load(std::sync::atomic::Ordering::Relaxed),
                    )
                } else if sh.auto_accept.load(std::sync::atomic::Ordering::Relaxed) {
                    true
                } else {
                    let ask = sh
                        .next_ask
                        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    let (tx, rx) = tokio::sync::oneshot::channel();
                    sh.pending
                        .lock()
                        .unwrap_or_else(|p| p.into_inner())
                        .insert(ask, tx);
                    push(
                        &sh.events,
                        PbEvent::IncomingOffer {
                            id: ask,
                            peer_name: name.clone(),
                            name: shown_name(&offer.name),
                            size: offer.size,
                        },
                    );
                    let answer = tokio::select! {
                        r = tokio::time::timeout(pb_transport::files::OFFER_TIMEOUT, rx) => {
                            r.ok().and_then(|r| r.ok()).unwrap_or(false)
                        }
                        () = s.closed() => false,
                    };
                    end_ask(&sh.pending, ask, &sh.events);
                    answer
                };
                let file_name = shown_name(&offer.name);
                let started = std::time::Instant::now();
                let guard = CancelGuard::new(true);
                let received = receive_offer_with(
                    &mut s,
                    offer,
                    accept,
                    &sh.incoming_dir,
                    DEFAULT_MAX_FILE,
                    pb_transport::files::TransferOpts::new(
                        Some(progress_events(
                            sh.events.clone(),
                            name.clone(),
                            file_name.clone(),
                            true,
                        )),
                        Some(guard.cancel.clone()),
                    ),
                )
                .await;
                drop(guard);
                match received {
                    Ok(Received::File { path, size }) => {
                        applog::file_done(
                            "received",
                            &file_name,
                            size,
                            started.elapsed().as_secs_f64(),
                        );
                        push(
                            &sh.events,
                            PbEvent::FileReceived {
                                peer_name: name.clone(),
                                path: path.display().to_string(),
                                name: path
                                    .file_name()
                                    .map(|n| n.to_string_lossy().into_owned())
                                    .unwrap_or(file_name),
                                size,
                            },
                        );
                    }
                    Ok(Received::Text(t)) => push(
                        &sh.events,
                        PbEvent::SharedText {
                            peer_name: name.clone(),
                            text: t,
                            is_link: false,
                        },
                    ),
                    Ok(Received::Link(l)) => push(
                        &sh.events,
                        PbEvent::SharedText {
                            peer_name: name.clone(),
                            text: l,
                            is_link: true,
                        },
                    ),
                    Ok(Received::Image(png)) => {
                        // Apart from received files, so a restart never
                        // takes a picture for one (see `recover_incoming`).
                        let images = sh.incoming_dir.join(IMAGE_DIR);
                        let path = images.join(image_file_name(&png));
                        // Up to 10 MiB: off the async threads (2 on the phone).
                        let p = path.clone();
                        let saved = tokio::task::spawn_blocking(move || {
                            std::fs::create_dir_all(&images).and_then(|()| std::fs::write(&p, &png))
                        })
                        .await
                        .unwrap_or_else(|e| Err(std::io::Error::other(e.to_string())));
                        push(
                            &sh.events,
                            match saved {
                                Ok(()) => PbEvent::RemoteImage {
                                    peer_name: name.clone(),
                                    path: path.display().to_string(),
                                },
                                Err(e) => PbEvent::TransferFailed {
                                    peer_name: name.clone(),
                                    reason: e.to_string(),
                                },
                            },
                        );
                    }
                    Err(pb_transport::Error::Declined) => {}
                    Err(e) => {
                        applog::file_failed("received", &file_name, &e.to_string());
                        push(
                            &sh.events,
                            PbEvent::TransferFailed {
                                peer_name: name.clone(),
                                reason: e.to_string(),
                            },
                        );
                    }
                }
                break "transfer done".to_owned();
            }
            Ok(Some(_)) => {}
            Ok(None) => break "closed".to_owned(),
            Err(e) => break e.to_string(),
        }
    };
    applog::session_close(&reason);
    push(
        &sh.events,
        PbEvent::Disconnected {
            peer_name: name,
            reason,
        },
    );
}

/// The camera is free for the next request.
fn release_camera(sh: &Shared) {
    *sh.camera_current.lock().unwrap_or_else(|p| p.into_inner()) = None;
    sh.camera_busy
        .store(false, std::sync::atomic::Ordering::Release);
}

/// A PC asked to use this phone's camera: ask (or resume), then run the session.
async fn camera_session(
    s: Session,
    start: pb_protocol::control::CameraStart,
    sh: Arc<Shared>,
    name: String,
) -> String {
    use std::sync::atomic::Ordering;
    let peer = s.peer;
    if sh.camera_busy.swap(true, Ordering::AcqRel) {
        // The same PC asking again: its old session is stale (it restarted
        // before the phone noticed the lost link).
        let stale = match sh
            .camera_current
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .as_ref()
        {
            Some((p, closer)) if supersedes(Some(p), &peer) => {
                closer.close(b"camera superseded");
                true
            }
            _ => false,
        };
        let mut free = false;
        if stale {
            for _ in 0..60 {
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                if !sh.camera_busy.swap(true, Ordering::AcqRel) {
                    free = true;
                    break;
                }
            }
        }
        if !free {
            let _ = pb_transport::camera::serve_camera(s, start, None).await;
            return "camera busy".into();
        }
    }
    *sh.camera_current.lock().unwrap_or_else(|p| p.into_inner()) = Some((peer, s.closer()));
    let now = std::time::Instant::now();
    let stopped_by_user = sh
        .camera_user_stop
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .as_ref()
        .is_some_and(|g| g.admits(&peer, now));
    if stopped_by_user {
        let _ = pb_transport::camera::serve_camera(s, start, None).await;
        release_camera(&sh);
        return "camera stopped by the user".into();
    }
    let resume = take_grace(
        &mut sh.camera_grace.lock().unwrap_or_else(|p| p.into_inner()),
        &peer,
        now,
    );
    let ask = sh.next_ask.fetch_add(1, Ordering::Relaxed);
    let (tx, rx) = tokio::sync::oneshot::channel();
    sh.camera_pending
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .insert(ask, tx);
    push(
        &sh.events,
        PbEvent::CameraRequest {
            id: ask,
            peer_name: name,
            lens: start.lens.min(1),
            width: camera_size(start.width, start.height).0,
            height: camera_size(start.width, start.height).1,
            codecs: codec_list(&start.codecs),
            resume,
        },
    );
    // Our ask, then the camera permission and opening the camera.
    let answer = tokio::select! {
        r = tokio::time::timeout(std::time::Duration::from_secs(40), rx) => {
            r.ok().and_then(|r| r.ok()).flatten()
        }
        () = s.closed() => None,
    };
    end_ask(&sh.camera_pending, ask, &sh.events);
    let Some((acc, cam)) = answer else {
        let _ = pb_transport::camera::serve_camera(s, start, None).await;
        release_camera(&sh);
        return "camera declined".into();
    };
    let accept = pb_protocol::control::CameraAccept {
        id: 0,
        codec: acc.codec.into(),
        width: acc.width,
        height: acc.height,
        rotation: acc.rotation.into(),
        lens: acc.lens.into(),
    };
    let server = match pb_transport::camera::serve_camera(s, start, Some(accept)).await {
        Ok(server) => server,
        Err(e) => {
            if let Some(l) = sh.camera_listener.lock().ok().and_then(|g| g.clone()) {
                let _ = l.on_stopped(false);
            }
            release_camera(&sh);
            return e.to_string();
        }
    };
    let (sink, mut events) = server.into_parts();
    cam.ready(sink);
    let mut closed_by_pc = false;
    while let Some(ev) = events.recv().await {
        use pb_transport::camera::CameraEvent as E;
        match ev {
            E::Feedback(pts) => cam
                .pacer
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .on_feedback(std::time::Instant::now(), pts),
            E::Lens(lens) => {
                if let Some(l) = sh.camera_listener.lock().ok().and_then(|g| g.clone()) {
                    let _ = l.on_lens(lens as u8);
                }
            }
            E::Closed { by_peer } => {
                closed_by_pc = by_peer;
                break;
            }
        }
    }
    let by_user = cam.stopped.load(Ordering::Acquire);
    if !closed_by_pc || by_user {
        let slot = if by_user {
            &sh.camera_user_stop
        } else {
            &sh.camera_grace
        };
        *slot.lock().unwrap_or_else(|p| p.into_inner()) =
            Some(Grace::new(peer, std::time::Instant::now()));
    }
    if let Some(l) = sh.camera_listener.lock().ok().and_then(|g| g.clone()) {
        let _ = l.on_stopped(!by_user && !closed_by_pc);
    }
    release_camera(&sh);
    "camera ended".into()
}

/// The screen is free for the next request.
fn release_screen(sh: &Shared) {
    *sh.screen_current.lock().unwrap_or_else(|p| p.into_inner()) = None;
    sh.screen_busy
        .store(false, std::sync::atomic::Ordering::Release);
}

/// A PC asked to see this screen: ask (or resume), then run the session.
async fn screen_session(
    s: Session,
    start: pb_protocol::control::ScreenStart,
    sh: Arc<Shared>,
    name: String,
) -> String {
    use std::sync::atomic::Ordering;
    let peer = s.peer;
    if sh.screen_busy.swap(true, Ordering::AcqRel) {
        // The same PC asking again: end its stale session as a lost link
        // (the capture waits), so this request resumes it.
        let stale = match sh
            .screen_current
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .as_ref()
        {
            Some((p, closer)) if supersedes(Some(p), &peer) => {
                closer.close(b"screen superseded");
                true
            }
            _ => false,
        };
        let mut free = false;
        if stale {
            for _ in 0..60 {
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                if !sh.screen_busy.swap(true, Ordering::AcqRel) {
                    free = true;
                    break;
                }
            }
        }
        if !free {
            let _ = pb_transport::screen::serve_screen(s, start, None).await;
            return "screen busy".into();
        }
    }
    *sh.screen_current.lock().unwrap_or_else(|p| p.into_inner()) = Some((peer, s.closer()));
    let now = std::time::Instant::now();
    let stopped_by_user = sh
        .screen_user_stop
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .as_ref()
        .is_some_and(|g| g.admits(&peer, now));
    if stopped_by_user {
        let _ = pb_transport::screen::serve_screen(s, start, None).await;
        release_screen(&sh);
        return "screen stopped by the user".into();
    }
    let resume = take_grace(
        &mut sh.screen_grace.lock().unwrap_or_else(|p| p.into_inner()),
        &peer,
        now,
    );
    let ask = sh.next_ask.fetch_add(1, Ordering::Relaxed);
    let (tx, rx) = tokio::sync::oneshot::channel();
    sh.screen_pending
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .insert(ask, tx);
    push(
        &sh.events,
        PbEvent::ScreenRequest {
            id: ask,
            peer_name: name,
            max_long_side: start.max_long_side,
            sharp: start.sharp,
            dim: start.dim,
            codecs: codec_list(&start.codecs),
            resume,
        },
    );
    // Our ask, then Android's capture prompt.
    let answer = tokio::select! {
        r = tokio::time::timeout(std::time::Duration::from_secs(40), rx) => {
            r.ok().and_then(|r| r.ok()).flatten()
        }
        () = s.closed() => None,
    };
    end_ask(&sh.screen_pending, ask, &sh.events);
    let Some((acc, screen)) = answer else {
        let _ = pb_transport::screen::serve_screen(s, start, None).await;
        release_screen(&sh);
        return "screen declined".into();
    };
    let accept = pb_protocol::control::ScreenAccept {
        id: 0,
        codec: acc.codec.into(),
        width: acc.width,
        height: acc.height,
        rotation: acc.rotation.into(),
    };
    let server = match pb_transport::screen::serve_screen(s, start, Some(accept)).await {
        Ok(server) => server,
        Err(e) => {
            release_screen(&sh);
            if let Some(l) = sh.screen_listener.lock().ok().and_then(|g| g.clone()) {
                let _ = l.on_stopped(false);
            }
            return e.to_string();
        }
    };
    let (sink, mut events) = server.into_parts();
    screen.ready(sink);
    let mut closed_by_pc = false;
    while let Some(ev) = events.recv().await {
        use pb_transport::screen::ScreenEvent as E;
        let listener = sh.screen_listener.lock().ok().and_then(|g| g.clone());
        match ev {
            E::Feedback(pts) => screen
                .pacer
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .on_feedback(std::time::Instant::now(), pts),
            E::State(st) => {
                if let Some(l) = listener {
                    let _ = if st.codec_failed {
                        l.on_codec_failed()
                    } else {
                        l.on_state(st.active, st.dim)
                    };
                }
            }
            E::Input(e) => {
                if let Some(l) = listener {
                    forward_screen_input(&*l, e);
                }
            }
            E::Closed { by_peer } => {
                closed_by_pc = by_peer;
                break;
            }
        }
    }
    let by_user = screen.stopped.load(Ordering::Acquire);
    let record = screen_end_record(by_user, screen.locked.load(Ordering::Acquire), closed_by_pc);
    if let Some(r) = record {
        let slot = match r {
            EndSlot::UserStop => &sh.screen_user_stop,
            EndSlot::Grace => &sh.screen_grace,
        };
        *slot.lock().unwrap_or_else(|p| p.into_inner()) =
            Some(Grace::new(peer, std::time::Instant::now()));
    }
    // Kotlin hears the end before another request can start (it checks
    // which session ended, so a quick resume is never paused by it).
    if let Some(l) = sh.screen_listener.lock().ok().and_then(|g| g.clone()) {
        let _ = l.on_stopped(record == Some(EndSlot::Grace));
    }
    release_screen(&sh);
    "screen ended".into()
}

fn forward_screen_input(l: &dyn ScreenListener, e: pb_protocol::InputEvent) {
    use pb_protocol::{InputEvent as E, TouchPhase};
    let _ = match e {
        E::Touch { phase, x, y } => {
            let p = match phase {
                TouchPhase::Hover => 0,
                TouchPhase::Down => 1,
                TouchPhase::Move => 2,
                TouchPhase::Up => 3,
            };
            l.on_touch(p, x, y)
        }
        E::Pinch { out, x, y } => l.on_pinch(out, x, y),
        E::Key { scancode, pressed } => l.on_key(scancode, pressed),
        E::Char { unit } => l.on_char(unit),
        E::MouseButton {
            button,
            pressed: true,
        } => l.on_button(button),
        E::Wheel { dx, dy } => l.on_wheel(dx, dy),
        E::MouseButton { .. } | E::MouseMove { .. } => Ok(()),
    };
}

/// Hand a PC's notification action to Kotlin (off the async threads: it
/// sends Android intents).
async fn run_notify_action(
    sh: &Shared,
    a: pb_protocol::control::NotifyAction,
) -> Result<(), PbError> {
    use pb_protocol::control::{NotifyActionKind as K, MAX_REPLY_TEXT};
    let off = || fail("Notifications on PC is off on the phone");
    if !sh.notify_enabled.load(std::sync::atomic::Ordering::Relaxed) {
        return Err(off());
    }
    let handler = sh
        .notify_handler
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .clone()
        .ok_or_else(off)?;
    let kind = match K::try_from(a.kind) {
        Ok(K::Dismiss) => PbNotifyActionKind::Dismiss,
        Ok(K::Button) => PbNotifyActionKind::Button,
        Ok(K::Reply) => PbNotifyActionKind::Reply,
        Ok(K::Open) => PbNotifyActionKind::Open,
        Ok(K::Silence) => PbNotifyActionKind::Silence,
        Err(_) => return Err(fail("Unknown action")),
    };
    let text = pb_transport::notify::truncate_utf8(&a.text, MAX_REPLY_TEXT).to_owned();
    tokio::task::spawn_blocking(move || handler.on_action(a.key, kind, a.index, text))
        .await
        .map_err(fail)?
}

#[uniffi::export(async_runtime = "tokio")]
impl PbNode {
    pub fn peer_id(&self) -> String {
        self.node.peer_id().to_string()
    }

    pub fn fingerprint(&self) -> String {
        self.node.peer_id().fingerprint()
    }

    /// False when another app holds the discovery port: moved peers can't be found automatically.
    pub fn discovery_active(&self) -> bool {
        self.node.discovery_active()
    }

    pub fn addresses(&self) -> Vec<String> {
        self.node.addrs()
    }

    pub fn peers(&self) -> Vec<PbPeer> {
        self.node
            .paired_peers()
            .into_iter()
            .map(|p| PbPeer {
                id: p.id.to_string(),
                fingerprint: p.id.fingerprint(),
                name: p.name,
            })
            .collect()
    }

    /// Start pairing with a code shown by another device. Returns the 6-digit
    /// SAS to show the user; call [`Self::confirm_pairing`] with their answer.
    pub async fn join_pairing(&self, code: String) -> Result<String, PbError> {
        let code: PairingCode = code.parse().map_err(fail)?;
        let joining = self.node.join(&code).await.map_err(fail)?;
        let sas = joining.sas.clone();
        *self.joining.lock().await = Some(joining);
        Ok(sas)
    }

    /// The user's answer to "does the other device show the same code?".
    /// Returns the paired device's name.
    pub async fn confirm_pairing(&self, accept: bool) -> Result<String, PbError> {
        let joining = self
            .joining
            .lock()
            .await
            .take()
            .ok_or_else(|| fail("no pairing in progress"))?;
        joining.decide(accept).await.map(|p| p.name).map_err(fail)
    }
}

impl PbNode {
    fn find(&self, peer_id: &str) -> Result<pb_identity::PeerId, PbError> {
        self.node
            .paired_peers()
            .into_iter()
            .find(|p| p.id.to_string() == peer_id)
            .map(|p| p.id)
            .ok_or_else(|| fail("not a paired device"))
    }
}

#[uniffi::export(async_runtime = "tokio")]
impl PbNode {
    /// True when the paired PC with this id answered with another identity
    /// the last time this phone tried to reach it (it was reset or
    /// reinstalled): it no longer recognises this phone. Unknown ids: false.
    pub fn peer_changed(&self, peer_id: String) -> bool {
        changed_peer(&self.node, &peer_id)
    }

    /// Unix ms of the most recent PC control request refused because
    /// "Relay control" was not running on this phone; 0 if none.
    pub fn control_refused_at(&self) -> u64 {
        self.shared.control_refused.get()
    }

    pub fn unpair(&self, peer_id: String) -> bool {
        self.node
            .paired_peers()
            .into_iter()
            .find(|p| p.id.to_string() == peer_id)
            .is_some_and(|p| self.node.unpair(&p.id))
    }

    /// [`Self::unpair`] for the Remove button: `false` when it was not
    /// paired; an error when the removal could not be saved, and then the
    /// device is still paired and can still connect: tell the user.
    pub fn try_unpair(&self, peer_id: String) -> Result<bool, PbError> {
        match self
            .node
            .paired_peers()
            .into_iter()
            .find(|p| p.id.to_string() == peer_id)
        {
            Some(p) => self.node.try_unpair(&p.id).map_err(fail),
            None => Ok(false),
        }
    }

    /// Send clipboard text to a paired device (user action only).
    pub async fn send_clipboard(
        &self,
        peer_id: String,
        text: String,
        sensitive: bool,
    ) -> Result<(), PbError> {
        if text.len() > MAX_CLIPBOARD_TEXT {
            return Err(fail(format!("text is over {MAX_CLIPBOARD_TEXT} bytes")));
        }
        let peer = self
            .node
            .paired_peers()
            .into_iter()
            .find(|p| p.id.to_string() == peer_id)
            .ok_or_else(|| fail("not a paired device"))?;
        let session = self.node.connect(&peer.id).await.map_err(fail)?;
        session
            .send(ControlMsg::ClipboardText(ClipboardText { text, sensitive }))
            .await
            .map_err(fail)?;
        session.finish().await;
        Ok(())
    }

    /// Share a file with a paired device. `fd` is a detached file descriptor
    /// (ParcelFileDescriptor.detachFd()); this call takes ownership of it.
    pub async fn send_file_fd(
        &self,
        peer_id: String,
        fd: i32,
        name: String,
        size: u64,
        mime: String,
    ) -> Result<(), PbError> {
        // Take ownership before anything can fail: Kotlin detached the fd.
        let file = fd_to_file(fd)?;
        let peer = self.find(&peer_id)?;
        let reader = tokio::fs::File::from_std(file);
        let started = std::time::Instant::now();
        let guard = CancelGuard::new(false);
        let sent = self
            .node
            .send_file_with(
                &peer,
                &name,
                size,
                &mime,
                reader,
                pb_transport::files::TransferOpts::new(
                    Some(progress_events(
                        self.shared.events.clone(),
                        self.node
                            .paired_peers()
                            .into_iter()
                            .find(|p| p.id == peer)
                            .map(|p| p.name)
                            .unwrap_or_default(),
                        name.clone(),
                        false,
                    )),
                    Some(guard.cancel.clone()),
                ),
            )
            .await;
        drop(guard);
        match &sent {
            Ok(_) => applog::file_done("sent", &name, size, started.elapsed().as_secs_f64()),
            Err(e) => applog::file_failed("sent", &name, &e.to_string()),
        }
        sent.map(|_| ()).map_err(fail)
    }

    /// Send a picture (PNG, read from the detached `fd`) for the PC to put on
    /// its clipboard. This call takes ownership of the fd. Refused over
    /// `MAX_CLIPBOARD_IMAGE`; the PC declines it when its image switch is off.
    pub async fn send_clipboard_image_fd(&self, peer_id: String, fd: i32) -> Result<(), PbError> {
        use tokio::io::AsyncReadExt;
        let file = fd_to_file(fd)?;
        let peer = self.find(&peer_id)?;
        let mut png = Vec::new();
        tokio::fs::File::from_std(file)
            .take(MAX_CLIPBOARD_IMAGE as u64 + 1)
            .read_to_end(&mut png)
            .await
            .map_err(fail)?;
        if png.len() > MAX_CLIPBOARD_IMAGE {
            return Err(fail("the picture is over 10 MB"));
        }
        self.node
            .send_clipboard_image(&peer, png)
            .await
            .map_err(fail)
    }

    /// Share text or a link with a paired device.
    pub async fn send_text(
        &self,
        peer_id: String,
        text: String,
        is_link: bool,
    ) -> Result<(), PbError> {
        let peer = self.find(&peer_id)?;
        self.node
            .send_text(&peer, &text, is_link)
            .await
            .map_err(fail)
    }

    /// Universal Clipboard, phone -> PC. Skips text that just arrived from
    /// the PC so the two clipboards never bounce the same text.
    pub async fn sync_clipboard(&self, text: String, sensitive: bool) -> Result<(), PbError> {
        if !self
            .shared
            .clipboard_sync
            .load(std::sync::atomic::Ordering::Relaxed)
            || !syncable(&text)
        {
            return Ok(());
        }
        let h = clip_hash(&text);
        if *self
            .shared
            .last_remote_clip
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            == h
        {
            return Ok(());
        }
        // All peers at once: one that is away must not delay or block the rest.
        let tasks: Vec<_> = self
            .node
            .paired_peers()
            .into_iter()
            .map(|p| {
                let node = self.node.clone();
                let update = pb_protocol::control::ClipboardUpdate {
                    text: text.clone(),
                    hash: h.to_vec(),
                    sensitive,
                };
                tokio::spawn(async move {
                    let s = node.connect(&p.id).await?;
                    s.send(ControlMsg::ClipboardUpdate(update)).await?;
                    s.finish().await;
                    Ok::<_, pb_transport::Error>(())
                })
            })
            .collect();
        let mut first_err = None;
        for t in tasks {
            if let Ok(Err(e)) = t.await {
                first_err.get_or_insert(fail(e));
            }
        }
        first_err.map_or(Ok(()), Err)
    }

    /// Register (service connected) or clear (service gone) the PC-control handler.
    pub fn set_input_listener(&self, listener: Option<Arc<dyn InputListener>>) {
        self.shared.control_refused.listener(listener.is_some());
        if let Ok(mut g) = self.shared.input.lock() {
            *g = listener;
        }
    }

    /// Accept PC control requests without asking.
    pub fn set_allow_control(&self, on: bool) {
        self.shared
            .allow_control
            .store(on, std::sync::atomic::Ordering::Relaxed);
    }

    /// End the running PC control session now (the user turned control off).
    pub fn stop_control(&self) {
        if let Some(c) = self
            .shared
            .control
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .take()
        {
            c.close(b"stopped on the phone");
        }
    }

    pub fn set_auto_accept(&self, on: bool) {
        self.shared
            .auto_accept
            .store(on, std::sync::atomic::Ordering::Relaxed);
    }

    /// A notification was posted or updated (no-op while "Notifications on
    /// PC" is off).
    pub fn notify_posted(&self, n: PbNotification) {
        let (wire, icon) = to_wire(n);
        if !self
            .shared
            .notify_enabled
            .load(std::sync::atomic::Ordering::Relaxed)
        {
            // Kept track of, so a later sync is right; nothing is sent.
            self.shared.notify.note(wire);
            return;
        }
        let _rt = self.shared.rt.enter();
        self.shared.notify.post(wire, icon);
    }

    pub fn notify_removed(&self, key: String) {
        if !self
            .shared
            .notify_enabled
            .load(std::sync::atomic::Ordering::Relaxed)
        {
            self.shared.notify.forget(&key);
            return;
        }
        let _rt = self.shared.rt.enter();
        self.shared.notify.remove(&key);
    }

    /// What is on the phone already when the listener connects (not sent).
    pub fn notify_seed(&self, list: Vec<PbNotification>) {
        self.shared
            .notify
            .seed(list.into_iter().map(|n| to_wire(n).0).collect());
    }

    pub fn set_notify_enabled(&self, on: bool) {
        self.shared
            .notify_enabled
            .store(on, std::sync::atomic::Ordering::Relaxed);
    }

    /// Answer a `ScreenRequest`: `None` declines. Accepting returns the
    /// session to feed encoded frames into.
    pub fn answer_screen(&self, id: u64, accept: Option<PbScreenAccept>) -> Option<Arc<PbScreen>> {
        let tx = self
            .shared
            .screen_pending
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(&id)?;
        match accept {
            Some(a) => {
                let screen = Arc::new(PbScreen::new(a.codec));
                tx.send(Some((a, screen.clone()))).ok()?;
                Some(screen)
            }
            None => {
                let _ = tx.send(None);
                None
            }
        }
    }

    /// Answer a `CameraRequest`: `None` declines. Accepting returns the
    /// session to feed encoded frames into.
    pub fn answer_camera(&self, id: u64, accept: Option<PbCameraAccept>) -> Option<Arc<PbScreen>> {
        let tx = self
            .shared
            .camera_pending
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(&id)?;
        match accept {
            Some(a) => {
                let cam = Arc::new(PbScreen::new(a.codec));
                tx.send(Some((a, cam.clone()))).ok()?;
                Some(cam)
            }
            None => {
                let _ = tx.send(None);
                None
            }
        }
    }

    /// Register (the audio player exists) or clear the receiver of PC audio.
    /// Without one, PC to phone requests are refused as unavailable.
    pub fn set_audio_sink(&self, sink: Option<Arc<dyn AudioSink>>) {
        self.shared.audio.set_sink(sink);
    }

    /// Accept an `AudioRequest`: send audio in this format (the request's
    /// rate and channels) with `codec` (0 raw PCM, 1 Opus). Returns the handle
    /// to feed frames into: PCM, or one Opus packet per call.
    pub fn answer_audio_accept(
        &self,
        id: u64,
        sample_rate: u32,
        channels: u32,
        codec: u32,
    ) -> Option<Arc<PbAudioOut>> {
        self.shared.audio.accept(id, sample_rate, channels, codec)
    }

    /// Refuse an `AudioRequest` with a wire reason code (1 declined,
    /// 2 unavailable, 3 busy, 4 bad parameters, 5 no permission,
    /// 6 app blocks capture; others read as declined).
    pub fn answer_audio_refuse(&self, id: u64, reason: u8) {
        self.shared.audio.refuse(id, reason);
    }

    /// End the audio session (or the open question) of `direction` (0 phone to
    /// PC, 1 PC to phone). The PC is told. False when none is running.
    pub fn stop_audio(&self, direction: u8) -> bool {
        self.shared.audio.stop(direction)
    }

    /// One log line about the audio arriving from the PC (counts only).
    pub fn audio_rx_stats(&self) -> String {
        self.shared.audio.rx_stats_line()
    }

    /// Whether `direction` has a question open or a session running.
    pub fn audio_active(&self, direction: u8) -> bool {
        self.shared.audio.active(direction)
    }

    /// Register or clear the camera session handler.
    pub fn set_camera_listener(&self, listener: Option<Arc<dyn CameraListener>>) {
        *self
            .shared
            .camera_listener
            .lock()
            .unwrap_or_else(|p| p.into_inner()) = listener;
    }

    /// The user pressed Stop while the link was down: that PC's camera
    /// requests are declined quietly from now on.
    pub fn camera_user_stopped(&self) {
        stop_during_grace(
            &mut self
                .shared
                .camera_grace
                .lock()
                .unwrap_or_else(|p| p.into_inner()),
            &mut self
                .shared
                .camera_user_stop
                .lock()
                .unwrap_or_else(|p| p.into_inner()),
            std::time::Instant::now(),
        );
    }

    /// The user pressed Stop while the link was down (the capture waited for
    /// a resume): that PC is declined quietly from now on.
    pub fn screen_user_stopped(&self) {
        stop_during_grace(
            &mut self
                .shared
                .screen_grace
                .lock()
                .unwrap_or_else(|p| p.into_inner()),
            &mut self
                .shared
                .screen_user_stop
                .lock()
                .unwrap_or_else(|p| p.into_inner()),
            std::time::Instant::now(),
        );
    }

    /// Register (screen capture ready) or clear the screen session handler.
    pub fn set_screen_listener(&self, listener: Option<Arc<dyn ScreenListener>>) {
        *self
            .shared
            .screen_listener
            .lock()
            .unwrap_or_else(|p| p.into_inner()) = listener;
    }

    /// Register (listener connected) or clear the notification action handler.
    pub fn set_notify_handler(&self, handler: Option<Arc<dyn NotifyHandler>>) {
        *self
            .shared
            .notify_handler
            .lock()
            .unwrap_or_else(|p| p.into_inner()) = handler;
    }

    pub fn set_clipboard_sync(&self, on: bool) {
        self.shared
            .clipboard_sync
            .store(on, std::sync::atomic::Ordering::Relaxed);
    }

    /// Apply pictures copied on the PC to this phone's clipboard.
    pub fn set_clipboard_images(&self, on: bool) {
        self.shared
            .clipboard_images
            .store(on, std::sync::atomic::Ordering::Relaxed);
    }

    /// Stop the file transfers running in one direction (`incoming` true: files from the PC,
    /// false: files this phone sends). Partial files are deleted; the PC is told.
    pub fn cancel_transfers(&self, incoming: bool) {
        for (_, inc, c) in CANCELS.lock().unwrap_or_else(|p| p.into_inner()).iter() {
            if *inc == incoming {
                c.cancel();
            }
        }
    }

    /// Reply to an `IncomingOffer` event.
    pub fn answer_offer(&self, id: u64, accept: bool) {
        if let Some(tx) = self
            .shared
            .pending
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(&id)
        {
            let _ = tx.send(accept);
        }
    }

    /// Find paired devices again on the current network (call on start and
    /// whenever the phone's network changes).
    pub async fn rediscover(&self) {
        self.node.rediscover().await;
    }

    /// Stop the node. (Never name a UniFFI method `close`.)
    pub async fn shutdown(&self) {
        if let Some(h) = self.spawner.lock().ok().and_then(|mut g| g.take()) {
            h.abort();
        }
        if let Ok(mut g) = self.shared.input.lock() {
            *g = None;
        }
        self.node.shutdown().await;
    }

    /// Start using this phone as the PC's touchpad and keyboard. Fails if
    /// the PC declines (control turned off there).
    pub async fn start_input(&self, peer_id: String) -> Result<Arc<PbInput>, PbError> {
        let peer = self.find(&peer_id)?;
        let sender = self.node.open_input(&peer).await.map_err(fail)?;
        Ok(Arc::new(PbInput {
            sender: Mutex::new(Some(sender)),
        }))
    }
}

/// A running touchpad/keyboard session. Every call only queues an event
/// (never blocks the UI thread); moves are merged when the link is busy.
#[derive(uniffi::Object)]
pub struct PbInput {
    sender: Mutex<Option<pb_transport::input::InputSender>>,
}

impl PbInput {
    fn send(&self, e: pb_protocol::InputEvent) -> bool {
        self.sender
            .lock()
            .ok()
            .and_then(|g| g.as_ref().map(|s| s.send(e)))
            .unwrap_or(false)
    }
}

#[uniffi::export]
impl PbInput {
    /// Relative pointer motion in PC pixels (before the PC's acceleration).
    pub fn move_by(&self, dx: i16, dy: i16) -> bool {
        self.send(pb_protocol::InputEvent::MouseMove { dx, dy })
    }

    /// 0 left, 1 right, 2 middle.
    pub fn button(&self, button: u8, pressed: bool) -> bool {
        self.send(pb_protocol::InputEvent::MouseButton { button, pressed })
    }

    /// Scroll in 1/120 notch units; positive `dy` scrolls up.
    pub fn scroll(&self, dx: i16, dy: i16) -> bool {
        self.send(pb_protocol::InputEvent::Wheel { dx, dy })
    }

    /// PC set-1 scan code; 0xE0xx for extended keys.
    pub fn key(&self, scancode: u16, pressed: bool) -> bool {
        self.send(pb_protocol::InputEvent::Key { scancode, pressed })
    }

    /// At most 4096 UTF-16 units per call (a paste, not a document).
    pub fn type_text(&self, text: String) -> bool {
        text.encode_utf16()
            .take(4096)
            .all(|unit| self.send(pb_protocol::InputEvent::Char { unit }))
    }

    pub fn is_open(&self) -> bool {
        self.sender
            .lock()
            .ok()
            .is_some_and(|g| g.as_ref().is_some_and(|s| s.is_open()))
    }

    /// End the session now; queued typing is dropped (the PC releases any
    /// keys still held when the session ends).
    pub fn stop(&self) {
        if let Some(s) = self.sender.lock().ok().and_then(|mut g| g.take()) {
            s.cancel();
        }
    }
}

/// JNI entry point. The only `unsafe` in this crate.
#[cfg(target_os = "android")]
#[allow(unsafe_code)]
mod android_init {
    use std::ffi::c_void;

    use jni::{
        objects::{JClass, JObject},
        JNIEnv,
    };

    /// Take ownership of a detached Android file descriptor.
    pub fn file_from_fd(fd: i32) -> Result<std::fs::File, super::PbError> {
        use std::os::fd::FromRawFd;
        if fd < 0 {
            return Err(super::fail("invalid file descriptor"));
        }
        // SAFETY: Kotlin passes ParcelFileDescriptor.detachFd(), which hands
        // over sole ownership of an open descriptor; File closes it on drop.
        Ok(unsafe { std::fs::File::from_raw_fd(fd) })
    }

    /// Called once from Kotlin with the Application context before
    /// [`super::start_node`]; iroh needs it to read system network settings
    /// (without it iroh aborts under `panic = "abort"`; see DECISIONS.md).
    #[no_mangle]
    pub extern "system" fn Java_dev_phonebridge_app_NativeInit_initContext(
        env: JNIEnv,
        _class: JClass,
        context: JObject,
    ) {
        // ndk-context asserts it is initialised only once per process, and the
        // service can be created several times in one process (Stop, then
        // start again; sticky restarts). A second call must be a no-op.
        static INSTALLED: std::sync::Once = std::sync::Once::new();
        if INSTALLED.is_completed() {
            return;
        }
        // UniFFI's tokio runtime is created lazily on the first async call
        // with one worker per core (8 here). Two are plenty for this
        // network-bound workload and halve idle threads and stacks.
        // SAFETY: runs on the first service start, before any runtime or
        // Rust thread exists; nothing reads the environment concurrently.
        unsafe { std::env::set_var("TOKIO_WORKER_THREADS", "2") };
        let (Ok(vm), Ok(global)) = (env.get_java_vm(), env.new_global_ref(&context)) else {
            return;
        };
        let context_ptr = global.as_obj().as_raw() as *mut c_void;
        // ndk-context requires the reference to live until process exit.
        std::mem::forget(global);
        // SAFETY: `vm` is the process JavaVM and `context_ptr` is a leaked
        // global reference, so both stay valid until the process exits.
        INSTALLED.call_once(|| unsafe {
            iroh::dns::install_android_jni_context(vm.get_java_vm_pointer().cast(), context_ptr);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_locked_stop_records_no_user_stop_and_no_grace() {
        // Android 15 QPR1+ stops the projection when the phone locks: that
        // PC is neither declined (user stop) nor auto-resumed (grace).
        assert_eq!(screen_end_record(false, true, false), None);
        assert_eq!(
            screen_end_record(true, false, false),
            Some(EndSlot::UserStop)
        );
        assert_eq!(
            screen_end_record(true, false, true),
            Some(EndSlot::UserStop)
        );
        assert_eq!(screen_end_record(false, false, false), Some(EndSlot::Grace));
        assert_eq!(screen_end_record(false, false, true), None, "PC closed it");
    }

    #[derive(Default)]
    struct Seen(Mutex<Vec<u64>>);
    impl EventListener for Seen {
        fn on_event(&self, e: PbEvent) -> Result<(), PbError> {
            if let PbEvent::AskEnded { id } = e {
                self.0.lock().unwrap().push(id);
            }
            Ok(())
        }
    }

    #[test]
    fn clipboard_sync_skips_empty_and_oversized_text() {
        assert!(!syncable(""));
        assert!(syncable("hi"));
        assert!(syncable(&"a".repeat(MAX_CLIPBOARD_TEXT)));
        assert!(!syncable(&"a".repeat(MAX_CLIPBOARD_TEXT + 1)));
    }

    #[test]
    fn peer_codec_lists_keep_only_known_codecs() {
        assert_eq!(codec_list(&[1, 2, 3]), vec![1, 2, 3]);
        assert_eq!(codec_list(&[257, 0, 3, 4, u32::MAX, 1]), vec![3, 1]);
    }

    #[test]
    fn offer_names_shown_to_the_user_are_safe_and_short() {
        let shown = shown_name(&format!("report\u{202e}fdp.exe{}", "x".repeat(70_000)));
        assert!(!shown.contains('\u{202e}'));
        assert!(shown.starts_with("reportfdp.exe"));
        assert!(shown.len() <= 200);
    }

    #[test]
    fn a_question_dropped_without_an_answer_is_withdrawn() {
        let seen = Arc::new(Seen::default());
        let events: Events = seen.clone();
        let pending: Mutex<std::collections::HashMap<u64, ()>> = Mutex::default();
        pending.lock().unwrap().insert(7, ());
        pending.lock().unwrap().insert(8, ());
        // 8 was answered by the app: its entry is gone already.
        pending.lock().unwrap().remove(&8);
        end_ask(&pending, 7, &events);
        end_ask(&pending, 8, &events);
        assert_eq!(*seen.0.lock().unwrap(), vec![7]);
        assert!(pending.lock().unwrap().is_empty());
    }

    #[test]
    fn whole_received_files_survive_a_restart_and_only_leftovers_go() {
        let dir = std::env::temp_dir().join(format!("pb-ffi-incoming-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(IMAGE_DIR)).unwrap();
        std::fs::write(dir.join("photo.jpg"), b"12345").unwrap();
        std::fs::write(dir.join("clipboard-0011223344556677.png"), b"peer file").unwrap();
        std::fs::write(dir.join(".photo.jpg.partial-00000000000000ab"), b"12").unwrap();
        std::fs::write(dir.join("placeholder.bin"), b"").unwrap();
        std::fs::write(dir.join(IMAGE_DIR).join("clipboard-ab.png"), b"png").unwrap();
        let mut got = recover_incoming(&dir);
        got.sort();
        assert_eq!(
            got,
            vec![
                (dir.join("clipboard-0011223344556677.png"), 9),
                (dir.join("photo.jpg"), 5),
            ]
        );
        let mut left: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        left.sort();
        assert_eq!(left, vec!["clipboard-0011223344556677.png", "photo.jpg"]);
        assert!(recover_incoming(&dir.join("missing")).is_empty());
    }

    fn sample(text: &str) -> PbNotification {
        PbNotification {
            key: "0|bank|1|null|1".into(),
            package: "com.bank".into(),
            app_name: "Bank".into(),
            title: "Bank".into(),
            text: text.into(),
            when_ms: 5,
            conversation: String::new(),
            lines: vec!["l1".into()],
            quiet: true,
            private: false,
            call: false,
            actions: vec![PbNotifyActionDesc {
                label: "Copy".into(),
                reply: false,
            }],
            icon_png: Some(vec![1, 2]),
            picture_jpeg: Some(vec![3]),
        }
    }

    #[test]
    fn to_wire_fills_otp_and_maps_fields() {
        let (n, icon) = to_wire(sample("Your OTP is 482913"));
        assert_eq!(n.otp, "482913");
        assert_eq!(n.key, "0|bank|1|null|1");
        assert_eq!(n.package, "com.bank");
        assert_eq!(n.app_name, "Bank");
        assert_eq!(n.when_ms, 5);
        assert_eq!(n.lines, ["l1"]);
        assert!(n.quiet);
        assert_eq!(n.actions[0].label, "Copy");
        assert_eq!(n.picture, [3]);
        assert_eq!(icon, Some(vec![1, 2]));
    }

    #[test]
    fn to_wire_without_code_leaves_otp_empty() {
        let (n, _) = to_wire(sample("Dinner at 8?"));
        assert!(n.otp.is_empty());
    }
}

#[cfg(test)]
mod screen_tests {
    use std::time::{Duration, Instant};

    use pb_identity::Identity;

    use super::*;

    #[test]
    fn grace_admits_same_peer_within_30s() {
        let a = Identity::from_seed([1; 32]).peer_id();
        let b = Identity::from_seed([2; 32]).peer_id();
        let t0 = Instant::now();
        let g = Grace::new(a, t0);
        assert!(g.admits(&a, t0 + Duration::from_secs(29)));
        assert!(!g.admits(&a, t0 + Duration::from_secs(31)));
        assert!(!g.admits(&b, t0 + Duration::from_secs(1)));
    }

    #[test]
    fn another_pc_does_not_take_the_grace() {
        let a = Identity::from_seed([1; 32]).peer_id();
        let b = Identity::from_seed([2; 32]).peer_id();
        let t0 = Instant::now();
        let mut slot = Some(Grace::new(a, t0));
        assert!(!take_grace(&mut slot, &b, t0), "B asks: a new share");
        assert!(slot.is_some(), "A's grace is still there for A");
        assert!(take_grace(&mut slot, &a, t0));
        assert!(slot.is_none());
    }

    #[test]
    fn stop_during_grace_declines_quietly() {
        let a = Identity::from_seed([1; 32]).peer_id();
        let t0 = Instant::now();
        let mut grace = Some(Grace::new(a, t0));
        let mut user_stop = None;
        let t1 = t0 + Duration::from_secs(10);
        stop_during_grace(&mut grace, &mut user_stop, t1);
        assert!(grace.is_none(), "nothing to resume");
        let g = user_stop.expect("that PC is declined quietly");
        assert!(g.admits(&a, t1 + Duration::from_secs(29)));
    }

    #[test]
    fn flush_stops_at_the_first_refusal() {
        let rec = |pts| pb_video::Record {
            pts_us: pts,
            flags: 0,
            payload: vec![0; 10],
        };
        let mut sent = Vec::new();
        let refused = flush_pending(vec![rec(1), rec(2), rec(3)], |r| {
            let ok = r.pts_us != 2;
            if ok {
                sent.push(r.pts_us);
            }
            ok
        });
        // Frame 3 refers to the refused frame 2: not sent; a key is needed.
        assert!(refused);
        assert_eq!(sent, vec![1]);
        assert!(!flush_pending(vec![rec(4)], |_| true));
    }

    #[test]
    fn same_pc_supersedes_its_stale_session() {
        let a = Identity::from_seed([1; 32]).peer_id();
        let b = Identity::from_seed([2; 32]).peer_id();
        // A restarted PC asks before the phone noticed the old link died.
        assert!(supersedes(Some(&a), &a));
        // Another PC never takes over a running session.
        assert!(!supersedes(Some(&a), &b));
        assert!(!supersedes(None, &a));
    }

    #[test]
    fn advice_maps_pacer_decisions() {
        let sent = advice(pb_video::Decision {
            send: true,
            request_key: false,
            new_bitrate: None,
        });
        assert!(sent.sent && !sent.request_key && sent.new_bitrate == 0);
        let drop = advice(pb_video::Decision {
            send: false,
            request_key: true,
            new_bitrate: Some(8_400_000),
        });
        assert!(!drop.sent && drop.request_key && drop.new_bitrate == 8_400_000);
    }

    #[test]
    fn send_lens_before_the_streams_open_is_harmless() {
        // The PC learns the starting lens from `CameraAccept`; a report made
        // before the session is up has nowhere to go and is dropped.
        let cam = PbScreen::new(pb_video::Codec::H264 as u8);
        cam.send_lens(1);
        cam.send_lens(7);
        assert!(cam.sink.get().is_none());
    }
}

#[cfg(test)]
mod image_tests {
    use super::*;

    #[test]
    fn a_pc_image_is_applied_only_with_clipboard_sync_and_the_image_switch() {
        assert!(image_accepted(true, true));
        assert!(!image_accepted(true, false));
        assert!(!image_accepted(false, true));
        assert!(!image_accepted(false, false));
    }

    #[test]
    fn a_received_image_gets_a_safe_name_that_follows_its_content() {
        let a = image_file_name(&[1, 2, 3]);
        assert_eq!(a, image_file_name(&[1, 2, 3]));
        assert_ne!(a, image_file_name(&[1, 2, 4]));
        assert!(a.starts_with("clipboard-") && a.ends_with(".png"), "{a}");
        assert!(a
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.'));
    }
}

#[cfg(test)]
mod attention_tests {
    use super::*;
    use pb_identity::{Identity, MemoryPeerStore, PairedPeer};

    async fn bind(seed: u8, store: SharedStore) -> (Node, pb_transport::NodeEvents) {
        let config = NodeConfig {
            device_name: format!("node {seed}"),
            port: None,
            discovery_port: None,
            tuning: Default::default(),
        };
        Node::bind(&Identity::from_seed([seed; 32]), store, config)
            .await
            .unwrap()
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn peer_changed_is_false_for_a_healthy_peer_and_unknown_ids() {
        let store: SharedStore = Arc::new(Mutex::new(MemoryPeerStore::default()));
        let pc = Identity::from_seed([2; 32]).peer_id();
        store.lock().unwrap().pin(PairedPeer {
            id: pc,
            name: "PC".into(),
            addrs: vec![],
        });
        let (node, _ev) = bind(1, store).await;
        assert!(!changed_peer(&node, &pc.to_string()));
        let stranger = Identity::from_seed([3; 32]).peer_id();
        assert!(!changed_peer(&node, &stranger.to_string()));
        assert!(!changed_peer(&node, "not hex"));
        assert!(!changed_peer(&node, ""));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn peer_changed_once_the_pc_answers_with_another_identity() {
        // The PC was reset: a new identity now answers at its saved address.
        let (new_pc, _pc_ev) = bind(4, Arc::new(Mutex::new(MemoryPeerStore::default()))).await;
        let old_pc = Identity::from_seed([2; 32]).peer_id();
        let store: SharedStore = Arc::new(Mutex::new(MemoryPeerStore::default()));
        store.lock().unwrap().pin(PairedPeer {
            id: old_pc,
            name: "PC".into(),
            addrs: vec![format!("127.0.0.1:{}", new_pc.port())],
        });
        let (phone, _ev) = bind(1, store).await;
        assert!(!changed_peer(&phone, &old_pc.to_string()));
        assert!(phone.connect(&old_pc).await.is_err());
        assert!(changed_peer(&phone, &old_pc.to_string()));
    }

    #[test]
    fn only_a_request_refused_for_the_service_being_off_is_remembered() {
        assert_eq!(control_gate(false, false, false), ControlGate::NoService);
        assert_eq!(control_gate(false, false, true), ControlGate::NoService);
        assert!(control_gate(false, false, false).stamps_refusal());
        // Busy wins over everything, and is not something to fix.
        assert_eq!(control_gate(true, false, false), ControlGate::Busy);
        assert_eq!(control_gate(true, true, true), ControlGate::Busy);
        assert!(!ControlGate::Busy.stamps_refusal());
        assert_eq!(control_gate(false, true, true), ControlGate::Allowed);
        assert!(!ControlGate::Allowed.stamps_refusal());
        // Asking may end in a decline or a timeout: never stamped.
        assert_eq!(control_gate(false, true, false), ControlGate::Ask);
        assert!(!ControlGate::Ask.stamps_refusal());
    }

    #[test]
    fn the_refusal_stamp_ends_once_a_request_is_served_or_the_service_connects() {
        let stamp = RefusalStamp::default();
        assert_eq!(stamp.get(), 0);
        stamp.gate(&ControlGate::NoService, 1_000);
        assert_eq!(stamp.get(), 1_000);
        // Other outcomes leave it as it is.
        for g in [ControlGate::Busy, ControlGate::Allowed, ControlGate::Ask] {
            stamp.gate(&g, 2_000);
        }
        assert_eq!(stamp.get(), 1_000);
        stamp.listener(false);
        assert_eq!(stamp.get(), 1_000);
        // The service connected (its listener registered): cleared.
        stamp.listener(true);
        assert_eq!(stamp.get(), 0);
        // Refused again, then a later request is accepted: cleared.
        stamp.gate(&ControlGate::NoService, 3_000);
        assert_eq!(stamp.get(), 3_000);
        stamp.served();
        assert_eq!(stamp.get(), 0);
    }

    #[test]
    fn unix_ms_is_wall_clock_milliseconds() {
        // 2020-01-01 in ms: a real clock is past it; seconds would not be.
        assert!(unix_ms() > 1_577_836_800_000);
    }
}

#[cfg(test)]
mod log_tests {
    use super::*;

    #[test]
    fn the_log_surface_round_trips() {
        let (root, _g) = crate::applog::tests::fresh("surface");
        assert!(log_init(root.display().to_string(), 10 * 1024 * 1024));
        log_set_tz_offset_secs(19_800);
        log_event("events".into(), "hello".into(), r#"{"a":1}"#.into());
        log_flush();
        assert!(log_size_bytes() > 10);
        let days = log_days();
        assert_eq!(days.len(), 1, "{days:?}");
        assert_eq!(days[0].len(), 10);
        log_set_enabled(false);
        let before = log_size_bytes();
        log_event("events".into(), "ignored".into(), "{}".into());
        log_flush();
        assert_eq!(log_size_bytes(), before);
        log_set_enabled(true);
        assert!(log_clear_all());
        assert_eq!(log_size_bytes(), 0);
        assert!(log_days().is_empty());
    }

    #[test]
    fn uninitialised_calls_are_harmless() {
        let _g = crate::applog::tests::SERIAL
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        crate::applog::reset();
        assert_eq!(log_size_bytes(), 0);
        assert!(log_days().is_empty());
        assert!(!log_clear_all());
        log_event("events".into(), "x".into(), "{}".into());
        log_flush();
        log_set_enabled(true);
    }
}
