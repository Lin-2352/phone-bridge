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
use pb_protocol::control::{ClipboardText, ControlMsg, MAX_CLIPBOARD_TEXT};
use pb_transport::{Node, NodeConfig, PendingPairing, Session, SharedStore};
use tokio::sync::mpsc;

pub enum Cmd {
    Pair,
    SendClipboard(String),
    Decide { id: u64, accept: bool },
    Unpair(PeerId),
}

pub enum UiEvent {
    Ready {
        fingerprint: String,
    },
    Code(String),
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
    log: Log,
    ui: impl Fn(UiEvent) + Send + Sync + 'static,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let ui = Arc::new(ui);
    std::fs::create_dir_all(&opts.dir)?;
    let identity = pb_identity::vault::open_identity(&opts.dir, &pb_platform_win_wrapper())?;
    let store: SharedStore = Arc::new(Mutex::new(FilePeerStore::open(opts.dir.join("peers.txt"))?));
    let (node, mut ev) = Node::bind(
        &identity,
        store,
        NodeConfig {
            device_name: opts.name.clone(),
            port: Some(opts.port),
        },
    )
    .await?;
    log.line(&format!("READY id={} port={}", node.peer_id(), node.port()));
    refresh(&node, &peers);
    ui(UiEvent::Ready {
        fingerprint: node.peer_id().fingerprint(),
    });

    let mut pending: HashMap<u64, PendingPairing> = HashMap::new();
    let mut next_id = 0u64;
    if opts.pair_on_start {
        open_pairing(&node, &log, &*ui);
    }
    loop {
        tokio::select! {
            cmd = cmds.recv() => match cmd {
                None => break,
                Some(Cmd::Pair) => open_pairing(&node, &log, &*ui),
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
                next_id += 1;
                log.line(&format!("SAS={} peer=\"{}\"", p.sas, p.peer_name));
                ui(UiEvent::Sas { id: next_id, peer_name: p.peer_name.clone(), sas: p.sas.clone() });
                pending.insert(next_id, p);
            }
            Some(s) = ev.sessions.recv() => {
                tokio::spawn(pump(s, log.clone(), ui.clone()));
            }
        }
    }
    node.shutdown().await;
    Ok(())
}

fn open_pairing(node: &Node, log: &Log, ui: &(impl Fn(UiEvent) + ?Sized)) {
    match node.open_pairing(node.addrs()) {
        Ok(code) => {
            log.line(&format!("CODE={code}"));
            ui(UiEvent::Code(code.to_string()));
        }
        Err(e) => ui(UiEvent::Info(format!("Could not start pairing: {e}"))),
    }
}

async fn pump(mut s: Session, log: Log, ui: Arc<impl Fn(UiEvent) + Send + Sync + 'static>) {
    log.line(&format!("SESSION \"{}\"", s.peer_name));
    loop {
        match s.recv().await {
            Ok(Some(ControlMsg::ClipboardText(c))) => {
                if c.sensitive {
                    log.line(&format!("CLIP from \"{}\" (sensitive)", s.peer_name));
                } else {
                    log.line(&format!("CLIP from \"{}\": {}", s.peer_name, c.text));
                }
                ui(UiEvent::Clipboard {
                    from: s.peer_name.clone(),
                    text: c.text,
                    sensitive: c.sensitive,
                });
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
