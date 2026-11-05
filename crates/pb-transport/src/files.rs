//! AirDrop-style transfers over a session: offer, accept/decline, then the
//! file bytes on their own QUIC stream with an end-to-end SHA-256 check.
//!
//! Sender: [`Node::send_file`] / [`Node::send_text`] open a fresh session per
//! transfer. Receiver: the app's session loop calls [`receive_offer`] when an
//! [`Offer`] arrives. Files are written to `.<name>.partial-<random>` and only
//! renamed into place after the hash matches; any failure deletes the partial.

use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use pb_identity::PeerId;
use pb_protocol::control::{
    ControlMsg, FileHeader, Offer, OfferKind, OfferReply, TransferDone, MAX_CLIPBOARD_IMAGE,
};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncRead, AsyncReadExt};

use crate::{tx, write_frame, Error, FrameReader, Node, Session};

/// Default largest file we accept (4 GiB); apps can pass a different cap.
pub const DEFAULT_MAX_FILE: u64 = 4 << 30;
/// How long a sender waits for the receiver's accept/decline.
pub const OFFER_TIMEOUT: Duration = Duration::from_secs(120);
const CHUNK: usize = 256 * 1024;

/// The next read size: what is left, at most `chunk`. Compared as `u64`, so
/// a remainder of 4 GiB or more is never cut to its low 32 bits on 32-bit.
fn chunk_len(remaining: u64, chunk: usize) -> usize {
    usize::try_from(remaining.min(chunk as u64)).unwrap_or(chunk)
}
/// Received bytes are hashed and written in batches of this size.
const BATCH: usize = 1024 * 1024;
/// Bytes, not characters: file systems limit names to 255 bytes (ext4) and
/// the partial file adds 26 more.
const MAX_NAME_BYTES: usize = 200;
/// Receiver close code: the file did not arrive intact or could not be saved.
pub(crate) const CLOSE_TRANSFER_FAILED: u32 = 400;
/// Receiver close code: the file is saved under its final name. Not 0: the
/// QUIC stack also closes with 0 when a connection is dropped or the
/// endpoint shuts down, which must not read as success.
pub(crate) const CLOSE_SAVED: u32 = 200;

/// Close code of whichever side cancelled the transfer on purpose.
pub(crate) const CLOSE_CANCELLED: u32 = 300;

/// Progress callback: `(bytes_done, bytes_total)`.
pub type Progress = Arc<dyn Fn(u64, u64) + Send + Sync>;

/// Cancels one in-flight send or receive. Cloneable; any clone can cancel.
///
/// Cancelling is cooperative and clean: the transfer stops at its next wait
/// (connecting, waiting for the accept, each chunk), tells the other device
/// (the connection closes with a dedicated code, so it reports
/// [`Error::PeerCancelled`] instead of a transport error) and returns
/// [`Error::Cancelled`]. A receive always deletes its partial file; there is
/// no resume, so no `.partial-*` is ever kept.
#[derive(Clone, Default)]
pub struct Cancel(Arc<CancelInner>);

#[derive(Default)]
struct CancelInner {
    flag: std::sync::atomic::AtomicBool,
    wake: tokio::sync::Notify,
}

impl Cancel {
    pub fn new() -> Self {
        Self::default()
    }

    /// Ask the transfer to stop. Safe from any thread, any number of times.
    pub fn cancel(&self) {
        self.0.flag.store(true, std::sync::atomic::Ordering::SeqCst);
        self.0.wake.notify_waiters();
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.flag.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// Resolves once cancelled (immediately if it already was).
    pub async fn cancelled(&self) {
        loop {
            let notified = self.0.wake.notified();
            tokio::pin!(notified);
            // Registered before the flag check, so a cancel in between is not lost.
            notified.as_mut().enable();
            if self.is_cancelled() {
                return;
            }
            notified.await;
        }
    }
}

impl std::fmt::Debug for Cancel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Cancel({})", self.is_cancelled())
    }
}

/// Optional extras of a transfer: progress reports and a cancel handle.
#[derive(Clone, Default)]
pub struct TransferOpts {
    pub progress: Option<Progress>,
    pub cancel: Option<Cancel>,
}

impl TransferOpts {
    pub fn new(progress: Option<Progress>, cancel: Option<Cancel>) -> Self {
        Self { progress, cancel }
    }
}

/// Run `f`, or give up with [`Error::Cancelled`] as soon as `cancel` fires.
async fn or_cancel<T>(
    cancel: &Option<Cancel>,
    f: impl std::future::Future<Output = Result<T, Error>>,
) -> Result<T, Error> {
    match cancel {
        None => f.await,
        Some(c) => tokio::select! {
            biased;
            _ = c.cancelled() => Err(Error::Cancelled),
            r = f => r,
        },
    }
}

/// What made a receive refuse a file before taking it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// Over the size cap.
    OverCap,
    /// Not enough free space in the destination folder.
    NoSpace,
}

/// Whether a file of `size` bytes would be refused (cap, then disk space).
/// The same check [`receive_offer`] makes, so an app can warn first.
pub fn refusal(dir: &Path, size: u64, max_size: u64) -> Option<Refusal> {
    if size > max_size {
        Some(Refusal::OverCap)
    } else if !room_for(dir, size) {
        Some(Refusal::NoSpace)
    } else {
        None
    }
}

/// A stream can stop (STOP_SENDING / reset) a moment before the close frame
/// that says why arrives: wait briefly for it, so a cancel is told apart
/// from a failure.
async fn settle(conn: &iroh::endpoint::Connection) {
    if conn.close_reason().is_none() {
        let _ = tokio::time::timeout(Duration::from_secs(2), conn.closed()).await;
    }
}

/// If the connection was closed by the other side's deliberate cancel.
fn peer_cancelled(conn: &iroh::endpoint::Connection) -> bool {
    matches!(
        conn.close_reason(),
        Some(iroh::endpoint::ConnectionError::ApplicationClosed(c))
            if c.error_code == CLOSE_CANCELLED.into()
    )
}

/// What a completed receive produced.
#[derive(Debug, PartialEq, Eq)]
pub enum Received {
    File {
        path: PathBuf,
        size: u64,
    },
    Text(String),
    Link(String),
    /// A clipboard image (PNG bytes), kept in memory: never written to the
    /// receive folder.
    Image(Vec<u8>),
}

/// Make a sender-supplied name safe for any file system: no directories, no
/// `..`, no control or reserved characters, no Windows device names, no
/// trailing dots/spaces, bounded length. Never returns an empty name.
/// How a received file's name could hurt on Windows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileRisk {
    Normal,
    /// Runs code or mounts something when opened: always asked about.
    Risky,
    /// A shortcut type Explorer reads by itself (its icon can point at a
    /// remote share, which leaks the password hash): refused unless allowed.
    Shortcut,
}

const SHORTCUT_EXTS: &[&str] = &["lnk", "url", "scf", "library-ms", "searchconnector-ms"];
const RISKY_EXTS: &[&str] = &[
    "exe",
    "msi",
    "msix",
    "appx",
    "com",
    "bat",
    "cmd",
    "ps1",
    "psm1",
    "vbs",
    "vbe",
    "js",
    "jse",
    "wsf",
    "hta",
    "jar",
    "reg",
    "cpl",
    "scr",
    "pif",
    "msc",
    "chm",
    "appref-ms",
    "settingcontent-ms",
    "iso",
    "img",
    "vhd",
    "vhdx",
    "xll",
];

/// The class of `name` by the extension Windows acts on: case-insensitive,
/// after the trailing dots and spaces Windows drops ("a.exe." runs as "a.exe").
pub fn classify(name: &str) -> FileRisk {
    let trimmed = name.trim_end_matches(['.', ' ']);
    let Some((_, ext)) = trimmed.rsplit_once('.') else {
        return FileRisk::Normal;
    };
    let ext = ext.to_ascii_lowercase();
    if SHORTCUT_EXTS.contains(&ext.as_str()) {
        FileRisk::Shortcut
    } else if RISKY_EXTS.contains(&ext.as_str()) {
        FileRisk::Risky
    } else {
        FileRisk::Normal
    }
}

pub fn sanitize_file_name(raw: &str) -> String {
    // Keep only the last path component, whatever separator the sender used.
    let base = raw.rsplit(['/', '\\']).next().unwrap_or("");
    let mut s: String = base
        .chars()
        // Bidi overrides would show `fdp.exe` reversed as `exe.pdf`.
        .filter(|c| !pb_identity::is_hidden_format(*c))
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '|' | '?' | '*' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect();
    s = s
        .trim()
        .trim_end_matches(['.', ' '])
        .trim_start_matches('.')
        .to_owned();
    if s.len() > MAX_NAME_BYTES {
        // Keep the extension when shortening; cut on a character boundary.
        let (stem, ext) = match s.rfind('.') {
            Some(i) if s.len() - i <= 16 => (s[..i].to_owned(), s[i..].to_owned()),
            _ => (s.clone(), String::new()),
        };
        let mut keep = MAX_NAME_BYTES - ext.len();
        while !stem.is_char_boundary(keep) {
            keep -= 1;
        }
        s = stem[..keep].to_owned() + &ext;
    }
    let stem_upper = s.split('.').next().unwrap_or("").to_ascii_uppercase();
    const RESERVED: [&str; 22] = [
        "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
        "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
    ];
    if RESERVED.contains(&stem_upper.as_str()) {
        s = format!("_{s}");
    }
    if s.is_empty() {
        s = "file".to_owned();
    }
    s
}

/// Claim a free name in `dir` by creating it empty (so two transfers of the
/// same name at once cannot both pick it); the caller renames over it.
fn reserve_unique(dir: &Path, name: &str) -> std::io::Result<PathBuf> {
    loop {
        let p = unique_path(dir, name);
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&p)
        {
            Ok(_) => return Ok(p),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
}

/// `dir/name`, or `dir/name (2).ext`, `(3)`... if taken.
pub fn unique_path(dir: &Path, name: &str) -> PathBuf {
    let first = dir.join(name);
    if !first.exists() {
        return first;
    }
    let (stem, ext) = match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i..]),
        _ => (name, ""),
    };
    (2..)
        .map(|n| dir.join(format!("{stem} ({n}){ext}")))
        .find(|p| !p.exists())
        .expect("unbounded range")
}

pub(crate) fn new_id() -> u64 {
    let mut b = [0u8; 8];
    let _ = getrandom::fill(&mut b);
    u64::from_le_bytes(b)
}

/// Wait for the reply to `id`, skipping anything else.
pub(crate) async fn await_reply(s: &mut Session, id: u64) -> Result<bool, Error> {
    Ok(await_answer(s, id).await?.0)
}

/// The reply to `id`: accepted, and if not, whether it was too large.
async fn await_answer(s: &mut Session, id: u64) -> Result<(bool, bool), Error> {
    tokio::time::timeout(OFFER_TIMEOUT, async {
        loop {
            match s.recv().await? {
                Some(ControlMsg::OfferReply(r)) if r.id == id => {
                    return Ok((r.accept, r.too_large))
                }
                Some(_) => continue,
                None => return Err(Error::Closed),
            }
        }
    })
    .await
    .map_err(|_| Error::Timeout)?
}

/// Receiver side: let the sender close after reading our last reply.
pub(crate) async fn wait_for_close(session: &Session) {
    let _ = tokio::time::timeout(Duration::from_secs(10), session.conn.closed()).await;
}

/// After our last message, wait until the receiver closes, so nothing we
/// sent is discarded by an early close.
pub(crate) async fn await_peer_close(s: &mut Session) {
    let _ = tokio::time::timeout(Duration::from_secs(60), async {
        while let Ok(Some(_)) = s.recv().await {}
    })
    .await;
}

impl Node {
    /// Send a file (any reader of exactly `size` bytes) to a paired peer.
    /// Returns the SHA-256 the receiver verified.
    pub async fn send_file<R: AsyncRead + Unpin>(
        &self,
        peer: &PeerId,
        name: &str,
        size: u64,
        mime: &str,
        reader: R,
        progress: Option<Progress>,
    ) -> Result<[u8; 32], Error> {
        self.send_file_with(
            peer,
            name,
            size,
            mime,
            reader,
            TransferOpts::new(progress, None),
        )
        .await
    }

    /// [`Node::send_file`] with progress and a [`Cancel`] handle. On cancel
    /// the receiver is told, deletes its partial file and returns
    /// [`Error::PeerCancelled`]; this returns [`Error::Cancelled`].
    pub async fn send_file_with<R: AsyncRead + Unpin>(
        &self,
        peer: &PeerId,
        name: &str,
        size: u64,
        mime: &str,
        reader: R,
        opts: TransferOpts,
    ) -> Result<[u8; 32], Error> {
        self.send_stream(peer, OfferKind::File, name, size, mime, reader, opts)
            .await
    }

    /// Send a PNG for the peer to put on its clipboard. Refuses anything over
    /// [`MAX_CLIPBOARD_IMAGE`] before connecting.
    pub async fn send_clipboard_image(&self, peer: &PeerId, png: Vec<u8>) -> Result<(), Error> {
        if png.len() > MAX_CLIPBOARD_IMAGE {
            return Err(Error::TooLarge);
        }
        let size = png.len() as u64;
        self.send_stream(
            peer,
            OfferKind::ClipboardImage,
            "clipboard.png",
            size,
            "image/png",
            std::io::Cursor::new(png),
            TransferOpts::default(),
        )
        .await
        .map(|_| ())
    }

    #[allow(clippy::too_many_arguments)]
    async fn send_stream<R: AsyncRead + Unpin>(
        &self,
        peer: &PeerId,
        kind: OfferKind,
        name: &str,
        size: u64,
        mime: &str,
        reader: R,
        opts: TransferOpts,
    ) -> Result<[u8; 32], Error> {
        let mut s = or_cancel(&opts.cancel, self.connect(peer)).await?;
        match send_on(&mut s, kind, name, size, mime, reader, &opts).await {
            Err(Error::Cancelled) => {
                // Tell the receiver on purpose, so it cleans up and says why.
                s.conn.close(CLOSE_CANCELLED.into(), b"cancelled");
                Err(Error::Cancelled)
            }
            Err(e @ (Error::Transport(_) | Error::Closed)) => {
                settle(&s.conn).await;
                Err(if peer_cancelled(&s.conn) {
                    Error::PeerCancelled
                } else {
                    e
                })
            }
            other => other,
        }
    }

    /// Send text or a link (small, inline in the offer).
    pub async fn send_text(&self, peer: &PeerId, text: &str, link: bool) -> Result<(), Error> {
        if text.len() > pb_protocol::control::MAX_CLIPBOARD_TEXT {
            return Err(Error::TooLarge);
        }
        let mut s = self.connect(peer).await?;
        let id = new_id();
        let kind = if link {
            OfferKind::Link
        } else {
            OfferKind::Text
        };
        s.send(ControlMsg::Offer(Offer {
            id,
            kind: kind as i32,
            name: String::new(),
            size: text.len() as u64,
            mime: "text/plain".into(),
            text: text.to_owned(),
        }))
        .await?;
        let accepted = await_reply(&mut s, id).await?;
        // We have read the reply, which is the last message: we close.
        s.conn.close(0u32.into(), b"done");
        if accepted {
            Ok(())
        } else {
            Err(Error::Declined)
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn send_on<R: AsyncRead + Unpin>(
    s: &mut Session,
    kind: OfferKind,
    name: &str,
    size: u64,
    mime: &str,
    mut reader: R,
    opts: &TransferOpts,
) -> Result<[u8; 32], Error> {
    let (cancel, progress) = (&opts.cancel, &opts.progress);
    let id = new_id();
    or_cancel(
        cancel,
        s.send(ControlMsg::Offer(Offer {
            id,
            kind: kind as i32,
            name: name.to_owned(),
            size,
            mime: mime.to_owned(),
            text: String::new(),
        })),
    )
    .await?;
    if let (false, too_large) = or_cancel(cancel, await_answer(s, id)).await? {
        s.conn.close(0u32.into(), b"declined");
        return Err(if too_large {
            Error::TooLarge
        } else {
            Error::Declined
        });
    }
    let mut out = s.conn.open_uni().await.map_err(tx)?;
    or_cancel(cancel, write_frame(&mut out, &FileHeader { id })).await?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; CHUNK];
    let mut done = 0u64;
    while done < size {
        let want = chunk_len(size - done, CHUNK);
        let n = reader
            .read(&mut buf[..want])
            .await
            .map_err(|e| Error::Io(e.to_string()))?;
        if n == 0 {
            return Err(Error::Io(format!(
                "source ended after {done} of {size} bytes"
            )));
        }
        hasher.update(&buf[..n]);
        // Cancel also interrupts a write stalled by flow control.
        or_cancel(cancel, async { out.write_all(&buf[..n]).await.map_err(tx) }).await?;
        done += n as u64;
        if let Some(p) = progress {
            p(done, size);
        }
    }
    out.finish().map_err(tx)?;
    let sha: [u8; 32] = hasher.finalize().into();
    or_cancel(
        cancel,
        s.send(ControlMsg::TransferDone(TransferDone {
            id,
            sha256: sha.to_vec(),
        })),
    )
    .await?;
    // Our last message is out: waiting for the receiver's close is
    // cancellable too (a cancel here still closes with the cancel code).
    or_cancel(cancel, async {
        await_peer_close(s).await;
        Ok(())
    })
    .await?;
    // The receiver closes with CLOSE_SAVED only once the file is saved.
    match s.conn.close_reason() {
        Some(iroh::endpoint::ConnectionError::ApplicationClosed(c))
            if c.error_code == CLOSE_SAVED.into() =>
        {
            Ok(sha)
        }
        Some(iroh::endpoint::ConnectionError::ApplicationClosed(c))
            if c.error_code == CLOSE_TRANSFER_FAILED.into() =>
        {
            Err(Error::Io("the other device could not save the file".into()))
        }
        Some(iroh::endpoint::ConnectionError::ApplicationClosed(c))
            if c.error_code == CLOSE_CANCELLED.into() =>
        {
            Err(Error::PeerCancelled)
        }
        None => Err(Error::Timeout),
        Some(_) => Err(Error::Closed),
    }
}

/// Free space left after a transfer, at minimum.
const KEEP_FREE: u64 = 256 * 1024 * 1024;

fn room_for(dir: &Path, size: u64) -> bool {
    // The directory may not exist yet; its parent is on the same volume.
    let probe = if dir.exists() {
        dir
    } else {
        dir.parent().unwrap_or(dir)
    };
    match fs4::available_space(probe) {
        Ok(free) => free >= size.saturating_mul(2).saturating_add(KEEP_FREE),
        // Unknown: do not refuse on a failed query.
        Err(_) => true,
    }
}

/// Handle an incoming [`Offer`] on `session`: reply, and for files receive
/// the bytes into `dir`, verify the hash, and move into place.
pub async fn receive_offer(
    session: &mut Session,
    offer: Offer,
    accept: bool,
    dir: &Path,
    max_size: u64,
    progress: Option<Progress>,
) -> Result<Received, Error> {
    receive_offer_with(
        session,
        offer,
        accept,
        dir,
        max_size,
        TransferOpts::new(progress, None),
    )
    .await
}

/// [`receive_offer`] with progress and a [`Cancel`] handle. On cancel the
/// partial file is deleted, the sender is told (it returns
/// [`Error::PeerCancelled`]) and this returns [`Error::Cancelled`]. If the
/// sender cancels, this returns [`Error::PeerCancelled`] and also leaves
/// nothing behind.
pub async fn receive_offer_with(
    session: &mut Session,
    offer: Offer,
    accept: bool,
    dir: &Path,
    max_size: u64,
    opts: TransferOpts,
) -> Result<Received, Error> {
    let progress = opts.progress.clone();
    let cancel = opts.cancel.clone();
    let id = offer.id;
    let kind = OfferKind::try_from(offer.kind).unwrap_or(OfferKind::File);
    // Too big for the cap, or for the disk: the file is written once here and
    // may be copied once more by the app (Android moves it into MediaStore),
    // and the device must keep some room to stay usable.
    let too_big = match kind {
        OfferKind::File => refusal(dir, offer.size, max_size).is_some(),
        OfferKind::ClipboardImage => offer.size > MAX_CLIPBOARD_IMAGE as u64,
        OfferKind::Text | OfferKind::Link => false,
    };
    if !accept || too_big {
        session
            .send(ControlMsg::OfferReply(OfferReply {
                id,
                accept: false,
                too_large: too_big,
            }))
            .await?;
        // The sender closes once it has read the reply; closing first could
        // discard the reply (same rule as the pairing close handshake).
        wait_for_close(session).await;
        return Err(if too_big {
            Error::TooLarge
        } else {
            Error::Declined
        });
    }
    session
        .send(ControlMsg::OfferReply(OfferReply {
            id,
            accept: true,
            too_large: false,
        }))
        .await?;
    match kind {
        OfferKind::Text | OfferKind::Link => {
            wait_for_close(session).await;
            // Only a web address gets a one-tap "open": a peer's upi: or tel: stays plain text.
            return Ok(if kind == OfferKind::Link && is_web_link(&offer.text) {
                Received::Link(offer.text)
            } else {
                Received::Text(offer.text)
            });
        }
        OfferKind::ClipboardImage => {
            // In memory only (bounded by MAX_CLIPBOARD_IMAGE above): the
            // receive folder is the user's, often synced or indexed.
            return match receive_bytes(session, id, offer.size, None, progress, &cancel).await {
                Ok(png) => {
                    session.conn.close(CLOSE_SAVED.into(), b"saved");
                    Ok(Received::Image(png))
                }
                Err(e) => Err(fail_receive(session, e).await),
            };
        }
        OfferKind::File => {}
    }

    let name = sanitize_file_name(&offer.name);
    tokio::fs::create_dir_all(dir)
        .await
        .map_err(|e| Error::Io(e.to_string()))?;
    // A local random suffix: the offer id is the peer's choice and may repeat.
    let partial = dir.join(format!(".{name}.partial-{:016x}", new_id()));
    let result = async {
        receive_bytes(session, id, offer.size, Some(&partial), progress, &cancel).await?;
        let dir = dir.to_owned();
        let (src, n) = (partial.clone(), name.clone());
        tokio::task::spawn_blocking(move || -> std::io::Result<PathBuf> {
            let dest = reserve_unique(&dir, &n)?;
            std::fs::rename(&src, &dest).inspect_err(|_| {
                let _ = std::fs::remove_file(&dest);
            })?;
            Ok(dest)
        })
        .await
        .map_err(|e| Error::Io(e.to_string()))?
        .map_err(|e| Error::Io(e.to_string()))
    }
    .await;
    match result {
        Ok(dest) => {
            // We read the sender's TransferDone; closing now discards nothing.
            session.conn.close(CLOSE_SAVED.into(), b"saved");
            Ok(Received::File {
                path: dest,
                size: offer.size,
            })
        }
        Err(e) => {
            let _ = tokio::fs::remove_file(&partial).await;
            Err(fail_receive(session, e).await)
        }
    }
}

/// End a failed receive: tell the sender why (cancelled or failed) and
/// return the error to report.
async fn fail_receive(session: &Session, e: Error) -> Error {
    if matches!(e, Error::Transport(_) | Error::Closed) {
        settle(&session.conn).await;
    }
    // Read before closing ourselves: our own close would replace it.
    let theirs = peer_cancelled(&session.conn);
    if matches!(e, Error::Cancelled) {
        session.conn.close(CLOSE_CANCELLED.into(), b"cancelled");
        Error::Cancelled
    } else if theirs {
        Error::PeerCancelled
    } else {
        session
            .conn
            .close(CLOSE_TRANSFER_FAILED.into(), b"transfer failed");
        e
    }
}

/// Receive the offered bytes into `partial`, or into memory (returned) when
/// it is `None`; either way the size and the sender's SHA-256 are checked.
async fn receive_bytes(
    session: &mut Session,
    id: u64,
    size: u64,
    partial: Option<&Path>,
    progress: Option<Progress>,
    cancel: &Option<Cancel>,
) -> Result<Vec<u8>, Error> {
    let stream = or_cancel(cancel, async {
        tokio::time::timeout(Duration::from_secs(30), session.conn.accept_uni())
            .await
            .map_err(|_| Error::Timeout)?
            .map_err(tx)
    })
    .await?;
    let mut reader = FrameReader::new(stream);
    let header: FileHeader = or_cancel(cancel, async {
        tokio::time::timeout(Duration::from_secs(30), reader.next())
            .await
            .map_err(|_| Error::Timeout)??
            .ok_or(Error::Closed)
    })
    .await?;
    if header.id != id {
        return Err(Error::Protocol("file stream for a different offer".into()));
    }
    // Hashing and disk writes run on a blocking thread, overlapped with the
    // network reads, in batches of about 1 MiB: QUIC hands over a few KB at a
    // time, and one hash + write + thread hop per piece capped receives at
    // ~27 MB/s on the phone.
    let (chunks, mut queue) = tokio::sync::mpsc::channel::<Vec<u8>>(8);
    let path = partial.map(Path::to_owned);
    let writer = tokio::task::spawn_blocking(move || -> std::io::Result<([u8; 32], Vec<u8>)> {
        use std::io::Write;
        let mut file = match &path {
            Some(p) => Some(std::io::BufWriter::with_capacity(
                BATCH,
                std::fs::File::create(p)?,
            )),
            None => None,
        };
        let mut mem = Vec::new();
        let mut hasher = Sha256::new();
        while let Some(chunk) = queue.blocking_recv() {
            hasher.update(&chunk);
            match &mut file {
                Some(f) => f.write_all(&chunk)?,
                None => mem.extend_from_slice(&chunk),
            }
        }
        if let Some(f) = &mut file {
            f.flush()?;
        }
        Ok((hasher.finalize().into(), mem))
    });
    let mut done = 0u64;
    // Bytes that arrived in the same packet as the header.
    let mut batch = std::mem::take(&mut reader.buf);
    let mut buf = vec![0u8; CHUNK];
    let read: Result<(), Error> = async {
        loop {
            if done + batch.len() as u64 > size {
                return Err(Error::Protocol(
                    "sender sent more bytes than offered".into(),
                ));
            }
            if batch.len() >= BATCH {
                done += batch.len() as u64;
                let full = std::mem::replace(&mut batch, Vec::with_capacity(BATCH + CHUNK));
                if chunks.send(full).await.is_err() {
                    // The writer stopped (disk error): its result says why.
                    return Ok(());
                }
                if let Some(p) = &progress {
                    p(done, size);
                }
            }
            // A sender that stops sending must not hold the partial file forever.
            let chunk = or_cancel(cancel, async {
                tokio::time::timeout(Duration::from_secs(30), reader.recv.read(&mut buf))
                    .await
                    .map_err(|_| Error::Timeout)?
                    .map_err(tx)
            })
            .await?;
            match chunk {
                Some(n) => batch.extend_from_slice(&buf[..n]),
                None => break,
            }
        }
        if !batch.is_empty() {
            done += batch.len() as u64;
            let _ = chunks.send(std::mem::take(&mut batch)).await;
            if let Some(p) = &progress {
                p(done, size);
            }
        }
        Ok(())
    }
    .await;
    drop(chunks);
    // Always wait for the writer, so the file is closed before any cleanup.
    let written = writer.await.map_err(|e| Error::Io(e.to_string()))?;
    read?;
    let (ours, mem) = written.map_err(|e| Error::Io(e.to_string()))?;
    if done != size {
        return Err(Error::Protocol(format!("received {done} of {size} bytes")));
    }
    let theirs = or_cancel(cancel, async {
        tokio::time::timeout(Duration::from_secs(30), async {
            loop {
                match session.recv().await? {
                    Some(ControlMsg::TransferDone(d)) if d.id == id => return Ok(d.sha256),
                    Some(_) => continue,
                    None => return Err(Error::Closed),
                }
            }
        })
        .await
        .map_err(|_| Error::Timeout)?
    })
    .await?;
    if theirs.as_slice() != ours.as_slice() {
        return Err(Error::HashMismatch);
    }
    Ok(mem)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disguised_executables_are_risky() {
        for n in [
            "photo.jpg.exe",
            "A.EXE.",
            "a.exe ",
            "x.Ps1",
            "setup.MSI",
            "b.iso",
        ] {
            assert_eq!(classify(n), FileRisk::Risky, "{n}");
        }
        for n in ["photo.jpg", "notes.txt", "exe", "a.exe.txt", "report.pdf"] {
            assert_eq!(classify(n), FileRisk::Normal, "{n}");
        }
    }

    #[test]
    fn shortcuts_are_shortcuts() {
        for n in [
            "a.lnk",
            "B.URL",
            "c.scf",
            "d.library-ms",
            "e.searchConnector-ms",
            "f.LNK. ",
        ] {
            assert_eq!(classify(n), FileRisk::Shortcut, "{n}");
        }
    }

    #[test]
    fn chunk_len_never_truncates_large_remainders() {
        // On 32-bit Android, `(remaining as usize)` keeps only the low 32
        // bits: 4 GiB left read as 0 bytes, which failed the send.
        assert_eq!(chunk_len(1 << 32, CHUNK), CHUNK);
        assert_eq!(chunk_len((1 << 32) + 5, CHUNK), CHUNK);
        assert_eq!(chunk_len(5, CHUNK), 5);
        assert_eq!(chunk_len(0, CHUNK), 0);
    }

    #[test]
    fn hidden_bidi_and_zero_width_characters_are_removed_from_names() {
        assert_eq!(sanitize_file_name("report\u{202e}fdp.exe"), "reportfdp.exe");
        assert_eq!(
            sanitize_file_name("\u{feff}a\u{200b}b\u{2066}c\u{2069}.txt"),
            "abc.txt"
        );
        assert_eq!(sanitize_file_name("\u{202e}"), "file");
    }

    #[test]
    fn names_are_made_safe() {
        assert_eq!(sanitize_file_name("photo.jpg"), "photo.jpg");
        assert_eq!(sanitize_file_name("../../etc/passwd"), "passwd");
        assert_eq!(
            sanitize_file_name("C:\\Windows\\system32\\evil.dll"),
            "evil.dll"
        );
        assert_eq!(
            sanitize_file_name("a<b>c:d\"e|f?g*h.txt"),
            "a_b_c_d_e_f_g_h.txt"
        );
        assert_eq!(sanitize_file_name("CON.txt"), "_CON.txt");
        assert_eq!(sanitize_file_name("lpt1"), "_lpt1");
        assert_eq!(sanitize_file_name("trailing.  . "), "trailing");
        assert_eq!(sanitize_file_name(".hidden"), "hidden");
        assert_eq!(sanitize_file_name(""), "file");
        assert_eq!(sanitize_file_name(".."), "file");
        assert_eq!(sanitize_file_name("bell\u{7}\nname"), "bell__name");
        let long = format!("{}.jpeg", "x".repeat(500));
        let s = sanitize_file_name(&long);
        assert_eq!(s.len(), MAX_NAME_BYTES);
        assert!(s.ends_with(".jpeg"));
        // Multi-byte names are limited by bytes, never split inside a character.
        let s = sanitize_file_name(&format!("{}.txt", "\u{1F600}".repeat(100)));
        assert!(
            s.len() <= MAX_NAME_BYTES && s.ends_with(".txt"),
            "{}",
            s.len()
        );
    }

    #[test]
    fn sanitizer_never_returns_a_path_on_random_input() {
        // Deterministic pseudo-random names built from awkward characters.
        let alphabet: Vec<char> = "ab./\\:*?\"<>|\u{0}\u{7}  .C0N\u{202e}é".chars().collect();
        let mut seed = 0x1234_5678_9abc_def0u64;
        for _ in 0..5000 {
            let len = (seed % 40) as usize;
            let mut name = String::new();
            for _ in 0..len {
                seed = seed
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                name.push(alphabet[(seed >> 33) as usize % alphabet.len()]);
            }
            let s = sanitize_file_name(&name);
            assert!(!s.is_empty());
            assert!(
                !s.contains(['/', '\\', ':', '*', '?', '"', '<', '>', '|']),
                "{s:?}"
            );
            assert!(!s.chars().any(char::is_control), "{s:?}");
            assert!(
                !s.starts_with('.') && !s.ends_with('.') && !s.ends_with(' '),
                "{s:?}"
            );
            assert_ne!(s, "..");
        }
    }

    #[test]
    fn unique_path_appends_a_counter() {
        let dir = std::env::temp_dir().join(format!("pb-unique-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(unique_path(&dir, "a.txt"), dir.join("a.txt"));
        std::fs::write(dir.join("a.txt"), b"1").unwrap();
        assert_eq!(unique_path(&dir, "a.txt"), dir.join("a (2).txt"));
        std::fs::write(dir.join("a (2).txt"), b"2").unwrap();
        assert_eq!(unique_path(&dir, "a.txt"), dir.join("a (3).txt"));
        std::fs::write(dir.join("noext"), b"1").unwrap();
        assert_eq!(unique_path(&dir, "noext"), dir.join("noext (2)"));
    }
}

/// Whether a shared link may be opened with one tap on the other device.
pub(crate) fn is_web_link(text: &str) -> bool {
    let t = text.trim_start().to_ascii_lowercase();
    t.starts_with("https://") || t.starts_with("http://")
}

#[cfg(test)]
mod link_tests {
    use super::is_web_link;

    #[test]
    fn web_addresses_are_links() {
        assert!(is_web_link("https://example.com/a?b=c"));
        assert!(is_web_link("http://example.com"));
        assert!(is_web_link("  HTTPS://EXAMPLE.COM"));
    }

    /// One tap on a "link" must not pay, call or open local files.
    #[test]
    fn other_schemes_are_only_text() {
        for t in [
            "upi://pay?pa=x",
            "tel:123",
            "intent://x#Intent;end",
            "file:///c:/x",
            "ms-settings:",
            "javascript:alert(1)",
            "https",
            "",
        ] {
            assert!(!is_web_link(t), "{t}");
        }
    }
}
