//! "Phone camera on the PC": one session carries the consent, lens switches
//! and feedback on its control stream, and video records phone → PC on one
//! uni stream (the screen session's pattern without the input stream).
//! See `docs/WIRE_PROTOCOL.md`, "Phone camera".

use pb_identity::PeerId;
use pb_protocol::control::{
    CameraAccept, CameraControl, CameraStart, ControlMsg, OfferReply, ScreenFeedback,
};
use pb_video::record::{Decoder, Record, MAGIC};
use tokio::sync::{mpsc, watch};

use crate::{
    screen::{ByteQueue, ScreenSink, ACCEPT_TIMEOUT, RECORDS_CHANNEL, STREAM_TIMEOUT},
    tx, Error, Node, Session, SessionCloser,
};

/// PC side of a running camera session.
pub struct CameraView {
    pub accept: CameraAccept,
    /// Video records in order, all of them (backpressure, like the screen).
    pub records: mpsc::Receiver<Record>,
    lens: watch::Receiver<Option<u8>>,
    ctl: mpsc::UnboundedSender<ControlMsg>,
    closer: SessionCloser,
}

impl CameraView {
    /// A shareable remote (feedback, lens, close) and the record stream.
    pub fn into_parts(self) -> (CameraRemote, mpsc::Receiver<Record>) {
        (
            CameraRemote {
                accept: self.accept,
                lens: self.lens,
                ctl: self.ctl,
                closer: self.closer,
            },
            self.records,
        )
    }

    pub fn feedback(&self, presented_pts_us: i64) {
        feedback(&self.ctl, presented_pts_us);
    }

    pub fn switch_lens(&self, lens: u32) {
        switch_lens(&self.ctl, lens);
    }

    /// The lens the phone last said it uses (`None` until it says).
    pub fn lens(&self) -> Option<u8> {
        *self.lens.borrow()
    }

    /// Follows `lens()`: changes each time the phone reports a lens.
    pub fn lens_updates(&self) -> watch::Receiver<Option<u8>> {
        self.lens.clone()
    }

    pub fn close(&self) {
        self.closer.close(b"camera closed on the PC");
    }
}

/// The PC's handle on a camera session (thread-safe).
pub struct CameraRemote {
    pub accept: CameraAccept,
    lens: watch::Receiver<Option<u8>>,
    ctl: mpsc::UnboundedSender<ControlMsg>,
    closer: SessionCloser,
}

impl CameraRemote {
    pub fn feedback(&self, presented_pts_us: i64) {
        feedback(&self.ctl, presented_pts_us);
    }

    pub fn switch_lens(&self, lens: u32) {
        switch_lens(&self.ctl, lens);
    }

    /// The lens the phone last said it uses (`None` until it says).
    pub fn lens(&self) -> Option<u8> {
        *self.lens.borrow()
    }

    /// Follows `lens()`: changes each time the phone reports a lens.
    pub fn lens_updates(&self) -> watch::Receiver<Option<u8>> {
        self.lens.clone()
    }

    pub fn close(&self) {
        self.closer.close(b"camera closed on the PC");
    }
}

fn feedback(ctl: &mpsc::UnboundedSender<ControlMsg>, presented_pts_us: i64) {
    let _ = ctl.send(ControlMsg::ScreenFeedback(ScreenFeedback {
        presented_pts_us,
    }));
}

fn switch_lens(ctl: &mpsc::UnboundedSender<ControlMsg>, lens: u32) {
    let _ = ctl.send(ControlMsg::CameraControl(CameraControl { lens }));
}

impl Node {
    /// Ask `peer` to use its camera; `on_asked` runs once the request has
    /// been sent. Returns once the phone accepted and the stream is open.
    pub async fn open_camera(
        &self,
        peer: &PeerId,
        mut req: CameraStart,
        on_asked: impl FnOnce(),
    ) -> Result<CameraView, Error> {
        let mut s = self.connect(peer).await?;
        req.id = crate::files::new_id();
        let id = req.id;
        s.send(ControlMsg::CameraStart(req)).await?;
        on_asked();
        let accept = tokio::time::timeout(ACCEPT_TIMEOUT, async {
            loop {
                match s.recv().await? {
                    Some(ControlMsg::CameraAccept(a)) if a.id == id => return Ok(a),
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
                for r in recs {
                    if rec_tx.send(r).await.is_err() {
                        return;
                    }
                }
            }
        });
        let closer = s.closer();
        let (ctl, mut ctl_rx) = mpsc::unbounded_channel::<ControlMsg>();
        let (lens_tx, lens) = watch::channel(None);
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
                        Ok(Some(ControlMsg::CameraLens(l))) => {
                            // Every report counts, even the same value again.
                            lens_tx.send_replace(Some(l.lens));
                        }
                        Ok(Some(_)) => {}
                        _ => break,
                    },
                }
            }
            s.conn.close(0u32.into(), b"camera done");
        });
        Ok(CameraView {
            accept,
            records,
            lens,
            ctl,
            closer,
        })
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum CameraEvent {
    Feedback(i64),
    /// The PC wants the other lens.
    Lens(u32),
    /// The session ended. `by_peer`: the PC closed it on purpose, not a
    /// lost link.
    Closed {
        by_peer: bool,
    },
}

/// Phone side of a running camera session.
pub struct CameraServer {
    pub events: mpsc::UnboundedReceiver<CameraEvent>,
    sink: ScreenSink,
}

impl CameraServer {
    /// The sending half (a clone, thread-safe) and the events.
    pub fn into_parts(self) -> (ScreenSink, mpsc::UnboundedReceiver<CameraEvent>) {
        (self.sink, self.events)
    }

    pub fn send(&self, r: Record) -> crate::screen::SendOutcome {
        self.sink.send(r)
    }

    /// Tell the PC which lens is in use (0 back, 1 front).
    pub fn send_lens(&self, lens: u8) {
        self.sink.send_lens(lens);
    }

    pub fn close(&self) {
        self.sink.close();
    }
}

/// Phone side: answer a `CameraStart`. `None` declines.
pub async fn serve_camera(
    mut session: Session,
    start: CameraStart,
    answer: Option<CameraAccept>,
) -> Result<CameraServer, Error> {
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
    session.send(ControlMsg::CameraAccept(accept)).await?;
    let conn = session.conn.clone();
    let mut out = conn.open_uni().await.map_err(tx)?;
    out.write_all(MAGIC).await.map_err(tx)?;
    let (ev_tx, events) = mpsc::unbounded_channel();
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
                        let _ = ev_tx.send(CameraEvent::Feedback(f.presented_pts_us));
                    }
                    Ok(Some(ControlMsg::CameraControl(c))) => {
                        let _ = ev_tx.send(CameraEvent::Lens(c.lens));
                    }
                    Ok(Some(_)) => {}
                    _ => break,
                },
            }
        }
        let by_peer = matches!(
            session.conn.close_reason(),
            Some(iroh::endpoint::ConnectionError::ApplicationClosed(_))
        );
        let _ = ev_tx.send(CameraEvent::Closed { by_peer });
        session.conn.close(0u32.into(), b"camera done");
    });
    Ok(CameraServer {
        events,
        sink: ScreenSink::new(video, queue, ctl, closer),
    })
}
