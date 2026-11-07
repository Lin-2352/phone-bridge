//! Phone Bridge transport: an iroh QUIC [`Node`] that only talks to paired
//! peers, the pairing exchange, and [`Session`]s carrying control messages.
//!
//! Privacy defaults: `presets::Minimal` (no n0 relay, no DNS/pkarr publishing),
//! so nothing leaves the local network unless a relay is configured later.
//! GSO is disabled on Android (see docs/DECISIONS.md, finding 1).

pub mod addr_rank;
pub mod audio;
pub mod camera;
pub mod discovery;
pub mod files;
pub mod input;
pub mod notify;
pub mod pairing;
pub mod quota;
pub mod screen;
pub mod tuning;

use std::{
    collections::HashMap,
    fmt,
    net::SocketAddr,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use iroh::{
    endpoint::{presets, Connection, ConnectionError, RecvStream, SendStream},
    Endpoint, EndpointAddr, EndpointId, RelayMode, SecretKey, TransportAddr,
};
use pairing::{HostPairing, JoinPairing, PairError, Ticket};
use pb_identity::{Identity, PairedPeer, PeerId, PeerStore};
pub use pb_protocol::UpdateSide;
use pb_protocol::{
    control::{Control, ControlMsg, Heartbeat, Pair, PairConfirm, PairMsg, PairingCode},
    decode_frame, encode_frame, Hello,
};
use prost::Message;
use tokio::sync::{mpsc, oneshot};

pub const SESSION_ALPN: &[u8] = b"pb/0";
pub const PAIR_ALPN: &[u8] = b"pb-pair/0";

/// Heartbeat interval; a session with no traffic for 3 intervals is dead.
pub const HEARTBEAT: Duration = Duration::from_secs(5);
/// How long a session waits for anything from a peer before it is called dead.
pub const DEFAULT_WATCHDOG: Duration = Duration::from_secs(15);
/// The same for audio: a Wi-Fi outage of tens of seconds is waited out, as the sound comes back by
/// itself when the link does. It must stay under [`tuning::IDLE_TIMEOUT`].
pub const AUDIO_WATCHDOG: Duration = Duration::from_secs(45);
/// Unauthenticated handshake steps must finish within this.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(20);
/// An incoming QUIC handshake on a local link takes milliseconds; one that
/// dawdles is holding a connection slot.
const INCOMING_HANDSHAKE: Duration = Duration::from_secs(5);
/// Incoming connections handled at once.
const MAX_INCOMING: usize = 16;
/// Pairing connections handled at once, from their own pool: idle ones
/// (anyone on the LAN can open them while a code is shown) must not take
/// the slots paired devices connect through.
const MAX_PAIRING: usize = pairing::MAX_PAIRING_SLOTS;
/// How long either side waits for the user to confirm the SAS.
pub const CONFIRM_TIMEOUT: Duration = Duration::from_secs(120);

const CLOSE_NOT_PAIRED: u32 = 403;
const CLOSE_PAIR_FAILED: u32 = 400;
/// A paired peer opened more sessions than its quota allows (M7.8).
const CLOSE_BUSY: u32 = 429;
/// This device closed every connection ("Disconnect everything"); the pairing is untouched.
const CLOSE_DISCONNECTED: u32 = 410;
/// The joiner could not save the pairing: the host takes its own back.
const CLOSE_PAIR_NOT_SAVED: u32 = 500;
/// The peer's `Hello` names a protocol version this build does not speak.
const CLOSE_INCOMPATIBLE: u32 = 426;

#[derive(Debug)]
pub enum Error {
    Transport(String),
    Protocol(String),
    NotPaired,
    UnknownPeer,
    Pairing(PairError),
    Rejected,
    Timeout,
    Closed,
    /// The other device declined the offer.
    Declined,
    /// The offer exceeds the size limit.
    TooLarge,
    /// Something answers at the saved address but is not the paired device:
    /// it was reinstalled or reset (its identity is new), or another device
    /// answers there.
    PeerChanged,
    /// The received bytes do not match the sender's SHA-256.
    HashMismatch,
    /// This device cancelled the transfer.
    Cancelled,
    /// The other device cancelled the transfer.
    PeerCancelled,
    Io(String),
    /// The peer's `Hello` does not list audio: it predates the feature.
    Unsupported,
    /// The peer, or this node's own slot, refused an audio request for this
    /// reason (`Declined` has its own variant).
    AudioRefused(pb_protocol::control::AudioReason),
    /// The list of paired devices could not be saved, so the change the user
    /// asked for (pair, remove) did not happen.
    Storage(String),
    /// The two devices can't talk: the named side needs a newer Relay.
    VersionMismatch(UpdateSide),
    /// The other device has too many requests from this one open or recent; try again in a moment.
    Busy,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Transport(e) => write!(f, "transport error: {e}"),
            Self::Protocol(e) => write!(f, "protocol error: {e}"),
            Self::NotPaired => f.write_str("peer is not paired with this device"),
            Self::UnknownPeer => f.write_str("no such paired peer"),
            Self::Pairing(e) => write!(f, "pairing failed: {e}"),
            Self::Rejected => f.write_str("pairing refused: the code expired or was already used, or the other device declined"),
            Self::Timeout => f.write_str("timed out"),
            Self::Closed => f.write_str("connection closed"),
            Self::Declined => f.write_str("the other device declined"),
            Self::TooLarge => f.write_str("too large to send"),
            Self::PeerChanged => f.write_str("another device answers for this peer: if it was reinstalled or reset, remove it and pair again"),
            Self::HashMismatch => f.write_str("file was corrupted in transit (hash mismatch); nothing was saved"),
            Self::Busy => f.write_str("the other device is busy with too many requests from this one; try again in a moment"),
            Self::Cancelled => f.write_str("transfer cancelled"),
            Self::PeerCancelled => f.write_str("the other device cancelled the transfer"),
            Self::Io(e) => write!(f, "file error: {e}"),
            Self::Unsupported => f.write_str("the other device does not support this yet: update it"),
            Self::AudioRefused(r) => write!(f, "audio refused: {r:?}"),
            Self::VersionMismatch(UpdateSide::ThisDevice) => f.write_str("this device runs an older version of Relay than the other device: update Relay here"),
            Self::VersionMismatch(UpdateSide::OtherDevice) => f.write_str("the other device runs an older version of Relay: update Relay there"),
            Self::VersionMismatch(UpdateSide::Both) => f.write_str("the other device runs a different version of Relay: update both to the latest version"),
            Self::Storage(e) => write!(f, "could not save the list of paired devices ({e}): nothing changed, try again"),
        }
    }
}

impl std::error::Error for Error {}

impl From<PairError> for Error {
    fn from(e: PairError) -> Self {
        Self::Pairing(e)
    }
}

fn tx<E: fmt::Display>(e: E) -> Error {
    Error::Transport(e.to_string())
}

fn storage(e: std::io::Error) -> Error {
    Error::Storage(e.to_string())
}

/// A failed batch included a learned address that is not saved: a
/// wrong-key answer may have come from it, not from a saved address.
fn has_unsaved_candidate(cands: &[SocketAddr], saved: &[String]) -> bool {
    cands.iter().any(|c| !saved.contains(&c.to_string()))
}

/// A dial that failed because the device answering is not the paired one:
/// TLS rejects its certificate (its key is not the saved peer's).
fn is_identity_mismatch(e: &Error) -> bool {
    matches!(e, Error::Transport(m) if m.contains("invalid peer certificate"))
}

pub type SharedStore = Arc<Mutex<dyn PeerStore + Send>>;

pub struct NodeConfig {
    /// User-visible name of this device, sent only to paired / pairing peers.
    pub device_name: String,
    /// Fixed UDP port; `None` picks a free one.
    pub port: Option<u16>,
    /// UDP port for finding paired devices again after a network change
    /// (`discovery::DISCOVERY_PORT` in the apps); `None` disables it.
    pub discovery_port: Option<u16>,
    /// QUIC transport settings; the default is what the apps use.
    pub tuning: tuning::Tuning,
}

/// A pairing request waiting for the local user's decision.
pub struct PendingPairing {
    pub peer: PeerId,
    pub peer_name: String,
    pub sas: String,
    decide: oneshot::Sender<bool>,
    done: oneshot::Receiver<Result<PairedPeer, Error>>,
    conn: Connection,
}

impl PendingPairing {
    /// Resolves when the other device gave up (declined, cancelled, went
    /// away) before this user decided: close the prompt. Owns what it needs,
    /// so it can run in its own task while the prompt holds `self`.
    pub fn withdrawn(&self) -> impl std::future::Future<Output = ()> + Send + 'static {
        let conn = self.conn.clone();
        async move {
            let _ = conn.closed().await;
        }
    }

    /// Report the user's decision and wait for the outcome (both sides must accept).
    pub async fn decide(self, accept: bool) -> Result<PairedPeer, Error> {
        let _ = self.decide.send(accept);
        self.done.await.map_err(|_| Error::Closed)?
    }
}

/// The joiner's side after the SAS is known.
pub struct Joining {
    pub peer_name: String,
    pub sas: String,
    conn: Connection,
    send: SendStream,
    recv: FrameReader,
    peer: PairedPeer,
    node: Node,
}

impl Joining {
    pub async fn decide(mut self, accept: bool) -> Result<PairedPeer, Error> {
        write_frame(
            &mut self.send,
            &Pair::from(PairMsg::Confirm(PairConfirm { accepted: accept })),
        )
        .await?;
        if !accept {
            // Nothing to wait for: the outcome is "not paired" whatever the
            // other user answers. Closing tells the host at once.
            let _ = self.send.finish();
            self.conn.close(CLOSE_PAIR_FAILED.into(), b"declined");
            return Err(Error::Rejected);
        }
        let theirs = tokio::time::timeout(CONFIRM_TIMEOUT, self.recv.next::<Pair>())
            .await
            .map_err(|_| Error::Timeout)??;
        // The host sends `accepted: true` only once it has saved the pairing
        // (it reads our Confirm before sending its own).
        let remote_ok = matches!(
            theirs.and_then(|p| p.msg),
            Some(PairMsg::Confirm(PairConfirm { accepted: true }))
        );
        // Close handshake: the host finishes its stream right after its
        // Confirm, so once we see its end-of-stream closing discards nothing.
        let _ = self.send.finish();
        let _ = tokio::time::timeout(Duration::from_secs(5), self.recv.next::<Pair>()).await;
        if !(accept && remote_ok) {
            self.conn.close(0u32.into(), b"not paired");
            return Err(Error::Rejected);
        }
        if let Err(e) = self.node.store_try_pin(self.peer.clone()).await {
            // The host waits for this close and takes its pairing back.
            self.conn
                .close(CLOSE_PAIR_NOT_SAVED.into(), b"pairing not saved");
            return Err(e);
        }
        self.conn.close(0u32.into(), b"paired");
        // Paired again just now: whatever answered before is this device.
        self.node.forget_changed(&self.peer.id);
        Ok(self.peer)
    }
}

struct Inner {
    endpoint: Endpoint,
    me: PeerId,
    device_name: String,
    store: SharedStore,
    ticket: Mutex<Option<Ticket>>,
    /// Wrong pairing tokens, per source and in total (outlives the rotation of the code).
    bad_tokens: Mutex<pairing::BadTokenLimits>,
    /// Per-peer limits on open sessions and file offers.
    quotas: quota::PeerQuotas,
    pairings: mpsc::Sender<PendingPairing>,
    sessions: mpsc::Sender<Session>,
    discovery: Mutex<Option<discovery::Discovery>>,
    candidates: discovery::Candidates,
    /// Open connections per peer, so unpairing can cut them at once. Weak:
    /// a connection still closes when its last user drops it.
    live: Mutex<Vec<(PeerId, iroh::endpoint::WeakConnectionHandle)>>,
    /// When a failed dial last probed the network (see `dial_known`).
    last_probe: Mutex<Option<Instant>>,
    /// Paired peers whose last dial reached a device with a different identity.
    changed: Mutex<Changed>,
}

/// The "reinstalled or reset" bookkeeping, under one lock.
#[derive(Default)]
struct Changed {
    /// Paired peers whose last dial reached a device with a different identity.
    peers: Vec<PeerId>,
    /// Per peer, bumped whenever that peer proves itself, pairs again or is
    /// unpaired: its dial that started before then and fails with a
    /// mismatch afterwards is stale. Another peer's news changes nothing.
    epochs: HashMap<PeerId, u64>,
    /// Peers whose wrong-key answers prove nothing: a dial that included
    /// discovery candidates (anyone on the LAN can plant one) met a
    /// different identity. iroh 1.3 keeps every address it dialled as a
    /// path for that id and sends to all of them on later connects, with
    /// no way to drop one, so the forged address can answer again even
    /// when only saved addresses are asked. The endpoint (and so that
    /// memory) lives as long as the process, so this does too: unpair and
    /// pairing the same device again do not clear it. A real reset of such
    /// a peer is reported after the next restart only.
    untrusted: Vec<PeerId>,
}

fn lock<T: ?Sized>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

/// This device on the network.
#[derive(Clone)]
pub struct Node {
    inner: Arc<Inner>,
    audio_slots: audio::AudioSlots,
}

/// Receivers for events the node produces; keep them polled.
pub struct NodeEvents {
    pub pairings: mpsc::Receiver<PendingPairing>,
    pub sessions: mpsc::Receiver<Session>,
}

/// Ports to try in turn: the fixed one, then any port the system picks.
pub(crate) fn bind_attempts(fixed: Option<u16>) -> Vec<Option<u16>> {
    match fixed {
        Some(p) => vec![Some(p), None],
        None => vec![None],
    }
}

impl Node {
    /// [Node::bind], falling back to a port the system picks when the fixed one can't be bound
    /// (another app, or a Windows excluded port range). Peers learn the port from discovery and
    /// the pairing code, so only the first contact after a change may take longer.
    pub async fn bind_or_any(
        identity: &Identity,
        store: SharedStore,
        config: NodeConfig,
    ) -> Result<(Self, NodeEvents), Error> {
        let mut last = None;
        for port in bind_attempts(config.port) {
            let attempt = NodeConfig {
                device_name: config.device_name.clone(),
                port,
                discovery_port: config.discovery_port,
                tuning: config.tuning.clone(),
            };
            match Self::bind(identity, store.clone(), attempt).await {
                Ok(bound) => return Ok(bound),
                Err(e) => last = Some(e),
            }
        }
        Err(last.expect("at least one attempt"))
    }

    /// Whether the discovery port could be bound: when it can't, other devices can't find this
    /// one after it moves network until it is paired or its address is entered again.
    pub fn discovery_active(&self) -> bool {
        lock(&self.inner.discovery).is_some()
    }

    pub async fn bind(
        identity: &Identity,
        store: SharedStore,
        config: NodeConfig,
    ) -> Result<(Self, NodeEvents), Error> {
        let transport = config.tuning.transport_config();
        let mut builder = Endpoint::builder(presets::Minimal)
            .secret_key(SecretKey::from_bytes(&identity.seed()))
            .alpns(vec![SESSION_ALPN.to_vec(), PAIR_ALPN.to_vec()])
            .transport_config(transport)
            .relay_mode(RelayMode::Disabled);
        if let Some(port) = config.port {
            builder = builder
                .bind_addr(SocketAddr::from(([0, 0, 0, 0], port)))
                .map_err(tx)?;
        }
        let endpoint = builder.bind().await.map_err(tx)?;
        let (pair_tx, pair_rx) = mpsc::channel(4);
        let (sess_tx, sess_rx) = mpsc::channel(16);
        let node = Self {
            audio_slots: Default::default(),
            inner: Arc::new(Inner {
                endpoint,
                me: identity.peer_id(),
                device_name: config.device_name,
                store,
                ticket: Mutex::new(None),
                bad_tokens: Mutex::default(),
                quotas: quota::PeerQuotas::default(),
                pairings: pair_tx,
                sessions: sess_tx,
                discovery: Mutex::new(None),
                candidates: Mutex::default(),
                live: Mutex::default(),
                last_probe: Mutex::new(None),
                changed: Mutex::default(),
            }),
        };
        tokio::spawn(node.clone().accept_loop());
        if let Some(port) = config.discovery_port {
            node.start_discovery(port).await;
        }
        Ok((
            node,
            NodeEvents {
                pairings: pair_rx,
                sessions: sess_rx,
            },
        ))
    }

    pub fn peer_id(&self) -> PeerId {
        self.inner.me
    }

    /// Direct addresses other devices can dial (`ip:port`).
    /// Best first (the real LAN before virtual adapters): pairing keeps only the first few.
    pub fn addrs(&self) -> Vec<String> {
        let addrs: Vec<std::net::SocketAddr> =
            self.inner.endpoint.addr().ip_addrs().copied().collect();
        addr_rank::rank_addrs(addrs, &addr_rank::local_ifaces())
            .into_iter()
            .map(|a| a.to_string())
            .collect()
    }

    /// The bound IPv4 UDP port.
    pub fn port(&self) -> u16 {
        self.inner
            .endpoint
            .bound_sockets()
            .iter()
            .find(|s| s.is_ipv4())
            .map(|s| s.port())
            .unwrap_or(0)
    }

    pub fn paired_peers(&self) -> Vec<PairedPeer> {
        lock(&self.inner.store).list()
    }

    /// True when the last attempt to reach this paired peer found a device
    /// with a different identity: the other side was reinstalled or reset
    /// and must be paired again.
    pub fn peer_changed(&self, peer: &PeerId) -> bool {
        lock(&self.inner.changed).peers.contains(peer)
    }

    /// Taken before a dial to `_peer`; `note_mismatch` compares it.
    fn changed_epoch(&self, peer: &PeerId) -> u64 {
        lock(&self.inner.changed)
            .epochs
            .get(peer)
            .copied()
            .unwrap_or(0)
    }

    /// A dial that started at `epoch` reached a device with a different
    /// identity: flag the peer. True when it was flagged.
    /// Not flagged when the peer proved itself or was unpaired since
    /// (`epoch` moved on), is no longer paired, or its mismatches are
    /// untrusted (see `Changed::untrusted`).
    fn note_mismatch(&self, peer: &PeerId, epoch: u64) -> bool {
        // Never both locks at once (the store is shared with the app).
        // `unpair` removes from the store and then bumps the epoch: if it
        // runs between this check and the lock below, the epoch has moved;
        // if after the push, its own clean-up removes the entry.
        if lock(&self.inner.store).get(peer).is_none() {
            return false;
        }
        let mut ch = lock(&self.inner.changed);
        if ch.epochs.get(peer).copied().unwrap_or(0) != epoch || ch.untrusted.contains(peer) {
            return false;
        }
        if !ch.peers.contains(peer) {
            ch.peers.push(*peer);
        }
        true
    }

    /// A dial that included discovery candidates met a different identity.
    fn distrust_mismatches(&self, peer: &PeerId) {
        let mut ch = lock(&self.inner.changed);
        if !ch.untrusted.contains(peer) {
            ch.untrusted.push(*peer);
        }
    }

    /// The peer proved its paired identity (paired again, or connected to
    /// us with it): an earlier "changed" result no longer holds.
    fn forget_changed(&self, peer: &PeerId) {
        let mut ch = lock(&self.inner.changed);
        ch.peers.retain(|p| p != peer);
        *ch.epochs.entry(*peer).or_default() += 1;
    }

    /// Revoke a peer: removed from the store, so it can no longer connect,
    /// and anything it has open now (a transfer, a control session) ends.
    /// False when it was not paired, and also when the removal could not be
    /// saved (it is then still paired); [`Self::try_unpair`] tells them apart.
    /// Closes every open connection with `reason` (the "Disconnect everything" action). Pairing is untouched:
    /// the peers can connect again whenever this device allows it.
    pub async fn close_all(&self, reason: &[u8]) {
        let conns: Vec<_> = lock(&self.inner.live)
            .drain(..)
            .filter_map(|(_, w)| w.upgrade())
            .collect();
        for c in &conns {
            c.close(CLOSE_DISCONNECTED.into(), reason);
        }
    }

    /// Admits a file offer from `peer` against its quota (at most 30 a minute, and its in-flight bytes plus this
    /// one must fit in `free`, the free space on the disk). Hold the permit while the file is received; dropping
    /// it releases the reservation. A refusal is the answer to send back (too many requests, or no space).
    pub fn offer_permit(
        &self,
        peer: PeerId,
        size: u64,
        free: u64,
    ) -> Result<quota::OfferPermit, quota::QuotaRefusal> {
        self.inner.quotas.offer(peer, size, free, Instant::now())
    }

    pub fn unpair(&self, peer: &PeerId) -> bool {
        self.try_unpair(peer).unwrap_or(false)
    }

    /// [`Self::unpair`] for the user's Remove button: `Ok(false)` when it was
    /// not paired, [`Error::Storage`] when the removal could not be saved.
    /// After an error nothing changed: it is still paired (in memory and on
    /// disk), its connections stay open, and the user must be told.
    pub fn try_unpair(&self, peer: &PeerId) -> Result<bool, Error> {
        // Store first: a mismatch noted after the bump below sees it gone.
        let removed = lock(&self.inner.store).try_remove(peer).map_err(storage)?;
        // `untrusted` stays (see there). The epoch entry stays too, so a
        // later pairing never reuses a value an old dial captured.
        self.forget_changed(peer);
        lock(&self.inner.candidates).remove(peer);
        self.inner.quotas.forget(peer);
        lock(&self.inner.live).retain(|(p, w)| {
            if p != peer {
                return true;
            }
            if let Some(c) = w.upgrade() {
                c.close(CLOSE_NOT_PAIRED.into(), b"unpaired");
            }
            false
        });
        Ok(removed)
    }

    /// Save a pairing the user is told about, off the async threads (the
    /// store writes and flushes a file).
    async fn store_try_pin(&self, peer: PairedPeer) -> Result<(), Error> {
        let store = self.inner.store.clone();
        tokio::task::spawn_blocking(move || lock(&store).try_pin(peer))
            .await
            .map_err(|e| Error::Storage(e.to_string()))?
            .map_err(storage)
    }

    /// Take back a pairing saved by [`Self::store_try_pin`].
    async fn store_try_remove(&self, peer: PeerId) -> Result<bool, Error> {
        let store = self.inner.store.clone();
        tokio::task::spawn_blocking(move || lock(&store).try_remove(&peer))
            .await
            .map_err(|e| Error::Storage(e.to_string()))?
            .map_err(storage)
    }

    /// Remember an authenticated connection (and forget finished ones).
    fn track(&self, peer: PeerId, conn: &Connection) {
        let mut live = lock(&self.inner.live);
        live.retain(|(_, w)| w.upgrade().is_some_and(|c| c.close_reason().is_none()));
        live.push((peer, conn.weak_handle()));
    }

    /// Connections still open (tests check nothing is kept alive).
    #[cfg(test)]
    fn live_count(&self) -> usize {
        lock(&self.inner.live)
            .iter()
            .filter(|(_, w)| w.upgrade().is_some_and(|c| c.close_reason().is_none()))
            .count()
    }

    /// Open a pairing window and return the code to show. Replaces any
    /// earlier code. Valid for [`pairing::PAIRING_TTL`], single use.
    pub fn open_pairing(&self, addrs: Vec<String>) -> Result<PairingCode, Error> {
        self.open_pairing_for(addrs, pairing::PAIRING_TTL)
    }

    /// Like [`Self::open_pairing`] with a shorter lifetime, for rotating
    /// codes. Issuing a new code invalidates the previous one immediately.
    pub fn open_pairing_for(
        &self,
        addrs: Vec<String>,
        ttl: Duration,
    ) -> Result<PairingCode, Error> {
        let ticket = Ticket::with_ttl(Instant::now(), ttl)?;
        let code = PairingCode {
            host_id: *self.inner.me.as_bytes(),
            token: ticket.token(),
            addrs,
        };
        *lock(&self.inner.ticket) = Some(ticket);
        // A new code starts its global count again; a source that was blocked stays blocked.
        lock(&self.inner.bad_tokens).new_ticket();
        Ok(code)
    }

    pub fn close_pairing(&self) {
        *lock(&self.inner.ticket) = None;
    }

    /// Join a pairing window on another device using its code.
    pub async fn join(&self, code: &PairingCode) -> Result<Joining, Error> {
        let host =
            PeerId::from_bytes(code.host_id).map_err(|_| Error::Pairing(PairError::Malformed))?;
        let conn = self.dial(host, &code.addrs, PAIR_ALPN).await?;
        let (mut send, recv) = conn.open_bi().await.map_err(tx)?;
        let mut recv = FrameReader::new(recv);
        let (join, start) = JoinPairing::start(
            code,
            self.inner.me,
            self.inner.device_name.clone(),
            self.addrs(),
        )?;
        write_frame(&mut send, &Pair::from(PairMsg::Start(start))).await?;
        // A refused code makes the host close the connection (code 400).
        let first = tokio::time::timeout(HANDSHAKE_TIMEOUT, recv.next::<Pair>())
            .await
            .map_err(|_| Error::Timeout)?
            .map_err(|e| closed_by_peer(&conn, e))?;
        let nonce = match first.and_then(|p| p.msg) {
            Some(PairMsg::Nonce(n)) => n,
            _ => {
                conn.close(CLOSE_PAIR_FAILED.into(), b"pairing refused");
                return Err(Error::Rejected);
            }
        };
        let (reveal, sas) = join.on_nonce(&nonce)?;
        write_frame(&mut send, &Pair::from(PairMsg::Reveal(reveal))).await?;
        let peer = PairedPeer {
            id: host,
            name: pb_identity::clean_peer_name(&nonce.device_name),
            addrs: pb_identity::clean_peer_addrs(&code.addrs),
        };
        Ok(Joining {
            peer_name: pb_identity::clean_peer_name(&nonce.device_name),
            sas,
            conn,
            send,
            recv,
            peer,
            node: self.clone(),
        })
    }

    /// Open a session to a paired peer using its stored addresses.
    pub async fn connect(&self, peer: &PeerId) -> Result<Session, Error> {
        let known = lock(&self.inner.store)
            .get(peer)
            .ok_or(Error::UnknownPeer)?;
        let epoch = self.changed_epoch(peer);
        let conn = match self.dial_known(*peer, &known.addrs).await {
            Ok(c) => {
                self.forget_changed(peer);
                c
            }
            // Known limit: iroh keeps every address dialled for this peer as
            // a path and tries them all on the next connect. If another
            // device keeps answering at an old *saved* address while the
            // paired one is reachable elsewhere, its handshake can win and
            // land here although the peer did not change. The phone's next
            // incoming session clears the flag (`accept_session`); a real fix
            // needs a way to drop a remote path in iroh. Forged discovery
            // candidates that fail a dial are handled in `dial_known` (one
            // retry with the saved addresses, then `Changed::untrusted`).
            // Remaining gap, accepted: a planted address that was dialled in
            // a batch that SUCCEEDED (or timed out) stays a remembered path
            // but the peer is not distrusted, so if the phone then goes
            // offline and the planter answers there, this can still read as
            // a reset. The user-facing copy says another device may be the
            // cause, and removing the pairing asks for confirmation.
            Err(e) if is_identity_mismatch(&e) => {
                if self.note_mismatch(peer, epoch) {
                    return Err(Error::PeerChanged);
                }
                if lock(&self.inner.store).get(peer).is_none() {
                    return Err(Error::UnknownPeer);
                }
                return Err(Error::Transport(
                    "a device with another identity answered for this peer".into(),
                ));
            }
            Err(e) => return Err(e),
        };
        self.track(*peer, &conn);
        // Unpaired while we were dialling: do not use the new connection.
        if lock(&self.inner.store).get(peer).is_none() {
            conn.close(CLOSE_NOT_PAIRED.into(), b"unpaired");
            return Err(Error::UnknownPeer);
        }
        let started = async {
            let (send, recv) = conn.open_bi().await.map_err(tx)?;
            Session::start(
                conn.clone(),
                *peer,
                send,
                recv,
                &self.inner.device_name,
                &known.name,
            )
            .await
        }
        .await;
        // Whichever step noticed the peer's close, report why it closed.
        started.map_err(|e| closed_by_peer(&conn, e))
    }

    /// Tell the node the device's network changed (a roam, another band, a new address). iroh can't
    /// see this on Android, so the app says so; calling it when nothing changed does no harm.
    pub async fn network_changed(&self) {
        self.inner.endpoint.network_change().await;
    }

    pub async fn shutdown(&self) {
        self.stop_discovery();
        self.inner.endpoint.close().await;
    }

    /// Dial a paired peer, recovering from a network change: addresses
    /// learned by discovery and the saved ones are tried together; if none
    /// answers, probe the local network and try what answers. Learned
    /// addresses are saved only after the authenticated connection succeeds.
    async fn dial_known(&self, peer: PeerId, saved: &[String]) -> Result<Connection, Error> {
        let try_candidates = |cands: Vec<SocketAddr>, also: &[String], wait: Duration| {
            let mut strs: Vec<String> = cands.iter().map(|a| a.to_string()).collect();
            strs.extend(
                also.iter()
                    .filter(|a| !strs.contains(a))
                    .cloned()
                    .collect::<Vec<_>>(),
            );
            async move {
                match self.dial_for(peer, &strs, SESSION_ALPN, wait).await {
                    Ok(c) => {
                        // Save only the address the authenticated connection
                        // actually uses (QUIC validated it with the peer's key),
                        // never the rest of the batch. It may be neither a
                        // candidate nor saved: iroh can move to a better path.
                        if let Some(used) = used_addr(&c) {
                            self.confirm_addrs(&peer, &[used]).await;
                        }
                        Ok(c)
                    }
                    Err(e) => {
                        // Stale or forged: do not try them again on every connect.
                        self.drop_candidates(&peer, &cands);
                        Err(e)
                    }
                }
            }
        };
        // All paths are tried at once, so a stale or forged candidate costs
        // nothing when a saved address works (and vice versa).
        let cands = self.candidates(&peer);
        let had_candidates = !cands.is_empty();
        let unsaved = has_unsaved_candidate(&cands, saved);
        let mut first = try_candidates(cands, saved, Duration::from_secs(15)).await;
        if had_candidates && matches!(&first, Err(e) if is_identity_mismatch(e)) {
            // The wrong key may have come from a planted candidate (any LAN
            // host can send the discovery tag); those were dropped above.
            // Ask the saved addresses alone once. iroh may still send to the
            // planted address (it remembers dialled paths), so from now on
            // this peer's mismatches prove nothing (`Changed::untrusted`).
            if unsaved {
                self.distrust_mismatches(&peer);
            }
            if !saved.is_empty() {
                first = try_candidates(Vec::new(), saved, Duration::from_secs(15)).await;
            }
        }
        if first.is_ok() || lock(&self.inner.discovery).is_none() {
            return first;
        }
        // Saved addresses failed: maybe the network changed. Ask once and
        // wait for an answer (failed candidates were dropped above, so
        // whatever appears now is fresh). While a peer is away, every copy
        // or send fails here: probe at most every 30 s, not on each one.
        {
            let mut last = lock(&self.inner.last_probe);
            if last.is_some_and(|t| t.elapsed() < Duration::from_secs(30)) {
                return first;
            }
            *last = Some(Instant::now());
        }
        self.rediscover().await;
        for _ in 0..30 {
            let cands = self.candidates(&peer);
            if !cands.is_empty() {
                let unsaved = has_unsaved_candidate(&cands, saved);
                let r = try_candidates(cands, &[], Duration::from_secs(10)).await;
                if matches!(&r, Err(e) if is_identity_mismatch(e)) {
                    // Only learned addresses were asked: the saved ones
                    // already gave their answer (`first`).
                    if unsaved {
                        self.distrust_mismatches(&peer);
                    }
                    return first;
                }
                return r;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        first
    }

    async fn dial(&self, peer: PeerId, addrs: &[String], alpn: &[u8]) -> Result<Connection, Error> {
        self.dial_for(peer, addrs, alpn, Duration::from_secs(15))
            .await
    }

    async fn dial_for(
        &self,
        peer: PeerId,
        addrs: &[String],
        alpn: &[u8],
        wait: Duration,
    ) -> Result<Connection, Error> {
        let id = EndpointId::from_bytes(peer.as_bytes()).map_err(tx)?;
        let parts = addrs
            .iter()
            .filter_map(|a| a.parse::<SocketAddr>().ok())
            .map(TransportAddr::Ip);
        let addr = EndpointAddr::from_parts(id, parts);
        tokio::time::timeout(wait, self.inner.endpoint.connect(addr, alpn))
            .await
            .map_err(|_| Error::Timeout)?
            .map_err(tx)
    }

    async fn accept_loop(self) {
        // Bound the work any LAN host can make us do: a few connections in
        // flight at once; the rest are refused until one finishes.
        let slots = Arc::new(tokio::sync::Semaphore::new(MAX_INCOMING));
        let pair_slots = Arc::new(tokio::sync::Semaphore::new(MAX_PAIRING));
        while let Some(incoming) = self.inner.endpoint.accept().await {
            let Ok(permit) = slots.clone().try_acquire_owned() else {
                drop(incoming);
                continue;
            };
            let node = self.clone();
            let pair_slots = pair_slots.clone();
            tokio::spawn(async move {
                let Ok(Ok(conn)) = tokio::time::timeout(INCOMING_HANDSHAKE, incoming).await else {
                    return;
                };
                let Ok(peer) = PeerId::from_bytes(*conn.remote_id().as_bytes()) else {
                    return;
                };
                if conn.alpn() == PAIR_ALPN {
                    // Pairing is only possible while this device shows a code.
                    if lock(&node.inner.ticket).is_none() {
                        conn.close(CLOSE_PAIR_FAILED.into(), b"not pairing");
                        return;
                    }
                    drop(permit);
                    let Ok(_pairing) = pair_slots.try_acquire_owned() else {
                        conn.close(CLOSE_PAIR_FAILED.into(), b"busy");
                        return;
                    };
                    // Time-boxed as a whole: a stalled peer cannot hold the slot for ever.
                    let _ = pairing::within_slot(node.host_pairing(conn, peer)).await;
                } else {
                    // A paired peer is authenticated: free the slot for the
                    // next handshake while its session runs.
                    drop(permit);
                    node.accept_session(conn, peer).await;
                }
            });
        }
    }

    async fn accept_session(&self, conn: Connection, peer: PeerId) {
        // Authorisation happens before any application stream is read.
        let Some(known) = lock(&self.inner.store).get(&peer) else {
            conn.close(CLOSE_NOT_PAIRED.into(), b"not paired");
            return;
        };
        // A paired peer may only have so many sessions open at once; the rest are told it is busy.
        let Some(permit) = self.inner.quotas.open_session(peer) else {
            conn.close(CLOSE_BUSY.into(), b"busy");
            return;
        };
        // The slot is held for as long as the connection lives.
        let held = conn.clone();
        tokio::spawn(async move {
            let _ = held.closed().await;
            drop(permit);
        });
        // The TLS handshake proved it holds the paired key: not changed.
        self.forget_changed(&peer);
        self.track(peer, &conn);
        // Unpaired between the check above and `track`: `unpair` closed only
        // the connections it could see, not this one.
        if lock(&self.inner.store).get(&peer).is_none() {
            conn.close(CLOSE_NOT_PAIRED.into(), b"unpaired");
            return;
        }
        let Ok(Ok((send, recv))) = tokio::time::timeout(HEARTBEAT * 3, conn.accept_bi()).await
        else {
            return;
        };
        if let Ok(session) =
            Session::start(conn, peer, send, recv, &self.inner.device_name, &known.name).await
        {
            let _ = self.inner.sessions.send(session).await;
        }
    }

    async fn host_pairing(&self, conn: Connection, joiner: PeerId) -> Result<(), Error> {
        let fail = |conn: &Connection| conn.close(CLOSE_PAIR_FAILED.into(), b"pairing failed");
        // The joiner is not authenticated yet: every step before the user's
        // decision is time-boxed.
        let (mut send, recv) = tokio::time::timeout(HANDSHAKE_TIMEOUT, conn.accept_bi())
            .await
            .map_err(|_| Error::Timeout)?
            .map_err(tx)?;
        let mut recv = FrameReader::new(recv);
        let first = tokio::time::timeout(HANDSHAKE_TIMEOUT, recv.next::<Pair>())
            .await
            .map_err(|_| Error::Timeout)??;
        let Some(PairMsg::Start(start)) = first.and_then(|p| p.msg) else {
            fail(&conn);
            return Err(Error::Protocol("expected PairStart".into()));
        };
        let started = {
            let mut ticket = lock(&self.inner.ticket);
            let mut limits = lock(&self.inner.bad_tokens);
            // The remote IP is the source; a relayed connection has none, so those share one source.
            let source = used_addr(&conn).map_or(std::net::IpAddr::from([0, 0, 0, 0]), |a| a.ip());
            HostPairing::start(
                &mut ticket,
                &mut limits,
                source,
                self.inner.me,
                joiner,
                &start,
                Instant::now(),
            )
        };
        let (host, mut nonce) = match started {
            Ok(v) => v,
            Err(e) => {
                fail(&conn);
                return Err(e.into());
            }
        };
        nonce.device_name = self.inner.device_name.clone();
        write_frame(&mut send, &Pair::from(PairMsg::Nonce(nonce))).await?;
        let second = tokio::time::timeout(HANDSHAKE_TIMEOUT, recv.next::<Pair>())
            .await
            .map_err(|_| Error::Timeout)??;
        let Some(PairMsg::Reveal(reveal)) = second.and_then(|p| p.msg) else {
            fail(&conn);
            return Err(Error::Protocol("expected PairReveal".into()));
        };
        let sas = match host.reveal(&reveal) {
            Ok(s) => s,
            Err(e) => {
                fail(&conn);
                return Err(e.into());
            }
        };

        let (decide_tx, decide_rx) = oneshot::channel();
        let (done_tx, done_rx) = oneshot::channel();
        let pending = PendingPairing {
            peer: joiner,
            peer_name: pb_identity::clean_peer_name(&start.device_name),
            sas,
            decide: decide_tx,
            done: done_rx,
            conn: conn.clone(),
        };
        if self.inner.pairings.send(pending).await.is_err() {
            fail(&conn);
            return Err(Error::Closed);
        }
        // A joiner that leaves first ends this at once (its prompt is shut
        // through `PendingPairing::withdrawn`).
        let accept = tokio::select! {
            r = tokio::time::timeout(CONFIRM_TIMEOUT, decide_rx) => r.unwrap_or(Ok(false)).unwrap_or(false),
            _ = conn.closed() => false,
        };
        // The joiner sends its Confirm before reading ours, so read it first:
        // the pairing is then saved (or not) before we answer, and the joiner
        // never believes in a pairing this side failed to save.
        let theirs = tokio::time::timeout(CONFIRM_TIMEOUT, recv.next::<Pair>()).await;
        let remote_ok = matches!(
            theirs,
            Ok(Ok(Some(Pair {
                msg: Some(PairMsg::Confirm(PairConfirm { accepted: true }))
            })))
        );
        let peer = PairedPeer {
            id: joiner,
            name: pb_identity::clean_peer_name(&start.device_name),
            addrs: pb_identity::clean_peer_addrs(&start.addrs),
        };
        let mut result = if accept && remote_ok {
            self.store_try_pin(peer.clone()).await.map(|()| peer)
        } else {
            Err(Error::Rejected)
        };
        let confirmed = write_frame(
            &mut send,
            &Pair::from(PairMsg::Confirm(PairConfirm {
                accepted: result.is_ok(),
            })),
        )
        .await;
        let _ = send.finish();
        // Our finish tells the joiner it may close; wait for that rather than
        // closing abruptly. Its close code says whether it kept the pairing.
        let closed = match confirmed {
            Ok(()) => tokio::time::timeout(Duration::from_secs(5), conn.closed())
                .await
                .ok(),
            Err(_) => None,
        };
        let joiner_lost_it = matches!(
            &closed,
            Some(ConnectionError::ApplicationClosed(c))
                if c.error_code == CLOSE_PAIR_NOT_SAVED.into()
        );
        if let Ok(p) = &result {
            if joiner_lost_it || confirmed.is_err() {
                // The joiner did not get, or could not keep, the pairing:
                // take ours back so neither side is paired alone.
                let _ = self.store_try_remove(p.id).await;
                result = Err(Error::Storage(
                    "the other device could not save the pairing".into(),
                ));
            } else {
                self.forget_changed(&p.id);
            }
        }
        let _ = done_tx.send(result);
        Ok(())
    }
}

/// Turn a failure on a connection the peer closed into a meaningful error.
/// The direct address a connection is using (the selected path).
fn used_addr(conn: &Connection) -> Option<SocketAddr> {
    let paths = conn.paths();
    let mut any = None;
    for p in paths.iter() {
        if let TransportAddr::Ip(a) = p.remote_addr() {
            if p.is_selected() {
                return Some(*a);
            }
            any.get_or_insert(*a);
        }
    }
    any
}

fn closed_by_peer(conn: &Connection, fallback: Error) -> Error {
    match conn.close_reason() {
        Some(ConnectionError::ApplicationClosed(c)) if c.error_code == CLOSE_NOT_PAIRED.into() => {
            Error::NotPaired
        }
        Some(ConnectionError::ApplicationClosed(c)) if c.error_code == CLOSE_PAIR_FAILED.into() => {
            Error::Rejected
        }
        Some(ConnectionError::ApplicationClosed(c)) if c.error_code == CLOSE_BUSY.into() => {
            Error::Busy
        }
        Some(ConnectionError::ApplicationClosed(c))
            if c.error_code == CLOSE_INCOMPATIBLE.into() =>
        {
            Error::VersionMismatch(pb_protocol::side_from_close_reason(&c.reason))
        }
        _ => fallback,
    }
}

/// Reads length-prefixed frames from a QUIC stream.
struct FrameReader {
    recv: RecvStream,
    buf: Vec<u8>,
}

impl FrameReader {
    fn new(recv: RecvStream) -> Self {
        Self {
            recv,
            buf: Vec::new(),
        }
    }

    /// Next message, or `None` when the peer finished the stream.
    async fn next<M: Message + Default>(&mut self) -> Result<Option<M>, Error> {
        loop {
            if let Some((msg, used)) =
                decode_frame::<M>(&self.buf).map_err(|e| Error::Protocol(e.to_string()))?
            {
                self.buf.drain(..used);
                return Ok(Some(msg));
            }
            let mut chunk = [0u8; 4096];
            match self.recv.read(&mut chunk).await.map_err(tx)? {
                Some(n) => self.buf.extend_from_slice(&chunk[..n]),
                None if self.buf.is_empty() => return Ok(None),
                None => return Err(Error::Protocol("stream ended mid-frame".into())),
            }
        }
    }
}

async fn write_frame<M: Message>(send: &mut SendStream, msg: &M) -> Result<(), Error> {
    let bytes = encode_frame(msg).map_err(|e| Error::Protocol(e.to_string()))?;
    send.write_all(&bytes).await.map_err(tx)
}

/// An authenticated session with a paired peer.
pub struct Session {
    pub peer: PeerId,
    pub peer_name: String,
    pub remote_hello: Hello,
    conn: Connection,
    send: Arc<tokio::sync::Mutex<SendStream>>,
    recv: FrameReader,
    heartbeat: tokio::task::JoinHandle<()>,
    /// How long `recv` waits for the peer before [`Error::Timeout`].
    watchdog: Duration,
    /// When the peer last sent anything: `recv`'s deadline counts from here,
    /// not from each call, so a `select!` that keeps cancelling `recv` still
    /// notices a silent peer.
    last_rx: Instant,
}

impl Session {
    async fn start(
        conn: Connection,
        peer: PeerId,
        mut send: SendStream,
        recv: RecvStream,
        my_name: &str,
        peer_name: &str,
    ) -> Result<Self, Error> {
        let hello = Hello::new(my_name, pb_protocol::local_capabilities());
        write_frame(&mut send, &Control::from(ControlMsg::Hello(hello))).await?;
        let mut recv = FrameReader::new(recv);
        let first = tokio::time::timeout(HEARTBEAT * 3, recv.next::<Control>()).await;
        let remote_hello = match first {
            Ok(Ok(Some(Control {
                msg: Some(ControlMsg::Hello(h)),
            }))) => h,
            Ok(Err(e)) => return Err(closed_by_peer(&conn, e)),
            Err(_) => return Err(Error::Timeout),
            _ => return Err(Error::Protocol("expected Hello".into())),
        };
        let local = Hello::new(my_name, Vec::new());
        if let Some(side) = pb_protocol::update_side(&local, &remote_hello) {
            // Its own code: 400 reads as "pairing refused" on the other side. The reason says
            // which device must update, so the other side can name it too.
            conn.close(
                CLOSE_INCOMPATIBLE.into(),
                pb_protocol::close_reason_for(side),
            );
            return Err(Error::VersionMismatch(side));
        }
        let send = Arc::new(tokio::sync::Mutex::new(send));
        let hb_send = send.clone();
        let heartbeat = tokio::spawn(async move {
            let mut seq = 0u64;
            loop {
                tokio::time::sleep(HEARTBEAT).await;
                seq += 1;
                let msg = Control::from(ControlMsg::Heartbeat(Heartbeat { seq }));
                if write_frame(&mut *hb_send.lock().await, &msg).await.is_err() {
                    break;
                }
            }
        });
        Ok(Self {
            peer,
            peer_name: peer_name.to_owned(),
            remote_hello,
            conn,
            send,
            recv,
            heartbeat,
            watchdog: DEFAULT_WATCHDOG,
            last_rx: Instant::now(),
        })
    }

    /// Waits this long for a silent peer instead of [`DEFAULT_WATCHDOG`].
    pub fn set_watchdog(&mut self, watchdog: Duration) {
        self.watchdog = watchdog;
    }

    pub async fn send(&self, msg: ControlMsg) -> Result<(), Error> {
        write_frame(&mut *self.send.lock().await, &Control::from(msg)).await
    }

    /// Next application message. Heartbeats are consumed here; silence for the
    /// watchdog (three heartbeat intervals unless set) ends the session with [`Error::Timeout`].
    pub async fn recv(&mut self) -> Result<Option<ControlMsg>, Error> {
        loop {
            let deadline = tokio::time::Instant::from_std(self.last_rx + self.watchdog);
            let next = tokio::time::timeout_at(deadline, self.recv.next::<Control>())
                .await
                .map_err(|_| Error::Timeout)??;
            self.last_rx = Instant::now();
            match next.map(|c| c.msg) {
                None => return Ok(None),
                Some(Some(ControlMsg::Heartbeat(_))) | Some(None) => continue,
                Some(Some(m)) => return Ok(Some(m)),
            }
        }
    }

    pub async fn finish(self) {
        self.heartbeat.abort();
        let _ = self.send.lock().await.finish();
        let _ = tokio::time::timeout(Duration::from_secs(2), self.conn.closed()).await;
        self.conn.close(0u32.into(), b"bye");
    }
}

/// Ends a session from elsewhere, e.g. the user stops PC control while the
/// input loop owns the [`Session`]. Weak: it does not keep the connection open.
#[derive(Clone)]
pub struct SessionCloser(iroh::endpoint::WeakConnectionHandle);

impl SessionCloser {
    pub fn close(&self, reason: &[u8]) {
        if let Some(c) = self.0.upgrade() {
            c.close(0u32.into(), reason);
        }
    }
}

impl Session {
    /// Resolves when the connection closes (either side, or timeout).
    pub async fn closed(&self) {
        let _ = self.conn.closed().await;
    }

    /// The peer's address on the path this session uses now.
    pub fn remote_addr(&self) -> Option<SocketAddr> {
        used_addr(&self.conn)
    }

    /// A handle that can end this session from another task.
    pub fn closer(&self) -> SessionCloser {
        SessionCloser(self.conn.weak_handle())
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.heartbeat.abort();
    }
}

#[cfg(test)]
mod tests;
