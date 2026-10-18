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
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};

use crate::{tx, write_frame, Error, FrameReader, Node, Session};

/// Default largest file we accept (4 GiB); apps can pass a different cap.
pub const DEFAULT_MAX_FILE: u64 = 4 << 30;
/// How long a sender waits for the receiver's accept/decline.
pub const OFFER_TIMEOUT: Duration = Duration::from_secs(120);
const CHUNK: usize = 256 * 1024;
const MAX_NAME_CHARS: usize = 200;

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
    if s.chars().count() > MAX_NAME_CHARS {
        // Keep the extension when shortening.
        let (stem, ext) = match s.rfind('.') {
            Some(i) if s.len() - i <= 16 => (s[..i].to_owned(), s[i..].to_owned()),
            _ => (s.clone(), String::new()),
        };
        let keep = MAX_NAME_CHARS - ext.chars().count();
        s = stem.chars().take(keep).collect::<String>() + &ext;
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
    tokio::time::timeout(OFFER_TIMEOUT, async {
        loop {
            match s.recv().await? {
                Some(ControlMsg::OfferReply(r)) if r.id == id => return Ok(r.accept),
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
        if !await_reply(&mut s, id).await? {
            s.conn.close(0u32.into(), b"declined");
            return Err(Error::Declined);
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
        Ok(sha)
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
    let too_big = kind == OfferKind::File && offer.size > max_size;
    if !accept || too_big {
        session
            .send(ControlMsg::OfferReply(OfferReply { id, accept: false }))
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
        .send(ControlMsg::OfferReply(OfferReply { id, accept: true }))
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
    let result = receive_bytes(session, id, offer.size, &partial, progress).await;
    match result {
        Ok(()) => {
            let dest = unique_path(dir, &name);
            tokio::fs::rename(&partial, &dest)
                .await
                .map_err(|e| Error::Io(e.to_string()))?;
            // We read the sender's TransferDone; closing now discards nothing.
            session.conn.close(0u32.into(), b"received");
            Ok(Received::File {
                path: dest,
                size: offer.size,
            })
        }
        Err(e) => {
            let _ = tokio::fs::remove_file(&partial).await;
            session.conn.close(400u32.into(), b"transfer failed");
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
    let header: FileHeader = reader.next().await?.ok_or(Error::Closed)?;
    if header.id != id {
        return Err(Error::Protocol("file stream for a different offer".into()));
    }
    let mut file = tokio::fs::File::create(partial)
        .await
        .map_err(|e| Error::Io(e.to_string()))?;
    let mut hasher = Sha256::new();
    let mut done = 0u64;
    // Bytes that arrived in the same packet as the header.
    let leftover = std::mem::take(&mut reader.buf);
    let mut buf = vec![0u8; CHUNK];
    let mut pending = leftover;
    loop {
        if !pending.is_empty() {
            if done + pending.len() as u64 > size {
                return Err(Error::Protocol(
                    "sender sent more bytes than offered".into(),
                ));
            }
            hasher.update(&pending);
            file.write_all(&pending)
                .await
                .map_err(|e| Error::Io(e.to_string()))?;
            done += pending.len() as u64;
            if let Some(p) = &progress {
                p(done, size);
            }
            pending.clear();
        }
        // A sender that stops sending must not hold the partial file forever.
        let chunk = tokio::time::timeout(Duration::from_secs(30), reader.recv.read(&mut buf))
            .await
            .map_err(|_| Error::Timeout)?;
        match chunk.map_err(tx)? {
            Some(n) => pending.extend_from_slice(&buf[..n]),
            None => break,
        }
    }
    file.flush().await.map_err(|e| Error::Io(e.to_string()))?;
    drop(file);
    if done != size {
        return Err(Error::Protocol(format!("received {done} of {size} bytes")));
    }
    let ours: [u8; 32] = hasher.finalize().into();
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
        assert_eq!(s.chars().count(), MAX_NAME_CHARS);
        assert!(s.ends_with(".jpeg"));
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
