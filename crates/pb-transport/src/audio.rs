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
//! u32 LE] [payload]`. The payload is what `AudioAccept.codec` says: raw PCM
//! (i16 LE), split to fit, or one Opus packet. An empty [`AudioFrame`] from
//! an Opus session stands for a lost packet, for the decoder to conceal.
//!
//! Every peer-supplied field is checked before it is trusted: the format on
//! `AudioStart` and `AudioAccept`, and each packet's length (a header, then a
//! payload [`audio_payload_ok`] accepts). The sender's QUIC buffer holds about
//! 100 ms ([`crate::tuning::DATAGRAM_SEND_BUFFER`]) and the receiver queue
//! [`QUEUE_MS`], each dropping the oldest, so neither end can fall behind.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use iroh::endpoint::{Connection, SendDatagramError};
use pb_identity::PeerId;
use pb_protocol::control::{
    audio_payload_ok, AudioAccept, AudioReason, AudioStart, AudioStop, ControlMsg,
    AUDIO_CODEC_OPUS, AUDIO_PHONE_TO_PC,
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

/// Unsent Opus QUIC may hold before new packets are skipped: about 200 ms at
/// 160 kbit/s. The shared datagram buffer is sized for PCM and would hold a
/// whole second of Opus.
pub const OPUS_SEND_BACKLOG: usize = 4 * 1024;

/// Length of an Opus packet the phone sends, for counting what a gap cost.
const OPUS_FRAME_MS: usize = 20;

/// One audio datagram.
pub fn encode_packet(seq: u32, sent_ms: u32, pcm: &[u8]) -> Vec<u8> {
    let mut p = Vec::with_capacity(PACKET_HEADER + pcm.len());
    p.extend_from_slice(&seq.to_le_bytes());
    p.extend_from_slice(&sent_ms.to_le_bytes());
    p.extend_from_slice(pcm);
    p
}

/// (sequence, sender clock, payload) of a datagram, `None` when it is
/// malformed: too short, or a payload that breaks the limits of `codec`.
pub fn decode_packet(p: &[u8], codec: u32, channels: u32) -> Option<(u32, u32, &[u8])> {
    let (head, pcm) = p.split_at_checked(PACKET_HEADER)?;
    if !audio_payload_ok(codec, pcm.len(), channels) {
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

/// How long a gap is waited on before giving up on the missing packet. Wi-Fi
/// sometimes swaps neighbours; anything later than this is a real loss.
pub const REORDER_WAIT: Duration = Duration::from_millis(15);
/// Packets held while waiting; beyond this the gap is given up at once.
const REORDER_HOLD: usize = 8;

/// What the receiver does next, in play order.
#[derive(Debug, PartialEq, Eq)]
pub enum Reordered {
    /// PCM to play.
    Pcm(Vec<u8>),
    /// This many packets never came.
    Gap(u32),
}

/// Puts arriving packets back in order: a packet that arrives ahead of its
/// predecessor waits briefly for it; late and duplicate packets are dropped.
#[derive(Debug, Default)]
pub struct Reorder {
    next: Option<u32>,
    held: std::collections::BTreeMap<u32, Vec<u8>>,
    pub lost: u64,
    pub late: u64,
}

impl Reorder {
    /// A packet arrived. Returns what can be played now.
    pub fn push(&mut self, seq: u32, pcm: Vec<u8>) -> Vec<Reordered> {
        let next = *self.next.get_or_insert(seq);
        if seq < next || self.held.contains_key(&seq) {
            self.late += 1;
            return Vec::new();
        }
        self.held.insert(seq, pcm);
        let mut out = Vec::new();
        self.drain(&mut out);
        if self.held.len() > REORDER_HOLD {
            self.give_up(&mut out);
        }
        out
    }

    /// Whether a packet is being held for one that has not come.
    pub fn waiting(&self) -> bool {
        !self.held.is_empty()
    }

    /// The wait is over: count what is missing as lost and play on.
    pub fn release(&mut self) -> Vec<Reordered> {
        let mut out = Vec::new();
        self.give_up(&mut out);
        out
    }

    fn drain(&mut self, out: &mut Vec<Reordered>) {
        while let Some(next) = self.next {
            let Some(p) = self.held.remove(&next) else {
                break;
            };
            out.push(Reordered::Pcm(p));
            self.next = Some(next.wrapping_add(1));
        }
    }

    fn give_up(&mut self, out: &mut Vec<Reordered>) {
        let (Some(next), Some((&first, _))) = (self.next, self.held.iter().next()) else {
            return;
        };
        let missing = first - next;
        self.lost += u64::from(missing);
        self.next = Some(first);
        out.push(Reordered::Gap(missing));
        self.drain(out);
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

/// Upper limits of the packet delay bins in the log, in ms. One more bin holds
/// everything later.
pub const AGE_BIN_LIMITS_MS: [u32; 8] = [20, 40, 60, 80, 100, 150, 200, 300];
/// Number of packet delay bins.
pub const AGE_BINS: usize = AGE_BIN_LIMITS_MS.len() + 1;

/// The bin for a packet `age_ms` later than the path's best lately.
pub fn age_bin(age_ms: u32) -> usize {
    AGE_BIN_LIMITS_MS
        .iter()
        .position(|&limit| age_ms <= limit)
        .unwrap_or(AGE_BIN_LIMITS_MS.len())
}

/// How long a stretch of [`AgeTracker`] lasts, in ms. It remembers two.
const AGE_WINDOW_MS: u64 = 15_000;

/// A packet's age: its delay above the lowest the path has shown lately. The
/// two ends' clocks differ and drift, so only the difference means anything, and
/// the lowest delay ever would make every packet look later as the clocks part.
#[derive(Debug, Default)]
pub struct AgeTracker {
    low: Option<i64>,
    earlier: Option<i64>,
    window_start_ms: u64,
}

impl AgeTracker {
    /// A packet with this `delay` (arrival minus the sender's clock) arrived
    /// at `now_ms`. Returns its age in ms.
    pub fn observe(&mut self, now_ms: u64, delay: i64) -> u32 {
        if now_ms.saturating_sub(self.window_start_ms) >= AGE_WINDOW_MS {
            self.window_start_ms = now_ms;
            self.earlier = self.low.take();
        }
        let low = self.low.map_or(delay, |l| l.min(delay));
        self.low = Some(low);
        let best = self.earlier.map_or(low, |e| e.min(low));
        (delay - best) as u32
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
    /// Open network paths (more than one can reorder packets).
    pub paths: usize,
    /// Round trip on the selected path, in ms (0 when unknown).
    pub rtt_ms: u64,
    /// Receiver: frames dropped because the consumer fell behind.
    pub queue_dropped: u64,
    /// Receiver: the longest wait between two packets at the sender since the
    /// last read: a stall there, before the network.
    pub max_sent_gap_ms: u32,
    /// Receiver: packets so far in each age bin ([`AGE_BIN_LIMITS_MS`]).
    pub age_bins: [u64; AGE_BINS],
    /// Sender: frames not sent because too much was waiting in QUIC.
    pub skipped_frames: u64,
    /// Sender: the most bytes that waited in QUIC since the last read.
    pub peak_backlog_bytes: u32,
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

/// Lost Opus packets to conceal: none for a stall, which would only add delay.
pub fn opus_frames_to_conceal(missing: u32) -> u32 {
    if (missing as usize).saturating_mul(OPUS_FRAME_MS) <= FILL_MAX_MS {
        missing
    } else {
        0
    }
}

#[derive(Default)]
struct RxCounters {
    packets: AtomicU64,
    lost: AtomicU64,
    late: AtomicU64,
    max_age_ms: AtomicU32,
    age_bins: [AtomicU64; AGE_BINS],
    last_sent_ms: AtomicU32,
    max_sent_gap_ms: AtomicU32,
}

impl RxCounters {
    fn note_age(&self, age: u32) {
        self.max_age_ms.fetch_max(age, Ordering::Relaxed);
        self.age_bins[age_bin(age)].fetch_add(1, Ordering::Relaxed);
    }

    /// The sender's clock on a packet. A long wait between two means the sending
    /// end stalled before it got to the network.
    fn note_sent(&self, sent_ms: u32) {
        let last = self.last_sent_ms.fetch_max(sent_ms, Ordering::Relaxed);
        if last != 0 {
            self.max_sent_gap_ms
                .fetch_max(sent_ms.saturating_sub(last), Ordering::Relaxed);
        }
    }

    fn take_max_sent_gap_ms(&self) -> u32 {
        self.max_sent_gap_ms.swap(0, Ordering::Relaxed)
    }
}

/// What the sending end did with its frames (counts only).
#[derive(Default)]
pub struct TxCounters {
    skipped: AtomicU64,
    peak_backlog: AtomicUsize,
}

impl TxCounters {
    /// Bytes waiting in QUIC's send buffer when a frame was handed over.
    pub fn note_backlog(&self, bytes: usize) {
        self.peak_backlog.fetch_max(bytes, Ordering::Relaxed);
    }

    /// A frame the guard did not send because too much was waiting.
    pub fn note_skipped(&self) {
        self.skipped.fetch_add(1, Ordering::Relaxed);
    }

    /// The most that waited since the last read.
    pub fn take_peak_backlog(&self) -> usize {
        self.peak_backlog.swap(0, Ordering::Relaxed)
    }

    pub fn skipped(&self) -> u64 {
        self.skipped.load(Ordering::Relaxed)
    }
}

/// Bytes waiting to be sent. QUIC reports free space, so the size of its buffer is the most free
/// space ever seen: the buffer is empty at the start of a session, and after any quiet moment.
pub(crate) fn send_backlog(capacity: &AtomicUsize, free: usize) -> usize {
    capacity.fetch_max(free, Ordering::Relaxed).max(free) - free
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
    dropped: AtomicU64,
}

impl FrameQueue {
    fn new(cap: usize) -> Self {
        Self {
            state: Mutex::default(),
            notify: Notify::new(),
            cap,
            dropped: AtomicU64::new(0),
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
                self.dropped.fetch_add(1, Ordering::Relaxed);
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
    tx: TxCounters,
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
    /// The most free space QUIC's send buffer has shown: its size.
    send_capacity: AtomicUsize,
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
            paths: self.conn.paths().iter().count(),
            rtt_ms: self
                .conn
                .paths()
                .iter()
                .find(|p| p.is_selected())
                .and_then(|p| self.conn.rtt(p.id()))
                .map_or(0, |d| d.as_millis() as u64),
            queue_dropped: self.shared.queue.dropped.load(Ordering::Relaxed),
            max_sent_gap_ms: c.take_max_sent_gap_ms(),
            age_bins: std::array::from_fn(|i| c.age_bins[i].load(Ordering::Relaxed)),
            skipped_frames: self.shared.tx.skipped(),
            peak_backlog_bytes: self.shared.tx.take_peak_backlog() as u32,
        }
    }

    /// The address the audio is using now, for the log.
    pub fn path_addr(&self) -> Option<std::net::SocketAddr> {
        crate::used_addr(&self.conn)
    }

    /// Sending end: send one PCM frame, split into datagrams that fit the
    /// path, or one Opus packet whole. Never blocks; QUIC drops the oldest
    /// unsent audio beyond about 100 ms. Fails on the receiving end, on an
    /// invalid frame (see [`audio_payload_ok`]) and once the session has ended.
    pub fn send_frame(&self, pcm: Vec<u8>) -> Result<(), Error> {
        if !self.sender {
            return Err(Error::Protocol("this end receives audio".into()));
        }
        let opus = self.accept.codec == AUDIO_CODEC_OPUS;
        if !audio_payload_ok(self.accept.codec, pcm.len(), self.accept.channels) {
            return Err(Error::Protocol(format!(
                "audio frame of {} bytes breaks the limits of its codec",
                pcm.len()
            )));
        }
        if self.shared.ended.load(Ordering::SeqCst) {
            return Err(Error::Closed);
        }
        let Some(max) = self.conn.max_datagram_size() else {
            return Err(Error::Closed);
        };
        let per_packet = if opus {
            // Never split: half a packet decodes to nothing.
            let backlog = send_backlog(&self.send_capacity, self.conn.datagram_send_buffer_space());
            self.shared.tx.note_backlog(backlog);
            // With the phone's small buffer this never fires: QUIC drops the oldest audio itself.
            // It is for a larger buffer, where keeping the oldest would send stale sound.
            if backlog > OPUS_SEND_BACKLOG {
                self.shared.tx.note_skipped();
                return Ok(()); // skipped: the receiver conceals the gap
            }
            pcm.len()
        } else {
            pcm_per_packet(max, self.accept.channels)
        };
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
        queue: FrameQueue::new(if accept.codec == AUDIO_CODEC_OPUS {
            OPUS_SEND_BACKLOG
        } else {
            queue_cap_bytes(accept.sample_rate, accept.channels)
        }),
        ended: AtomicBool::new(false),
        detail: Mutex::new(String::new()),
        stopping: AtomicBool::new(false),
        events: ev_tx,
        closer: session.closer(),
        slot,
        counters: RxCounters::default(),
        tx: TxCounters::default(),
    });
    let conn = session.conn.clone();
    if !sender {
        tokio::spawn(read_loop(
            conn.clone(),
            accept.codec,
            accept.sample_rate,
            accept.channels,
            shared.clone(),
        ));
    }

    // Control loop: AudioStop either way, and the end of the session.
    let id = start.id;
    let ctl_shared = shared.clone();
    let mut session = session;
    session.set_watchdog(crate::AUDIO_WATCHDOG);
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
                            Err(Error::Timeout) => format!(
                                "control channel silent for {} s (heartbeat timeout)",
                                crate::AUDIO_WATCHDOG.as_secs()
                            ),
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
        send_capacity: AtomicUsize::new(0),
    }
}

/// Receiving end: datagrams to queue. Never waits for the consumer; the
/// queue drops the oldest instead.
async fn read_loop(
    conn: Connection,
    codec: u32,
    sample_rate: u32,
    channels: u32,
    shared: Arc<Shared>,
) {
    let format = Format {
        codec,
        sample_rate,
        channels,
    };
    let began = Instant::now();
    let mut order = Reorder::default();
    let mut waiting_since: Option<Instant> = None;
    let mut packet_len = 0usize;
    let mut ages = AgeTracker::default();
    loop {
        let read = match waiting_since {
            Some(t) => {
                match tokio::time::timeout(
                    REORDER_WAIT.saturating_sub(t.elapsed()),
                    conn.read_datagram(),
                )
                .await
                {
                    Ok(r) => r,
                    Err(_) => {
                        // The missing packet is not coming.
                        let outs = order.release();
                        waiting_since = None;
                        play(&shared, outs, packet_len, &format, &order);
                        continue;
                    }
                }
            }
            None => conn.read_datagram().await,
        };
        let packet = match read {
            Ok(p) => p,
            Err(e) => {
                shared.why(format!("audio packets ended: {e}"));
                shared.end(AudioEvent::Stopped { by_peer: false });
                return;
            }
        };
        let Some((seq, sent_ms, pcm)) = decode_packet(&packet, codec, channels) else {
            shared.end_and_close(
                AudioEvent::Violation(format!("bad audio packet of {} bytes", packet.len())),
                b"bad audio packet",
            );
            return;
        };
        let c = &shared.counters;
        c.packets.fetch_add(1, Ordering::Relaxed);
        let now_ms = began.elapsed().as_millis() as u64;
        let age = ages.observe(now_ms, now_ms as i64 - i64::from(sent_ms));
        c.note_age(age);
        c.note_sent(sent_ms);
        packet_len = pcm.len();
        let outs = order.push(seq, pcm.to_vec());
        waiting_since = match (order.waiting(), waiting_since) {
            (true, Some(t)) => Some(t),
            (true, None) => Some(Instant::now()),
            (false, _) => None,
        };
        play(&shared, outs, packet_len, &format, &order);
    }
}

/// What a session's packets hold.
struct Format {
    codec: u32,
    sample_rate: u32,
    channels: u32,
}

/// Queue what the reorder step released, standing in for a short gap (silence
/// for PCM, empty frames for the Opus decoder to conceal), and publish the
/// counters.
fn play(
    shared: &Shared,
    outs: Vec<Reordered>,
    packet_len: usize,
    format: &Format,
    order: &Reorder,
) {
    for o in outs {
        match o {
            Reordered::Pcm(p) => {
                shared.queue.push(p);
            }
            Reordered::Gap(missing) if format.codec == AUDIO_CODEC_OPUS => {
                for _ in 0..opus_frames_to_conceal(missing) {
                    shared.queue.push(Vec::new());
                }
            }
            Reordered::Gap(missing) => {
                let fill = fill_bytes(missing, packet_len, format.sample_rate, format.channels);
                if fill > 0 {
                    shared.queue.push(vec![0; fill]);
                }
            }
        }
    }
    shared.counters.lost.store(order.lost, Ordering::Relaxed);
    shared.counters.late.store(order.late, Ordering::Relaxed);
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
        assert_eq!(
            decode_packet(&p, 0, 2),
            Some((7, 1234, &[1u8, 0, 2, 0][..]))
        );
    }

    #[test]
    fn malformed_packets_are_refused() {
        assert_eq!(decode_packet(&[], 0, 2), None);
        assert_eq!(decode_packet(&[0; 7], 0, 2), None, "short header");
        assert_eq!(decode_packet(&[0; 8], 0, 2), None, "no PCM");
        assert_eq!(
            decode_packet(&[0; 14], 0, 2),
            None,
            "not whole stereo samples"
        );
        assert!(decode_packet(&[0; 14], 0, 1).is_some());
        assert_eq!(
            decode_packet(&vec![0; PACKET_HEADER + 65_536 + 4], 0, 2),
            None,
            "over the frame limit"
        );
    }

    #[test]
    fn an_opus_packet_is_any_size_up_to_its_limit() {
        let p = encode_packet(1, 2, &[9, 8, 7]);
        assert_eq!(
            decode_packet(&p, AUDIO_CODEC_OPUS, 2),
            Some((1, 2, &[9u8, 8, 7][..])),
            "three bytes would not be whole PCM samples"
        );
        assert_eq!(decode_packet(&[0; 8], AUDIO_CODEC_OPUS, 2), None, "empty");
        assert_eq!(decode_packet(&[0; 8 + 1276], AUDIO_CODEC_OPUS, 2), None);
    }

    #[test]
    fn short_opus_gaps_are_concealed_and_stalls_are_not() {
        assert_eq!(opus_frames_to_conceal(0), 0);
        assert_eq!(opus_frames_to_conceal(2), 2, "40 ms");
        assert_eq!(opus_frames_to_conceal(3), 0, "a stall adds no delay");
        assert_eq!(opus_frames_to_conceal(u32::MAX), 0);
    }

    #[test]
    fn age_is_the_delay_above_the_best_the_path_did_lately() {
        let mut t = AgeTracker::default();
        assert_eq!(t.observe(0, 100), 0, "the first packet is the best so far");
        assert_eq!(t.observe(20, 130), 30);
        assert_eq!(t.observe(40, 90), 0, "a new best");
        assert_eq!(t.observe(60, 110), 20);
    }

    #[test]
    fn the_rolling_minimum_forgets_clock_drift() {
        // The phone's clock slips 1 ms a second against the PC's. Measured
        // against the lowest delay ever, every packet would look ever later.
        let mut t = AgeTracker::default();
        let mut worst = 0;
        for s in 0..600u64 {
            let age = t.observe(s * 1000, 50 + s as i64);
            if s > 60 {
                worst = worst.max(age);
            }
        }
        assert!(worst <= 30, "aged {worst} ms from drift alone");
    }

    #[test]
    fn a_late_packet_still_stands_out_from_the_rolling_minimum() {
        let mut t = AgeTracker::default();
        for s in 0..300u64 {
            t.observe(s * 20, 50);
        }
        assert_eq!(t.observe(300 * 20, 150), 100);
    }

    #[test]
    fn each_noted_age_lands_in_its_bin_and_the_worst_is_kept() {
        let c = RxCounters::default();
        c.note_age(30);
        c.note_age(90);
        c.note_age(20);
        assert_eq!(c.max_age_ms.load(Ordering::Relaxed), 90);
        assert_eq!(c.age_bins[age_bin(90)].load(Ordering::Relaxed), 1);
        assert_eq!(c.age_bins[age_bin(20)].load(Ordering::Relaxed), 1);
    }

    #[test]
    fn the_longest_wait_between_sends_is_noted_and_restarts_when_read() {
        let c = RxCounters::default();
        c.note_sent(100);
        c.note_sent(120);
        c.note_sent(300);
        c.note_sent(280); // swapped with its neighbour on the way
        c.note_sent(320);
        assert_eq!(c.take_max_sent_gap_ms(), 180);
        assert_eq!(c.take_max_sent_gap_ms(), 0);
    }

    #[test]
    fn each_age_falls_in_the_bin_for_its_range() {
        assert_eq!(age_bin(0), 0);
        assert_eq!(age_bin(20), 0);
        assert_eq!(age_bin(21), 1);
        assert_eq!(age_bin(100), 4);
        assert_eq!(age_bin(101), 5);
        assert_eq!(age_bin(300), AGE_BIN_LIMITS_MS.len() - 1);
        assert_eq!(
            age_bin(301),
            AGE_BIN_LIMITS_MS.len(),
            "the last bin is open"
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

    fn pcm(n: u8) -> Vec<u8> {
        vec![n; 4]
    }

    fn played(out: &[Reordered]) -> Vec<u8> {
        out.iter()
            .filter_map(|o| match o {
                Reordered::Pcm(p) => Some(p[0]),
                Reordered::Gap(_) => None,
            })
            .collect()
    }

    #[test]
    fn in_order_packets_play_at_once() {
        let mut r = Reorder::default();
        assert_eq!(played(&r.push(5, pcm(5))), [5], "the first sets the start");
        assert_eq!(played(&r.push(6, pcm(6))), [6]);
        assert!(!r.waiting());
        assert_eq!((r.lost, r.late), (0, 0));
    }

    #[test]
    fn swapped_neighbours_are_put_back_in_order() {
        let mut r = Reorder::default();
        r.push(1, pcm(1));
        assert!(r.push(3, pcm(3)).is_empty(), "3 waits for 2");
        assert!(r.waiting());
        assert_eq!(played(&r.push(2, pcm(2))), [2, 3]);
        assert!(!r.waiting());
        assert_eq!((r.lost, r.late), (0, 0), "nothing was lost");
    }

    #[test]
    fn a_missing_packet_is_given_up_after_the_wait() {
        let mut r = Reorder::default();
        r.push(1, pcm(1));
        r.push(4, pcm(4));
        r.push(5, pcm(5));
        let out = r.release();
        assert_eq!(out[0], Reordered::Gap(2));
        assert_eq!(played(&out), [4, 5]);
        assert_eq!(r.lost, 2);
        assert!(r.push(3, pcm(3)).is_empty(), "now late");
        assert_eq!(r.late, 1);
    }

    #[test]
    fn too_many_waiting_packets_give_up_at_once() {
        let mut r = Reorder::default();
        r.push(0, pcm(0));
        let mut out = Vec::new();
        for seq in 2..=10u32 {
            out = r.push(seq, pcm(seq as u8));
        }
        assert_eq!(out[0], Reordered::Gap(1));
        assert_eq!(played(&out).len(), 9);
        assert_eq!(r.lost, 1);
        assert!(!r.waiting());
    }

    #[test]
    fn duplicates_and_old_packets_are_dropped() {
        let mut r = Reorder::default();
        r.push(7, pcm(7));
        assert!(r.push(7, pcm(7)).is_empty());
        assert!(r.push(2, pcm(2)).is_empty());
        r.push(9, pcm(9));
        assert!(r.push(9, pcm(9)).is_empty(), "a held duplicate");
        assert_eq!(r.late, 3);
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
