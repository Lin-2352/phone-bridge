//! Phone Bridge transport: an iroh QUIC [`Node`] that only talks to paired
//! peers, the pairing exchange, and [`Session`]s carrying control messages.
//!
//! Privacy defaults: `presets::Minimal` (no n0 relay, no DNS/pkarr publishing),
//! so nothing leaves the local network unless a relay is configured later.
//! GSO is disabled on Android (see docs/DECISIONS.md, finding 1).

pub mod pairing;

use std::{
    fmt,
    net::SocketAddr,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use iroh::{
    endpoint::{presets, Connection, ConnectionError, QuicTransportConfig, RecvStream, SendStream},
    Endpoint, EndpointAddr, EndpointId, RelayMode, SecretKey, TransportAddr,
};
use pairing::{HostPairing, JoinPairing, PairError, Ticket};
use pb_identity::{Identity, PairedPeer, PeerId, PeerStore};
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
/// How long either side waits for the user to confirm the SAS.
pub const CONFIRM_TIMEOUT: Duration = Duration::from_secs(120);

const CLOSE_NOT_PAIRED: u32 = 403;
const CLOSE_PAIR_FAILED: u32 = 400;

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

pub type SharedStore = Arc<Mutex<dyn PeerStore + Send>>;

pub struct NodeConfig {
    /// User-visible name of this device, sent only to paired / pairing peers.
    pub device_name: String,
    /// Fixed UDP port; `None` picks a free one.
    pub port: Option<u16>,
}

/// A pairing request waiting for the local user's decision.
pub struct PendingPairing {
    pub peer: PeerId,
    pub peer_name: String,
    pub sas: String,
    decide: oneshot::Sender<bool>,
    done: oneshot::Receiver<Result<PairedPeer, Error>>,
}

impl PendingPairing {
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
    store: SharedStore,
}

impl Joining {
    pub async fn decide(mut self, accept: bool) -> Result<PairedPeer, Error> {
        write_frame(
            &mut self.send,
            &Pair::from(PairMsg::Confirm(PairConfirm { accepted: accept })),
        )
        .await?;
        let theirs = tokio::time::timeout(CONFIRM_TIMEOUT, self.recv.next::<Pair>())
            .await
            .map_err(|_| Error::Timeout)??;
        let remote_ok = matches!(
            theirs.and_then(|p| p.msg),
            Some(PairMsg::Confirm(PairConfirm { accepted: true }))
        );
        // Close handshake: the host finishes its stream only after reading our
        // Confirm, so once we see its end-of-stream both Confirms were read
        // and closing cannot discard anything.
        let _ = self.send.finish();
        let _ = tokio::time::timeout(Duration::from_secs(5), self.recv.next::<Pair>()).await;
        self.conn.close(0u32.into(), b"paired");
        if accept && remote_ok {
            lock(&self.store).pin(self.peer.clone());
            Ok(self.peer)
        } else {
            Err(Error::Rejected)
        }
    }
}

struct Inner {
    endpoint: Endpoint,
    me: PeerId,
    device_name: String,
    store: SharedStore,
    ticket: Mutex<Option<Ticket>>,
    pairings: mpsc::Sender<PendingPairing>,
    sessions: mpsc::Sender<Session>,
}

fn lock<T: ?Sized>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

/// This device on the network.
#[derive(Clone)]
pub struct Node {
    inner: Arc<Inner>,
}

/// Receivers for events the node produces; keep them polled.
pub struct NodeEvents {
    pub pairings: mpsc::Receiver<PendingPairing>,
    pub sessions: mpsc::Receiver<Session>,
}

impl Node {
    pub async fn bind(
        identity: &Identity,
        store: SharedStore,
        config: NodeConfig,
    ) -> Result<(Self, NodeEvents), Error> {
        let transport = QuicTransportConfig::builder()
            .enable_segmentation_offload(!cfg!(target_os = "android"))
            .keep_alive_interval(HEARTBEAT)
            .build();
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
            inner: Arc::new(Inner {
                endpoint,
                me: identity.peer_id(),
                device_name: config.device_name,
                store,
                ticket: Mutex::new(None),
                pairings: pair_tx,
                sessions: sess_tx,
            }),
        };
        tokio::spawn(node.clone().accept_loop());
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
    pub fn addrs(&self) -> Vec<String> {
        self.inner
            .endpoint
            .addr()
            .ip_addrs()
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

    /// Revoke a peer: removed from the store, so it can no longer connect.
    pub fn unpair(&self, peer: &PeerId) -> bool {
        lock(&self.inner.store).remove(peer)
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
        let first = recv
            .next::<Pair>()
            .await
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
            name: nonce.device_name.clone(),
            addrs: code.addrs.clone(),
        };
        Ok(Joining {
            peer_name: nonce.device_name,
            sas,
            conn,
            send,
            recv,
            peer,
            store: self.inner.store.clone(),
        })
    }

    /// Open a session to a paired peer using its stored addresses.
    pub async fn connect(&self, peer: &PeerId) -> Result<Session, Error> {
        let known = lock(&self.inner.store)
            .get(peer)
            .ok_or(Error::UnknownPeer)?;
        let conn = self.dial(*peer, &known.addrs, SESSION_ALPN).await?;
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

    pub async fn shutdown(&self) {
        self.inner.endpoint.close().await;
    }

    async fn dial(&self, peer: PeerId, addrs: &[String], alpn: &[u8]) -> Result<Connection, Error> {
        let id = EndpointId::from_bytes(peer.as_bytes()).map_err(tx)?;
        let parts = addrs
            .iter()
            .filter_map(|a| a.parse::<SocketAddr>().ok())
            .map(TransportAddr::Ip);
        let addr = EndpointAddr::from_parts(id, parts);
        tokio::time::timeout(
            Duration::from_secs(15),
            self.inner.endpoint.connect(addr, alpn),
        )
        .await
        .map_err(|_| Error::Timeout)?
        .map_err(tx)
    }

    async fn accept_loop(self) {
        while let Some(incoming) = self.inner.endpoint.accept().await {
            let node = self.clone();
            tokio::spawn(async move {
                let Ok(conn) = incoming.await else { return };
                let Ok(peer) = PeerId::from_bytes(*conn.remote_id().as_bytes()) else {
                    return;
                };
                if conn.alpn() == PAIR_ALPN {
                    let _ = node.host_pairing(conn, peer).await;
                } else {
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
        let (mut send, recv) = conn.accept_bi().await.map_err(tx)?;
        let mut recv = FrameReader::new(recv);
        let Some(PairMsg::Start(start)) = recv.next::<Pair>().await?.and_then(|p| p.msg) else {
            fail(&conn);
            return Err(Error::Protocol("expected PairStart".into()));
        };
        let started = {
            let mut ticket = lock(&self.inner.ticket);
            HostPairing::start(&mut ticket, self.inner.me, joiner, &start, Instant::now())
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
        let Some(PairMsg::Reveal(reveal)) = recv.next::<Pair>().await?.and_then(|p| p.msg) else {
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
            peer_name: start.device_name.clone(),
            sas,
            decide: decide_tx,
            done: done_rx,
        };
        if self.inner.pairings.send(pending).await.is_err() {
            fail(&conn);
            return Err(Error::Closed);
        }
        let accept = tokio::time::timeout(CONFIRM_TIMEOUT, decide_rx)
            .await
            .unwrap_or(Ok(false))
            .unwrap_or(false);
        write_frame(
            &mut send,
            &Pair::from(PairMsg::Confirm(PairConfirm { accepted: accept })),
        )
        .await?;
        let theirs = tokio::time::timeout(CONFIRM_TIMEOUT, recv.next::<Pair>()).await;
        let remote_ok = matches!(
            theirs,
            Ok(Ok(Some(Pair {
                msg: Some(PairMsg::Confirm(PairConfirm { accepted: true }))
            })))
        );
        let _ = send.finish();
        let result = if accept && remote_ok {
            let peer = PairedPeer {
                id: joiner,
                name: start.device_name,
                addrs: start.addrs,
            };
            lock(&self.inner.store).pin(peer.clone());
            Ok(peer)
        } else {
            Err(Error::Rejected)
        };
        // Our finish (above, after reading the joiner's Confirm) tells the
        // joiner it may close; wait for that rather than closing abruptly.
        let _ = tokio::time::timeout(Duration::from_secs(5), conn.closed()).await;
        let _ = done_tx.send(result);
        Ok(())
    }
}

/// Turn a failure on a connection the peer closed into a meaningful error.
fn closed_by_peer(conn: &Connection, fallback: Error) -> Error {
    match conn.close_reason() {
        Some(ConnectionError::ApplicationClosed(c)) if c.error_code == CLOSE_NOT_PAIRED.into() => {
            Error::NotPaired
        }
        Some(ConnectionError::ApplicationClosed(c)) if c.error_code == CLOSE_PAIR_FAILED.into() => {
            Error::Rejected
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
        let hello = Hello::new(my_name, vec!["clipboard".into()]);
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
        if !remote_hello.is_compatible() {
            conn.close(400u32.into(), b"incompatible protocol version");
            return Err(Error::Protocol(format!(
                "peer speaks protocol {}, we speak {}",
                remote_hello.protocol_version,
                pb_protocol::PROTOCOL_VERSION
            )));
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
        })
    }

    pub async fn send(&self, msg: ControlMsg) -> Result<(), Error> {
        write_frame(&mut *self.send.lock().await, &Control::from(msg)).await
    }

    /// Next application message. Heartbeats are consumed here; silence for
    /// three heartbeat intervals ends the session with [`Error::Timeout`].
    pub async fn recv(&mut self) -> Result<Option<ControlMsg>, Error> {
        loop {
            let next = tokio::time::timeout(HEARTBEAT * 3, self.recv.next::<Control>())
                .await
                .map_err(|_| Error::Timeout)??;
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

impl Drop for Session {
    fn drop(&mut self) {
        self.heartbeat.abort();
    }
}

#[cfg(test)]
mod tests;
