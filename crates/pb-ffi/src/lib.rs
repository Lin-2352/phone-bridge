//! UniFFI surface for the Android app: start the node, join a pairing code,
//! confirm the SAS, send clipboard text; events are pushed to a Kotlin listener.

use std::sync::{Arc, Mutex};

use pb_identity::FilePeerStore;
use pb_protocol::control::{ClipboardText, ControlMsg, PairingCode, MAX_CLIPBOARD_TEXT};
use pb_transport::{Joining, Node, NodeConfig, Session, SharedStore};

uniffi::setup_scaffolding!();

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
    /// A file transfer is under way (at most every 0.7 s, and once at the end).
    TransferProgress {
        peer_name: String,
        name: String,
        done: u64,
        total: u64,
        incoming: bool,
    },
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
    pending: Mutex<std::collections::HashMap<u64, tokio::sync::oneshot::Sender<bool>>>,
    last_remote_clip: Mutex<[u8; 32]>,
    input: Mutex<Option<Arc<dyn InputListener>>>,
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
    };
    // Anything still in incoming/ was left by a process the system killed
    // mid-transfer or mid-save; it would otherwise use storage forever.
    if let Ok(entries) = std::fs::read_dir(dir.join("incoming")) {
        for e in entries.flatten() {
            let _ = std::fs::remove_file(e.path());
        }
    }
    let (node, ev) = Node::bind(&identity, store, config).await.map_err(fail)?;
    let shared = Arc::new(Shared {
        events: listener,
        incoming_dir: dir.join("incoming"),
        auto_accept: std::sync::atomic::AtomicBool::new(true),
        clipboard_sync: std::sync::atomic::AtomicBool::new(false),
        pending: Mutex::default(),
        last_remote_clip: Mutex::new([0; 32]),
        input: Mutex::new(None),
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
    use pb_transport::files::{receive_offer, Received, DEFAULT_MAX_FILE};
    let name = s.peer_name.clone();
    push(
        &sh.events,
        PbEvent::Connected {
            peer_name: name.clone(),
        },
    );
    let reason = loop {
        match s.recv().await {
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
                // Accept only while the accessibility service is running.
                let listener = sh.input.lock().ok().and_then(|g| g.clone());
                if let Some(l) = &listener {
                    let _ = l.on_control(true, name.clone());
                }
                let l2 = listener.clone();
                let _ = pb_transport::input::receive_input(
                    &mut s,
                    start,
                    listener.is_some(),
                    move |e| {
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
                        }
                    },
                )
                .await;
                if let Some(l) = &listener {
                    let _ = l.on_control(false, name.clone());
                }
                break "input done".to_owned();
            }
            Ok(Some(ControlMsg::Offer(offer))) => {
                let accept = if sh.auto_accept.load(std::sync::atomic::Ordering::Relaxed) {
                    true
                } else {
                    let (tx, rx) = tokio::sync::oneshot::channel();
                    sh.pending
                        .lock()
                        .unwrap_or_else(|p| p.into_inner())
                        .insert(offer.id, tx);
                    push(
                        &sh.events,
                        PbEvent::IncomingOffer {
                            id: offer.id,
                            peer_name: name.clone(),
                            name: offer.name.clone(),
                            size: offer.size,
                        },
                    );
                    let answer = tokio::time::timeout(pb_transport::files::OFFER_TIMEOUT, rx)
                        .await
                        .ok()
                        .and_then(|r| r.ok())
                        .unwrap_or(false);
                    sh.pending
                        .lock()
                        .unwrap_or_else(|p| p.into_inner())
                        .remove(&offer.id);
                    answer
                };
                let file_name = offer.name.clone();
                match receive_offer(
                    &mut s,
                    offer,
                    accept,
                    &sh.incoming_dir,
                    DEFAULT_MAX_FILE,
                    Some(progress_events(
                        sh.events.clone(),
                        name.clone(),
                        file_name.clone(),
                        true,
                    )),
                )
                .await
                {
                    Ok(Received::File { path, size }) => push(
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
                    ),
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
                    Err(pb_transport::Error::Declined) => {}
                    Err(e) => push(
                        &sh.events,
                        PbEvent::TransferFailed {
                            peer_name: name.clone(),
                            reason: e.to_string(),
                        },
                    ),
                }
                break "transfer done".to_owned();
            }
            Ok(Some(_)) => {}
            Ok(None) => break "closed".to_owned(),
            Err(e) => break e.to_string(),
        }
    };
    push(
        &sh.events,
        PbEvent::Disconnected {
            peer_name: name,
            reason,
        },
    );
}

#[uniffi::export(async_runtime = "tokio")]
impl PbNode {
    pub fn peer_id(&self) -> String {
        self.node.peer_id().to_string()
    }

    pub fn fingerprint(&self) -> String {
        self.node.peer_id().fingerprint()
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
    pub fn unpair(&self, peer_id: String) -> bool {
        self.node
            .paired_peers()
            .into_iter()
            .find(|p| p.id.to_string() == peer_id)
            .is_some_and(|p| self.node.unpair(&p.id))
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
        self.node
            .send_file(
                &peer,
                &name,
                size,
                &mime,
                reader,
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
            )
            .await
            .map(|_| ())
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
            || text.is_empty()
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
        if let Ok(mut g) = self.shared.input.lock() {
            *g = listener;
        }
    }

    pub fn set_auto_accept(&self, on: bool) {
        self.shared
            .auto_accept
            .store(on, std::sync::atomic::Ordering::Relaxed);
    }

    pub fn set_clipboard_sync(&self, on: bool) {
        self.shared
            .clipboard_sync
            .store(on, std::sync::atomic::Ordering::Relaxed);
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

    pub fn stop(&self) {
        if let Some(s) = self.sender.lock().ok().and_then(|mut g| g.take()) {
            s.stop();
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
