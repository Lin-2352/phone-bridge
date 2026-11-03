//! Audio sharing, either direction: one session carries raw 16-bit PCM on a
//! uni stream, from the sending side to the receiving side.
//!
//! The requester (who dials and sends `AudioStart`) and the responder (who
//! answers with `AudioAccept`) are fixed by who connected; which of them
//! *sends* the PCM is `AudioStart.direction`. One [`AudioSession`] type serves
//! every role: [`AudioSession::send_frame`] on the sending end,
//! [`AudioSession::recv_frame`] on the receiving end.
//!
//! PCM travels in QUIC datagrams, never on a reliable stream. A stream would
//! retransmit in order and QUIC would buffer many seconds of audio during a
//! Wi-Fi stall, which the listener then hears late. A lost datagram is a
//! short gap; a stall leaves nothing behind it.
//!
//! Each datagram: `[4 bytes: sequence, u32 LE] [4 bytes: sender clock in ms,
//! u32 LE] [raw PCM, i16 LE]`.
//!
//! Every peer-supplied field is checked before it is trusted: the format on
//! `AudioStart` and `AudioAccept`, and each packet's length (a header, then
//! 1..=64 KiB of whole sample groups). The sender's QUIC buffer holds about
//! 100 ms ([`crate::tuning::DATAGRAM_SEND_BUFFER`]) and the receiver queue
//! [`QUEUE_MS`], each dropping the oldest, so neither end can fall behind.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use iroh::endpoint::{Connection, SendDatagramError};
use pb_identity::PeerId;
use pb_protocol::control::{
    audio_frame_len_ok, AudioAccept, AudioReason, AudioStart, AudioStop, ControlMsg,
    AUDIO_PHONE_TO_PC,
};
use pb_protocol::CAP_AUDIO;
use tokio::sync::{mpsc, Notify};

use crate::{lock, screen::ACCEPT_TIMEOUT, Error, Node, Session, SessionCloser};

/// How much audio the receiver keeps queued before dropping the oldest.
pub const QUEUE_MS: usize = 100;

/// Bytes before the PCM in every audio datagram.
pub const PACKET_HEADER: usize = 8;

/// A lost stretch up to this long is filled with silence so playback keeps
/// its timing; a longer one is a stall, and filling it would only add delay.
pub const FILL_MAX_MS: usize = 40;

/// One audio datagram.
pub fn encode_packet(seq: u32, sent_ms: u32, pcm: &[u8]) -> Vec<u8> {
    let mut p = Vec::with_capacity(PACKET_HEADER + pcm.len());
    p.extend_from_slice(&seq.to_le_bytes());
    p.extend_from_slice(&sent_ms.to_le_bytes());
    p.extend_from_slice(pcm);
    p
}

/// (sequence, sender clock, PCM) of a datagram, `None` when it is malformed:
/// too short, or PCM that breaks the format limits.
pub fn decode_packet(p: &[u8], channels: u32) -> Option<(u32, u32, &[u8])> {
    let (head, pcm) = p.split_at_checked(PACKET_HEADER)?;
    if !audio_frame_len_ok(pcm.len(), channels) {
        return None;
    }
    let seq = u32::from_le_bytes(head[..4].try_into().ok()?);
    let sent = u32::from_le_bytes(head[4..].try_into().ok()?);
    Some((seq, sent, pcm))
}

/// PCM bytes that fit one datagram of `max_datagram` bytes, in whole sample
/// groups. Zero when not even one group fits.
pub fn pcm_per_packet(max_datagram: usize, channels: u32) -> usize {
    let group = 2 * channels as usize;
    if group == 0 {
        return 0;
    }
    let room = max_datagram.saturating_sub(PACKET_HEADER);
    room - room % group
}

/// Order of arriving packets: late ones are refused, gaps are counted.
#[derive(Debug, Default)]
pub struct SeqTracker {
    next: Option<u32>,
    pub lost: u64,
    pub late: u64,
}

impl SeqTracker {
    /// `None`: the packet is late, drop it. `Some(n)`: play it, `n` packets
    /// before it never came.
    pub fn accept(&mut self, seq: u32) -> Option<u32> {
        let missing = match self.next {
            Some(next) if seq < next => {
                self.late += 1;
                return None;
            }
            Some(next) => seq - next,
            None => 0,
        };
        self.next = Some(seq.saturating_add(1));
        self.lost += u64::from(missing);
        Some(missing)
    }
}

/// Bytes of silence to play for `missing` lost packets of `packet_len` bytes.
/// Zero when the gap is over [`FILL_MAX_MS`].
pub fn fill_bytes(missing: u32, packet_len: usize, sample_rate: u32, channels: u32) -> usize {
    let total = (missing as usize).saturating_mul(packet_len);
    let cap = sample_rate as usize * channels as usize * 2 * FILL_MAX_MS / 1000;
    if total <= cap {
        total
    } else {
        0
    }
}

/// How the audio is doing, for the log. Metadata only.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AudioStats {
    /// Datagrams handed to QUIC (sender) or received (receiver).
    pub packets: u64,
    /// Receiver: packets that never came.
    pub lost: u64,
    /// Receiver: packets that came too late to play.
    pub late: u64,
    /// Receiver: worst delay since the last read, above the lowest delay seen.
    pub max_age_ms: u32,
    /// QUIC packets the connection counts as lost.
    pub quic_lost: u64,
}

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
struct RxCounters {
    packets: AtomicU64,
    lost: AtomicU64,
    late: AtomicU64,
    max_age_ms: AtomicU32,
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
    /// `stop` was called: the control task is telling the peer, then waits
    /// for it to close, so letting go of the session must not cut that off.
    stopping: AtomicBool,
    /// What ended it, in words (metadata only, for logs): the first reason wins.
    detail: Mutex<String>,
    events: mpsc::UnboundedSender<AudioEvent>,
    closer: SessionCloser,
    slot: AudioSlot,
    counters: RxCounters,
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

    /// Record why, unless the session already ended for another reason.
    fn why(&self, text: impl Into<String>) {
        let mut d = lock(&self.detail);
        if !self.ended.load(Ordering::SeqCst) && d.is_empty() {
            *d = text.into();
        }
    }

    /// `end`, and close the connection (a violation or a local close).
    fn end_and_close(&self, ev: AudioEvent, reason: &[u8]) {
        self.why(String::from_utf8_lossy(reason).into_owned());
        self.end(ev);
        self.closer.close(reason);
    }
}

enum Cmd {
    Stop,
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
    conn: Connection,
    began: Instant,
    next_seq: AtomicU32,
}

impl AudioSession {
    /// Whether this end sends the PCM (else it receives it).
    /// Why the session ended, in words for the log (empty while it runs).
    pub fn end_detail(&self) -> String {
        lock(&self.shared.detail).clone()
    }

    pub fn is_sender(&self) -> bool {
        self.sender
    }

    /// How the audio is doing; the receiver's worst age restarts at each read.
    pub fn stats(&self) -> AudioStats {
        let c = &self.shared.counters;
        AudioStats {
            packets: c.packets.load(Ordering::Relaxed),
            lost: c.lost.load(Ordering::Relaxed),
            late: c.late.load(Ordering::Relaxed),
            max_age_ms: c.max_age_ms.swap(0, Ordering::Relaxed),
            quic_lost: self.conn.stats().lost_packets,
        }
    }

    /// Sending end: send one PCM frame, split into datagrams that fit the
    /// path. Never blocks; QUIC drops the oldest unsent audio beyond about
    /// 100 ms. Fails on the receiving end, on an invalid frame (see
    /// [`audio_frame_len_ok`]) and once the session has ended.
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
        if self.shared.ended.load(Ordering::SeqCst) {
            return Err(Error::Closed);
        }
        let Some(max) = self.conn.max_datagram_size() else {
            return Err(Error::Closed);
        };
        let per_packet = pcm_per_packet(max, self.accept.channels);
        if per_packet == 0 {
            return Err(Error::Protocol("the path is too small for audio".into()));
        }
        let sent_ms = self.began.elapsed().as_millis() as u32;
        for chunk in pcm.chunks(per_packet) {
            let seq = self.next_seq.fetch_add(1, Ordering::Relaxed);
            match self
                .conn
                .send_datagram(encode_packet(seq, sent_ms, chunk).into())
            {
                Ok(()) => {
                    self.shared.counters.packets.fetch_add(1, Ordering::Relaxed);
                }
                // The path shrank between the check and the send: this audio
                // is lost like any other packet.
                Err(SendDatagramError::TooLarge) => {}
                Err(e) => {
                    self.shared.why(format!("audio send error: {e}"));
                    self.shared.end_and_close(
                        AudioEvent::WriteFailed(e.to_string()),
                        b"audio write failed",
                    );
                    return Err(Error::Closed);
                }
            }
        }
        Ok(())
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
        self.shared.stopping.store(true, Ordering::SeqCst);
        let _ = self.cmd.send(Cmd::Stop);
    }

    /// Close the connection at once, without telling the peer why.
    pub fn close(&self) {
        // After `stop`, dropping the session must not cut the stop short: the
        // control task closes once the peer has answered (or after 2 s).
        if self.shared.stopping.load(Ordering::SeqCst) {
            return;
        }
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

    if s.conn.max_datagram_size().is_none() {
        s.conn.close(0u32.into(), b"no datagrams");
        return Err(Error::Protocol(
            "the peer cannot receive audio packets".into(),
        ));
    }
    let sender = req.direction != AUDIO_PHONE_TO_PC;
    Ok(start_session(s, req, accept, sender, slot))
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
    if session.conn.max_datagram_size().is_none() {
        refuse(session, start.id, AudioReason::Unavailable).await;
        return Err(Error::AudioRefused(AudioReason::Unavailable));
    }
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

    let sender = start.direction == AUDIO_PHONE_TO_PC;
    Ok(start_session(session, start, accept, sender, slot))
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
    sender: bool,
    slot: AudioSlot,
) -> AudioSession {
    let (ev_tx, events) = mpsc::unbounded_channel();
    let (cmd, mut cmd_rx) = mpsc::unbounded_channel::<Cmd>();
    let shared = Arc::new(Shared {
        queue: FrameQueue::new(queue_cap_bytes(accept.sample_rate, accept.channels)),
        ended: AtomicBool::new(false),
        detail: Mutex::new(String::new()),
        stopping: AtomicBool::new(false),
        events: ev_tx,
        closer: session.closer(),
        slot,
        counters: RxCounters::default(),
    });
    let conn = session.conn.clone();
    if !sender {
        tokio::spawn(read_loop(
            conn.clone(),
            accept.sample_rate,
            accept.channels,
            shared.clone(),
        ));
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
                        ctl_shared.why("stopped here");
                        ctl_shared.end(AudioEvent::Stopped { by_peer: false });
                        // Waits for the peer to close, so the stop is not lost.
                        session.finish().await;
                        return;
                    }
                    None => break,
                },
                r = session.recv() => match r {
                    Ok(Some(ControlMsg::AudioStop(st))) if st.id == id => {
                        ctl_shared.why("the peer sent AudioStop");
                        ctl_shared.end(AudioEvent::Stopped { by_peer: true });
                        break;
                    }
                    Ok(Some(_)) => {}
                    other => {
                        ctl_shared.why(match other {
                            Err(Error::Timeout) => "control channel silent for 15 s (heartbeat timeout)".to_owned(),
                            Err(e) => format!("control channel error: {e}"),
                            _ => "control channel closed by the peer".to_owned(),
                        });
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
        conn,
        began: Instant::now(),
        next_seq: AtomicU32::new(0),
    }
}

/// Receiving end: datagrams to queue. Never waits for the consumer; the
/// queue drops the oldest instead.
async fn read_loop(conn: Connection, sample_rate: u32, channels: u32, shared: Arc<Shared>) {
    let began = Instant::now();
    let mut order = SeqTracker::default();
    // Lowest (arrival - sent) seen: the clocks differ, so age is measured
    // above the best the path has done.
    let mut best: Option<i64> = None;
    loop {
        let packet = match conn.read_datagram().await {
            Ok(p) => p,
            Err(e) => {
                shared.why(format!("audio packets ended: {e}"));
                shared.end(AudioEvent::Stopped { by_peer: false });
                return;
            }
        };
        let Some((seq, sent_ms, pcm)) = decode_packet(&packet, channels) else {
            shared.end_and_close(
                AudioEvent::Violation(format!("bad audio packet of {} bytes", packet.len())),
                b"bad audio packet",
            );
            return;
        };
        let c = &shared.counters;
        c.packets.fetch_add(1, Ordering::Relaxed);
        let Some(missing) = order.accept(seq) else {
            c.late.store(order.late, Ordering::Relaxed);
            continue;
        };
        c.lost.store(order.lost, Ordering::Relaxed);
        let delay = began.elapsed().as_millis() as i64 - i64::from(sent_ms);
        let low = best.map_or(delay, |b| b.min(delay));
        best = Some(low);
        c.max_age_ms
            .fetch_max((delay - low) as u32, Ordering::Relaxed);
        let fill = fill_bytes(missing, pcm.len(), sample_rate, channels);
        if fill > 0 {
            shared.queue.push(vec![0; fill]);
        }
        shared.queue.push(pcm.to_vec());
    }
}

#[cfg(test)]
mod queue_tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn the_cap_is_100_ms_of_pcm() {
        assert_eq!(queue_cap_bytes(48_000, 2), 19_200);
        assert_eq!(queue_cap_bytes(44_100, 1), 8_820);
    }

    #[test]
    fn a_packet_round_trips() {
        let p = encode_packet(7, 1234, &[1, 0, 2, 0]);
        assert_eq!(decode_packet(&p, 2), Some((7, 1234, &[1u8, 0, 2, 0][..])));
    }

    #[test]
    fn malformed_packets_are_refused() {
        assert_eq!(decode_packet(&[], 2), None);
        assert_eq!(decode_packet(&[0; 7], 2), None, "short header");
        assert_eq!(decode_packet(&[0; 8], 2), None, "no PCM");
        assert_eq!(decode_packet(&[0; 14], 2), None, "not whole stereo samples");
        assert!(decode_packet(&[0; 14], 1).is_some());
        assert_eq!(
            decode_packet(&vec![0; PACKET_HEADER + 65_536 + 4], 2),
            None,
            "over the frame limit"
        );
    }

    #[test]
    fn a_packet_holds_whole_sample_groups_within_the_datagram() {
        assert_eq!(pcm_per_packet(1200, 2), 1192);
        assert_eq!(pcm_per_packet(1201, 2), 1192);
        assert_eq!(pcm_per_packet(1201, 1), 1192);
        assert_eq!(pcm_per_packet(11, 2), 0);
        assert_eq!(pcm_per_packet(1200, 0), 0);
        assert_eq!(pcm_per_packet(0, 2), 0);
    }

    #[test]
    fn order_counts_gaps_and_refuses_late_packets() {
        let mut t = SeqTracker::default();
        assert_eq!(t.accept(5), Some(0), "the first packet sets the start");
        assert_eq!(t.accept(6), Some(0));
        assert_eq!(t.accept(9), Some(2));
        assert_eq!(t.accept(8), None, "late");
        assert_eq!(t.accept(9), None, "duplicate");
        assert_eq!(t.accept(10), Some(0));
        assert_eq!((t.lost, t.late), (2, 2));
    }

    #[test]
    fn short_gaps_are_filled_and_stalls_are_not() {
        // 40 ms of 48 kHz stereo is 7680 bytes.
        assert_eq!(fill_bytes(0, 960, 48_000, 2), 0);
        assert_eq!(fill_bytes(2, 960, 48_000, 2), 1920);
        assert_eq!(fill_bytes(8, 960, 48_000, 2), 7680);
        assert_eq!(
            fill_bytes(9, 960, 48_000, 2),
            0,
            "a stall leaves no silence"
        );
        assert_eq!(fill_bytes(u32::MAX, 65_536, 48_000, 2), 0);
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
