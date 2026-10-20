//! "Phone screen on the PC": one session carries the consent and window
//! state on its control stream, video records phone → PC on one uni stream,
//! and the window's input PC → phone on another (one consent covers both).
//! See `docs/WIRE_PROTOCOL.md`, "Phone screen".

use std::{
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};

use pb_identity::PeerId;
use pb_protocol::{
    control::{ControlMsg, OfferReply, ScreenAccept, ScreenFeedback, ScreenStart, ScreenState},
    InputDatagram, InputEvent,
};
use pb_video::{
    pacer::QUEUE_CAP,
    record::{self, Decoder, Record, MAGIC},
};
use tokio::sync::mpsc;

use crate::{
    input::{read_input, sender_on, InputSender},
    tx, Error, Node, Session, SessionCloser,
};

/// The phone may take this long: our ask, then Android's capture prompt.
const ACCEPT_TIMEOUT: Duration = Duration::from_secs(40);
/// Opening the streams after accepting.
const STREAM_TIMEOUT: Duration = Duration::from_secs(10);
/// Records waiting for the PC's decoder.
const RECORDS_CHANNEL: usize = 16;
/// The phone keeps capturing (paused) this long after a dropped link, so a
/// new request from the same PC resumes without Android's prompt.
pub const RESUME_GRACE: Duration = Duration::from_secs(30);

/// PC side of a running screen session.
pub struct ScreenView {
    pub accept: ScreenAccept,
    /// Video records in order, all of them: a decoder that falls behind
    /// slows the stream (the phone's pacer then drops to a key frame).
    pub records: mpsc::Receiver<Record>,
    /// The window's mouse and keyboard.
    pub input: InputSender,
    /// Phone → PC state (notices).
    pub state: mpsc::Receiver<ScreenState>,
    ctl: mpsc::UnboundedSender<ControlMsg>,
    closer: SessionCloser,
}

impl ScreenView {
    pub fn feedback(&self, presented_pts_us: i64) {
        let _ = self.ctl.send(ControlMsg::ScreenFeedback(ScreenFeedback {
            presented_pts_us,
        }));
    }

    pub fn set_state(&self, active: bool, dim: bool) {
        let _ = self.ctl.send(ControlMsg::ScreenState(ScreenState {
            active,
            dim,
            ..Default::default()
        }));
    }

    /// Decoding failed: ask the phone for the next codec.
    pub fn codec_failed(&self) {
        let _ = self.ctl.send(ControlMsg::ScreenState(ScreenState {
            active: true,
            codec_failed: true,
            ..Default::default()
        }));
    }

    pub fn close(&self) {
        self.closer.close(b"screen closed on the PC");
    }
}

impl Node {
    /// Ask `peer` to share its screen; `on_asked` runs once the request has
    /// reached it (the phone is asking its user).
    pub async fn open_screen(
        &self,
        peer: &PeerId,
        mut req: ScreenStart,
        on_asked: impl FnOnce(),
    ) -> Result<ScreenView, Error> {
        let mut s = self.connect(peer).await?;
        req.id = crate::files::new_id();
        let id = req.id;
        s.send(ControlMsg::ScreenStart(req)).await?;
        on_asked();
        let accept = tokio::time::timeout(ACCEPT_TIMEOUT, async {
            loop {
                match s.recv().await? {
                    Some(ControlMsg::ScreenAccept(a)) if a.id == id => return Ok(a),
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
        let conn = s.conn.clone();
        // Input: a stream exists for the peer only once data is on it.
        let mut out = conn.open_uni().await.map_err(tx)?;
        let hello = InputDatagram {
            seq: 0,
            event: InputEvent::MouseMove { dx: 0, dy: 0 },
        };
        out.write_all(&hello.encode()).await.map_err(tx)?;
        let input = sender_on(out, async {});
        let mut video = tokio::time::timeout(STREAM_TIMEOUT, conn.accept_uni())
            .await
            .map_err(|_| Error::Io("the phone did not open the video stream".into()))?
            .map_err(tx)?;
        let (rec_tx, records) = mpsc::channel(RECORDS_CHANNEL);
        tokio::spawn(async move {
            let mut decoder = Decoder::new();
            let mut buf = vec![0u8; 64 * 1024];
            while let Ok(Some(n)) = video.read(&mut buf).await {
                let Ok(recs) = decoder.push(&buf[..n]) else {
                    break;
                };
                // Backpressure, never dropping here: a missing P-frame would
                // corrupt the picture until the next key frame. A slow decoder
                // slows the stream, and the phone's pacer drops cleanly to a
                // key frame and lowers the bitrate.
                for r in recs {
                    if rec_tx.send(r).await.is_err() {
                        return;
                    }
                }
            }
        });
        let closer = s.closer();
        let (ctl, mut ctl_rx) = mpsc::unbounded_channel::<ControlMsg>();
        let (state_tx, state) = mpsc::channel(8);
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    m = ctl_rx.recv() => match m {
                        Some(m) => {
                            if s.send(m).await.is_err() {
                                break;
                            }
                        }
                        // The view was dropped: end the session.
                        None => break,
                    },
                    r = s.recv() => match r {
                        Ok(Some(ControlMsg::ScreenState(st))) => {
                            let _ = state_tx.try_send(st);
                        }
                        Ok(Some(_)) => {}
                        _ => break,
                    },
                }
            }
            s.conn.close(0u32.into(), b"screen done");
        });
        Ok(ScreenView {
            accept,
            records,
            input,
            state,
            ctl,
            closer,
        })
    }
}

/// Bytes waiting to be written, capped: a full queue refuses new records
/// (the pacer drops to the next key frame).
#[derive(Clone, Default)]
pub struct ByteQueue(Arc<AtomicUsize>);

impl ByteQueue {
    pub fn try_push(&self, len: usize) -> bool {
        let mut q = self.0.load(Ordering::Acquire);
        loop {
            if q + len > QUEUE_CAP {
                return false;
            }
            match self
                .0
                .compare_exchange_weak(q, q + len, Ordering::AcqRel, Ordering::Acquire)
            {
                Ok(_) => return true,
                Err(now) => q = now,
            }
        }
    }

    pub fn pop(&self, len: usize) {
        let mut q = self.0.load(Ordering::Acquire);
        while let Err(now) = self.0.compare_exchange_weak(
            q,
            q.saturating_sub(len),
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            q = now;
        }
    }

    pub fn len(&self) -> usize {
        self.0.load(Ordering::Acquire)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum SendOutcome {
    /// Queued; the bytes now waiting.
    Queued(usize),
    Full,
    Closed,
}

#[derive(Debug, PartialEq)]
pub enum ScreenEvent {
    Feedback(i64),
    State(ScreenState),
    Input(InputEvent),
    Closed,
}

/// Phone side of a running screen session.
pub struct ScreenServer {
    pub events: mpsc::UnboundedReceiver<ScreenEvent>,
    sink: ScreenSink,
}

impl ScreenServer {
    /// The sending half (shareable across threads) and the events.
    pub fn into_parts(self) -> (ScreenSink, mpsc::UnboundedReceiver<ScreenEvent>) {
        (self.sink, self.events)
    }

    pub fn send(&self, r: Record) -> SendOutcome {
        self.sink.send(r)
    }

    pub fn queued_bytes(&self) -> usize {
        self.sink.queued_bytes()
    }

    pub fn notice(&self, text: &str) {
        self.sink.notice(text);
    }

    pub fn close(&self) {
        self.sink.close();
    }
}

/// Sends video and notices on a running screen session.
#[derive(Clone)]
pub struct ScreenSink {
    video: mpsc::UnboundedSender<Vec<u8>>,
    queue: ByteQueue,
    ctl: mpsc::UnboundedSender<ControlMsg>,
    closer: SessionCloser,
}

impl ScreenSink {
    pub fn send(&self, r: Record) -> SendOutcome {
        let mut bytes = Vec::with_capacity(13 + r.payload.len());
        record::encode(&r, &mut bytes);
        if !self.queue.try_push(bytes.len()) {
            return SendOutcome::Full;
        }
        let len = bytes.len();
        if self.video.send(bytes).is_err() {
            self.queue.pop(len);
            return SendOutcome::Closed;
        }
        SendOutcome::Queued(self.queue.len())
    }

    pub fn queued_bytes(&self) -> usize {
        self.queue.len()
    }

    /// Something for the PC window to say.
    pub fn notice(&self, text: &str) {
        let _ = self.ctl.send(ControlMsg::ScreenState(ScreenState {
            active: true,
            notice: text.to_owned(),
            ..Default::default()
        }));
    }

    pub fn close(&self) {
        self.closer.close(b"screen stopped on the phone");
    }
}

/// Phone side: answer a `ScreenStart` (`None` declines) and run the session.
pub async fn serve_screen(
    mut session: Session,
    start: ScreenStart,
    answer: Option<ScreenAccept>,
) -> Result<ScreenServer, Error> {
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
    session.send(ControlMsg::ScreenAccept(accept)).await?;
    let conn = session.conn.clone();
    let mut out = conn.open_uni().await.map_err(tx)?;
    out.write_all(MAGIC).await.map_err(tx)?;
    let input = tokio::time::timeout(STREAM_TIMEOUT, conn.accept_uni())
        .await
        .map_err(|_| Error::Io("the PC did not open the input stream".into()))?
        .map_err(tx)?;
    let (ev_tx, events) = mpsc::unbounded_channel();
    // Input from the window.
    let input_tx = ev_tx.clone();
    let input_conn = conn.clone();
    tokio::spawn(async move {
        let r = read_input(input, |e| {
            if !matches!(e, InputEvent::MouseMove { dx: 0, dy: 0 }) {
                let _ = input_tx.send(ScreenEvent::Input(e));
            }
        })
        .await;
        if r.is_err() {
            input_conn.close(1u32.into(), b"input flood");
        }
    });
    // Video out, from a capped byte queue.
    let queue = ByteQueue::default();
    let (video, mut video_rx) = mpsc::unbounded_channel::<Vec<u8>>();
    let q = queue.clone();
    tokio::spawn(async move {
        while let Some(bytes) = video_rx.recv().await {
            let len = bytes.len();
            let ok = out.write_all(&bytes).await.is_ok();
            q.pop(len);
            if !ok {
                break;
            }
        }
        let _ = out.finish();
    });
    // Control both ways.
    let closer = session.closer();
    let (ctl, mut ctl_rx) = mpsc::unbounded_channel::<ControlMsg>();
    tokio::spawn(async move {
        loop {
            tokio::select! {
                m = ctl_rx.recv() => match m {
                    Some(m) => {
                        if session.send(m).await.is_err() {
                            break;
                        }
                    }
                    None => break,
                },
                r = session.recv() => match r {
                    Ok(Some(ControlMsg::ScreenFeedback(f))) => {
                        let _ = ev_tx.send(ScreenEvent::Feedback(f.presented_pts_us));
                    }
                    Ok(Some(ControlMsg::ScreenState(st))) => {
                        let _ = ev_tx.send(ScreenEvent::State(st));
                    }
                    Ok(Some(_)) => {}
                    _ => break,
                },
            }
        }
        let _ = ev_tx.send(ScreenEvent::Closed);
        session.conn.close(0u32.into(), b"screen done");
    });
    Ok(ScreenServer {
        events,
        sink: ScreenSink {
            video,
            queue,
            ctl,
            closer,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn byte_queue_refuses_past_the_cap() {
        let q = ByteQueue::default();
        assert!(q.try_push(QUEUE_CAP - 10));
        assert!(!q.try_push(11), "over the 1 MiB cap");
        assert!(q.try_push(10));
        q.pop(QUEUE_CAP);
        assert!(q.is_empty());
        q.pop(5); // never below zero
        assert_eq!(q.len(), 0);
    }
}
