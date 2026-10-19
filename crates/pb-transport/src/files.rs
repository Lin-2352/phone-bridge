//! AirDrop-style transfers over a session: offer, accept/decline, then the
//! file bytes on their own QUIC stream with an end-to-end SHA-256 check.
//!
//! Sender: [`Node::send_file`] / [`Node::send_text`] open a fresh session per
//! transfer. Receiver: the app's session loop calls [`receive_offer`] when an
//! [`Offer`] arrives. Files are written to `.<name>.partial-<id>` and only
//! renamed into place after the hash matches; any failure deletes the partial.

use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use pb_identity::PeerId;
use pb_protocol::control::{ControlMsg, FileHeader, Offer, OfferKind, OfferReply, TransferDone};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncRead, AsyncReadExt};

use crate::{tx, write_frame, Error, FrameReader, Node, Session};

/// Default largest file we accept (4 GiB); apps can pass a different cap.
pub const DEFAULT_MAX_FILE: u64 = 4 << 30;
/// How long a sender waits for the receiver's accept/decline.
pub const OFFER_TIMEOUT: Duration = Duration::from_secs(120);
const CHUNK: usize = 256 * 1024;
/// Received bytes are hashed and written in batches of this size.
const BATCH: usize = 1024 * 1024;
/// Bytes, not characters: file systems limit names to 255 bytes (ext4) and
/// the partial file adds 26 more.
const MAX_NAME_BYTES: usize = 200;
/// Receiver close code: the file did not arrive intact or could not be saved.
pub(crate) const CLOSE_TRANSFER_FAILED: u32 = 400;

/// Progress callback: `(bytes_done, bytes_total)`.
pub type Progress = Arc<dyn Fn(u64, u64) + Send + Sync>;

/// What a completed receive produced.
#[derive(Debug, PartialEq, Eq)]
pub enum Received {
    File { path: PathBuf, size: u64 },
    Text(String),
    Link(String),
}

/// Make a sender-supplied name safe for any file system: no directories, no
/// `..`, no control or reserved characters, no Windows device names, no
/// trailing dots/spaces, bounded length. Never returns an empty name.
pub fn sanitize_file_name(raw: &str) -> String {
    // Keep only the last path component, whatever separator the sender used.
    let base = raw.rsplit(['/', '\\']).next().unwrap_or("");
    let mut s: String = base
        .chars()
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
        mut reader: R,
        progress: Option<Progress>,
    ) -> Result<[u8; 32], Error> {
        let mut s = self.connect(peer).await?;
        let id = new_id();
        s.send(ControlMsg::Offer(Offer {
            id,
            kind: OfferKind::File as i32,
            name: name.to_owned(),
            size,
            mime: mime.to_owned(),
            text: String::new(),
        }))
        .await?;
        if let (false, too_large) = await_answer(&mut s, id).await? {
            s.conn.close(0u32.into(), b"declined");
            return Err(if too_large {
                Error::TooLarge
            } else {
                Error::Declined
            });
        }
        let mut out = s.conn.open_uni().await.map_err(tx)?;
        write_frame(&mut out, &FileHeader { id }).await?;
        let mut hasher = Sha256::new();
        let mut buf = vec![0u8; CHUNK];
        let mut done = 0u64;
        while done < size {
            let want = ((size - done) as usize).min(CHUNK);
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
            out.write_all(&buf[..n]).await.map_err(tx)?;
            done += n as u64;
            if let Some(p) = &progress {
                p(done, size);
            }
        }
        out.finish().map_err(tx)?;
        let sha: [u8; 32] = hasher.finalize().into();
        s.send(ControlMsg::TransferDone(TransferDone {
            id,
            sha256: sha.to_vec(),
        }))
        .await?;
        await_peer_close(&mut s).await;
        // The receiver closes with 0 only once the file is saved under its name.
        match s.conn.close_reason() {
            Some(iroh::endpoint::ConnectionError::ApplicationClosed(c))
                if c.error_code == 0u32.into() =>
            {
                Ok(sha)
            }
            Some(iroh::endpoint::ConnectionError::ApplicationClosed(c))
                if c.error_code == CLOSE_TRANSFER_FAILED.into() =>
            {
                Err(Error::Io("the other device could not save the file".into()))
            }
            None => Err(Error::Timeout),
            Some(_) => Err(Error::Closed),
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
    let id = offer.id;
    let kind = OfferKind::try_from(offer.kind).unwrap_or(OfferKind::File);
    // Too big for the cap, or for the disk: the file is written once here and
    // may be copied once more by the app (Android moves it into MediaStore),
    // and the device must keep some room to stay usable.
    let too_big = kind == OfferKind::File && (offer.size > max_size || !room_for(dir, offer.size));
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
            return Ok(if kind == OfferKind::Link {
                Received::Link(offer.text)
            } else {
                Received::Text(offer.text)
            });
        }
        OfferKind::File => {}
    }

    let name = sanitize_file_name(&offer.name);
    tokio::fs::create_dir_all(dir)
        .await
        .map_err(|e| Error::Io(e.to_string()))?;
    let partial = dir.join(format!(".{name}.partial-{id:016x}"));
    let result = async {
        receive_bytes(session, id, offer.size, &partial, progress).await?;
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
            session.conn.close(0u32.into(), b"received");
            Ok(Received::File {
                path: dest,
                size: offer.size,
            })
        }
        Err(e) => {
            let _ = tokio::fs::remove_file(&partial).await;
            session
                .conn
                .close(CLOSE_TRANSFER_FAILED.into(), b"transfer failed");
            Err(e)
        }
    }
}

async fn receive_bytes(
    session: &mut Session,
    id: u64,
    size: u64,
    partial: &Path,
    progress: Option<Progress>,
) -> Result<(), Error> {
    let stream = tokio::time::timeout(Duration::from_secs(30), session.conn.accept_uni())
        .await
        .map_err(|_| Error::Timeout)?
        .map_err(tx)?;
    let mut reader = FrameReader::new(stream);
    let header: FileHeader = tokio::time::timeout(Duration::from_secs(30), reader.next())
        .await
        .map_err(|_| Error::Timeout)??
        .ok_or(Error::Closed)?;
    if header.id != id {
        return Err(Error::Protocol("file stream for a different offer".into()));
    }
    // Hashing and disk writes run on a blocking thread, overlapped with the
    // network reads, in batches of about 1 MiB: QUIC hands over a few KB at a
    // time, and one hash + write + thread hop per piece capped receives at
    // ~27 MB/s on the phone.
    let (chunks, mut queue) = tokio::sync::mpsc::channel::<Vec<u8>>(8);
    let path = partial.to_owned();
    let writer = tokio::task::spawn_blocking(move || -> std::io::Result<[u8; 32]> {
        use std::io::Write;
        let mut file = std::io::BufWriter::with_capacity(BATCH, std::fs::File::create(&path)?);
        let mut hasher = Sha256::new();
        while let Some(chunk) = queue.blocking_recv() {
            hasher.update(&chunk);
            file.write_all(&chunk)?;
        }
        file.flush()?;
        Ok(hasher.finalize().into())
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
            let chunk = tokio::time::timeout(Duration::from_secs(30), reader.recv.read(&mut buf))
                .await
                .map_err(|_| Error::Timeout)?;
            match chunk.map_err(tx)? {
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
    let ours = written.map_err(|e| Error::Io(e.to_string()))?;
    if done != size {
        return Err(Error::Protocol(format!("received {done} of {size} bytes")));
    }
    let theirs = tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            match session.recv().await? {
                Some(ControlMsg::TransferDone(d)) if d.id == id => return Ok(d.sha256),
                Some(_) => continue,
                None => return Err(Error::Closed),
            }
        }
    })
    .await
    .map_err(|_| Error::Timeout)??;
    if theirs.as_slice() != ours.as_slice() {
        return Err(Error::HashMismatch);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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
