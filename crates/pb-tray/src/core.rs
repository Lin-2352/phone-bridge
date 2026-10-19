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
    files::{receive_offer, Received, DEFAULT_MAX_FILE},
    Node, NodeConfig, PendingPairing, Session, SharedStore,
};
use sha2::{Digest, Sha256};
use tokio::sync::{mpsc, oneshot};

use crate::settings::Settings;

pub type SharedSettings = Arc<Mutex<Settings>>;

/// Universal Clipboard echo guard: the hash of the last text we received and
/// the last we sent, so neither side bounces the same text back and forth.
#[derive(Default)]
struct ClipSync {
    last_remote: [u8; 32],
    last_sent: [u8; 32],
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

/// A PC -> phone control request still waiting for the phone's answer.
static CONTROL_REQUEST: Mutex<Option<tokio::task::AbortHandle>> = Mutex::new(None);
/// The phone -> PC control session running now, so turning the setting off ends it.
static PHONE_CONTROL: Mutex<Option<pb_transport::SessionCloser>> = Mutex::new(None);

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
    /// The user closed the QR window: kill the current code now.
    CancelPair,
    SendClipboard(String),
    /// Files chosen in the dialog, dropped on the flyout, or sent via Explorer.
    SendFiles(Vec<PathBuf>),
    /// The local clipboard changed (from the clipboard listener).
    ClipboardChanged {
        text: String,
        sensitive: bool,
    },
    /// Start driving the phone with this PC's mouse and keyboard.
    ControlPhone,
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
}

pub enum UiEvent {
    Ready {
        fingerprint: String,
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
    Clipboard {
        from: String,
        text: String,
        sensitive: bool,
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
    FileReceived {
        from: String,
        path: PathBuf,
        size: u64,
    },
    /// A phone wants to control this PC and the user has not allowed it yet.
    AskControl {
        from: String,
        reply: oneshot::Sender<bool>,
    },
    /// Ask the user whether to accept (only when auto-accept is off).
    IncomingOffer {
        from: String,
        name: String,
        size: u64,
        reply: oneshot::Sender<bool>,
    },
    Info(String),
}

pub struct Options {
    pub dir: PathBuf,
    pub port: u16,
    pub name: String,
    pub pair_on_start: bool,
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
    }
}

fn refresh(node: &Node, peers: &Peers) {
    let list = node
        .paired_peers()
        .into_iter()
        .map(|p| (p.id, p.name))
        .collect();
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
    std::fs::create_dir_all(&opts.dir)?;
    // Partial files left by a tray that was killed mid-transfer.
    if let Ok(dir) = settings.lock().map(|s| s.save_dir.clone()) {
        for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') && name.contains(".partial-") {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
    let identity = pb_identity::vault::open_identity(&opts.dir, &pb_platform_win_wrapper())?;
    let store: SharedStore = Arc::new(Mutex::new(FilePeerStore::open(opts.dir.join("peers.txt"))?));
    let (node, mut ev) = Node::bind(
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
    // Peers may have moved to a new network since we last saw them.
    tokio::spawn({
        let node = node.clone();
        async move { node.rediscover().await }
    });
    log.line(&format!("READY id={} port={}", node.peer_id(), node.port()));
    refresh(&node, &peers);
    ui(UiEvent::Ready {
        fingerprint: node.peer_id().fingerprint(),
    });

    let clip: Arc<Mutex<ClipSync>> = Arc::default();
    let mut pending: HashMap<u64, PendingPairing> = HashMap::new();
    let mut next_id = 0u64;
    let mut rotation: Option<tokio::task::JoinHandle<()>> = None;
    if opts.pair_on_start {
        rotation = Some(start_rotation(&node, &log, &ui));
    }
    loop {
        tokio::select! {
            cmd = cmds.recv() => match cmd {
                None => break,
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
                Some(Cmd::Unpair(id)) => {
                    node.unpair(&id);
                    log.line(&format!("UNPAIRED {id}"));
                    refresh(&node, &peers);
                    ui(UiEvent::Info("Device removed".into()));
                }
                Some(Cmd::SendFiles(paths)) => {
                    let (node, log, ui) = (node.clone(), log.clone(), ui.clone());
                    let target = peers.lock().ok().and_then(|g| g.first().cloned());
                    tokio::spawn(async move {
                        let Some((id, name)) = target else {
                            ui(UiEvent::Info("Pair a device first".into()));
                            return;
                        };
                        for path in paths {
                            send_one_file(&node, &id, &name, &path, &log, &*ui).await;
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
                        if h == c.last_remote || h == c.last_sent {
                            continue;
                        }
                        c.last_sent = h;
                    }
                    let targets: Vec<(PeerId, String)> = peers.lock().map(|g| g.clone()).unwrap_or_default();
                    let (node, log, clip) = (node.clone(), log.clone(), clip.clone());
                    tokio::spawn(async move {
                        for (id, name) in targets {
                            let r = async {
                                let s = node.connect(&id).await?;
                                s.send(ControlMsg::ClipboardUpdate(ClipboardUpdate { text: text.clone(), hash: h.to_vec(), sensitive })).await?;
                                s.finish().await;
                                Ok::<_, pb_transport::Error>(())
                            }
                            .await;
                            match r {
                                Ok(()) if sensitive => log.line(&format!("CLIPSYNC sent a sensitive clip to \"{name}\"")),
                                Ok(()) => log.line(&format!("CLIPSYNC sent {} chars to \"{name}\"", text.chars().count())),
                                Err(e) => {
                                    log.line(&format!("CLIPSYNC to \"{name}\" failed: {e}"));
                                    // Not delivered: copying the same text again must retry.
                                    let mut c = clip.lock().unwrap_or_else(|p| p.into_inner());
                                    if c.last_sent == h {
                                        c.last_sent = [0; 32];
                                    }
                                }
                            }
                        }
                    });
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
                    let target = peers.lock().ok().and_then(|g| g.first().cloned());
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
                        match node.open_input_with(&id, asked).await {
                            Ok(sender) => {
                                log.line(&format!("CONTROL phone \"{name}\" started"));
                                ui(UiEvent::ControlReady { name, sender });
                            }
                            Err(e) => {
                                log.line(&format!("CONTROL phone \"{name}\" refused: {e}"));
                                let why = match e {
                                    pb_transport::Error::Declined => format!(
                                        "{name} declined your request to control it"
                                    ),
                                    pb_transport::Error::Timeout => {
                                        format!("{name} didn't answer your control request")
                                    }
                                    _ => format!("Couldn't reach {name} to control it"),
                                };
                                ui(UiEvent::Info(why));
                            }
                        }
                    });
                    *CONTROL_REQUEST.lock().unwrap_or_else(|p| p.into_inner()) = Some(task.abort_handle());
                }
                Some(Cmd::SendText { text, link }) => {
                    let (node, log, ui) = (node.clone(), log.clone(), ui.clone());
                    let target = peers.lock().ok().and_then(|g| g.first().cloned());
                    tokio::spawn(async move {
                        let Some((id, name)) = target else {
                            ui(UiEvent::Info("Pair a device first".into()));
                            return;
                        };
                        match node.send_text(&id, &text, link).await {
                            Ok(()) => {
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
                    let (node, log, ui) = (node.clone(), log.clone(), ui.clone());
                    let target = peers.lock().ok().and_then(|g| g.first().cloned());
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
                            let s = node.connect(&id).await?;
                            s.send(ControlMsg::ClipboardText(ClipboardText { text: text.clone(), sensitive: false })).await?;
                            s.finish().await;
                            Ok::<_, pb_transport::Error>(())
                        }
                        .await;
                        match result {
                            Ok(()) => {
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
                next_id += 1;
                log.line(&format!("SAS={} peer=\"{}\"", p.sas, p.peer_name));
                ui(UiEvent::Sas { id: next_id, peer_name: p.peer_name.clone(), sas: p.sas.clone() });
                pending.insert(next_id, p);
            }
            Some(s) = ev.sessions.recv() => {
                tokio::spawn(pump(s, log.clone(), ui.clone(), settings.clone(), clip.clone()));
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

async fn send_one_file(
    node: &Node,
    id: &PeerId,
    name: &str,
    path: &std::path::Path,
    log: &Log,
    ui: &(impl Fn(UiEvent) + ?Sized),
) {
    let file_name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "file".into());
    let result = async {
        let f = tokio::fs::File::open(path)
            .await
            .map_err(|e| pb_transport::Error::Io(e.to_string()))?;
        let size = f
            .metadata()
            .await
            .map_err(|e| pb_transport::Error::Io(e.to_string()))?
            .len();
        let started = Instant::now();
        node.send_file(id, &file_name, size, "", f, None).await?;
        Ok::<_, pb_transport::Error>((size, started.elapsed()))
    }
    .await;
    match result {
        Ok((size, took)) => {
            let mbps = size as f64 / 1e6 / took.as_secs_f64().max(1e-3);
            log.line(&format!(
                "FILE sent \"{file_name}\" {size} bytes to \"{name}\" in {:.2}s ({mbps:.1} MB/s)",
                took.as_secs_f64()
            ));
            ui(UiEvent::Info(format!("Sent {file_name} to {name}")));
        }
        Err(e) => {
            log.line(&format!("FILE send \"{file_name}\" failed: {e}"));
            ui(UiEvent::Info(format!("Could not send {file_name}: {e}")));
        }
    }
}

async fn pump(
    mut s: Session,
    log: Log,
    ui: Arc<impl Fn(UiEvent) + Send + Sync + 'static>,
    settings: SharedSettings,
    clip: Arc<Mutex<ClipSync>>,
) {
    log.line(&format!("SESSION \"{}\"", s.peer_name));
    loop {
        match s.recv().await {
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
                ui(UiEvent::Clipboard {
                    from: s.peer_name.clone(),
                    text: c.text,
                    sensitive: c.sensitive,
                });
            }
            Ok(Some(ControlMsg::ClipboardUpdate(u))) => {
                if !settings
                    .lock()
                    .map(|x| x.universal_clipboard)
                    .unwrap_or(false)
                {
                    log.line("CLIPSYNC ignored (Universal Clipboard is off)");
                    continue;
                }
                clip.lock().unwrap_or_else(|p| p.into_inner()).last_remote = clip_hash(&u.text);
                log.line(&format!(
                    "CLIPSYNC received {} chars from \"{}\"",
                    u.text.chars().count(),
                    s.peer_name
                ));
                ui(UiEvent::RemoteClipboard {
                    from: s.peer_name.clone(),
                    text: u.text,
                    sensitive: u.sensitive,
                });
            }
            Ok(Some(ControlMsg::InputStart(start))) => {
                let from = s.peer_name.clone();
                let mut allow = settings.lock().map(|x| x.allow_control).unwrap_or(false);
                if !allow {
                    // First time: ask. "Yes" turns the setting on (Settings can turn it off).
                    let (tx, rx) = oneshot::channel();
                    ui(UiEvent::AskControl {
                        from: from.clone(),
                        reply: tx,
                    });
                    allow = tokio::time::timeout(std::time::Duration::from_secs(60), rx)
                        .await
                        .ok()
                        .and_then(Result::ok)
                        .unwrap_or(false);
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
                let (auto, dir) = settings
                    .lock()
                    .map(|x| (x.auto_accept, x.save_dir.clone()))
                    .unwrap_or((false, std::env::temp_dir()));
                let accept = if auto {
                    true
                } else {
                    let (tx, rx) = oneshot::channel();
                    ui(UiEvent::IncomingOffer {
                        from: s.peer_name.clone(),
                        name: offer.name.clone(),
                        size: offer.size,
                        reply: tx,
                    });
                    tokio::time::timeout(pb_transport::files::OFFER_TIMEOUT, rx)
                        .await
                        .ok()
                        .and_then(Result::ok)
                        .unwrap_or(false)
                };
                let from = s.peer_name.clone();
                let started = Instant::now();
                match receive_offer(&mut s, offer, accept, &dir, DEFAULT_MAX_FILE, None).await {
                    Ok(Received::File { path, size }) => {
                        let mbps = size as f64 / 1e6 / started.elapsed().as_secs_f64().max(1e-3);
                        log.line(&format!(
                            "FILE received {} ({size} bytes, {mbps:.1} MB/s) from \"{from}\"",
                            path.display()
                        ));
                        ui(UiEvent::FileReceived { from, path, size });
                    }
                    Ok(Received::Text(t)) => ui(UiEvent::Clipboard {
                        from,
                        text: t,
                        sensitive: false,
                    }),
                    Ok(Received::Link(l)) => {
                        log.line(&format!("LINK from \"{from}\""));
                        ui(UiEvent::Link { from, url: l });
                    }
                    Err(e) => log.line(&format!("OFFER from \"{from}\" not received: {e}")),
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

#[cfg(windows)]
fn pb_platform_win_wrapper() -> pb_platform_win::Dpapi {
    pb_platform_win::Dpapi
}
