//! "Phone audio on the PC": the PC requests audio capture; the phone
//! streams raw 16-bit PCM frames on a uni stream.
//!
//! Wire format on the uni-stream:
//!   `[4 bytes: AUDIO_MAGIC "PBA0"]` followed by repeated
//!   `[4 bytes: frame_len as u32 LE] [frame_len bytes: raw PCM samples i16 LE]`

use pb_identity::PeerId;
use pb_protocol::control::{AudioAccept, AudioStart, AudioStop, ControlMsg, OfferReply};
use tokio::sync::mpsc;

use crate::{screen::ACCEPT_TIMEOUT, tx, Error, Node, Session, SessionCloser};

/// Magic header sent at the start of the audio uni-stream.
pub const AUDIO_MAGIC: &[u8; 4] = b"PBA0";

/// One chunk of raw PCM from the phone (i16 LE samples, interleaved channels).
pub struct AudioFrame {
    /// Raw bytes (pairs of i16 LE samples).
    pub samples: Vec<u8>,
}

/// PC side of a running audio session.
pub struct AudioView {
    pub accept: AudioAccept,
    /// PCM frames as they arrive.
    pub frames: mpsc::Receiver<AudioFrame>,
    ctl: mpsc::UnboundedSender<ControlMsg>,
    closer: SessionCloser,
}

impl AudioView {
    /// Close the QUIC session from the PC side.
    pub fn close(&self) {
        self.closer.close(b"audio closed on the PC");
    }

    /// Ask the phone to stop audio streaming gracefully.
    pub fn stop(&self) {
        let _ = self
            .ctl
            .send(ControlMsg::AudioStop(AudioStop { id: self.accept.id }));
    }
}

/// Phone side event.
#[derive(Debug)]
pub enum AudioEvent {
    /// The PC sent `AudioStop` or the session closed.
    Stopped { by_peer: bool },
}

/// Phone side of a running audio session.
pub struct AudioServer {
    pub accept: AudioAccept,
    pub events: mpsc::UnboundedReceiver<AudioEvent>,
    /// Write raw PCM bytes here; `send_frame` prepends the 4-byte length prefix.
    pub frame_tx: mpsc::UnboundedSender<Vec<u8>>,
}

impl AudioServer {
    /// Send one PCM chunk. Prepends the 4-byte little-endian length prefix.
    pub fn send_frame(&self, pcm: Vec<u8>) {
        let mut msg = Vec::with_capacity(4 + pcm.len());
        msg.extend_from_slice(&(pcm.len() as u32).to_le_bytes());
        msg.extend_from_slice(&pcm);
        let _ = self.frame_tx.send(msg);
    }
}

/// Opening the audio stream after accepting.
const STREAM_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);
/// Buffered frames waiting for the PC audio output to consume them.
const AUDIO_CHANNEL: usize = 64;

impl Node {
    /// PC side: request audio streaming from `peer`.
    ///
    /// `on_asked` is called once the `AudioStart` has been sent (before waiting
    /// for the phone's reply).
    pub async fn open_audio(
        &self,
        peer: &PeerId,
        mut req: AudioStart,
        on_asked: impl FnOnce(),
    ) -> Result<AudioView, Error> {
        let mut s = self.connect(peer).await?;
        req.id = crate::files::new_id();
        let id = req.id;
        s.send(ControlMsg::AudioStart(req)).await?;
        on_asked();

        // Wait for AudioAccept or a decline.
        let accept = tokio::time::timeout(ACCEPT_TIMEOUT, async {
            loop {
                match s.recv().await? {
                    Some(ControlMsg::AudioAccept(a)) if a.id == id => return Ok(a),
                    Some(ControlMsg::OfferReply(r)) if r.id == id && !r.accept => {
                        return Err(Error::Declined)
                    }
                    Some(_) => continue,
                    None => return Err(Error::Closed),
                }
            }
        })
        .await
        .map_err(|_| Error::Timeout)??;

        // Phone opens a uni-stream for PCM data.
        let conn = s.conn.clone();
        let mut audio_stream = tokio::time::timeout(STREAM_TIMEOUT, conn.accept_uni())
            .await
            .map_err(|_| Error::Io("the phone did not open the audio stream".into()))?
            .map_err(tx)?;

        let mut magic = [0u8; 4];
        audio_stream
            .read_exact(&mut magic)
            .await
            .map_err(|e| Error::Io(format!("audio stream header read failed: {e}")))?;
        if &magic != AUDIO_MAGIC {
            return Err(Error::Protocol("invalid audio stream magic".into()));
        }

        let (frame_tx, frames) = mpsc::channel(AUDIO_CHANNEL);

        // Pump task: read length-prefixed PCM frames and forward them.
        tokio::spawn(async move {
            let mut len_buf = [0u8; 4];
            loop {
                if audio_stream.read_exact(&mut len_buf).await.is_err() {
                    break;
                }
                let len = u32::from_le_bytes(len_buf) as usize;
                // Sanity check: reject empty or oversized frames (64 KiB).
                if len == 0 || len > 64 * 1024 {
                    break;
                }
                let mut samples = vec![0u8; len];
                if audio_stream.read_exact(&mut samples).await.is_err() {
                    break;
                }
                if frame_tx.send(AudioFrame { samples }).await.is_err() {
                    break;
                }
            }
        });

        let closer = s.closer();
        let (ctl, mut ctl_rx) = mpsc::unbounded_channel::<ControlMsg>();

        // Control loop: forward control requests (like AudioStop) and drain incoming messages.
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    m = ctl_rx.recv() => match m {
                        Some(m) => {
                            if s.send(m).await.is_err() {
                                break;
                            }
                        }
                        None => break,
                    },
                    r = s.recv() => match r {
                        Ok(Some(_)) => {}
                        _ => break,
                    },
                }
            }
            s.conn.close(0u32.into(), b"audio done");
        });

        Ok(AudioView {
            accept,
            frames,
            ctl,
            closer,
        })
    }
}

/// Phone side: answer an `AudioStart`.
///
/// Pass `answer = Some(AudioAccept { ... })` to accept, or `None` to decline.
pub async fn serve_audio(
    mut session: Session,
    start: AudioStart,
    answer: Option<AudioAccept>,
) -> Result<AudioServer, Error> {
    let Some(mut accept) = answer else {
        session
            .send(ControlMsg::OfferReply(OfferReply {
                id: start.id,
                accept: false,
                too_large: false,
            }))
            .await?;
        crate::files::wait_for_close(&session).await;
        return Err(Error::Declined);
    };
    accept.id = start.id;
    session
        .send(ControlMsg::AudioAccept(accept.clone()))
        .await?;

    let conn = session.conn.clone();
    let mut out = conn.open_uni().await.map_err(tx)?;
    out.write_all(AUDIO_MAGIC).await.map_err(tx)?;

    let (frame_tx, mut frame_rx) = mpsc::unbounded_channel::<Vec<u8>>();
    let (ev_tx, events) = mpsc::unbounded_channel();

    // Write task: forward pre-framed PCM bytes to the uni-stream.
    tokio::spawn(async move {
        while let Some(bytes) = frame_rx.recv().await {
            if out.write_all(&bytes).await.is_err() {
                break;
            }
        }
        let _ = out.finish();
    });

    // Control loop: watch for AudioStop or session close.
    tokio::spawn(async move {
        loop {
            match session.recv().await {
                Ok(Some(ControlMsg::AudioStop(_))) => {
                    let _ = ev_tx.send(AudioEvent::Stopped { by_peer: true });
                    break;
                }
                Ok(Some(_)) => {}
                _ => {
                    let _ = ev_tx.send(AudioEvent::Stopped { by_peer: false });
                    break;
                }
            }
        }
        session.conn.close(0u32.into(), b"audio done");
    });

    Ok(AudioServer {
        accept,
        events,
        frame_tx,
    })
}
