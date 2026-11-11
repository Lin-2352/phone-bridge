//! Platform-independent tray core: owns the node on a small tokio runtime and
//! talks to the UI thread through [`Cmd`] (UI -> core) and [`UiEvent`]
//! (core -> UI). No `unsafe` here.

use std::{
    collections::HashMap,
    io::Write,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Instant,
};

use pb_identity::{FilePeerStore, PeerId};
use pb_protocol::control::{ClipboardText, ClipboardUpdate, ControlMsg, MAX_CLIPBOARD_TEXT};
use pb_transport::{
    files::{receive_offer_with, Received, TransferOpts, DEFAULT_MAX_FILE},
    Node, NodeConfig, PendingPairing, Session, SharedStore,
};
use sha2::{Digest, Sha256};
use tokio::sync::{mpsc, oneshot};

use crate::transfers as tr;

use crate::settings::Settings;

pub type SharedSettings = Arc<Mutex<Settings>>;

/// Adds an entry to the PC timeline when "Keep history" is on. Metadata only:
/// callers build the entry with the `timeline::*_entry` helpers.
pub fn record(settings: &SharedSettings, e: crate::timeline::Entry) {
    if settings.lock().map(|s| s.keep_history).unwrap_or(false) {
        crate::timeline::with(|t| t.add(e));
        crate::ui::wake_timeline();
    }
}

/// Universal Clipboard echo guard: the hash of the last text we received and
/// the last we sent, so neither side bounces the same text back and forth.
#[derive(Default)]
struct ClipSync {
    text: crate::clip_image::EchoGuard,
    /// The same guard for pictures.
    image: crate::clip_image::ImageGuard,
}

fn clip_hash(text: &str) -> [u8; 32] {
    Sha256::digest(text.as_bytes()).into()
}

/// Rotating pairing codes, like an authenticator: a fresh code every
/// `ROTATE`, each valid a little longer (scan-to-connect grace), and the
/// whole pairing window closes after `ROUNDS` codes.
pub const ROTATE: std::time::Duration = std::time::Duration::from_secs(30);
const CODE_TTL: std::time::Duration = std::time::Duration::from_secs(35);
const ROUNDS: u32 = 4;

/// Paired devices that answered at their saved address as a different device
/// (reinstalled or reset): the transport's flags, copied by
/// [`Cmd::CheckPeers`], unpair and pairing, and per device when a session
/// opens or closes (an incoming session clears the transport's flag).
static CHANGED: Mutex<Vec<PeerId>> = Mutex::new(Vec::new());

/// Where a file that broke mid-transfer is kept so it can continue (M9.2); set once by [`run`].
static RESUME: std::sync::OnceLock<pb_transport::files::Resume> = std::sync::OnceLock::new();

pub fn is_changed(id: &PeerId) -> bool {
    CHANGED.lock().map(|g| g.contains(id)).unwrap_or(false)
}

/// Where the transport's "changed" flags come from: the node (or a test
/// stand-in).
pub(crate) trait ChangedFlags {
    fn changed(&self, id: &PeerId) -> bool;
    fn paired(&self) -> Vec<PeerId>;
}

impl ChangedFlags for Node {
    fn changed(&self, id: &PeerId) -> bool {
        self.peer_changed(id)
    }
    fn paired(&self) -> Vec<PeerId> {
        self.paired_peers().into_iter().map(|p| p.id).collect()
    }
}

/// Tells the UI the cache changed (set once by `run`).
static ON_CHANGED: std::sync::OnceLock<Box<dyn Fn() + Send + Sync>> = std::sync::OnceLock::new();

fn changed_cache() {
    if let Some(f) = ON_CHANGED.get() {
        f();
    }
}

/// Puts `id`'s flag in `cache`; true if that changed it (the UI repaints).
fn set_changed(cache: &mut Vec<PeerId>, id: &PeerId, changed: bool) -> bool {
    let had = cache.contains(id);
    if changed && !had {
        cache.push(*id);
    } else if !changed && had {
        cache.retain(|p| p != id);
    }
    had != changed
}

// Lock order: CHANGED, then the transport's own locks (read inside, so two
// overlapping updates cannot write a stale value last). The transport never
// takes CHANGED, and nothing here takes CHANGED while holding a transport
// lock.

/// Copies the transport's flag for `id` into the cache; the UI repaints if
/// it changed.
fn note_changed(flags: &impl ChangedFlags, id: &PeerId) {
    let changed = {
        let mut g = CHANGED.lock().unwrap_or_else(|p| p.into_inner());
        let now = flags.changed(id);
        set_changed(&mut g, id, now)
    };
    if changed {
        changed_cache();
    }
}

/// Copies every paired device's flag (and drops unpaired ones).
fn refresh_changed(flags: &impl ChangedFlags) {
    let changed = {
        let mut g = CHANGED.lock().unwrap_or_else(|p| p.into_inner());
        let list: Vec<PeerId> = flags
            .paired()
            .into_iter()
            .filter(|id| flags.changed(id))
            .collect();
        let differs = *g != list;
        *g = list;
        differs
    };
    if changed {
        changed_cache();
    }
}

/// Any dial of `peer` (connect, send, open a session): the transport clears
/// its flag when the connection works and sets it on `PeerChanged`, so the
/// note follows the attempt either way. Every tray-side dial goes through
/// here (`every_dial_notes_the_changed_flag`).
pub(crate) async fn dial<T>(
    flags: &impl ChangedFlags,
    peer: &PeerId,
    attempt: impl std::future::Future<Output = T>,
) -> T {
    let r = attempt.await;
    note_changed(flags, peer);
    r
}

/// Runs a session; the note follows the transport when it opens (an
/// incoming session clears the flag) and when it closes.
async fn with_session_notes<T>(
    flags: &impl ChangedFlags,
    peer: &PeerId,
    session: impl std::future::Future<Output = T>,
) -> T {
    note_changed(flags, peer);
    let r = session.await;
    note_changed(flags, peer);
    r
}

/// Paired (again): the transport cleared the device's flag.
fn on_paired(flags: &impl ChangedFlags) {
    refresh_changed(flags);
}

/// What the tray knows about reaching a paired device this run (in memory).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Contact {
    /// Sessions from it open now (each `pump`).
    pub open: u32,
    /// Unix seconds of the last successful exchange.
    pub last_seen: Option<u64>,
    /// How this PC reached it last this run (from the adapter that routes
    /// to the address the exchange used).
    pub path: crate::netpath::Path,
}

static CONTACT: Mutex<Vec<(PeerId, Contact)>> = Mutex::new(Vec::new());

pub fn contact(id: &PeerId) -> Contact {
    CONTACT
        .lock()
        .ok()
        .and_then(|g| g.iter().find(|(p, _)| p == id).map(|(_, c)| c.clone()))
        .unwrap_or_default()
}

fn with_contact(id: &PeerId, f: impl FnOnce(&mut Contact)) {
    let mut g = CONTACT.lock().unwrap_or_else(|p| p.into_inner());
    if let Some((_, c)) = g.iter_mut().find(|(p, _)| p == id) {
        f(c);
    } else {
        let mut c = Contact::default();
        f(&mut c);
        g.push((*id, c));
    }
}

/// `id` is in contact now (an exchange worked, or a session runs).
pub fn touch(id: &PeerId) {
    let now = crate::timeline::now_unix();
    with_contact(id, |c| c.last_seen = Some(now));
}

/// An outgoing exchange with `id` worked: stamp it and note the path. A
/// successful dial moves the address it used to the front of the saved
/// list, so that is the address used now.
fn seen(node: &Node, id: &PeerId) {
    let addr = node
        .paired_peers()
        .into_iter()
        .find(|p| p.id == *id)
        .and_then(|p| p.addrs.into_iter().next())
        .and_then(|a| a.parse().ok());
    seen_at(id, addr);
}

/// Contact with `id` over `addr` (when known): stamp it and note the path.
fn seen_at(id: &PeerId, addr: Option<std::net::SocketAddr>) {
    touch(id);
    if let Some(a) = addr {
        // Outside the lock: a quick routing-table query.
        let path = crate::netpath::path_to(a);
        with_contact(id, |c| c.path = path);
    }
}

/// Counts a session from a phone as open for as long as it lives.
struct OpenSession(PeerId);

impl OpenSession {
    fn new(id: PeerId, addr: Option<std::net::SocketAddr>) -> Self {
        seen_at(&id, addr);
        with_contact(&id, |c| c.open += 1);
        Self(id)
    }
}

impl Drop for OpenSession {
    fn drop(&mut self) {
        touch(&self.0);
        with_contact(&self.0, |c| c.open = c.open.saturating_sub(1));
    }
}

/// A PC -> phone control request still waiting for the phone's answer.
static CONTROL_REQUEST: Mutex<Option<tokio::task::AbortHandle>> = Mutex::new(None);
/// The task that is setting up the camera or waiting for the phone's answer.
static CAMERA_REQUEST: Mutex<Option<tokio::task::AbortHandle>> = Mutex::new(None);
/// The task that is waiting for the phone to accept a screen request.
static SCREEN_REQUEST: Mutex<Option<tokio::task::AbortHandle>> = Mutex::new(None);
/// The "Get phone clipboard" request in flight (one at a time).
static PULL: Mutex<crate::clip_pull::PullState> = Mutex::new(crate::clip_pull::PullState::new());
/// The phone -> PC control session running now, so turning the setting off ends it.
static PHONE_CONTROL: Mutex<Option<pb_transport::SessionCloser>> = Mutex::new(None);

/// Pause Relay now (the setting is already on): every open session ends and
/// the requests still waiting for a phone are dropped. The caller stops what
/// lives on the UI side (screen window, control hooks, camera).
/// The answer channel of an incoming-offer dialog. The wait on the core
/// side drops it when the offer is dead; the dialog polls [`alive`].
///
/// [`alive`]: OfferReply::alive
#[derive(Clone)]
pub struct OfferReply(Arc<Mutex<Option<oneshot::Sender<bool>>>>);

impl OfferReply {
    pub fn new(tx: oneshot::Sender<bool>) -> Self {
        Self(Arc::new(Mutex::new(Some(tx))))
    }

    /// True while the core still waits for an answer.
    pub fn alive(&self) -> bool {
        self.0
            .lock()
            .map(|g| g.as_ref().is_some_and(|t| !t.is_closed()))
            .unwrap_or(false)
    }

    /// Deliver the answer; false if the offer is dead or was answered.
    pub fn answer(&self, yes: bool) -> bool {
        let tx = self.0.lock().ok().and_then(|mut g| g.take());
        tx.is_some_and(|t| t.send(yes).is_ok())
    }
}

/// True while the request in `slot` still waits for the phone.
fn request_pending(slot: &Mutex<Option<tokio::task::AbortHandle>>) -> bool {
    slot.lock()
        .unwrap_or_else(|p| p.into_inner())
        .as_ref()
        .is_some_and(|h| !h.is_finished())
}

/// Aborts the request in `slot`; true if one was still waiting.
fn cancel_request(slot: &Mutex<Option<tokio::task::AbortHandle>>) -> bool {
    match slot.lock().unwrap_or_else(|p| p.into_inner()).take() {
        Some(h) if !h.is_finished() => {
            h.abort();
            true
        }
        _ => false,
    }
}

/// A phone-screen request is waiting for the phone's answer (not a window yet).
pub fn screen_pending() -> bool {
    request_pending(&SCREEN_REQUEST)
}

pub fn pause_now() {
    // Transfers first: they end as "Relay is paused" (and tell the peer),
    // not as a vanished connection.
    crate::transfers::cancel_all(crate::transfers::CancelBy::Pause);
    crate::pause::close_all();
    stop_phone_control();
    for slot in [&CONTROL_REQUEST, &CAMERA_REQUEST, &SCREEN_REQUEST] {
        if let Some(h) = slot.lock().unwrap_or_else(|p| p.into_inner()).take() {
            h.abort();
        }
    }
}

/// End phone -> PC control now (the user turned "Let my phone control this PC" off).
pub fn stop_phone_control() {
    if let Some(c) = PHONE_CONTROL
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .take()
    {
        c.close(b"stopped on the PC");
    }
}

pub enum Cmd {
    Pair,
    /// Try each paired device once, to learn which ones answer as someone else now.
    CheckPeers,
    /// The user closed the QR window: kill the current code now.
    CancelPair,
    SendClipboard(String),
    /// Files chosen in the dialog, dropped on the flyout, or sent via Explorer.
    SendFiles(Vec<PathBuf>),
    /// Queue: the next file waits (the one sending finishes).
    QueuePause,
    /// Queue: send what waits, phone by phone.
    QueueResume,
    /// Queue: take one waiting file out (it is not sending, so nothing to cancel on the wire).
    QueueRemove {
        peer: String,
        id: u64,
    },
    /// Queue: hold one file (a file already sending is stopped and kept with its token; the rest go on).
    QueueHold {
        peer: String,
        id: u64,
    },
    /// Queue: hold the file being sent from this path (the Transfers row only knows the path).
    QueueHoldPath(PathBuf),
    /// Queue: let a held file go again.
    QueueRelease {
        peer: String,
        id: u64,
    },
    /// Queue: cancel everything, what waits and what sends.
    QueueCancelAll,
    /// "Get phone clipboard" (hotkey or menu): ask the default phone for what it has copied.
    PullClipboard,
    /// The local clipboard changed (from the clipboard listener).
    ClipboardChanged {
        text: String,
        sensitive: bool,
    },
    /// The local clipboard holds a new picture (PNG bytes).
    ClipboardImageChanged {
        png: Vec<u8>,
    },
    /// Start driving the phone with this PC's mouse and keyboard.
    ControlPhone,
    /// Show the phone's screen in a window.
    ShowScreen,
    /// Give up on a phone-screen request the phone has not answered yet.
    CancelScreen,
    /// Use the phone's camera as a webcam on this PC.
    StartCamera,
    /// Stop the phone camera (and remove the virtual camera).
    StopCamera,
    /// Start audio sharing in this direction (the PC dials).
    StartAudio(crate::audio_state::Dir),
    /// Stop or cancel audio sharing in this direction.
    StopAudio(crate::audio_state::Dir),
    /// Switch the phone camera's lens (front or back).
    SwitchCameraLens,
    /// Install the camera again (one UAC prompt), then forget the start error.
    RepairCamera,
    /// Remove the camera from this PC (stop, sweep, one UAC prompt).
    RemoveCamera,
    /// Text or a link shared from Windows (share sheet).
    SendText {
        text: String,
        link: bool,
    },
    Decide {
        id: u64,
        accept: bool,
    },
    Unpair(PeerId),
    /// Act on a phone notification (from a toast).
    NotifyAction {
        peer: PeerId,
        key: String,
        kind: pb_protocol::control::NotifyActionKind,
        index: u32,
        text: String,
    },
}

pub enum UiEvent {
    Ready {
        fingerprint: String,
        /// Something to tell the user once at start (a taken discovery port).
        note: Option<&'static str>,
    },
    /// New QR code; `closes_in` is the time left for the whole pairing window.
    Code {
        code: String,
        refresh_in: std::time::Duration,
        closes_in: std::time::Duration,
    },
    /// Pairing window ended without anyone joining.
    PairingClosed,
    Sas {
        id: u64,
        peer_name: String,
        sas: String,
    },
    /// The phone left before the comparison was answered: close that prompt.
    SasWithdrawn {
        id: u64,
    },
    Clipboard {
        from: String,
        text: String,
        sensitive: bool,
    },
    /// Text shared from the phone's share sheet: offered on a toast with
    /// Copy; on the clipboard at once only with "Copy shared text
    /// automatically".
    SharedText {
        from: String,
        text: String,
    },
    /// A link shared from the other device: offered to open, clipboard untouched.
    Link {
        from: String,
        url: String,
    },
    /// Control-phone session accepted; the UI installs its input hooks.
    ControlReady {
        name: String,
        sender: pb_transport::input::InputSender,
    },
    /// Universal Clipboard: text copied on the other device.
    RemoteClipboard {
        from: String,
        text: String,
        sensitive: bool,
    },
    /// Universal Clipboard: a picture copied on the other device (PNG bytes).
    RemoteImage {
        from: String,
        png: Vec<u8>,
    },
    FileReceived {
        from: String,
        path: PathBuf,
        size: u64,
    },
    /// A phone wants to control this PC and the user has not allowed it yet.
    AskControl {
        from: String,
        /// The asking phone's id (hex). The core applies that phone's own permission; the dialog does not need it yet.
        #[allow(dead_code)]
        peer_id: String,
        reply: oneshot::Sender<bool>,
    },
    /// Ask the user whether to accept (only when auto-accept is off). The
    /// dialog closes itself when `reply` is no longer alive (timed out,
    /// paused, the phone went away).
    IncomingOffer {
        from: String,
        /// The offering phone's id (hex).
        #[allow(dead_code)] // read by the per-device permissions (M8.4)
        peer_id: String,
        name: String,
        size: u64,
        /// Where it would be saved.
        folder: PathBuf,
        reply: OfferReply,
    },
    /// The paired devices' "reinstalled or reset" notes may differ now (a
    /// check finished, or a session opened or closed): the UI repaints.
    PeersChecked,
    Info(String),
    /// A pill only (no balloon), asked for from a thread that is not the UI thread.
    Pill(String, u32),
    /// A phone notification arrived (or was re-sent quietly).
    Notify {
        peer: PeerId,
        /// The session's device name (the Timeline row names it).
        from: String,
        n: Box<pb_protocol::control::Notification>,
        logo: Option<PathBuf>,
        picture: Option<PathBuf>,
    },
    NotifyRemoved {
        peer: PeerId,
        key: String,
    },
    /// The keys active on that phone; toasts for any others go.
    NotifySync {
        peer: PeerId,
        keys: Vec<String>,
    },
    /// Ask whether to play this PC's audio on the phone (when the Settings
    /// toggle does not skip the question). The dialog closes itself when
    /// `reply` is no longer alive.
    AskAudioShare {
        from: String,
        reply: OfferReply,
    },
    /// An audio session started, ended or is being asked for: repaint.
    AudioChanged,
    /// The request for audio is waiting: show this on the pill only (no
    /// balloon, it would outlive the question).
    AudioAsking(String),
    /// The wait is over (started or cancelled): the pill goes.
    AudioAskEnded,
    /// The phone shares its screen: the "Asking…" pill goes.
    ScreenShown,
    /// The phone-screen window's Pin changed (to remember it).
    ScreenPinned(bool),
    ScreenDim(bool),
    /// Files dropped on the phone-screen window.
    ScreenFiles(Vec<PathBuf>),
    /// An action on a toast did not work on the phone.
    NotifyActionFailed {
        reason: String,
        /// The reply text, so the user does not lose it.
        reply: Option<String>,
    },
}

pub struct Options {
    pub dir: PathBuf,
    pub port: u16,
    pub name: String,
    pub pair_on_start: bool,
}

/// What this PC remembers per phone, shared with the UI thread (set once by [`run`]).
static DEVICES: std::sync::OnceLock<(Arc<Mutex<crate::devices::DeviceStore>>, PathBuf)> =
    std::sync::OnceLock::new();

/// Runs `f` on the per-phone store and saves it when `f` returns true (something changed).
pub fn update_devices(f: impl FnOnce(&mut crate::devices::DeviceStore) -> bool) -> bool {
    let Some((store, path)) = DEVICES.get() else {
        return false;
    };
    let Ok(mut g) = store.lock() else {
        return false;
    };
    let changed = f(&mut g);
    if changed {
        let _ = g.save(path);
    }
    changed
}

fn globals_of(s: &crate::settings::Settings) -> crate::devices::Globals {
    crate::devices::Globals {
        allow_control: s.allow_control,
        audio_skip_confirm: s.audio_skip_confirm_pc_to_phone,
        clipboard: s.universal_clipboard,
        notifications: s.notify_enabled,
        auto_accept: s.auto_accept,
    }
}

/// Gives every phone paired before per-phone permissions existed the old global switches, once, as the core
/// starts and before anything mirrors the globals (after that the globals mean "some phone has it", not the old
/// value). A phone that is still unmarked after this reads as closed.
pub fn migrate_paired_at_start(settings: &SharedSettings, paired: &[String]) {
    let Ok(g) = settings.lock().map(|s| globals_of(&s)) else {
        return;
    };
    update_devices(|d| d.migrate(paired, &g));
    sync_mirror(settings);
}

/// This phone's permissions. Read only: a phone paired before M8 was migrated at start, a phone paired since was
/// marked with fresh defaults, and anything else reads as closed.
pub fn perms_of(_settings: &SharedSettings, id: &str) -> crate::devices::DevicePerms {
    DEVICES
        .get()
        .and_then(|(d, _)| d.lock().ok())
        .map(|d| d.perms(id))
        .unwrap_or_default()
}

/// Keeps the global "control" and "auto-accept" switches (shown in the menu and Settings) equal to "some phone has
/// it on", so what the UI shows is what will happen.
pub fn sync_mirror(settings: &SharedSettings) {
    let (control, auto) = DEVICES
        .get()
        .and_then(|(d, _)| d.lock().ok())
        .map(|d| {
            (
                d.any(|p| p.control == crate::devices::Allow::Always),
                d.any(|p| p.auto_accept),
            )
        })
        .unwrap_or((false, false));
    if let Ok(mut s) = settings.lock() {
        s.allow_control = control;
        s.auto_accept = auto;
    }
}

/// Changes one permission for every paired phone (the global switches in the menu and Settings), then refreshes the
/// mirror. Phones from before per-phone permissions are migrated first, so none loses what it had.
pub fn set_all_perms(
    settings: &SharedSettings,
    paired: &[String],
    f: impl Fn(&mut crate::devices::DevicePerms),
) {
    update_devices(|d| {
        let mut changed = false;
        for id in paired {
            changed |= d.mark_new(id);
            let mut p = d.perms(id);
            f(&mut p);
            changed |= d.set_perms(id, &p);
        }
        changed
    });
    sync_mirror(settings);
}

/// The phone chosen in the "Send to" menu for the next send only (consumed by the next command that needs a target).
static NEXT_TARGET: Mutex<Option<(String, SendKind, Instant)>> = Mutex::new(None);

/// Which command a "Send to" choice is for, so a different command never picks it up.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SendKind {
    Files,
    Clipboard,
}

/// How long a "Send to" choice waits for its command (a file dialog can stay open for a while).
const NEXT_TARGET_TTL: std::time::Duration = std::time::Duration::from_secs(120);

/// Makes the next send of this kind go to the phone with this id (hex), once.
pub fn send_next_to(id: String, kind: SendKind) {
    *NEXT_TARGET.lock().unwrap_or_else(|p| p.into_inner()) = Some((id, kind, Instant::now()));
}

/// The send did not happen (the file dialog was cancelled, the clipboard had nothing): the choice is dropped.
pub fn clear_next_target() {
    NEXT_TARGET.lock().unwrap_or_else(|p| p.into_inner()).take();
}

/// Takes the pending choice if it is for `kind` and still fresh. A choice for another kind stays where it is.
fn take_next_target(kind: Option<SendKind>, now: Instant) -> Option<String> {
    let mut g = NEXT_TARGET.lock().unwrap_or_else(|p| p.into_inner());
    let (id, k, at) = g.clone()?;
    if now.duration_since(at) > NEXT_TARGET_TTL {
        g.take();
        return None;
    }
    if Some(k) != kind {
        return None;
    }
    g.take();
    Some(id)
}

/// The phone sends go to when none is chosen: the one picked for this send, else the saved default while it is
/// still paired, otherwise the first.
fn default_target(
    peers: &Peers,
    settings: &SharedSettings,
    kind: Option<SendKind>,
) -> Option<(PeerId, String)> {
    let once = take_next_target(kind, Instant::now());
    if let Some(once) = once {
        if let Ok(g) = peers.lock() {
            if let Some(p) = g.iter().find(|(id, _)| id.to_string() == once) {
                return Some(p.clone());
            }
        }
    }
    let saved = settings
        .lock()
        .map(|s| s.default_device.clone())
        .unwrap_or_default();
    let g = peers.lock().ok()?;
    let ids: Vec<String> = g.iter().map(|(id, _)| id.to_string()).collect();
    crate::devices::default_position(&ids, &saved).and_then(|i| g.get(i).cloned())
}

/// Paired devices, readable by the UI to build its menu.
pub type Peers = Arc<Mutex<Vec<(PeerId, String)>>>;

/// Append-only log for diagnosis. Never receives clipboard content that was
/// marked sensitive.
#[derive(Clone)]
pub struct Log(Arc<Mutex<Option<std::fs::File>>>, Instant);

impl Log {
    pub fn open(path: PathBuf) -> Self {
        let f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .ok();
        Self(Arc::new(Mutex::new(f)), Instant::now())
    }

    pub fn line(&self, msg: &str) {
        if let Ok(mut g) = self.0.lock() {
            if let Some(f) = g.as_mut() {
                let _ = writeln!(f, "[{:>8.3}] {msg}", self.1.elapsed().as_secs_f64());
                let _ = f.flush();
            }
        }
        crate::applog::from_line(msg);
    }
}

fn refresh(node: &Node, peers: &Peers) {
    let paired = node.paired_peers();
    if let Ok(mut g) = CONTACT.lock() {
        g.retain(|(id, _)| paired.iter().any(|p| p.id == *id));
    }
    let list = paired.into_iter().map(|p| (p.id, p.name)).collect();
    if let Ok(mut g) = peers.lock() {
        *g = list;
    }
}

/// Run the core until the command channel closes. `ui` must be cheap and
/// thread-safe (it posts a message to the UI thread).
pub async fn run(
    opts: Options,
    mut cmds: mpsc::UnboundedReceiver<Cmd>,
    peers: Peers,
    settings: SharedSettings,
    log: Log,
    ui: impl Fn(UiEvent) + Send + Sync + 'static,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let ui = Arc::new(ui);
    // The "reinstalled or reset" notes changed: the windows repaint.
    let _ = ON_CHANGED.set(Box::new({
        let ui = ui.clone();
        move || ui(UiEvent::PeersChecked)
    }));
    std::fs::create_dir_all(&opts.dir)?;
    // Files that were waiting when Relay last closed come back paused; the user resumes them.
    let qfile = opts.dir.join("queue.txt");
    if let Ok(text) = std::fs::read_to_string(&qfile) {
        if let Ok(mut q) = queues().lock() {
            *q = crate::queue::Queues::load(&text, |p| p.exists());
        }
    }
    let _ = QUEUE_FILE.set(qfile);
    // Kept partial files live in the data folder (private to this account), never in the save folder.
    match pb_transport::partials::PartialStore::open(
        &opts.dir.join("partials"),
        pb_transport::files::unix_now(),
    ) {
        Ok(store) => {
            let _ = RESUME.set(pb_transport::files::Resume {
                store: Arc::new(Mutex::new(store)),
                now: pb_transport::files::unix_now,
            });
        }
        Err(e) => log.line(&format!("RESUME partial store unavailable: {e}")),
    }
    // A virtual camera a crash left behind goes now (nothing is streaming).
    if crate::camera_register::is_registered() {
        let log = log.clone();
        std::thread::spawn(move || {
            crate::camera_device::sweep_leftovers(&|m| log.line(m));
            crate::camera::clear_frames();
        });
    }
    // Partial files left by a tray that was killed mid-transfer.
    if let Ok(dir) = settings.lock().map(|s| s.save_dir.clone()) {
        for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') && name.contains(".partial-") {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
    // Toast images of a previous run are dropped here.
    let images = Arc::new(ToastImages::new(opts.dir.join("toast-images")));
    // What this PC remembers per paired device (their Relay version, for now).
    let devices_path = opts.dir.join("devices.txt");
    let devices = Arc::new(Mutex::new(crate::devices::DeviceStore::load(&devices_path)));
    let _ = DEVICES.set((devices.clone(), devices_path.clone()));
    let identity = pb_identity::vault::open_identity(&opts.dir, &pb_platform_win_wrapper())?;
    let store: SharedStore = Arc::new(Mutex::new(FilePeerStore::open(opts.dir.join("peers.txt"))?));
    // A taken port (another app, or a Windows excluded range) falls back to one the system picks.
    let (node, mut ev) = Node::bind_or_any(
        &identity,
        store,
        NodeConfig {
            device_name: opts.name.clone(),
            port: Some(opts.port),
            discovery_port: Some(pb_transport::discovery::DISCOVERY_PORT),
            tuning: Default::default(),
        },
    )
    .await?;
    // Which peers speak keyed discovery lives next to the peer list (never inside it).
    node.set_discovery2_path(opts.dir.join("discovery2.txt"));
    // Peers may have moved to a new network since we last saw them.
    tokio::spawn({
        let node = node.clone();
        async move { node.rediscover().await }
    });
    log.line(&format!(
        "READY id={} port={} discovery={}",
        node.peer_id(),
        node.port(),
        node.discovery_active()
    ));
    refresh(&node, &peers);
    {
        let ids: Vec<String> = peers
            .lock()
            .map(|g| g.iter().map(|(i, _)| i.to_string()).collect())
            .unwrap_or_default();
        migrate_paired_at_start(&settings, &ids);
    }
    // Retry in the transfer window restarts a failed or cancelled send.
    {
        let (node, log, ui, settings, peers, rt) = (
            node.clone(),
            log.clone(),
            ui.clone(),
            settings.clone(),
            peers.clone(),
            tokio::runtime::Handle::current(),
        );
        crate::transfers::set_retry_handler(Box::new(move |row, cancel, info| {
            use crate::transfers as tr;
            if settings.lock().map(|s| s.paused).unwrap_or(false) {
                tr::cancel(row, tr::CancelBy::Pause);
                ui(UiEvent::Info("Relay is paused".into()));
                return;
            }
            let target = peers.lock().ok().and_then(|g| {
                g.iter()
                    .find(|(_, n)| *n == info.peer)
                    .or_else(|| g.first())
                    .cloned()
            });
            let Some((id, name)) = target else {
                tr::cancel(row, tr::CancelBy::User);
                ui(UiEvent::Info("Pair a phone first".into()));
                return;
            };
            // Retried by hand: a place it still holds in the queue goes, so it is not sent twice.
            if let Ok(mut q) = queues().lock() {
                q.of(&id.to_string()).drop_path(&info.path);
            }
            save_queues();
            let (node, log, ui, settings) =
                (node.clone(), log.clone(), ui.clone(), settings.clone());
            rt.spawn(async move {
                send_one_file(
                    &node,
                    &id,
                    &name,
                    &info.path,
                    row,
                    cancel,
                    // The same token as the failed attempt, so the phone's kept bytes are used.
                    retry_tokens()
                        .lock()
                        .ok()
                        .and_then(|t| t.get(&(name.clone(), info.path.clone())).copied())
                        .unwrap_or_else(pb_transport::files::new_resume_token),
                    &log,
                    &*ui,
                    &settings,
                )
                .await;
            });
        }));
    }
    ui(UiEvent::Ready {
        fingerprint: node.peer_id().fingerprint(),
        note: crate::strings::discovery_note(node.discovery_active()),
    });
    let clip: Arc<Mutex<ClipSync>> = Arc::default();
    // Ask each phone once what notifications it has (quiet copies and the
    // active keys), so toasts left from before match the phone again.
    let start_paused = settings.lock().map(|s| s.paused).unwrap_or(false);
    for p in node.paired_peers().into_iter().filter(|_| !start_paused) {
        let (node, log, ui, settings, clip, images) = (
            node.clone(),
            log.clone(),
            ui.clone(),
            settings.clone(),
            clip.clone(),
            images.clone(),
        );
        tokio::spawn(async move {
            match dial(&node, &p.id, node.notify_sync_request(&p.id)).await {
                // An older phone app never answers: do not keep the session
                // (and its heartbeats) open.
                Ok(s) => {
                    seen(&node, &p.id);
                    let closer = s.closer();
                    let pumped = pump(s, node.clone(), log, ui, settings, clip, images);
                    if tokio::time::timeout(std::time::Duration::from_secs(30), pumped)
                        .await
                        .is_err()
                    {
                        closer.close(b"sync timeout");
                    }
                }
                Err(e) => log.line(&format!("NOTIFY sync with \"{}\" skipped: {e}", p.name)),
            }
        });
    }

    let mut pending: HashMap<u64, PendingPairing> = HashMap::new();
    let mut next_id = 0u64;
    // Ids of pairing prompts whose phone has left.
    let (gone_tx, mut gone_rx) = tokio::sync::mpsc::unbounded_channel::<u64>();
    let mut rotation: Option<tokio::task::JoinHandle<()>> = None;
    if opts.pair_on_start {
        rotation = Some(start_rotation(&node, &log, &ui));
    }
    loop {
        tokio::select! {
            cmd = cmds.recv() => match cmd {
                None => break,
                // Paused: nothing talks to a phone.
                Some(c) if crate::pause::needs_network(&c)
                    && settings.lock().map(|s| s.paused).unwrap_or(false) =>
                {
                    if let Some(reason) = crate::pause::clipboard_drop_log(&c) {
                        log.line(reason);
                    }
                    if !crate::pause::is_quiet(&c) {
                        ui(UiEvent::Info("Relay is paused".into()));
                    }
                }
                Some(Cmd::Pair) => {
                    if let Some(r) = rotation.take() {
                        r.abort();
                    }
                    rotation = Some(start_rotation(&node, &log, &ui));
                }
                Some(Cmd::CancelPair) => {
                    if let Some(r) = rotation.take() {
                        r.abort();
                        node.close_pairing();
                        log.line("PAIRING window closed by user; code revoked");
                    }
                }
                Some(Cmd::Decide { id, accept }) => {
                    if let Some(p) = pending.remove(&id) {
                        let (node, peers, log, ui) = (node.clone(), peers.clone(), log.clone(), ui.clone());
                        tokio::spawn(async move {
                            match p.decide(accept).await {
                                Ok(peer) => {
                                    log.line(&format!("PAIRED {} \"{}\"", peer.id, peer.name));
                                    // A phone paired now starts from fresh defaults, never from the old global switches.
                                    update_devices(|d| d.mark_new(&peer.id.to_string()));
                                    on_paired(&node);
                                    refresh(&node, &peers);
                                    ui(UiEvent::Info(format!("Paired with {}", peer.name)));
                                }
                                Err(e) => {
                                    log.line(&format!("PAIRING not completed: {e}"));
                                    ui(UiEvent::Info(format!("Pairing not completed: {e}")));
                                }
                            }
                        });
                    }
                }
                Some(Cmd::CheckPeers) => {
                    let targets: Vec<PeerId> = peers.lock().map(|g| g.iter().map(|(id, _)| *id).collect()).unwrap_or_default();
                    let (node, ui) = (node.clone(), ui.clone());
                    tokio::spawn(async move {
                        for id in targets {
                            // Only the outcome matters: the node records it.
                            if let Ok(Ok(_)) = tokio::time::timeout(std::time::Duration::from_secs(5), dial(&node, &id, node.connect(&id))).await {
                                seen(&node, &id);
                            }
                        }
                        refresh_changed(&node);
                        ui(UiEvent::PeersChecked);
                    });
                }
                Some(Cmd::Unpair(id)) => {
                    // A local stop first: removing the device must not read as a lost link.
                    crate::audio::stop_all();
                    match node.try_unpair(&id) {
                        Ok(_) => {
                            refresh_changed(&node);
                            log.line(&format!("UNPAIRED {id}"));
                            if let Ok(mut q) = queues().lock() {
                                q.forget(&id.to_string());
                            }
                            save_queues();
                            if let Ok(mut d) = devices.lock() {
                                if d.forget(&id.to_string()) {
                                    let _ = d.save(&devices_path);
                                }
                            }
                            refresh(&node, &peers);
                            // If it was the default device, the next one by name takes over (or none).
                            let rest: Vec<(String, String)> = peers
                                .lock()
                                .map(|g| g.iter().map(|(i, n)| (i.to_string(), n.clone())).collect())
                                .unwrap_or_default();
                            if let Ok(mut s) = settings.lock() {
                                let next = crate::devices::default_after_removal(
                                    &rest,
                                    &id.to_string(),
                                    &s.default_device,
                                );
                                if next != s.default_device {
                                    s.default_device = next;
                                    let _ = s.save(&opts.dir.join("settings.txt"));
                                }
                            }
                            // Its toasts go with it.
                            ui(UiEvent::NotifySync { peer: id, keys: Vec::new() });
                            ui(UiEvent::Info("Device removed".into()));
                        }
                        Err(e) => {
                            // Not saved: the device is still paired.
                            log.line(&format!("UNPAIR {id} failed: {e}"));
                            refresh(&node, &peers);
                            ui(UiEvent::Info(format!("Couldn't remove the device: {e}")));
                        }
                    }
                }
                Some(Cmd::NotifyAction { peer, key, kind, index, text }) => {
                    let name = peers
                        .lock()
                        .ok()
                        .and_then(|g| g.iter().find(|(id, _)| *id == peer).map(|(_, n)| n.clone()))
                        .unwrap_or_else(|| "your phone".into());
                    let (node, log, ui) = (node.clone(), log.clone(), ui.clone());
                    tokio::spawn(async move {
                        let reply = (kind == pb_protocol::control::NotifyActionKind::Reply).then(|| text.clone());
                        let action = pb_protocol::control::NotifyAction {
                            id: 0,
                            key: key.clone(),
                            kind: kind as i32,
                            index,
                            text,
                        };
                        let r = dial(&node, &peer, node.notify_action(&peer, action)).await;
                        let hash = pb_transport::notify::key_hash(&key);
                        if r.is_ok() {
                            seen(&node, &peer);
                        }
                        match r {
                            Ok(res) if res.ok => log.line(&format!("NOTIFY action {kind:?} key={hash} ok=true")),
                            Ok(res) => {
                                log.line(&format!("NOTIFY action {kind:?} key={hash} ok=false"));
                                ui(UiEvent::NotifyActionFailed { reason: res.reason, reply });
                            }
                            // A clear made while the phone is away fails quietly.
                            Err(e) if kind == pb_protocol::control::NotifyActionKind::Dismiss => {
                                log.line(&format!("NOTIFY action {kind:?} key={hash} failed: {e}"));
                            }
                            Err(e) => {
                                log.line(&format!("NOTIFY action {kind:?} key={hash} failed: {e}"));
                                ui(UiEvent::NotifyActionFailed {
                                    reason: format!("Couldn't reach {name}"),
                                    reply,
                                });
                            }
                        }
                    });
                }
                Some(Cmd::QueuePause) => {
                    queue_set_paused(true);
                    // Pause means pause: the file sending stops too and keeps its place and token. Files
                    // arriving from the phone go on (this queue only governs what the PC sends).
                    crate::transfers::cancel_all_sending(crate::transfers::CancelBy::Pause);
                    save_queues();
                    ui(UiEvent::PeersChecked);
                }
                Some(Cmd::QueueHold { peer, id }) => {
                    let sending = queues().lock().ok().and_then(|mut q| {
                        let s = q.of(&peer);
                        let path = s.items().iter().find(|i| i.id == id).map(|i| i.path.clone());
                        if s.hold(id) {
                            path
                        } else {
                            None
                        }
                    });
                    if let Some(path) = sending {
                        crate::transfers::cancel_sending_path(&path, crate::transfers::CancelBy::Pause);
                    }
                    save_queues();
                    ui(UiEvent::PeersChecked);
                }
                Some(Cmd::QueueHoldPath(path)) => {
                    let held = queues().lock().is_ok_and(|mut q| q.hold_sending_path(&path));
                    if held {
                        crate::transfers::cancel_sending_path(&path, crate::transfers::CancelBy::Pause);
                    }
                    save_queues();
                    ui(UiEvent::PeersChecked);
                }
                Some(Cmd::QueueRelease { peer, id }) => {
                    if let Ok(mut q) = queues().lock() {
                        q.of(&peer).release(id);
                    }
                    save_queues();
                    ui(UiEvent::PeersChecked);
                    let known: Vec<(PeerId, String)> =
                        peers.lock().map(|g| g.clone()).unwrap_or_default();
                    if let Some((id, name)) = known.iter().find(|(i, _)| i.to_string() == peer).cloned() {
                        let (node, log, ui, settings) =
                            (node.clone(), log.clone(), ui.clone(), settings.clone());
                        tokio::spawn(async move {
                            drain_queue(&node, &id, &name, &log, &*ui, &settings).await;
                        });
                    }
                }
                Some(Cmd::QueueCancelAll) => {
                    // The queue first, so nothing starts behind the file that is stopped.
                    if let Ok(mut q) = queues().lock() {
                        q.clear_all_waiting();
                    }
                    crate::transfers::cancel_all_sending(crate::transfers::CancelBy::User);
                    save_queues();
                    ui(UiEvent::PeersChecked);
                }
                Some(Cmd::QueueRemove { peer, id }) => {
                    if let Ok(mut q) = queues().lock() {
                        q.of(&peer).remove_waiting(id);
                    }
                    save_queues();
                    ui(UiEvent::PeersChecked);
                }
                Some(Cmd::QueueResume) => {
                    // Files that were missing (a USB stick, a share) are looked for again first.
                    if let Ok(mut q) = queues().lock() {
                        q.recheck(|p| p.exists());
                    }
                    queue_set_paused(false);
                    save_queues();
                    ui(UiEvent::PeersChecked);
                    let waiting = queues()
                        .lock()
                        .map(|q| q.peers_with_waiting())
                        .unwrap_or_default();
                    let known: Vec<(PeerId, String)> =
                        peers.lock().map(|g| g.clone()).unwrap_or_default();
                    for key in waiting {
                        let Some((id, name)) = known.iter().find(|(i, _)| i.to_string() == key).cloned()
                        else {
                            continue;
                        };
                        let (node, log, ui, settings) =
                            (node.clone(), log.clone(), ui.clone(), settings.clone());
                        tokio::spawn(async move {
                            drain_queue(&node, &id, &name, &log, &*ui, &settings).await;
                        });
                    }
                }
                Some(Cmd::SendFiles(paths)) => {
                    let (node, log, ui, settings) =
                        (node.clone(), log.clone(), ui.clone(), settings.clone());
                    let target = default_target(&peers, &settings, Some(SendKind::Files));
                    tokio::spawn(async move {
                        let Some((id, name)) = target else {
                            ui(UiEvent::Info("Pair a phone first".into()));
                            return;
                        };
                        send_batch(&node, &id, &name, paths, &log, &*ui, &settings).await;
                    });
                }
                Some(Cmd::PullClipboard) => {
                    let Some((id, name)) = default_target(&peers, &settings, None) else {
                        ui(UiEvent::Pill("Pair a phone first".into(), 5));
                        continue;
                    };
                    if !PULL.lock().unwrap_or_else(|p| p.into_inner()).start(std::time::Instant::now(), &id.to_string()) {
                        continue;
                    }
                    let want_image = settings
                        .lock()
                        .map(|s| crate::clip_image::enabled(s.universal_clipboard, s.share_images))
                        .unwrap_or(false);
                    log.line(&format!("PULL asking \"{name}\" for its clipboard"));
                    ui(UiEvent::Pill(format!("Asking {name} for its clipboard"), 3));
                    let (node, log, ui): (_, _, Arc<dyn Fn(UiEvent) + Send + Sync>) = (node.clone(), log.clone(), ui.clone());
                    tokio::spawn(async move {
                        let sent = async {
                            let s = dial(&node, &id, node.connect(&id)).await?;
                            s.send(ControlMsg::ClipboardPull(pb_protocol::control::ClipboardPull { want_image })).await?;
                            s.finish().await;
                            Ok::<_, pb_transport::Error>(())
                        }
                        .await;
                        match sent {
                            Ok(()) => {
                                tokio::time::sleep(crate::clip_pull::PULL_WAIT).await;
                                if PULL.lock().unwrap_or_else(|p| p.into_inner()).timed_out(std::time::Instant::now()) {
                                    log.line("PULL no answer from the phone");
                                    ui(UiEvent::Pill(crate::clip_pull::NO_ANSWER.into(), 6));
                                }
                            }
                            Err(e) => {
                                PULL.lock().unwrap_or_else(|p| p.into_inner()).answered_from(&id.to_string());
                                log.line(&format!("PULL could not reach \"{name}\": {e}"));
                                ui(UiEvent::Pill(format!("Could not reach {name}"), 6));
                            }
                        }
                    });
                }
                Some(Cmd::ClipboardChanged { text, sensitive }) => {
                    let on = settings.lock().map(|s| s.universal_clipboard).unwrap_or(false);
                    if !on || text.is_empty() || text.len() > MAX_CLIPBOARD_TEXT {
                        continue;
                    }
                    let h = clip_hash(&text);
                    {
                        let mut c = clip.lock().unwrap_or_else(|p| p.into_inner());
                        // Don't echo what just arrived, or resend the same text.
                        if !c.text.should_send(h, std::time::Instant::now()) {
                            log.line("CLIPSYNC skipped: same text as a moment ago (echo or repeat)");
                            continue;
                        }
                    }
                    let targets: Vec<(PeerId, String)> = peers.lock().map(|g| g.clone()).unwrap_or_default();
                    let (node, log, clip, settings) =
                        (node.clone(), log.clone(), clip.clone(), settings.clone());
                    let text = Arc::new(text);
                    tokio::spawn(async move {
                        let errors = crate::clip_fanout::send_all(targets, |(id, name)| {
                            let (node, log, clip, settings, text) =
                                (node.clone(), log.clone(), clip.clone(), settings.clone(), text.clone());
                            async move {
                                let r = async {
                                    let s = dial(&node, &id, node.connect(&id)).await?;
                                    s.send(ControlMsg::ClipboardUpdate(ClipboardUpdate { text: (*text).clone(), hash: h.to_vec(), sensitive })).await?;
                                    s.finish().await;
                                    Ok::<_, pb_transport::Error>(())
                                }
                                .await;
                                if r.is_ok() {
                                    seen(&node, &id);
                                    record(
                                        &settings,
                                        crate::timeline::clip_text_entry(&name, crate::timeline::Dir::ToPhone, &text, sensitive),
                                    );
                                }
                                match r {
                                    Ok(()) if sensitive => log.line(&format!("CLIPSYNC sent a sensitive clip to \"{name}\"")),
                                    Ok(()) => log.line(&format!("CLIPSYNC sent {} chars to \"{name}\"", text.chars().count())),
                                    Err(e) => {
                                        log.line(&format!("CLIPSYNC to \"{name}\" failed: {e}"));
                                        // Not delivered: copying the same text again must retry.
                                        let mut c = clip.lock().unwrap_or_else(|p| p.into_inner());
                                        c.text.send_failed(h);
                                    }
                                }
                            }
                        }).await;
                        if !errors.is_empty() {
                            log.line("CLIPSYNC sender task failed; the copy remains retryable");
                            clip.lock().unwrap_or_else(|p| p.into_inner()).text.send_failed(h);
                        }
                    });
                }
                Some(Cmd::ClipboardImageChanged { png }) => {
                    let on = settings
                        .lock()
                        .map(|s| crate::clip_image::enabled(s.universal_clipboard, s.share_images))
                        .unwrap_or(false);
                    if !on {
                        continue;
                    }
                    let Some(h) = clip.lock().unwrap_or_else(|p| p.into_inner()).image.should_send(&png) else {
                        continue;
                    };
                    let targets: Vec<(PeerId, String)> = peers.lock().map(|g| g.clone()).unwrap_or_default();
                    let (node, log, clip, settings) =
                        (node.clone(), log.clone(), clip.clone(), settings.clone());
                    let png: Arc<[u8]> = png.into();
                    tokio::spawn(async move {
                        let errors = crate::clip_fanout::send_all(targets, |(id, name)| {
                            let (node, log, clip, settings, png) =
                                (node.clone(), log.clone(), clip.clone(), settings.clone(), png.clone());
                            async move {
                                match dial(&node, &id, node.send_clipboard_image(&id, png.clone())).await {
                                    Ok(()) => {
                                        seen(&node, &id);
                                        record(
                                            &settings,
                                            crate::timeline::clip_image_entry(&name, crate::timeline::Dir::ToPhone, &png),
                                        );
                                        log.line(&format!("CLIPSYNC sent an image ({} bytes) to \"{name}\"", png.len()))
                                    }
                                    Err(e) => {
                                        log.line(&format!("CLIPSYNC image to \"{name}\" failed: {e}"));
                                        // Not delivered: copying the same picture again must retry.
                                        clip.lock().unwrap_or_else(|p| p.into_inner()).image.send_failed(h);
                                    }
                                }
                            }
                        }).await;
                        if !errors.is_empty() {
                            log.line("CLIPSYNC sender task failed; the copy remains retryable");
                            clip.lock().unwrap_or_else(|p| p.into_inner()).image.send_failed(h);
                        }
                    });
                }
                Some(Cmd::ShowScreen) => {
                    let target = default_target(&peers, &settings, None);
                    let Some((id, name)) = target else {
                        ui(UiEvent::Info("Pair a phone first".into()));
                        continue;
                    };
                    let mut request = SCREEN_REQUEST.lock().unwrap_or_else(|p| p.into_inner());
                    // One request at a time: a second click must not ask the phone twice.
                    if crate::screen_window::is_open()
                        || request.as_ref().is_some_and(|h| !h.is_finished())
                    {
                        continue;
                    }
                    let (node, log, settings) = (node.clone(), log.clone(), settings.clone());
                    let ui: Arc<dyn Fn(UiEvent) + Send + Sync> = ui.clone();
                    *request = Some(
                        tokio::spawn(show_screen(node, id, name, log, ui, settings, false))
                            .abort_handle(),
                    );
                }
                Some(Cmd::CancelScreen) => {
                    if cancel_request(&SCREEN_REQUEST) {
                        log.line("SCREEN request cancelled by the user");
                        ui(UiEvent::Info("You cancelled the screen request".into()));
                    }
                }
                Some(Cmd::StartCamera) => {
                    let target = default_target(&peers, &settings, None);
                    let Some((id, name)) = target else {
                        ui(UiEvent::Info("Pair a phone first".into()));
                        continue;
                    };
                    let mut request = CAMERA_REQUEST.lock().unwrap_or_else(|p| p.into_inner());
                    if crate::camera::is_running() || request.as_ref().is_some_and(|h| !h.is_finished()) {
                        continue;
                    }
                    let (node, log) = (node.clone(), log.clone());
                    let ui: Arc<dyn Fn(UiEvent) + Send + Sync> = ui.clone();
                    *request = Some(tokio::spawn(show_camera(node, id, name, log, ui, false)).abort_handle());
                }
                Some(Cmd::StopCamera) => {
                    // Stop also cancels a request the phone has not answered yet.
                    if let Some(h) = CAMERA_REQUEST.lock().unwrap_or_else(|p| p.into_inner()).take() {
                        if !h.is_finished() {
                            h.abort();
                            log.line("CAMERA request cancelled by the user");
                        }
                    }
                    log.line("CAMERA stopped by the user");
                    if crate::camera::is_streaming() {
                        if let Some((id, _)) = default_target(&peers, &settings, None) {
                            touch(&id);
                        }
                    }
                    tokio::task::spawn_blocking(crate::camera::stop);
                }
                Some(Cmd::StartAudio(d)) => {
                    let target = default_target(&peers, &settings, None);
                    let Some((id, name)) = target else {
                        ui(UiEvent::Info("Pair a phone first".into()));
                        continue;
                    };
                    let ui: Arc<dyn Fn(UiEvent) + Send + Sync> = ui.clone();
                    if !crate::audio::start(node.clone(), id, name, d, log.clone(), ui, settings.clone()) {
                        log.line("AUDIO start ignored: already asking or running");
                    }
                }
                Some(Cmd::StopAudio(d)) => {
                    log.line("AUDIO stop requested by the user");
                    crate::audio::stop(d);
                }
                Some(Cmd::SwitchCameraLens) => crate::camera::switch_lens(),
                Some(Cmd::RepairCamera) => {
                    let (log, ui) = (log.clone(), ui.clone());
                    tokio::spawn(async move {
                        // Nothing may hold the old files while they are replaced.
                        let _ = tokio::task::spawn_blocking(crate::camera::stop).await;
                        if install_camera(&log, &*ui).await {
                            crate::camera::set_last_error(None);
                        }
                    });
                }
                Some(Cmd::RemoveCamera) => {
                    let (log, ui) = (log.clone(), ui.clone());
                    tokio::task::spawn_blocking(move || remove_camera(&log, &*ui));
                }
                Some(Cmd::ControlPhone) => {
                    // A second press while the phone is still asking cancels the request.
                    if let Some(h) = CONTROL_REQUEST.lock().unwrap_or_else(|p| p.into_inner()).take() {
                        if !h.is_finished() {
                            h.abort();
                            log.line("CONTROL request cancelled");
                            ui(UiEvent::Info("You cancelled the control request".into()));
                            continue;
                        }
                    }
                    let (node, log, ui) = (node.clone(), log.clone(), ui.clone());
                    let target = default_target(&peers, &settings, None);
                    let Some((id, name)) = target else {
                        ui(UiEvent::Info("Pair a phone first".into()));
                        continue;
                    };
                    ui(UiEvent::Info(format!(
                        "Connecting to {name}\u{2026} Ctrl+Alt+P cancels"
                    )));
                    let task = tokio::spawn(async move {
                        let asked = {
                            let (ui, name) = (ui.clone(), name.clone());
                            move || {
                                ui(UiEvent::Info(format!(
                                    "Asking {name} to allow control\u{2026} Ctrl+Alt+P cancels"
                                )))
                            }
                        };
                        match dial(&node, &id, node.open_input_with(&id, asked)).await {
                            Ok(sender) => {
                                seen(&node, &id);
                                log.line(&format!("CONTROL phone \"{name}\" started"));
                                ui(UiEvent::ControlReady { name, sender });
                            }
                            Err(e) => {
                                log.line(&format!("CONTROL phone \"{name}\" refused: {e}"));
                                let why = match e {
                                    pb_transport::Error::Declined => format!(
                                        "{name} didn't allow control. If it didn't ask you, turn on Relay control on the phone"
                                    ),
                                    pb_transport::Error::Timeout => {
                                        format!("{name} didn't answer your control request")
                                    }
                                    e => crate::transfers::session_failure(&e, &name)
                                        .unwrap_or_else(|| format!("Couldn't reach {name} to control it")),
                                };
                                ui(UiEvent::Info(why));
                            }
                        }
                    });
                    *CONTROL_REQUEST.lock().unwrap_or_else(|p| p.into_inner()) = Some(task.abort_handle());
                }
                Some(Cmd::SendText { text, link }) => {
                    let (node, log, ui, settings) =
                        (node.clone(), log.clone(), ui.clone(), settings.clone());
                    let target = default_target(&peers, &settings, None);
                    tokio::spawn(async move {
                        let Some((id, name)) = target else {
                            ui(UiEvent::Info("Pair a device first".into()));
                            return;
                        };
                        match dial(&node, &id, node.send_text(&id, &text, link)).await {
                            Ok(()) => {
                                seen(&node, &id);
                                if !link {
                                    record(
                                        &settings,
                                        crate::timeline::clip_text_entry(&name, crate::timeline::Dir::ToPhone, &text, false),
                                    );
                                }
                                log.line(&format!("SHARE sent {} to \"{name}\"", if link { "link" } else { "text" }));
                                ui(UiEvent::Info(format!("Sent to {name}")));
                            }
                            Err(e) => {
                                log.line(&format!("SHARE failed: {e}"));
                                ui(UiEvent::Info(format!("Could not send to {name}: {e}")));
                            }
                        }
                    });
                }
                Some(Cmd::SendClipboard(text)) => {
                    let (node, log, ui, settings) =
                        (node.clone(), log.clone(), ui.clone(), settings.clone());
                    let target = default_target(&peers, &settings, Some(SendKind::Clipboard));
                    tokio::spawn(async move {
                        let Some((id, name)) = target else {
                            ui(UiEvent::Info("Pair a device first".into()));
                            return;
                        };
                        if text.len() > MAX_CLIPBOARD_TEXT {
                            ui(UiEvent::Info("Clipboard text is too large to send".into()));
                            return;
                        }
                        let result = async {
                            let s = dial(&node, &id, node.connect(&id)).await?;
                            s.send(ControlMsg::ClipboardText(ClipboardText { text: text.clone(), sensitive: false })).await?;
                            s.finish().await;
                            Ok::<_, pb_transport::Error>(())
                        }
                        .await;
                        match result {
                            Ok(()) => {
                                seen(&node, &id);
                                record(
                                    &settings,
                                    crate::timeline::clip_text_entry(&name, crate::timeline::Dir::ToPhone, &text, false),
                                );
                                log.line(&format!("SENT {} bytes to \"{name}\"", text.len()));
                                ui(UiEvent::Info(format!("Sent to {name}")));
                            }
                            Err(e) => {
                                log.line(&format!("SEND failed: {e}"));
                                ui(UiEvent::Info(format!("Could not send to {name}: {e}")));
                            }
                        }
                    });
                }
            },
            Some(p) = ev.pairings.recv() => {
                // Someone joined with a valid code: stop issuing new ones.
                if let Some(r) = rotation.take() {
                    r.abort();
                }
                node.close_pairing();
                if settings.lock().map(|s| s.paused).unwrap_or(false) {
                    log.line("PAIRING refused: Relay is paused");
                    continue;
                }
                next_id += 1;
                log.line(&format!("SAS={} peer=\"{}\"", p.sas, p.peer_name));
                ui(UiEvent::Sas { id: next_id, peer_name: p.peer_name.clone(), sas: p.sas.clone() });
                let (gone, id, tx) = (p.withdrawn(), next_id, gone_tx.clone());
                tokio::spawn(async move {
                    gone.await;
                    let _ = tx.send(id);
                });
                pending.insert(next_id, p);
            }
            Some(id) = gone_rx.recv() => {
                // Only while the user has not decided: a decided prompt is already out of `pending`.
                if pending.remove(&id).is_some() {
                    log.line(&format!("PAIRING prompt {id} withdrawn by the phone"));
                    ui(UiEvent::SasWithdrawn { id });
                }
            }
            Some(s) = ev.sessions.recv() => {
                if settings.lock().map(|x| x.paused).unwrap_or(false) {
                    s.closer().close(b"paused");
                    log.line("SESSION refused: Relay is paused");
                    continue;
                }
                // Their Relay version, remembered per device (empty from Relay 0.2.0: nothing stored).
                if !s.remote_hello.app_version.is_empty() {
                    if let Ok(mut d) = devices.lock() {
                        if d.set(&s.peer.to_string(), "app_version", &s.remote_hello.app_version) {
                            let _ = d.save(&devices_path);
                        }
                    }
                }
                tokio::spawn(pump(s, node.clone(), log.clone(), ui.clone(), settings.clone(), clip.clone(), images.clone()));
            }
        }
    }
    node.shutdown().await;
    Ok(())
}

fn start_rotation(
    node: &Node,
    log: &Log,
    ui: &Arc<impl Fn(UiEvent) + Send + Sync + 'static>,
) -> tokio::task::JoinHandle<()> {
    let (node, log, ui) = (node.clone(), log.clone(), ui.clone());
    tokio::spawn(async move {
        for round in 0..ROUNDS {
            match node.open_pairing_for(node.addrs(), CODE_TTL) {
                Ok(code) => {
                    log.line(&format!("CODE round={} {code}", round + 1));
                    ui(UiEvent::Code {
                        code: code.to_string(),
                        refresh_in: ROTATE,
                        closes_in: ROTATE * (ROUNDS - round),
                    });
                }
                Err(e) => {
                    ui(UiEvent::Info(format!("Could not start pairing: {e}")));
                    return;
                }
            }
            tokio::time::sleep(ROTATE).await;
        }
        node.close_pairing();
        log.line("PAIRING window expired; code revoked");
        ui(UiEvent::PairingClosed);
    })
}

/// Sends `paths` one after the other as one batch ("2 of 5"). A cancel or
/// a pause stops the files that have not started.
async fn send_batch(
    node: &Node,
    id: &PeerId,
    name: &str,
    paths: Vec<PathBuf>,
    log: &Log,
    ui: &(impl Fn(UiEvent) + ?Sized),
    settings: &SharedSettings,
) {
    let key = id.to_string();
    for path in paths {
        let file_name = file_name_of(&path);
        let size = tokio::fs::metadata(&path)
            .await
            .map(|m| m.len())
            .unwrap_or(0);
        let added = queues().lock().ok().map(|mut q| {
            q.of(&key).enqueue(
                path,
                file_name.clone(),
                size,
                pb_transport::files::new_resume_token(),
            )
        });
        let paused = queues()
            .lock()
            .map(|mut q| q.of(&key).is_paused())
            .unwrap_or(false);
        if matches!(added, Some(Ok(_))) && paused {
            ui(UiEvent::Info(format!(
                "The queue is paused: {file_name} will go to {name} when you choose Resume queue on the Transfers page"
            )));
        }
        if !matches!(added, Some(Ok(_))) {
            ui(UiEvent::Info(format!(
                "{name} already has {} files waiting; {file_name} was not added",
                crate::queue::MAX_PER_DEVICE
            )));
        }
    }
    save_queues();
    drain_queue(node, id, name, log, ui, settings).await;
}

/// Sends the waiting items of one phone, one after the other. Several callers may run this at
/// once: `next()` hands out an item only while none is sending, so the sends never overlap.
async fn drain_queue(
    node: &Node,
    id: &PeerId,
    name: &str,
    log: &Log,
    ui: &(impl Fn(UiEvent) + ?Sized),
    settings: &SharedSettings,
) {
    use crate::transfers as tr;
    let key = id.to_string();
    let batch = tr::new_batch_id();
    loop {
        if tr::batch_stopped(batch) {
            break;
        }
        let Some(item) = queues().lock().ok().and_then(|mut q| q.of(&key).next()) else {
            break;
        };
        let total = queues()
            .lock()
            .map(|q| q.get(&key).map_or(1, |s| s.pending().max(1)))
            .unwrap_or(1);
        let (row, cancel) = tr::start(tr::NewTransfer {
            dir: tr::Dir::Send,
            peer: name.to_owned(),
            name: item.name.clone(),
            size: item.size,
            path: Some(item.path.clone()),
            batch: Some(tr::Batch {
                id: batch,
                index: 1,
                total,
            }),
        });
        let token_key = (name.to_owned(), item.path.clone());
        if let Ok(mut t) = retry_tokens().lock() {
            t.insert(token_key.clone(), item.token);
        }
        let outcome = send_one_file(
            node, id, name, &item.path, row, cancel, item.token, log, ui, settings,
        )
        .await;
        let mut stop = false;
        // A pause puts the file back in the queue: the stopped transfer's own row would list it twice.
        let mut drop_row = false;
        let mut held_one = false;
        if let Ok(mut q) = queues().lock() {
            match outcome {
                SendOutcome::Done => q.of(&key).finish(item.id),
                // Relay itself stopped it (Pause, Quit): the file keeps its place and token, and the
                // whole queue waits for the person to resume.
                SendOutcome::Cancelled("pause" | "quit") => {
                    // One file held by the person: it is parked and the queue goes on without it.
                    drop_row = outcome == SendOutcome::Cancelled("pause");
                    if q.of(&key).stopped_for_hold(item.id) {
                        held_one = true;
                    } else {
                        q.of(&key).retry(item.id);
                        q.set_all_paused(true);
                        stop = true;
                    }
                }
                // The person pressed Cancel on this file, or the other device cancelled it: only
                // that file goes, the rest is still wanted ("Cancel all" clears the queue first).
                SendOutcome::Cancelled(_) => {
                    q.of(&key).cancel(item.id);
                }
                SendOutcome::Failed => q.of(&key).fail(item.id),
            }
        }
        if outcome == SendOutcome::Done {
            if let Ok(mut t) = retry_tokens().lock() {
                t.remove(&token_key);
            }
        }
        if held_one {
            // Holding one file is not Relay pausing: the batch goes on with the next.
            tr::unstop_batch(batch);
        }
        if drop_row {
            tr::dismiss(row);
        }
        save_queues();
        if stop {
            break;
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SendOutcome {
    Done,
    /// Why: "user" (Cancel), "peer" (the other device), "pause" or "quit" (Relay itself stopped it).
    Cancelled(&'static str),
    Failed,
}

/// Bytes this attempt moved, from the row's progress before and after it. Progress carries over from an
/// earlier attempt, so only a change means the other device accepted this time; a restart at zero also counts.
fn attempt_bytes(before: u64, after: u64) -> u64 {
    if after == before {
        0
    } else {
        after.max(1)
    }
}

/// The resume token of each file by phone and path, so the Retry button on a failed row continues the
/// transfer instead of sending the whole file again.
type RetryTokens = Mutex<std::collections::HashMap<(String, PathBuf), [u8; 16]>>;

fn retry_tokens() -> &'static RetryTokens {
    static T: std::sync::OnceLock<RetryTokens> = std::sync::OnceLock::new();
    T.get_or_init(Mutex::default)
}

fn queues() -> &'static Mutex<crate::queue::Queues> {
    static Q: std::sync::OnceLock<Mutex<crate::queue::Queues>> = std::sync::OnceLock::new();
    Q.get_or_init(|| Mutex::new(crate::queue::Queues::default()))
}

/// Pauses or resumes every phone's queue: the next file waits, the one sending finishes.
pub fn queue_set_paused(paused: bool) {
    if let Ok(mut q) = queues().lock() {
        q.set_all_paused(paused);
    }
}

/// What the Transfers page lists for the queue: files that wait behind the one sending, are paused, or
/// lost their source. The file sending is in the transfer registry, not here.
pub fn queue_rows() -> Vec<crate::pages::transfers::QueuedRow> {
    use crate::queue::QueueState;
    let Ok(q) = queues().lock() else {
        return Vec::new();
    };
    q.rows_all()
        .into_iter()
        .filter(|(_, i)| {
            !matches!(
                i.state,
                QueueState::Sending | QueueState::Done | QueueState::Failed
            )
        })
        .map(|(peer, i)| crate::pages::transfers::QueuedRow {
            held: q.get(&peer).is_some_and(|s| {
                s.items()
                    .iter()
                    .any(|x| x.id == i.id && x.state == QueueState::Paused)
            }),
            needs_file: i.state == QueueState::NeedsFile,
            peer,
            id: i.id,
            status: match i.state {
                QueueState::Paused => "Paused".to_owned(),
                QueueState::NeedsFile => "Needs the file again".to_owned(),
                _ => "Waiting".to_owned(),
            },
            name: i.name,
        })
        .collect()
}

/// True while any queue is paused (the Transfers page shows "Resume").
pub fn queue_paused() -> bool {
    queues().lock().map(|q| q.any_paused()).unwrap_or(false)
}

static QUEUE_FILE: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();

/// Writes `queue.txt` (unfinished items, tokens included) next to the other private state.
fn save_queues() {
    let (Some(path), Ok(q)) = (QUEUE_FILE.get(), queues().lock()) else {
        return;
    };
    let _ = std::fs::write(path, q.save());
}

fn file_name_of(path: &std::path::Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "file".into())
}

/// One tracked send: progress and cancel go through the transfer registry;
/// every outcome ends in the registry (the window) and a toast.
#[allow(clippy::too_many_arguments)]
async fn send_one_file(
    node: &Node,
    id: &PeerId,
    name: &str,
    path: &std::path::Path,
    row: u64,
    cancel: pb_transport::files::Cancel,
    token: [u8; 16],
    log: &Log,
    ui: &(impl Fn(UiEvent) + ?Sized),
    settings: &SharedSettings,
) -> SendOutcome {
    use crate::transfers as tr;
    let file_name = file_name_of(path);
    let started = Instant::now();
    let mut total = 0u64;
    let result = async {
        let f = tokio::fs::File::open(path)
            .await
            .map_err(|e| pb_transport::Error::Io(e.to_string()))?;
        let size = f
            .metadata()
            .await
            .map_err(|e| pb_transport::Error::Io(e.to_string()))?
            .len();
        total = size;
        log.line(&format!(
            "FILE send start \"{file_name}\" {size} bytes to \"{name}\""
        ));
        crate::applog::emit(crate::applog::file_started("sent", &file_name, size));
        drop(f);
        // The queue item's token serves every attempt of this file: after a broken link the receiver
        // keeps what it got and the next attempt sends only the rest (it falls back to the whole file when the
        // receiver kept nothing or the file changed).
        let mut attempt = 0u32;
        loop {
            let opts = TransferOpts::new(Some(tr::progress_cb(row)), Some(cancel.clone()));
            // Progress is for the whole file across attempts; whether THIS attempt moved is what says the
            // other device accepted. A prompt left unanswered ends in a timeout with nothing moved.
            let before = tr::progress_of(row).map_or(0, |(d, _)| d);
            let p = path.to_path_buf();
            let r = dial(
                node,
                id,
                node.send_file_resumable(
                    id,
                    &file_name,
                    size,
                    "",
                    move || std::fs::File::open(&p).map(tokio::fs::File::from_std),
                    token,
                    opts,
                ),
            )
            .await;
            match r {
                Ok(_) => break,
                Err(e)
                    if crate::queue::should_retry(
                        matches!(
                            e,
                            pb_transport::Error::Transport(_)
                                | pb_transport::Error::Closed
                                | pb_transport::Error::Timeout
                        ),
                        attempt,
                        cancel.is_cancelled(),
                        attempt_bytes(before, tr::progress_of(row).map_or(0, |(d, _)| d)),
                    ) =>
                {
                    attempt += 1;
                    log.line(&format!(
                        "FILE send \"{file_name}\" interrupted, retry {attempt} of {}",
                        crate::queue::MAX_RETRIES
                    ));
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                }
                Err(e) => return Err(e),
            }
        }
        Ok::<_, pb_transport::Error>(size)
    }
    .await;
    let took = started.elapsed();
    let mut outcome = SendOutcome::Done;
    match result {
        Ok(size) => {
            seen(node, id);
            tr::finish(row, None);
            let mbps = size as f64 / 1e6 / took.as_secs_f64().max(1e-3);
            log.line(&format!(
                "FILE sent \"{file_name}\" {size} bytes to \"{name}\" in {:.2}s ({mbps:.1} MB/s)",
                took.as_secs_f64()
            ));
            record(
                settings,
                crate::timeline::file_entry(
                    name,
                    crate::timeline::Dir::ToPhone,
                    &file_name,
                    size,
                    Some(path),
                ),
            );
            ui(UiEvent::Info(format!("Sent {file_name} to {name}")));
        }
        Err(e) => {
            let done = tr::progress_of(row).map_or(0, |(d, _)| d);
            let mbps = done as f64 / 1e6 / took.as_secs_f64().max(1e-3);
            match e {
                pb_transport::Error::Cancelled | pb_transport::Error::PeerCancelled => {
                    let by = if matches!(e, pb_transport::Error::PeerCancelled) {
                        tr::cancel(row, tr::CancelBy::Peer);
                        "peer"
                    } else {
                        tr::cancel_reason(row)
                    };
                    outcome = SendOutcome::Cancelled(by);
                    log.line(&format!(
                        "FILE send \"{file_name}\" cancelled by {by} after {done} of {total} bytes ({mbps:.1} MB/s)"
                    ));
                    crate::applog::emit(crate::applog::file_cancelled(
                        "sent",
                        &file_name,
                        done,
                        by,
                        took.as_secs_f64(),
                        mbps,
                    ));
                    if by == "peer" {
                        ui(UiEvent::Info(format!("{name} cancelled {file_name}")));
                    }
                }
                e => {
                    outcome = SendOutcome::Failed;
                    log.line(&format!("FILE send \"{file_name}\" failed: {e}"));
                    let f = tr::describe(
                        tr::Dir::Send,
                        &e,
                        &tr::Ctx {
                            peer: name,
                            file: &file_name,
                            folder: "",
                            refusal: None,
                        },
                    );
                    // A pause or cancel that raced the failure keeps its own wording.
                    let live = tr::fail(row, f.clone());
                    if live {
                        ui(UiEvent::Info(format!("{}. {}", f.title, f.hint)));
                    }
                }
            }
        }
    }
    outcome
}

/// Runs a session with the "reinstalled or reset" note following the
/// transport on open and close (`with_session_notes`).
async fn pump(
    s: Session,
    node: Node,
    log: Log,
    ui: Arc<impl Fn(UiEvent) + Send + Sync + 'static>,
    settings: SharedSettings,
    clip: Arc<Mutex<ClipSync>>,
    images: Arc<ToastImages>,
) {
    let peer = s.peer;
    // A pause ends it from outside.
    let _registered = crate::pause::register(s.closer());
    with_session_notes(
        &node,
        &peer,
        pump_session(s, log, ui, settings, clip, images),
    )
    .await;
}

async fn pump_session(
    mut s: Session,
    log: Log,
    ui: Arc<impl Fn(UiEvent) + Send + Sync + 'static>,
    settings: SharedSettings,
    clip: Arc<Mutex<ClipSync>>,
    images: Arc<ToastImages>,
) {
    log.line(&format!("SESSION \"{}\"", s.peer_name));
    let _open = OpenSession::new(s.peer, s.remote_addr());
    let mut notify_rate =
        pb_transport::notify::RateWindow::new(pb_transport::notify::MAX_NOTIFY_PER_SEC);
    loop {
        let msg = s.recv().await;
        if matches!(
            msg,
            Ok(Some(
                ControlMsg::Notification(_)
                    | ControlMsg::NotifyIcon(_)
                    | ControlMsg::NotifyRemoved(_)
                    | ControlMsg::NotifySync(_)
            ))
        ) && !notify_rate.hit()
        {
            log.line(&format!(
                "NOTIFY flood from \"{}\": session closed",
                s.peer_name
            ));
            s.closer().close(b"notify flood");
            break;
        }
        match msg {
            Ok(Some(ControlMsg::NotifyIcon(i))) => {
                images.icon(&i.hash, &i.png);
            }
            Ok(Some(ControlMsg::Notification(n))) => {
                let tag = crate::notify::tag(&s.peer, &n.key);
                let logo = images.icon_path(&n.icon_hash);
                let picture = (!n.picture.is_empty())
                    .then(|| images.picture(&tag, &n.picture))
                    .flatten();
                log.line(&crate::notify::posted_log_line(&n.package, &n.key));
                ui(UiEvent::Notify {
                    peer: s.peer,
                    from: s.peer_name.clone(),
                    n: Box::new(n),
                    logo,
                    picture,
                });
            }
            Ok(Some(ControlMsg::NotifyRemoved(r))) => ui(UiEvent::NotifyRemoved {
                peer: s.peer,
                key: r.key,
            }),
            Ok(Some(ControlMsg::NotifySync(x))) => ui(UiEvent::NotifySync {
                peer: s.peer,
                keys: x.keys,
            }),
            Ok(Some(ControlMsg::ClipboardText(c))) => {
                if c.sensitive {
                    log.line(&format!("CLIP from \"{}\" (sensitive)", s.peer_name));
                } else {
                    log.line(&format!(
                        "CLIP from \"{}\" ({} chars)",
                        s.peer_name,
                        c.text.chars().count()
                    ));
                }
                record(
                    &settings,
                    crate::timeline::clip_text_entry(
                        &s.peer_name,
                        crate::timeline::Dir::ToPc,
                        &c.text,
                        c.sensitive,
                    ),
                );
                ui(UiEvent::Clipboard {
                    from: s.peer_name.clone(),
                    text: c.text,
                    sensitive: c.sensitive,
                });
            }
            Ok(Some(ControlMsg::ClipboardPullResult(r))) => {
                // Only the phone we asked may complete this request.
                if !PULL
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .answered_from(&s.peer.to_string())
                {
                    log.line("PULL ignored an unsolicited clipboard result");
                    continue;
                }
                log.line(&format!(
                    "PULL answer from \"{}\": {}",
                    s.peer_name, r.reason
                ));
                let reason = if r.reason.is_empty() {
                    "The phone sent nothing".to_owned()
                } else {
                    r.reason
                };
                ui(UiEvent::Pill(reason, 6));
            }
            Ok(Some(ControlMsg::ClipboardUpdate(u))) => {
                // A clip the PC asked for is wanted even with Universal Clipboard off.
                let pulled = {
                    let mut g = PULL.lock().unwrap_or_else(|p| p.into_inner());
                    let wanted = g.pending_from(&s.peer.to_string(), std::time::Instant::now());
                    g.answered_from(&s.peer.to_string());
                    wanted
                };
                if !pulled
                    && !settings
                        .lock()
                        .map(|x| x.universal_clipboard)
                        .unwrap_or(false)
                {
                    log.line("CLIPSYNC ignored (Universal Clipboard is off)");
                    continue;
                }
                clip.lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .text
                    .received(clip_hash(&u.text), std::time::Instant::now());
                if u.sensitive {
                    log.line(&format!(
                        "CLIPSYNC received a sensitive clip from \"{}\"",
                        s.peer_name
                    ));
                } else {
                    log.line(&format!(
                        "CLIPSYNC received {} chars from \"{}\"",
                        u.text.chars().count(),
                        s.peer_name
                    ));
                }
                record(
                    &settings,
                    crate::timeline::clip_text_entry(
                        &s.peer_name,
                        crate::timeline::Dir::ToPc,
                        &u.text,
                        u.sensitive,
                    ),
                );
                ui(UiEvent::RemoteClipboard {
                    from: s.peer_name.clone(),
                    text: u.text,
                    sensitive: u.sensitive,
                });
            }
            Ok(Some(ControlMsg::InputStart(start))) => {
                let from = s.peer_name.clone();
                let peer_hex = s.peer.to_string();
                let perm = perms_of(&settings, &peer_hex).control;
                let mut allow = perm == crate::devices::Allow::Always;
                if perm == crate::devices::Allow::Never {
                    log.line(&format!("INPUT from \"{from}\" refused: set to Never"));
                } else if !allow {
                    // First time: ask. "Yes" remembers it for this phone only (Settings can turn it off).
                    let (tx, rx) = oneshot::channel();
                    ui(UiEvent::AskControl {
                        from: from.clone(),
                        peer_id: s.peer.to_string(),
                        reply: tx,
                    });
                    allow = tokio::time::timeout(std::time::Duration::from_secs(60), rx)
                        .await
                        .ok()
                        .and_then(Result::ok)
                        .unwrap_or(false);
                    if allow {
                        update_devices(|d| {
                            let mut p = d.perms(&peer_hex);
                            p.control = crate::devices::Allow::Always;
                            d.set_perms(&peer_hex, &p)
                        });
                        sync_mirror(&settings);
                    }
                }
                log.line(&format!("INPUT from \"{from}\" allowed={allow}"));
                if allow {
                    ui(UiEvent::Info(format!("{from} is controlling this PC")));
                }
                if allow {
                    *PHONE_CONTROL.lock().unwrap_or_else(|p| p.into_inner()) = Some(s.closer());
                }
                let mut injector = pb_platform_win::input::Injector::default();
                let started = Instant::now();
                let mut n = 0u64;
                let r = pb_transport::input::receive_input(&mut s, start, allow, |e| {
                    n += 1;
                    injector.inject(e);
                })
                .await;
                drop(injector); // releases anything still held
                PHONE_CONTROL
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .take();
                if allow {
                    log.line(&format!(
                        "INPUT from \"{from}\" ended after {:.1}s, {n} events: {r:?}",
                        started.elapsed().as_secs_f32()
                    ));
                    ui(UiEvent::Info(format!("{from} stopped controlling this PC")));
                }
                break;
            }
            Ok(Some(ControlMsg::Offer(offer))) => {
                let auto = perms_of(&settings, &s.peer.to_string()).auto_accept;
                let (dir, allow_shortcuts) = settings
                    .lock()
                    .map(|x| (x.save_dir.clone(), x.allow_shortcut_files))
                    .unwrap_or((std::env::temp_dir(), false));
                let is_image = offer.kind == pb_protocol::control::OfferKind::ClipboardImage as i32;
                let is_file = offer.kind == pb_protocol::control::OfferKind::File as i32;
                let file_label = pb_transport::files::sanitize_file_name(&offer.name);
                let action = if crate::shared_text::bypasses_file_prompt(offer.kind) {
                    crate::motw::OfferAction::Accept
                } else {
                    crate::motw::offer_action(
                        auto,
                        if is_file {
                            crate::motw::risk(&file_label)
                        } else {
                            pb_transport::files::FileRisk::Normal
                        },
                        allow_shortcuts,
                    )
                };
                let refusal = is_file
                    .then(|| pb_transport::files::refusal(&dir, offer.size, DEFAULT_MAX_FILE))
                    .flatten();
                // A phone over its quota is refused before anything asks the person; the reservation is held
                // until the file is done.
                let permit = is_file
                    .then(|| s.offer_permit(offer.size, pb_transport::files::free_bytes(&dir)));
                let over_quota = permit.as_ref().and_then(|r| r.as_ref().err().copied());
                let _permit = permit.and_then(|r| r.ok());
                let mut expired = false;
                let accept = if let Some(why) = over_quota {
                    log.line(&format!(
                        "FILE refused: over quota ({why:?}) from \"{}\"",
                        s.peer_name
                    ));
                    ui(UiEvent::Info(match why {
                        pb_transport::quota::QuotaRefusal::NotEnoughSpace => format!(
                            "Relay refused a file from {}: its files would not fit on this disk",
                            s.peer_name
                        ),
                        _ => format!(
                            "Relay refused files from {}: too many requests",
                            s.peer_name
                        ),
                    }));
                    false
                } else if is_image {
                    // A clipboard picture is governed by the sharing switches, not
                    // the file prompt, and is never written to the save folder.
                    let on = settings
                        .lock()
                        .map(|x| crate::clip_image::enabled(x.universal_clipboard, x.share_images))
                        .unwrap_or(false);
                    if !on {
                        log.line("CLIPSYNC image ignored (image sharing is off)");
                    }
                    on
                } else if refusal.is_some() {
                    // Too big or no room: say so now, never ask "Save it?".
                    false
                } else if action == crate::motw::OfferAction::Refuse {
                    log.line(&format!("FILE refused a shortcut file \"{file_label}\""));
                    ui(UiEvent::Info(format!(
                        "Relay refused a shortcut file from {}: {file_label}",
                        s.peer_name
                    )));
                    false
                } else if action == crate::motw::OfferAction::Accept {
                    true
                } else {
                    let (tx, mut rx) = oneshot::channel();
                    ui(UiEvent::IncomingOffer {
                        from: s.peer_name.clone(),
                        peer_id: s.peer.to_string(),
                        name: file_label.clone(),
                        size: offer.size,
                        folder: dir.clone(),
                        reply: OfferReply::new(tx),
                    });
                    // Leaving this block drops `rx`: the dialog sees the
                    // offer is dead and closes itself.
                    tokio::select! {
                        r = &mut rx => r.unwrap_or(false),
                        _ = tokio::time::sleep(pb_transport::files::OFFER_TIMEOUT) => {
                            expired = true;
                            false
                        }
                        // Paused, or the phone went away while we asked.
                        _ = s.closed() => false,
                    }
                };
                let from = s.peer_name.clone();
                let started = Instant::now();
                let tracked = (is_file && (accept || refusal.is_some())).then(|| {
                    tr::start(tr::NewTransfer {
                        dir: tr::Dir::Receive,
                        peer: from.clone(),
                        name: file_label.clone(),
                        size: offer.size,
                        path: None,
                        batch: None,
                    })
                });
                if let (Some((row, _)), true) = (&tracked, accept) {
                    log.line(&format!(
                        "FILE receive start \"{file_label}\" {} bytes from \"{from}\"",
                        offer.size
                    ));
                    crate::applog::emit(crate::applog::file_started(
                        "received",
                        &file_label,
                        offer.size,
                    ));
                    let _ = row;
                }
                let mut opts = TransferOpts::new(
                    tracked.as_ref().map(|(row, _)| tr::progress_cb(*row)),
                    tracked.as_ref().map(|(_, c)| c.clone()),
                );
                if let Some(r) = RESUME.get() {
                    opts = opts.with_resume(r.clone());
                }
                let offer_size = offer.size;
                let pulled_image = if is_image && accept {
                    PULL.lock()
                        .unwrap_or_else(|p| p.into_inner())
                        .image_started(std::time::Instant::now(), &s.peer.to_string())
                } else {
                    None
                };
                let outcome =
                    receive_offer_with(&mut s, offer, accept, &dir, DEFAULT_MAX_FILE, opts).await;
                match outcome {
                    Ok(Received::File { path, size }) => {
                        // Downloaded-file treatment (SmartScreen, Protected View).
                        if let Err(e) = crate::motw::mark(&path) {
                            log.line(&format!("FILE not marked as from the internet: {e}"));
                        }
                        let mbps = size as f64 / 1e6 / started.elapsed().as_secs_f64().max(1e-3);
                        log.line(&format!(
                            "FILE received {} ({size} bytes, {mbps:.1} MB/s) from \"{from}\"",
                            path.display()
                        ));
                        let name = path
                            .file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_else(|| "file".into());
                        record(
                            &settings,
                            crate::timeline::file_entry(
                                &from,
                                crate::timeline::Dir::ToPc,
                                &name,
                                size,
                                Some(&path),
                            ),
                        );
                        if let Some((row, _)) = &tracked {
                            tr::finish(*row, Some(path.clone()));
                        }
                        ui(UiEvent::FileReceived { from, path, size });
                    }
                    Ok(Received::Text(t)) => {
                        record(
                            &settings,
                            crate::timeline::clip_text_entry(
                                &from,
                                crate::timeline::Dir::ToPc,
                                &t,
                                false,
                            ),
                        );
                        ui(UiEvent::SharedText { from, text: t })
                    }
                    Ok(Received::Link(l)) => {
                        log.line(&format!("LINK from \"{from}\""));
                        ui(UiEvent::Link { from, url: l });
                    }
                    Ok(Received::Image(png)) => {
                        log.line(&format!(
                            "CLIPSYNC received an image ({} bytes) from \"{from}\"",
                            png.len()
                        ));
                        record(
                            &settings,
                            crate::timeline::clip_image_entry(
                                &from,
                                crate::timeline::Dir::ToPc,
                                &png,
                            ),
                        );
                        clip.lock()
                            .unwrap_or_else(|p| p.into_inner())
                            .image
                            .received(&png);
                        if let Some(generation) = pulled_image {
                            PULL.lock()
                                .unwrap_or_else(|p| p.into_inner())
                                .image_finished(generation);
                        }
                        ui(UiEvent::RemoteImage { from, png });
                    }
                    Err(e) => {
                        if let Some(generation) = pulled_image {
                            if PULL
                                .lock()
                                .unwrap_or_else(|p| p.into_inner())
                                .image_finished(generation)
                            {
                                ui(UiEvent::Pill(
                                    "The phone's picture could not be received. Try again".into(),
                                    6,
                                ));
                            }
                        }
                        log.line(&format!("OFFER from \"{from}\" not received: {e}"));
                        let took = started.elapsed();
                        if let Some((row, _)) = &tracked {
                            let done = tr::progress_of(*row).map_or(0, |(d, _)| d);
                            let mbps = done as f64 / 1e6 / took.as_secs_f64().max(1e-3);
                            match e {
                                pb_transport::Error::Cancelled
                                | pb_transport::Error::PeerCancelled => {
                                    let by = if matches!(e, pb_transport::Error::PeerCancelled) {
                                        tr::cancel(*row, tr::CancelBy::Peer);
                                        ui(UiEvent::Info(format!("{from} cancelled {file_label}")));
                                        "peer"
                                    } else {
                                        tr::cancel_reason(*row)
                                    };
                                    log.line(&format!(
                                        "FILE receive \"{file_label}\" cancelled by {by} after {done} of {offer_size} bytes ({mbps:.1} MB/s)"
                                    ));
                                    crate::applog::emit(crate::applog::file_cancelled(
                                        "received",
                                        &file_label,
                                        done,
                                        by,
                                        took.as_secs_f64(),
                                        mbps,
                                    ));
                                }
                                e => {
                                    let f = tr::describe(
                                        tr::Dir::Receive,
                                        &e,
                                        &tr::Ctx {
                                            peer: &from,
                                            file: &file_label,
                                            folder: &dir.display().to_string(),
                                            refusal,
                                        },
                                    );
                                    // A pause that raced the failure keeps its wording.
                                    if tr::fail(*row, f.clone()) {
                                        ui(UiEvent::Info(format!("{}. {}", f.title, f.hint)));
                                    }
                                }
                            }
                        } else if expired && matches!(e, pb_transport::Error::Declined) {
                            ui(UiEvent::Info(format!(
                                "{from}'s file {file_label} wasn't saved: nobody answered in time. Send it again from the phone."
                            )));
                        }
                    }
                }
                break;
            }
            Ok(Some(_)) => {}
            Ok(None) => break,
            Err(e) => {
                log.line(&format!("SESSION \"{}\" ended: {e}", s.peer_name));
                break;
            }
        }
    }
}

use crate::toast_images::ToastImages;

/// Open the phone-screen window: ask the phone, then show its screen.
/// `again`: the link dropped a moment ago; retry quietly for 30 s (the
/// phone resumes without asking within its grace).
fn show_screen(
    node: Node,
    id: PeerId,
    name: String,
    log: Log,
    ui: Arc<dyn Fn(UiEvent) + Send + Sync>,
    settings: SharedSettings,
    again: bool,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> {
    Box::pin(async move {
        let (pinned, dim, sharp) = settings
            .lock()
            .map(|s| (s.screen_pinned, s.screen_dim, s.screen_sharp))
            .unwrap_or_default();
        let codecs = crate::screen_decode::supported_codecs();
        let start = pb_protocol::control::ScreenStart {
            id: 0,
            max_long_side: if sharp { 0 } else { 1920 },
            fps: 60,
            codecs: codecs.iter().map(|c| *c as u32).collect(),
            dim,
            sharp,
        };
        let deadline = Instant::now() + std::time::Duration::from_secs(30);
        loop {
            let t0 = Instant::now();
            // A retry after a lost link is for the dimmed "Reconnecting"
            // window: closed by the user, nothing is brought back.
            if again && !crate::screen_window::is_open() {
                return;
            }
            if !again {
                ui(UiEvent::Info(format!("Waiting for {name}\u{2026}")));
            }
            let asked = {
                let (ui, name, log) = (ui.clone(), name.clone(), log.clone());
                move || {
                    log.line(&format!(
                        "SCREEN asked after {} ms (connected)",
                        t0.elapsed().as_millis()
                    ));
                    if !again {
                        ui(UiEvent::Info(format!(
                            "Asking {name} to share its screen\u{2026}"
                        )))
                    }
                }
            };
            match dial(&node, &id, node.open_screen(&id, start.clone(), asked)).await {
                Ok(view) => {
                    seen(&node, &id);
                    log.line(&format!(
                        "SCREEN \"{name}\" sharing ({}x{}) accepted after {} ms",
                        view.accept.width,
                        view.accept.height,
                        t0.elapsed().as_millis()
                    ));
                    if again && !crate::screen_window::is_open() {
                        view.close();
                        return;
                    }
                    ui(UiEvent::ScreenShown);
                    let ended = {
                        let (node, log, ui, settings, name) = (
                            node.clone(),
                            log.clone(),
                            ui.clone(),
                            settings.clone(),
                            name.clone(),
                        );
                        let rt = tokio::runtime::Handle::current();
                        move |why: String| {
                            touch(&id);
                            log.line(&format!("SCREEN ended: {why}"));
                            if crate::screen_end::retries(&why) {
                                // Maybe only the link: try again for 30 s.
                                rt.spawn(show_screen(
                                    node.clone(),
                                    id,
                                    name.clone(),
                                    log.clone(),
                                    ui.clone(),
                                    settings.clone(),
                                    true,
                                ));
                            } else {
                                ui(UiEvent::Info(why));
                            }
                        }
                    };
                    let opts = crate::screen_window::WindowOpts {
                        log: Box::new({
                            let log = log.clone();
                            move |m: &str| log.line(m)
                        }),
                        name: name.clone(),
                        pinned,
                        dim,
                        codecs: codecs.clone(),
                        on_pin: Box::new({
                            let ui = ui.clone();
                            move |p| ui(UiEvent::ScreenPinned(p))
                        }),
                        on_dim: Box::new({
                            let ui = ui.clone();
                            move |d| ui(UiEvent::ScreenDim(d))
                        }),
                        on_files: Box::new({
                            let ui = ui.clone();
                            move |f| ui(UiEvent::ScreenFiles(f))
                        }),
                        on_ended: Box::new(ended),
                        on_closed: Box::new(move || touch(&id)),
                    };
                    crate::screen_window::open(view, opts, &tokio::runtime::Handle::current());
                    return;
                }
                Err(e)
                    if again
                        && !matches!(e, pb_transport::Error::Declined)
                        && Instant::now() < deadline =>
                {
                    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                }
                Err(e) => {
                    log.line(&format!("SCREEN \"{name}\" not shown: {e}"));
                    if again {
                        crate::screen_window::close();
                    }
                    ui(UiEvent::Info(match e {
                        _ if again => format!("{name} stopped sharing its screen"),
                        pb_transport::Error::Declined => format!("{name} didn't share its screen"),
                        pb_transport::Error::Timeout => format!("{name} didn't answer"),
                        e => crate::transfers::session_failure(&e, &name)
                            .unwrap_or_else(|| format!("Couldn't reach {name} to show its screen")),
                    }));
                    return;
                }
            }
        }
    })
}

/// Remove the camera from this PC: stop streaming, sweep leftovers, then
/// take the installed files and keys away (one UAC prompt).
fn remove_camera(log: &Log, ui: &(dyn Fn(UiEvent) + Send + Sync)) {
    use crate::camera_register::RegisterError;
    crate::camera::stop();
    crate::camera_device::sweep_leftovers(&|m| log.line(m));
    if !crate::camera_register::is_registered() {
        ui(UiEvent::Info("The phone camera is not installed".into()));
        return;
    }
    ui(UiEvent::Info(
        "Removing the phone camera: Windows will ask once for permission".into(),
    ));
    match crate::camera_register::request_unregister() {
        Ok(()) => {
            log.line("CAMERA removed from this PC");
            ui(UiEvent::Info("The phone camera was removed".into()));
        }
        Err(RegisterError::Declined) => {
            ui(UiEvent::Info("The phone camera was not removed".into()));
        }
        Err(RegisterError::Failed(c)) => {
            log.line(&format!("CAMERA remove failed: code {c}"));
            ui(UiEvent::Info(format!(
                "Couldn't remove the phone camera (code {c})"
            )));
        }
    }
}

/// Install the camera source (one UAC prompt). False when it did not work;
/// the reason is told to the user and kept for the camera window.
async fn install_camera(log: &Log, ui: &(dyn Fn(UiEvent) + Send + Sync)) -> bool {
    use crate::camera_register::RegisterError;
    let Some(installing) = crate::camera::InstallGuard::try_new() else {
        ui(UiEvent::Info(
            "The phone camera is already being set up: answer the Windows prompt".into(),
        ));
        return false;
    };
    ui(UiEvent::Info(
        "Setting up the phone camera: Windows will ask once for permission".into(),
    ));
    let sid = crate::ipc::user_sid().unwrap_or_default();
    // The guard moves into the job: aborting this task does not stop the
    // elevated installer, so it must not free the guard either.
    let r = tokio::task::spawn_blocking(move || {
        crate::camera::holding(installing, || {
            crate::camera_register::request_register(&sid)
        })
    })
    .await;
    let why = match r {
        Ok(Ok(())) => {
            log.line("CAMERA source installed");
            return true;
        }
        Ok(Err(RegisterError::Declined)) => {
            "The camera needs a one-time permission to install".to_string()
        }
        Ok(Err(RegisterError::Failed(c))) if c == crate::camera_register::EXIT_UNSAFE_FOLDER => {
            log.line("CAMERA install refused: other accounts can change Relay's folder");
            "Other accounts on this PC can change Relay's folder, so the camera can't be \
             installed from it. Move the Relay folder into your own user folder (for \
             example Downloads) and try again"
                .to_string()
        }
        Ok(Err(RegisterError::Failed(c))) => {
            log.line(&format!("CAMERA install failed: code {c}"));
            format!("Couldn't set up the camera (code {c})")
        }
        Err(_) => return false,
    };
    crate::camera::set_last_error(Some(why.clone()));
    ui(UiEvent::Info(why));
    false
}

/// Use the phone's camera: install the source once (one UAC prompt), ask
/// the phone, then run the session. `again`: the link dropped a moment ago;
/// retry quietly for 30 s (the virtual camera stays, repeating the last frame).
fn show_camera(
    node: Node,
    id: PeerId,
    name: String,
    log: Log,
    ui: Arc<dyn Fn(UiEvent) + Send + Sync>,
    again: bool,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> {
    Box::pin(async move {
        if !again
            && (!crate::camera_register::is_registered() || crate::camera_register::needs_install())
        {
            // Open during the install too, so the first start can be cancelled.
            let _pending = crate::camera::PendingGuard::new();
            if !install_camera(&log, &*ui).await {
                return;
            }
        }
        let codecs = crate::screen_decode::supported_codecs();
        let start = pb_protocol::control::CameraStart {
            id: 0,
            codecs: codecs.iter().map(|c| *c as u32).collect(),
            lens: crate::camera::lens(),
            width: 1920,
            height: 1080,
            fps: 30,
        };
        let deadline = Instant::now() + std::time::Duration::from_secs(30);
        loop {
            let t0 = Instant::now();
            if again && !crate::camera::is_running() {
                return; // stopped by the user meanwhile
            }
            if !again {
                ui(UiEvent::Info(format!("Waiting for {name}\u{2026}")));
            }
            let asked = {
                let (ui, name, log) = (ui.clone(), name.clone(), log.clone());
                move || {
                    log.line(&format!(
                        "CAMERA asked after {} ms (connected)",
                        t0.elapsed().as_millis()
                    ));
                    if !again {
                        ui(UiEvent::Info(format!(
                            "Asking {name} to share its camera\u{2026}"
                        )))
                    }
                }
            };
            // Open while the phone has not answered; any way out clears it.
            let pending = (!again).then(crate::camera::PendingGuard::new);
            let opened = dial(&node, &id, node.open_camera(&id, start.clone(), asked)).await;
            drop(pending);
            match opened {
                Ok(view) => {
                    seen(&node, &id);
                    log.line(&format!(
                        "CAMERA \"{name}\" streaming ({}x{}, rotation {}) accepted after {} ms",
                        view.accept.width,
                        view.accept.height,
                        view.accept.rotation,
                        t0.elapsed().as_millis()
                    ));
                    if again && !crate::camera::is_running() {
                        view.close();
                        return;
                    }
                    crate::camera::set_lens(view.accept.lens);
                    let codec = pb_video::Codec::from_u32(view.accept.codec)
                        .unwrap_or(pb_video::Codec::H264);
                    let ended = {
                        let (node, log, ui, name) =
                            (node.clone(), log.clone(), ui.clone(), name.clone());
                        let rt = tokio::runtime::Handle::current();
                        move |why: String| {
                            touch(&id);
                            log.line(&format!("CAMERA ended: {why}"));
                            if why.starts_with("Phone camera stopped") {
                                // Maybe only the link: try again for 30 s.
                                rt.spawn(show_camera(
                                    node.clone(),
                                    id,
                                    name.clone(),
                                    log.clone(),
                                    ui.clone(),
                                    true,
                                ));
                            } else {
                                crate::camera::stop();
                                ui(UiEvent::Info(why));
                            }
                        }
                    };
                    let opts = crate::camera::CameraOpts {
                        log: Box::new({
                            let log = log.clone();
                            move |m: &str| log.line(m)
                        }),
                        on_ended: Box::new(ended),
                    };
                    match crate::camera::start(
                        view,
                        codec,
                        opts,
                        &tokio::runtime::Handle::current(),
                    ) {
                        Ok(()) => ui(UiEvent::Info("Phone camera on".into())),
                        Err(e) => {
                            log.line(&format!("CAMERA start failed: {e}"));
                            crate::camera::stop();
                            ui(UiEvent::Info("Couldn't start the phone camera".into()));
                        }
                    }
                    return;
                }
                Err(e)
                    if again
                        && !matches!(e, pb_transport::Error::Declined)
                        && Instant::now() < deadline =>
                {
                    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                }
                Err(e) => {
                    log.line(&format!("CAMERA \"{name}\" not started: {e}"));
                    // Nothing keeps the camera alive without a session.
                    tokio::task::spawn_blocking(crate::camera::stop);
                    ui(UiEvent::Info(match e {
                        _ if again => format!("{name} stopped sharing its camera"),
                        pb_transport::Error::Declined => format!("{name} didn't share its camera"),
                        pb_transport::Error::Timeout => format!("{name} didn't answer"),
                        e => crate::transfers::session_failure(&e, &name)
                            .unwrap_or_else(|| format!("Couldn't reach {name} to use its camera")),
                    }));
                    return;
                }
            }
        }
    })
}

#[cfg(windows)]
fn pb_platform_win_wrapper() -> pb_platform_win::Dpapi {
    pb_platform_win::Dpapi
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_send_to_choice_is_only_taken_by_its_own_command_and_expires() {
        clear_next_target();
        send_next_to("a".repeat(64), SendKind::Files);
        let now = Instant::now();
        assert_eq!(
            take_next_target(None, now),
            None,
            "a screen request never takes it"
        );
        assert_eq!(
            take_next_target(Some(SendKind::Clipboard), now),
            None,
            "nor does the other kind"
        );
        assert_eq!(
            take_next_target(Some(SendKind::Files), now),
            Some("a".repeat(64))
        );
        assert_eq!(take_next_target(Some(SendKind::Files), now), None, "once");
        send_next_to("b".repeat(64), SendKind::Clipboard);
        let late = now + NEXT_TARGET_TTL + std::time::Duration::from_secs(1);
        assert_eq!(
            take_next_target(Some(SendKind::Clipboard), late),
            None,
            "a stale choice is dropped"
        );
        clear_next_target();
    }

    use super::*;

    #[test]
    fn a_prompt_left_unanswered_after_earlier_progress_is_not_a_broken_link() {
        // 500 bytes went before the link broke; the second attempt waits for a yes nobody gives.
        assert_eq!(
            attempt_bytes(500, 500),
            0,
            "nothing moved this attempt: no retry"
        );
        assert!(!crate::queue::should_retry(
            true,
            1,
            false,
            attempt_bytes(500, 500)
        ));
        // The yes came and bytes moved, then it broke again: that is retried.
        assert!(crate::queue::should_retry(
            true,
            1,
            false,
            attempt_bytes(500, 900)
        ));
        // The receiver kept nothing and the file restarted at zero, then broke: also retried.
        assert!(crate::queue::should_retry(
            true,
            1,
            false,
            attempt_bytes(500, 100)
        ));
    }

    #[tokio::test]
    async fn a_waiting_request_is_pending_until_cancelled() {
        let slot = Mutex::new(None);
        assert!(!request_pending(&slot));
        assert!(!cancel_request(&slot));
        let h = tokio::spawn(std::future::pending::<()>()).abort_handle();
        *slot.lock().unwrap() = Some(h);
        assert!(request_pending(&slot));
        assert!(cancel_request(&slot));
        assert!(!request_pending(&slot));
        assert!(!cancel_request(&slot));
    }

    /// Node calls that dial a peer (each one can set or clear the
    /// transport's "changed" flag). Split so this test does not match itself.
    const DIALS: [&str; 10] = [
        concat!("node.", "connect("),
        concat!("node.", "send_file("),
        concat!("node.", "send_text("),
        concat!("node.", "send_clipboard_image("),
        concat!("node.", "open_camera("),
        concat!("node.", "open_screen("),
        concat!("node.", "open_input("),
        concat!("node.", "open_input_with("),
        concat!("node.", "notify_action("),
        concat!("node.", "notify_sync_request("),
    ];

    /// Dialling calls in `body` that are not an argument of `dial(` (the
    /// text since the previous `;` must contain it).
    fn undialled(name: &str, body: &str) -> Vec<String> {
        let mut out = Vec::new();
        for needle in DIALS {
            for (at, _) in body.match_indices(needle) {
                let start = body[..at].rfind(';').map_or(0, |i| i + 1);
                if !body[start..at].contains(concat!("dial", "(")) {
                    let line = body[..at].lines().count();
                    out.push(format!("{name}:{line}: {needle}"));
                }
            }
        }
        out
    }

    /// Every tray-side dial goes through `dial`, so the "Reinstalled or
    /// reset" note follows each attempt (src/ scanned recursively).
    #[test]
    fn every_dial_notes_the_changed_flag() {
        fn walk(dir: &std::path::Path, out: &mut Vec<String>) {
            for e in std::fs::read_dir(dir).unwrap() {
                let path = e.unwrap().path();
                if path.is_dir() {
                    walk(&path, out);
                } else if path.extension().and_then(|x| x.to_str()) == Some("rs") {
                    let body = std::fs::read_to_string(&path).unwrap();
                    out.extend(undialled(&path.display().to_string(), &body));
                }
            }
        }
        let mut found = Vec::new();
        walk(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
            &mut found,
        );
        assert!(found.is_empty(), "dial these through `dial`: {found:#?}");
        // The check itself catches a bare call and passes a wrapped one.
        let bare = concat!("let s = node.", "connect(&id).await?;");
        let wrapped = concat!("let s = dial(&node, &id, node.", "connect(&id)).await?;");
        assert_eq!(undialled("x.rs", bare).len(), 1);
        assert!(undialled("x.rs", wrapped).is_empty());
    }

    /// A stand-in for the node's "changed" flags.
    struct FakeFlags {
        changed: Mutex<Vec<PeerId>>,
        paired: Vec<PeerId>,
    }

    impl FakeFlags {
        fn set(&self, id: &PeerId, on: bool) {
            let mut g = self.changed.lock().unwrap();
            g.retain(|p| p != id);
            if on {
                g.push(*id);
            }
        }
    }

    impl ChangedFlags for FakeFlags {
        fn changed(&self, id: &PeerId) -> bool {
            self.changed.lock().unwrap().contains(id)
        }
        fn paired(&self) -> Vec<PeerId> {
            self.paired.clone()
        }
    }

    /// The real hooks copy the flag into the cache at each point the
    /// transport may change it: a session opening and closing, a dial
    /// (either way) and pairing. One test: `on_paired` rewrites the cache.
    #[test]
    fn every_hook_copies_the_transport_flag_into_the_cache() {
        let id = pb_identity::Identity::from_seed([0xC3; 32]).peer_id();
        let flags = FakeFlags {
            changed: Mutex::new(Vec::new()),
            paired: vec![id],
        };
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        rt.block_on(async {
            // Session open clears a stale note; its close notes a new flag.
            CHANGED.lock().unwrap().push(id);
            with_session_notes(&flags, &id, async {
                assert!(!is_changed(&id), "cleared when the session opened");
                flags.set(&id, true);
            })
            .await;
            assert!(is_changed(&id), "noted when the session closed");

            // A dial that cleared the flag (connect worked) clears the note...
            flags.set(&id, false);
            dial(&flags, &id, async {}).await;
            assert!(!is_changed(&id), "a working dial clears the note");
            // ...and one that set it (PeerChanged) notes it, Err or not.
            flags.set(&id, true);
            let r: Result<(), ()> = dial(&flags, &id, async { Err(()) }).await;
            assert!(r.is_err() && is_changed(&id), "a failed dial notes it");
        });
        // Pairing again cleared it in the transport.
        flags.set(&id, false);
        on_paired(&flags);
        assert!(!is_changed(&id), "pairing again clears the note");
    }

    /// A phone that reconnects proves its identity, and the transport clears
    /// its "changed" flag: the cached note must follow, or the UI keeps
    /// saying "Reinstalled or reset".
    #[test]
    fn a_cleared_transport_flag_clears_the_cached_note() {
        let a = pb_identity::Identity::from_seed([0xA1; 32]).peer_id();
        let b = pb_identity::Identity::from_seed([0xB2; 32]).peer_id();
        let mut cache = vec![a, b];
        assert!(
            set_changed(&mut cache, &a, false),
            "cleared: the UI must refresh"
        );
        assert_eq!(cache, [b]);
        assert!(
            !set_changed(&mut cache, &a, false),
            "already clear: no refresh"
        );
        assert!(!set_changed(&mut cache, &b, true), "already noted");
        assert!(set_changed(&mut cache, &a, true));
        assert_eq!(cache, [b, a]);
    }

    #[test]
    fn open_sessions_are_counted_and_stamp_the_last_contact() {
        let id = pb_identity::Identity::from_seed([77; 32]).peer_id();
        assert_eq!(contact(&id), Contact::default());
        let a = OpenSession::new(id, None);
        let b = OpenSession::new(id, None);
        assert_eq!(contact(&id).open, 2);
        drop(a);
        assert_eq!(contact(&id).open, 1);
        drop(b);
        let c = contact(&id);
        assert_eq!(c.open, 0);
        assert!(c
            .last_seen
            .is_some_and(|t| t + 5 >= crate::timeline::now_unix()));
    }
}
