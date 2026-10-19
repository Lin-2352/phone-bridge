//! Shared mouse and keyboard: one device drives the other's pointer and keys.
//!
//! The controller opens a session, sends `InputStart`, and on acceptance
//! streams fixed 8-byte [`InputDatagram`] records on one unidirectional
//! stream. A reliable stream (not datagrams) keeps clicks and key releases
//! from ever being lost; on a local link its latency is the same. Pointer
//! moves queued faster than the link drains are merged into one.

use std::time::Duration;

use pb_protocol::{
    control::{ControlMsg, InputStart, OfferReply},
    InputDatagram, InputEvent, INPUT_DATAGRAM_LEN,
};
use tokio::sync::mpsc::{self, UnboundedSender};

use crate::{files::new_id, tx, Error, Node, PeerId, Session};

/// Handle for sending input; cheap to call from UI threads. Dropping it (or
/// calling [`InputSender::stop`]) ends the session.
pub struct InputSender {
    tx: UnboundedSender<InputEvent>,
    done: tokio::task::JoinHandle<()>,
}

impl InputSender {
    /// Queue an event; never blocks. Returns false once the session ended.
    pub fn send(&self, ev: InputEvent) -> bool {
        self.tx.send(ev).is_ok()
    }

    pub fn is_open(&self) -> bool {
        !self.tx.is_closed() && !self.done.is_finished()
    }

    pub fn stop(self) {
        drop(self.tx);
    }
}

impl Node {
    /// Ask `peer` to accept our input; on success events flow until the
    /// returned sender is dropped or the peer goes away.
    pub async fn open_input(&self, peer: &PeerId) -> Result<InputSender, Error> {
        let mut s = self.connect(peer).await?;
        let id = new_id();
        s.send(ControlMsg::InputStart(InputStart { id })).await?;
        if !crate::files::await_reply(&mut s, id).await? {
            s.conn.close(0u32.into(), b"declined");
            return Err(Error::Declined);
        }
        let mut out = s.conn.open_uni().await.map_err(tx)?;
        // A stream exists for the peer only once data is on it: send a no-op
        // move now so it does not time out waiting while the user is idle.
        let hello = InputDatagram {
            seq: 0,
            event: InputEvent::MouseMove { dx: 0, dy: 0 },
        };
        out.write_all(&hello.encode()).await.map_err(tx)?;
        let (tx, mut rx) = mpsc::unbounded_channel::<InputEvent>();
        let done = tokio::spawn(async move {
            let mut seq: u16 = 0;
            let mut buf = Vec::with_capacity(64 * INPUT_DATAGRAM_LEN);
            let mut push = |e: InputEvent, buf: &mut Vec<u8>| {
                seq = seq.wrapping_add(1);
                buf.extend_from_slice(&InputDatagram { seq, event: e }.encode());
            };
            // Stay inside the receiver's limits however fast the source is
            // (gaming mice report 8000 times a second, a paste is thousands
            // of keys): extra moves merge, other events wait their turn.
            let mut moves = Bucket::new(SEND_MOVES_PER_SEC, BURST);
            let mut others = Bucket::new(SEND_OTHER_PER_SEC, BURST);
            let mut pending: Option<(i32, i32)> = None;
            let mut open = true;
            'run: while open || pending.is_some() {
                let mut batch = Vec::new();
                let first = if pending.is_some() {
                    // A merged move is held back: wake when it may go.
                    tokio::select! {
                        e = rx.recv(), if open => Some(e),
                        () = tokio::time::sleep(moves.wait()) => None,
                    }
                } else {
                    Some(rx.recv().await)
                };
                match first {
                    Some(Some(e)) => {
                        batch.push(e);
                        while let Ok(e) = rx.try_recv() {
                            batch.push(e);
                        }
                    }
                    Some(None) => open = false,
                    None => {}
                }
                buf.clear();
                for e in batch {
                    match e {
                        InputEvent::MouseMove { dx, dy } => {
                            let (x, y) = pending.unwrap_or((0, 0));
                            pending = Some((x + i32::from(dx), y + i32::from(dy)));
                        }
                        other => {
                            // Order matters: the movement before a click goes first.
                            if let Some((x, y)) = pending.take() {
                                moves.take_anyway();
                                push_move(x, y, &mut push, &mut buf);
                            }
                            while !others.try_take() {
                                if out.write_all(&buf).await.is_err() {
                                    break 'run;
                                }
                                buf.clear();
                                tokio::time::sleep(others.wait()).await;
                            }
                            push(other, &mut buf);
                        }
                    }
                }
                if pending.is_some() && (!open || moves.try_take()) {
                    let (x, y) = pending.take().unwrap_or_default();
                    push_move(x, y, &mut push, &mut buf);
                }
                if !buf.is_empty() && out.write_all(&buf).await.is_err() {
                    break;
                }
            }
            let _ = out.finish();
            crate::files::await_peer_close(&mut s).await;
        });
        Ok(InputSender { tx, done })
    }
}

/// A merged move can exceed i16; split it.
fn push_move(
    mut x: i32,
    mut y: i32,
    push: &mut impl FnMut(InputEvent, &mut Vec<u8>),
    buf: &mut Vec<u8>,
) {
    loop {
        let dx = x.clamp(i16::MIN.into(), i16::MAX.into());
        let dy = y.clamp(i16::MIN.into(), i16::MAX.into());
        push(
            InputEvent::MouseMove {
                dx: dx as i16,
                dy: dy as i16,
            },
            buf,
        );
        x -= dx;
        y -= dy;
        if x == 0 && y == 0 {
            break;
        }
    }
}

/// Sender pacing; with the burst, any one second stays under the receiver's
/// limits below (800 + 100 moves, 250 + 100 other events).
const SEND_MOVES_PER_SEC: f64 = 800.0;
const SEND_OTHER_PER_SEC: f64 = 250.0;
const BURST: f64 = 100.0;

/// Token bucket: `rate` events per second, up to `burst` at once.
struct Bucket {
    tokens: f64,
    rate: f64,
    burst: f64,
    at: tokio::time::Instant,
}

impl Bucket {
    fn new(rate: f64, burst: f64) -> Self {
        Self {
            tokens: burst,
            rate,
            burst,
            at: tokio::time::Instant::now(),
        }
    }

    fn refill(&mut self) {
        let now = tokio::time::Instant::now();
        let gained = now.duration_since(self.at).as_secs_f64() * self.rate;
        self.tokens = (self.tokens + gained).min(self.burst);
        self.at = now;
    }

    fn try_take(&mut self) -> bool {
        self.refill();
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false
        }
    }

    /// Spend a token even if there is none (repaid by waiting later).
    fn take_anyway(&mut self) {
        self.refill();
        self.tokens -= 1.0;
    }

    /// How long until a token is available.
    fn wait(&self) -> Duration {
        Duration::from_secs_f64(((1.0 - self.tokens).max(0.0) / self.rate).max(0.001))
    }
}

const MAX_MOVES_PER_SEC: u32 = 1000;
const MAX_OTHER_PER_SEC: u32 = 400;
const MAX_RECORDS_PER_SEC: u32 = 2000;

/// Receiver side, called when `InputStart` arrives on a session. Answers,
/// then feeds every event to `on_event` until the controller stops.
pub async fn receive_input(
    session: &mut Session,
    start: InputStart,
    accept: bool,
    mut on_event: impl FnMut(InputEvent),
) -> Result<(), Error> {
    session
        .send(ControlMsg::OfferReply(OfferReply {
            id: start.id,
            accept,
            too_large: false,
        }))
        .await?;
    if !accept {
        crate::files::wait_for_close(session).await;
        return Err(Error::Declined);
    }
    let mut stream = tokio::time::timeout(Duration::from_secs(10), session.conn.accept_uni())
        .await
        .map_err(|_| Error::Io("controller did not open the input stream".into()))?
        .map_err(tx)?;
    let mut rec = [0u8; INPUT_DATAGRAM_LEN];
    // A real mouse and keyboard produce at most a few hundred events per
    // second. Beyond the budget, moves are dropped; a peer flooding clicks or
    // keys far past any human rate ends the session.
    let mut window = std::time::Instant::now();
    let (mut moves, mut others, mut total) = (0u32, 0u32, 0u32);
    while stream.read_exact(&mut rec).await.is_ok() {
        if window.elapsed() >= Duration::from_secs(1) {
            window = std::time::Instant::now();
            moves = 0;
            others = 0;
            total = 0;
        }
        // Every record counts, even ones we drop or cannot decode.
        total += 1;
        if total > MAX_RECORDS_PER_SEC {
            session.conn.close(1u32.into(), b"input flood");
            return Err(Error::Io("input flood".into()));
        }
        let Ok(d) = InputDatagram::decode(&rec) else {
            continue;
        };
        if let InputEvent::MouseMove { dx, dy } = d.event {
            if dx == 0 && dy == 0 {
                // The controller's opening no-op.
                continue;
            }
            moves += 1;
            if moves > MAX_MOVES_PER_SEC {
                continue;
            }
        } else {
            others += 1;
            if others > MAX_OTHER_PER_SEC {
                session.conn.close(1u32.into(), b"input flood");
                return Err(Error::Io("input flood".into()));
            }
        }
        on_event(d.event);
    }
    // Stream ended: we read last, so we close.
    session.conn.close(0u32.into(), b"input done");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn big_merged_moves_are_split_into_i16_steps() {
        let mut out = Vec::new();
        let mut events = Vec::new();
        let mut push = |e: InputEvent, _: &mut Vec<u8>| events.push(e);
        push_move(40_000, -5, &mut push, &mut out);
        assert_eq!(
            events,
            vec![
                InputEvent::MouseMove {
                    dx: i16::MAX,
                    dy: -5
                },
                InputEvent::MouseMove {
                    dx: (40_000 - i32::from(i16::MAX)) as i16,
                    dy: 0
                },
            ]
        );
    }
}
