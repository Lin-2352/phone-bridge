//! Audio sharing, either direction: one session carries raw 16-bit PCM on a
//! uni stream, from the sending side to the receiving side.
//!
//! The requester (who dials and sends `AudioStart`) and the responder (who
//! answers with `AudioAccept`) are fixed by who connected; which of them
//! *sends* the PCM is `AudioStart.direction`. One [`AudioSession`] type serves
//! every role: [`AudioSession::send_frame`] on the sending end,
//! [`AudioSession::recv_frame`] on the receiving end.
//!
//! Wire format on the uni stream:
//!   `[4 bytes: AUDIO_MAGIC "PBA0"]` followed by repeated
//!   `[4 bytes: frame_len as u32 LE] [frame_len bytes: raw PCM, i16 LE]`
//!
//! Every peer-supplied field is checked before it is trusted or allocated:
//! the format on `AudioStart` and `AudioAccept`, the magic, and each frame
//! length (1..=64 KiB, a whole number of sample groups). Both ends keep only
//! about 300 ms of audio queued and drop the oldest beyond that, so a slow
//! consumer never stalls the other side.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use iroh::endpoint::{ReadExactError, RecvStream, SendStream};
use pb_identity::PeerId;
use pb_protocol::control::{
    audio_frame_len_ok, AudioAccept, AudioReason, AudioStart, AudioStop, ControlMsg,
    AUDIO_PHONE_TO_PC,
};
use pb_protocol::CAP_AUDIO;
use tokio::sync::{mpsc, Notify};

use crate::{lock, screen::ACCEPT_TIMEOUT, tx, Error, Node, Session, SessionCloser};

/// Magic header sent at the start of the audio uni-stream.
pub const AUDIO_MAGIC: &[u8; 4] = b"PBA0";

/// How much audio either end keeps queued before dropping the oldest.
pub const QUEUE_MS: usize = 300;

/// Opening the data stream after the accept.
const STREAM_TIMEOUT: Duration = Duration::from_secs(10);

/// One chunk of raw PCM (i16 LE samples, interleaved channels).
pub struct AudioFrame {
    /// Raw bytes (pairs of i16 LE samples).
    pub samples: Vec<u8>,
}

/// What happened to a running session. Exactly one event is produced, the
/// first reason the session ended.
#[derive(Debug)]
pub enum AudioEvent {
    /// The session ended: `AudioStop` from the peer (`by_peer`), the link or
    /// the peer's stream ended, or this end stopped or dropped it.
    Stopped { by_peer: bool },
    /// Writing to the data stream failed (the peer stopped reading it).
    WriteFailed(String),
    /// The peer sent something the limits forbid (bad length, truncated
    /// frame). The session was closed.
    Violation(String),
}

/// Which directions have a session running on this node: one per direction.
#[derive(Clone, Default)]
pub struct AudioSlots(Arc<[AtomicBool; 2]>);

impl AudioSlots {
    /// Whether a session in `direction` is running (or being set up).
    pub fn is_busy(&self, direction: u32) -> bool {
        self.0
            .get(direction as usize)
            .is_some_and(|b| b.load(Ordering::SeqCst))
    }

    /// Take the slot for `direction`; `None` when it is busy or not a
    /// direction. Released when the guard is dropped or the session ends.
    pub fn try_acquire(&self, direction: u32) -> Option<AudioSlot> {
        let flag = self.0.get(direction as usize)?;
        flag.compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .ok()?;
        Some(AudioSlot {
            slots: self.clone(),
            direction: direction as usize,
            released: AtomicBool::new(false),
        })
    }
}

/// One direction's slot, held for the life of a session.
pub struct AudioSlot {
    slots: AudioSlots,
    direction: usize,
    released: AtomicBool,
}

impl AudioSlot {
    fn release(&self) {
        if !self.released.swap(true, Ordering::SeqCst) {
            self.slots.0[self.direction].store(false, Ordering::SeqCst);
        }
    }
}

impl Drop for AudioSlot {
    fn drop(&mut self) {
        self.release();
    }
}

/// Bytes of audio kept queued: [`QUEUE_MS`] of 16-bit PCM.
pub fn queue_cap_bytes(sample_rate: u32, channels: u32) -> usize {
    sample_rate as usize * channels as usize * 2 * QUEUE_MS / 1000
}

#[derive(Default)]
struct QueueState {
    frames: VecDeque<Vec<u8>>,
    bytes: usize,
    closed: bool,
}

/// Bounded queue that drops the oldest frames; one consumer.
struct FrameQueue {
    state: Mutex<QueueState>,
    notify: Notify,
    cap: usize,
}

impl FrameQueue {
    fn new(cap: usize) -> Self {
        Self {
            state: Mutex::default(),
            notify: Notify::new(),
            cap,
        }
    }

    /// Queue a frame, dropping the oldest while over the cap (the newest is
    /// always kept). `false` when the queue is closed.
    fn push(&self, frame: Vec<u8>) -> bool {
        let mut st = lock(&self.state);
        if st.closed {
            return false;
        }
        st.bytes += frame.len();
        st.frames.push_back(frame);
        while st.bytes > self.cap && st.frames.len() > 1 {
            if let Some(old) = st.frames.pop_front() {
                st.bytes -= old.len();
            }
        }
        drop(st);
        self.notify.notify_one();
        true
    }

    /// Next frame; after `close`, what is left and then `None`.
    async fn pop(&self) -> Option<Vec<u8>> {
        loop {
            {
                let mut st = lock(&self.state);
                if let Some(f) = st.frames.pop_front() {
                    st.bytes -= f.len();
                    return Some(f);
                }
                if st.closed {
                    return None;
                }
            }
            self.notify.notified().await;
        }
    }

    fn close(&self) {
        lock(&self.state).closed = true;
        self.notify.notify_one();
    }
}

/// State the session's tasks share with the [`AudioSession`] handle.
struct Shared {
    queue: FrameQueue,
    ended: AtomicBool,
    events: mpsc::UnboundedSender<AudioEvent>,
    closer: SessionCloser,
    slot: AudioSlot,
}

impl Shared {
    /// The session is over: report the first reason, stop the queue, free
    /// the direction.
    fn end(&self, ev: AudioEvent) {
        if !self.ended.swap(true, Ordering::SeqCst) {
            let _ = self.events.send(ev);
        }
        self.queue.close();
        self.slot.release();
    }

    /// `end`, and close the connection (a violation or a local close).
    fn end_and_close(&self, ev: AudioEvent, reason: &[u8]) {
        self.end(ev);
        self.closer.close(reason);
    }
}

enum Cmd {
    Stop,
}

enum Data {
    Send(SendStream),
    Recv(RecvStream),
}

/// A running audio session, on either side, in either direction.
///
/// Dropping it closes the session.
pub struct AudioSession {
    /// The accepted format (`sample_rate`, `channels`) and `id`.
    pub accept: AudioAccept,
    /// The request this session answers (direction, source).
    pub start: AudioStart,
    /// Why the session ended; see [`AudioEvent`].
    pub events: mpsc::UnboundedReceiver<AudioEvent>,
    sender: bool,
    shared: Arc<Shared>,
    cmd: mpsc::UnboundedSender<Cmd>,
}

impl AudioSession {
    /// Whether this end sends the PCM (else it receives it).
    pub fn is_sender(&self) -> bool {
        self.sender
    }

    /// Sending end: queue one PCM frame. Never blocks; beyond about 300 ms
    /// of unsent audio the oldest frames are dropped. Fails on the receiving
    /// end, on an invalid frame (see [`audio_frame_len_ok`]) and once the
    /// session has ended.
    pub fn send_frame(&self, pcm: Vec<u8>) -> Result<(), Error> {
        if !self.sender {
            return Err(Error::Protocol("this end receives audio".into()));
        }
        if !audio_frame_len_ok(pcm.len(), self.accept.channels) {
            return Err(Error::Protocol(format!(
                "audio frame of {} bytes is empty, over 64 KiB or not whole samples",
                pcm.len()
            )));
        }
        if self.shared.queue.push(pcm) {
            Ok(())
        } else {
            Err(Error::Closed)
        }
    }

    /// Receiving end: the next frame, `None` once the session has ended and
    /// the queued frames are used up (and always `None` on the sending end).
    pub async fn recv_frame(&mut self) -> Option<AudioFrame> {
        if self.sender {
            return None;
        }
        self.shared
            .queue
            .pop()
            .await
            .map(|samples| AudioFrame { samples })
    }

    /// Stop the session gracefully: tell the peer with `AudioStop`, then
    /// close. Either end may call it.
    pub fn stop(&self) {
        let _ = self.cmd.send(Cmd::Stop);
    }

    /// Close the connection at once, without telling the peer why.
    pub fn close(&self) {
        self.shared.end_and_close(
            AudioEvent::Stopped { by_peer: false },
            b"audio closed locally",
        );
    }
}

impl Drop for AudioSession {
    fn drop(&mut self) {
        self.close();
    }
}

impl Node {
    /// The per-direction session slots of this node. The app may ask
    /// [`AudioSlots::is_busy`] before bothering the user.
    pub fn audio_slots(&self) -> &AudioSlots {
        &self.audio_slots
    }

    /// Request an audio session with `peer`. `on_asked` runs once the
    /// request is sent (before waiting for the answer, which includes the
    /// other user's consent).
    ///
    /// Errors: `Protocol` for an invalid request (nothing is sent),
    /// `AudioRefused(Busy)` when this node already has a session in that
    /// direction, `Unsupported` when the peer predates audio, `Declined` /
    /// `AudioRefused(reason)` when the peer refused.
    pub async fn open_audio(
        &self,
        peer: &PeerId,
        req: AudioStart,
        on_asked: impl FnOnce(),
    ) -> Result<AudioSession, Error> {
        let slot = check_request(&req, &self.audio_slots)?;
        let s = self.connect(peer).await?;
        run_request(s, req, slot, on_asked).await
    }
}

/// [`Node::open_audio`] over a session you already have.
pub async fn request_audio(
    session: Session,
    req: AudioStart,
    slots: &AudioSlots,
    on_asked: impl FnOnce(),
) -> Result<AudioSession, Error> {
    let slot = check_request(&req, slots)?;
    run_request(session, req, slot, on_asked).await
}

fn check_request(req: &AudioStart, slots: &AudioSlots) -> Result<AudioSlot, Error> {
    req.validate().map_err(|e| Error::Protocol(e.to_string()))?;
    slots
        .try_acquire(req.direction)
        .ok_or(Error::AudioRefused(AudioReason::Busy))
}

fn refusal_error(reason: AudioReason) -> Error {
    match reason {
        AudioReason::Declined => Error::Declined,
        other => Error::AudioRefused(other),
    }
}

async fn run_request(
    mut s: Session,
    mut req: AudioStart,
    slot: AudioSlot,
    on_asked: impl FnOnce(),
) -> Result<AudioSession, Error> {
    // An older peer ignores the unknown tags and would leave us waiting for
    // the whole consent timeout.
    if !s.remote_hello.supports(CAP_AUDIO) {
        s.conn.close(0u32.into(), b"audio unsupported");
        return Err(Error::Unsupported);
    }
    req.id = crate::files::new_id();
    let id = req.id;
    s.send(ControlMsg::AudioStart(req.clone())).await?;
    on_asked();

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

    if !accept.accepted {
        s.conn.close(0u32.into(), b"audio refused");
        return Err(refusal_error(AudioReason::from_code(accept.reason)));
    }
    if let Err(e) = accept.validate() {
        s.conn.close(0u32.into(), b"bad audio accept");
        return Err(Error::Protocol(e.to_string()));
    }

    // The sender opens the uni stream; here that is us when the PC sends.
    let data = if req.direction == AUDIO_PHONE_TO_PC {
        open_data_in(&s).await
    } else {
        open_data_out(&s).await
    };
    let data = match data {
        Ok(d) => d,
        Err(e) => {
            s.conn.close(0u32.into(), b"audio stream failed");
            return Err(e);
        }
    };
    Ok(start_session(s, req, accept, data, slot))
}

async fn open_data_out(s: &Session) -> Result<Data, Error> {
    let mut out = tokio::time::timeout(STREAM_TIMEOUT, s.conn.open_uni())
        .await
        .map_err(|_| Error::Timeout)?
        .map_err(tx)?;
    out.write_all(AUDIO_MAGIC).await.map_err(tx)?;
    Ok(Data::Send(out))
}

async fn open_data_in(s: &Session) -> Result<Data, Error> {
    tokio::time::timeout(STREAM_TIMEOUT, async {
        let mut input = s.conn.accept_uni().await.map_err(tx)?;
        let mut magic = [0u8; 4];
        input
            .read_exact(&mut magic)
            .await
            .map_err(|e| Error::Io(format!("audio stream header read failed: {e}")))?;
        if &magic != AUDIO_MAGIC {
            return Err(Error::Protocol("invalid audio stream magic".into()));
        }
        Ok(Data::Recv(input))
    })
    .await
    .map_err(|_| Error::Io("the other side did not open the audio stream".into()))?
}

/// Answer an `AudioStart` that arrived on `session`.
///
/// The request is validated first (a bad one is refused with `BadParams`
/// and the user is never asked), then the direction's slot is taken (`Busy`
/// otherwise). `answer` is the consent decision: `Ok(accept)` carries the
/// format this side will use (`sample_rate` and `channels` are validated,
/// `id`, `accepted` and `reason` are filled in), `Err(reason)` refuses with
/// that reason. Refusals return `Err(Declined)` or `Err(AudioRefused(..))`.
pub async fn serve_audio(
    session: Session,
    start: AudioStart,
    answer: Result<AudioAccept, AudioReason>,
    slots: &AudioSlots,
) -> Result<AudioSession, Error> {
    if let Err(e) = start.validate() {
        refuse(session, start.id, AudioReason::BadParams).await;
        return Err(Error::Protocol(e.to_string()));
    }
    let Some(slot) = slots.try_acquire(start.direction) else {
        refuse(session, start.id, AudioReason::Busy).await;
        return Err(Error::AudioRefused(AudioReason::Busy));
    };
    let mut accept = match answer {
        Ok(a) => a,
        Err(reason) => {
            refuse(session, start.id, reason).await;
            return Err(refusal_error(reason));
        }
    };
    accept.id = start.id;
    accept.accepted = true;
    accept.reason = 0;
    if let Err(e) = accept.validate() {
        refuse(session, start.id, AudioReason::Unavailable).await;
        return Err(Error::Protocol(format!("own audio format: {e}")));
    }
    session
        .send(ControlMsg::AudioAccept(accept.clone()))
        .await?;

    // The sender opens the uni stream; here that is us when the phone sends.
    let data = if start.direction == AUDIO_PHONE_TO_PC {
        open_data_out(&session).await
    } else {
        open_data_in(&session).await
    };
    let data = match data {
        Ok(d) => d,
        Err(e) => {
            session.conn.close(0u32.into(), b"audio stream failed");
            return Err(e);
        }
    };
    Ok(start_session(session, start, accept, data, slot))
}

/// Tell the requester no, and keep the session open in the background until
/// it closes, so closing first cannot discard the reply.
async fn refuse(session: Session, id: u64, reason: AudioReason) {
    if session
        .send(ControlMsg::AudioAccept(AudioAccept::refused(id, reason)))
        .await
        .is_err()
    {
        return;
    }
    tokio::spawn(async move {
        crate::files::wait_for_close(&session).await;
    });
}

fn start_session(
    session: Session,
    start: AudioStart,
    accept: AudioAccept,
    data: Data,
    slot: AudioSlot,
) -> AudioSession {
    let (ev_tx, events) = mpsc::unbounded_channel();
    let (cmd, mut cmd_rx) = mpsc::unbounded_channel::<Cmd>();
    let shared = Arc::new(Shared {
        queue: FrameQueue::new(queue_cap_bytes(accept.sample_rate, accept.channels)),
        ended: AtomicBool::new(false),
        events: ev_tx,
        closer: session.closer(),
        slot,
    });
    let sender = matches!(data, Data::Send(_));
    match data {
        Data::Send(out) => {
            tokio::spawn(write_loop(out, shared.clone()));
        }
        Data::Recv(input) => {
            tokio::spawn(read_loop(input, accept.channels, shared.clone()));
        }
    }

    // Control loop: AudioStop either way, and the end of the session.
    let id = start.id;
    let ctl_shared = shared.clone();
    let mut session = session;
    tokio::spawn(async move {
        loop {
            tokio::select! {
                c = cmd_rx.recv() => match c {
                    Some(Cmd::Stop) => {
                        let _ = session.send(ControlMsg::AudioStop(AudioStop { id })).await;
                        ctl_shared.end(AudioEvent::Stopped { by_peer: false });
                        // Waits for the peer to close, so the stop is not lost.
                        session.finish().await;
                        return;
                    }
                    None => break,
                },
                r = session.recv() => match r {
                    Ok(Some(ControlMsg::AudioStop(st))) if st.id == id => {
                        ctl_shared.end(AudioEvent::Stopped { by_peer: true });
                        break;
                    }
                    Ok(Some(_)) => {}
                    _ => {
                        ctl_shared.end(AudioEvent::Stopped { by_peer: false });
                        break;
                    }
                },
            }
        }
        session.conn.close(0u32.into(), b"audio done");
    });

    AudioSession {
        accept,
        start,
        events,
        sender,
        shared,
        cmd,
    }
}

/// Sending end: queue to stream, with the length prefix.
async fn write_loop(mut out: SendStream, shared: Arc<Shared>) {
    while let Some(frame) = shared.queue.pop().await {
        let len = (frame.len() as u32).to_le_bytes();
        let res = match out.write_all(&len).await {
            Ok(()) => out.write_all(&frame).await,
            Err(e) => Err(e),
        };
        if let Err(e) = res {
            shared.end_and_close(
                AudioEvent::WriteFailed(e.to_string()),
                b"audio write failed",
            );
            return;
        }
    }
    let _ = out.finish();
}

/// Receiving end: stream to queue. Never waits for the consumer, so the
/// sender is never held up; the queue drops the oldest instead.
async fn read_loop(mut input: RecvStream, channels: u32, shared: Arc<Shared>) {
    let mut len_buf = [0u8; 4];
    loop {
        match input.read_exact(&mut len_buf).await {
            Ok(()) => {}
            // The sender finished the stream between frames.
            Err(ReadExactError::FinishedEarly(0)) => {
                shared.end_and_close(AudioEvent::Stopped { by_peer: true }, b"audio ended");
                return;
            }
            Err(ReadExactError::FinishedEarly(_)) => {
                shared.end_and_close(
                    AudioEvent::Violation("truncated audio frame header".into()),
                    b"truncated audio frame",
                );
                return;
            }
            Err(ReadExactError::ReadError(_)) => {
                shared.end(AudioEvent::Stopped { by_peer: false });
                return;
            }
        }
        let len = u32::from_le_bytes(len_buf) as usize;
        // Checked before allocating: the length is the peer's word.
        if !audio_frame_len_ok(len, channels) {
            shared.end_and_close(
                AudioEvent::Violation(format!("bad audio frame length {len}")),
                b"bad audio frame length",
            );
            return;
        }
        let mut samples = vec![0u8; len];
        match input.read_exact(&mut samples).await {
            Ok(()) => {}
            Err(ReadExactError::FinishedEarly(_)) => {
                shared.end_and_close(
                    AudioEvent::Violation("truncated audio frame".into()),
                    b"truncated audio frame",
                );
                return;
            }
            Err(ReadExactError::ReadError(_)) => {
                shared.end(AudioEvent::Stopped { by_peer: false });
                return;
            }
        }
        shared.queue.push(samples);
    }
}

#[cfg(test)]
mod queue_tests {
    use super::*;

    #[test]
    fn the_cap_is_300_ms_of_pcm() {
        assert_eq!(queue_cap_bytes(48_000, 2), 57_600);
        assert_eq!(queue_cap_bytes(44_100, 1), 26_460);
    }

    #[tokio::test]
    async fn the_oldest_frames_are_dropped_over_the_cap() {
        let q = FrameQueue::new(100);
        for i in 0..10u8 {
            assert!(q.push(vec![i; 40]));
        }
        // 100 bytes hold two 40-byte frames: the newest two.
        assert_eq!(q.pop().await, Some(vec![8; 40]));
        assert_eq!(q.pop().await, Some(vec![9; 40]));
    }

    #[tokio::test]
    async fn the_newest_frame_is_kept_even_when_larger_than_the_cap() {
        let q = FrameQueue::new(10);
        q.push(vec![1; 8]);
        q.push(vec![2; 64]);
        assert_eq!(q.pop().await, Some(vec![2; 64]));
    }

    #[tokio::test]
    async fn a_closed_queue_drains_then_ends_and_refuses_pushes() {
        let q = FrameQueue::new(1000);
        q.push(vec![1; 4]);
        q.close();
        assert!(!q.push(vec![2; 4]));
        assert_eq!(q.pop().await, Some(vec![1; 4]));
        assert_eq!(q.pop().await, None);
    }

    #[tokio::test]
    async fn pop_wakes_when_a_frame_arrives() {
        let q = Arc::new(FrameQueue::new(1000));
        let q2 = q.clone();
        let t = tokio::spawn(async move { q2.pop().await });
        tokio::time::sleep(Duration::from_millis(20)).await;
        q.push(vec![5; 4]);
        assert_eq!(t.await.unwrap(), Some(vec![5; 4]));
    }

    #[test]
    fn slots_are_per_direction_and_released_on_drop() {
        let s = AudioSlots::default();
        let a = s.try_acquire(0).unwrap();
        assert!(s.is_busy(0) && !s.is_busy(1));
        assert!(s.try_acquire(0).is_none());
        assert!(s.try_acquire(2).is_none(), "not a direction");
        drop(a);
        assert!(!s.is_busy(0));
        // A stale guard must not free a newer holder's slot.
        let old = s.try_acquire(1).unwrap();
        old.release();
        let new = s.try_acquire(1).unwrap();
        drop(old);
        assert!(s.is_busy(1));
        drop(new);
        assert!(!s.is_busy(1));
    }
}
