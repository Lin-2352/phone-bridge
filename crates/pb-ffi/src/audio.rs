//! Audio sharing for the phone: the responder side of `AudioStart` (the PC
//! dials, see `pb_transport::audio`). Direction 0 (phone to PC) is consented
//! on the phone: the app is asked and answers with [`PbNode::answer_audio_accept`]
//! / [`PbNode::answer_audio_refuse`]. Direction 1 (PC to phone) was consented on
//! the PC, so the phone only checks limits and busy, then plays.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use pb_protocol::control::{
    AudioAccept, AudioReason, AudioStart, AUDIO_PC_TO_PHONE, AUDIO_PHONE_TO_PC,
};
use pb_transport::audio::{serve_audio, AudioEvent, AudioSession, AudioSlots};
use pb_transport::Session;
use tokio::sync::{mpsc, oneshot, Notify};

use crate::{fail, push, Events, PbError, PbEvent};

/// How long the user has to answer an audio request (the PC waits 40 s).
pub(crate) const ASK_TIMEOUT: Duration = Duration::from_secs(35);

/// Why an audio session ended (`PbEvent::AudioEnded`).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum PbAudioEnd {
    /// The PC pressed Stop (or closed its side on purpose).
    PeerStopped,
    /// This phone ended it (Stop in the app or notification, sharing app closed).
    LocalStop,
    /// The link dropped.
    LinkLost,
    /// The PC broke the audio rules (bad frame, bad stream); the session was closed.
    Violation { message: String },
    /// Writing to the PC failed.
    WriteFailed { message: String },
}

/// Receives the PCM of a PC to phone session. Called on a Rust thread, one
/// frame at a time, in order. The frame is a whole number of 16-bit samples.
#[uniffi::export(with_foreign)]
pub trait AudioSink: Send + Sync {
    fn on_frame(&self, frame: Vec<u8>) -> Result<(), PbError>;
}

/// The phone's side of a phone to PC session: Kotlin feeds PCM in here.
#[derive(uniffi::Object)]
pub struct PbAudioOut {
    session: OnceLock<Arc<AudioSession>>,
    ctl: Arc<Ctl>,
}

/// Stop request shared by the session task and its handles.
#[derive(Default)]
struct Ctl {
    notify: Notify,
    local: AtomicBool,
}

impl Ctl {
    fn stop(&self) {
        self.local.store(true, Ordering::SeqCst);
        self.notify.notify_one();
    }
}

#[uniffi::export]
impl PbAudioOut {
    /// Send one PCM frame (16-bit LE interleaved, exactly the bytes of one
    /// capture buffer: copy the buffer to its length before calling). Frames
    /// given before the PC has the stream are dropped. Fails once the session
    /// ended or when the frame breaks the format rules.
    pub fn send_frame(&self, frame: Vec<u8>) -> Result<(), PbError> {
        match self.session.get() {
            Some(s) => s.send_frame(frame).map_err(fail),
            None => Ok(()),
        }
    }

    /// End the session (the PC is told). Idempotent.
    pub fn stop(&self) {
        self.ctl.stop();
    }

    /// One log line about the sending side (counts only).
    pub fn stats_line(&self) -> String {
        match self.session.get() {
            Some(s) => {
                let n = s.stats();
                format!(
                    "packets={} quic_lost={} paths={} rtt_ms={}",
                    n.packets, n.quic_lost, n.paths, n.rtt_ms
                )
            }
            None => "no session yet".into(),
        }
    }
}

/// The user's answer to an audio request.
enum Answer {
    Accept {
        sample_rate: u32,
        channels: u32,
        codec: u32,
        out: Arc<PbAudioOut>,
    },
    Refuse(AudioReason),
}

/// One direction's running state.
#[derive(Default)]
struct Dir {
    /// Being asked or running.
    busy: AtomicBool,
    ctl: Mutex<Option<Arc<Ctl>>>,
}

/// What the phone does with an `AudioStart` before anyone is asked.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Gate {
    Refuse(AudioReason),
    /// Phone audio goes to the PC: the phone's user decides.
    Ask,
    /// PC audio to the phone: the PC's user already decided.
    Accept,
}

/// Limits first, then the receiver being there, then busy. `valid`: the
/// request passed `AudioStart::validate`.
pub(crate) fn gate(direction: u32, valid: bool, has_sink: bool, busy: bool) -> Gate {
    if !valid {
        return Gate::Refuse(AudioReason::BadParams);
    }
    if direction == AUDIO_PC_TO_PHONE && !has_sink {
        return Gate::Refuse(AudioReason::Unavailable);
    }
    if busy {
        return Gate::Refuse(AudioReason::Busy);
    }
    if direction == AUDIO_PHONE_TO_PC {
        Gate::Ask
    } else {
        Gate::Accept
    }
}

/// How a session ended, from the transport's report and whether this phone
/// asked for the stop.
pub(crate) fn end_cause(ev: Option<AudioEvent>, local_stop: bool) -> PbAudioEnd {
    if local_stop {
        return PbAudioEnd::LocalStop;
    }
    match ev {
        Some(AudioEvent::Stopped { by_peer: true }) => PbAudioEnd::PeerStopped,
        Some(AudioEvent::Violation(message)) => PbAudioEnd::Violation { message },
        Some(AudioEvent::WriteFailed(message)) => PbAudioEnd::WriteFailed { message },
        Some(AudioEvent::Stopped { by_peer: false }) | None => PbAudioEnd::LinkLost,
    }
}

/// Audio state shared by the node and the session pumps.
pub(crate) struct AudioHub {
    events: Events,
    slots: AudioSlots,
    sink: Mutex<Option<Arc<dyn AudioSink>>>,
    dirs: [Dir; 2],
    pending: Mutex<HashMap<u64, oneshot::Sender<Answer>>>,
    next_id: AtomicU64,
    /// The receiving session's last stats line (PC to phone).
    rx_line: Mutex<String>,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

fn take_events(s: &mut AudioSession) -> mpsc::UnboundedReceiver<AudioEvent> {
    std::mem::replace(&mut s.events, mpsc::unbounded_channel().1)
}

impl AudioHub {
    pub(crate) fn new(events: Events, slots: AudioSlots) -> Self {
        Self {
            events,
            slots,
            sink: Mutex::new(None),
            dirs: [Dir::default(), Dir::default()],
            pending: Mutex::default(),
            next_id: AtomicU64::new(1),
            rx_line: Mutex::default(),
        }
    }

    pub(crate) fn rx_stats_line(&self) -> String {
        lock(&self.rx_line).clone()
    }

    pub(crate) fn set_sink(&self, sink: Option<Arc<dyn AudioSink>>) {
        *lock(&self.sink) = sink;
    }

    pub(crate) fn active(&self, direction: u8) -> bool {
        self.dirs
            .get(direction as usize)
            .is_some_and(|d| d.busy.load(Ordering::SeqCst))
    }

    /// Stop the session (or the question) of `direction`. False: none.
    pub(crate) fn stop(&self, direction: u8) -> bool {
        let Some(d) = self.dirs.get(direction as usize) else {
            return false;
        };
        match lock(&d.ctl).as_ref() {
            Some(c) => {
                c.stop();
                true
            }
            None => false,
        }
    }

    pub(crate) fn accept(
        &self,
        id: u64,
        sample_rate: u32,
        channels: u32,
        codec: u32,
    ) -> Option<Arc<PbAudioOut>> {
        let tx = lock(&self.pending).remove(&id)?;
        let out = Arc::new(PbAudioOut {
            session: OnceLock::new(),
            ctl: Arc::default(),
        });
        tx.send(Answer::Accept {
            sample_rate,
            channels,
            codec,
            out: out.clone(),
        })
        .ok()?;
        Some(out)
    }

    pub(crate) fn refuse(&self, id: u64, reason: u8) {
        if let Some(tx) = lock(&self.pending).remove(&id) {
            let _ = tx.send(Answer::Refuse(AudioReason::from_code(reason.into())));
        }
    }

    fn release(&self, direction: u32) {
        let d = &self.dirs[direction as usize];
        *lock(&d.ctl) = None;
        d.busy.store(false, Ordering::SeqCst);
    }

    /// A PC sent `AudioStart`: gate, ask if needed, serve, run to the end.
    pub(crate) async fn serve(&self, s: Session, start: AudioStart, name: String) -> String {
        let pid = s.peer.to_string();
        let direction = start.direction;
        let valid = start.validate().is_ok();
        let has_sink = lock(&self.sink).is_some();
        let was_busy = valid
            && self.dirs[direction as usize]
                .busy
                .swap(true, Ordering::SeqCst);
        let g = gate(direction, valid, has_sink, was_busy);
        if let Gate::Refuse(reason) = g {
            if valid && !was_busy {
                self.release(direction);
            }
            push(
                &self.events,
                PbEvent::AudioRefused {
                    peer_name: name,
                    peer_id: pid.clone(),
                    direction: direction.min(255) as u8,
                    source: start.source.min(255) as u8,
                    reason: reason.code() as u8,
                },
            );
            let _ = serve_audio(s, start, Err(reason), &self.slots).await;
            return format!("audio refused: {reason:?}");
        }
        let ctl: Arc<Ctl>;
        let mut out: Option<Arc<PbAudioOut>> = None;
        let accept = if g == Gate::Ask {
            let id = self.next_id.fetch_add(1, Ordering::Relaxed);
            let (tx, rx) = oneshot::channel();
            lock(&self.pending).insert(id, tx);
            // Stop works while the user is still being asked.
            let ask_ctl: Arc<Ctl> = Arc::default();
            *lock(&self.dirs[direction as usize].ctl) = Some(ask_ctl.clone());
            push(
                &self.events,
                PbEvent::AudioRequest {
                    id,
                    peer_name: name.clone(),
                    peer_id: pid.clone(),
                    source: start.source as u8,
                    sample_rate: start.sample_rate,
                    channels: start.channels,
                },
            );
            let answer = tokio::select! {
                r = tokio::time::timeout(ASK_TIMEOUT, rx) => r.ok().and_then(|r| r.ok()),
                () = s.closed() => None,
                () = ask_ctl.notify.notified() => None,
            };
            lock(&self.pending).remove(&id);
            match answer {
                Some(Answer::Accept {
                    sample_rate,
                    channels,
                    codec,
                    out: o,
                }) => {
                    ctl = o.ctl.clone();
                    *lock(&self.dirs[direction as usize].ctl) = Some(ctl.clone());
                    out = Some(o);
                    Ok(AudioAccept {
                        id: 0,
                        sample_rate,
                        channels,
                        accepted: true,
                        reason: 0,
                        codec,
                    })
                }
                other => {
                    let reason = match other {
                        Some(Answer::Refuse(r)) => r,
                        _ => AudioReason::Declined,
                    };
                    push(&self.events, PbEvent::AudioAskEnded { id });
                    self.release(direction);
                    let _ = serve_audio(s, start, Err(reason), &self.slots).await;
                    return format!("audio not accepted: {reason:?}");
                }
            }
        } else {
            ctl = Arc::default();
            *lock(&self.dirs[direction as usize].ctl) = Some(ctl.clone());
            Ok(AudioAccept {
                id: 0,
                sample_rate: start.sample_rate,
                channels: start.channels,
                accepted: true,
                reason: 0,
                codec: pb_protocol::control::AUDIO_CODEC_PCM,
            })
        };
        let source = start.source as u8;
        let mut session = match serve_audio(s, start, accept, &self.slots).await {
            Ok(a) => a,
            Err(e) => {
                push(
                    &self.events,
                    PbEvent::AudioEnded {
                        direction: direction as u8,
                        cause: end_cause(None, ctl.local.load(Ordering::SeqCst)),
                    },
                );
                self.release(direction);
                return e.to_string();
            }
        };
        push(
            &self.events,
            PbEvent::AudioStarted {
                peer_name: name,
                peer_id: pid.clone(),
                direction: direction as u8,
                source,
                sample_rate: session.accept.sample_rate,
                channels: session.accept.channels,
            },
        );
        let mut events = take_events(&mut session);
        let cause = if let Some(o) = out {
            let session = Arc::new(session);
            let _ = o.session.set(session.clone());
            let ev = tokio::select! {
                ev = events.recv() => ev,
                () = ctl.notify.notified() => {
                    session.stop();
                    drain(&mut events).await
                }
            };
            end_cause(ev, ctl.local.load(Ordering::SeqCst))
        } else {
            let ev;
            let mut seen = 0u32;
            loop {
                tokio::select! {
                    f = session.recv_frame() => match f {
                        Some(f) => {
                            seen += 1;
                            if seen.is_multiple_of(500) {
                                let n = session.stats();
                                *lock(&self.rx_line) = format!(
                                    "packets={} lost={} late={} max_age_ms={} quic_lost={} paths={} rtt_ms={}",
                                    n.packets, n.lost, n.late, n.max_age_ms, n.quic_lost, n.paths, n.rtt_ms
                                );
                            }
                            let sink = lock(&self.sink).clone();
                            if let Some(sink) = sink {
                                let _ = sink.on_frame(f.samples);
                            }
                        }
                        None => {
                            ev = drain(&mut events).await;
                            break;
                        }
                    },
                    () = ctl.notify.notified() => {
                        session.stop();
                        ev = drain(&mut events).await;
                        break;
                    }
                }
            }
            end_cause(ev, ctl.local.load(Ordering::SeqCst))
        };
        // The app hears the end before another request can start.
        push(
            &self.events,
            PbEvent::AudioEnded {
                direction: direction as u8,
                cause,
            },
        );
        self.release(direction);
        "audio ended".into()
    }
}

/// The session's end reason, which the transport reports within moments.
async fn drain(events: &mut mpsc::UnboundedReceiver<AudioEvent>) -> Option<AudioEvent> {
    tokio::time::timeout(Duration::from_secs(2), events.recv())
        .await
        .ok()
        .flatten()
}

#[cfg(test)]
mod tests {
    use super::*;
    use pb_identity::{Identity, MemoryPeerStore, PairedPeer};
    use pb_protocol::control::{ControlMsg, AUDIO_SOURCE_MIC};
    use pb_transport::{Node, NodeConfig, SharedStore};

    #[test]
    fn bad_limits_are_refused_before_anything_else() {
        assert_eq!(
            gate(0, false, false, true),
            Gate::Refuse(AudioReason::BadParams)
        );
        assert_eq!(
            gate(1, false, true, false),
            Gate::Refuse(AudioReason::BadParams)
        );
    }

    #[test]
    fn pc_audio_needs_a_player_and_is_never_asked_on_the_phone() {
        assert_eq!(
            gate(AUDIO_PC_TO_PHONE, true, false, false),
            Gate::Refuse(AudioReason::Unavailable)
        );
        assert_eq!(gate(AUDIO_PC_TO_PHONE, true, true, false), Gate::Accept);
    }

    #[test]
    fn phone_audio_always_asks_the_phone_user() {
        assert_eq!(gate(AUDIO_PHONE_TO_PC, true, false, false), Gate::Ask);
        assert_eq!(gate(AUDIO_PHONE_TO_PC, true, true, false), Gate::Ask);
    }

    #[test]
    fn a_busy_direction_is_refused_busy_in_both_directions() {
        assert_eq!(
            gate(AUDIO_PHONE_TO_PC, true, true, true),
            Gate::Refuse(AudioReason::Busy)
        );
        assert_eq!(
            gate(AUDIO_PC_TO_PHONE, true, true, true),
            Gate::Refuse(AudioReason::Busy)
        );
    }

    #[test]
    fn end_causes_follow_who_stopped() {
        let c = |ev, local| end_cause(ev, local);
        assert_eq!(
            c(Some(AudioEvent::Stopped { by_peer: true }), false),
            PbAudioEnd::PeerStopped
        );
        assert_eq!(
            c(Some(AudioEvent::Stopped { by_peer: false }), false),
            PbAudioEnd::LinkLost
        );
        assert_eq!(c(None, false), PbAudioEnd::LinkLost);
        assert_eq!(
            c(Some(AudioEvent::Stopped { by_peer: false }), true),
            PbAudioEnd::LocalStop
        );
        assert_eq!(c(None, true), PbAudioEnd::LocalStop);
        assert_eq!(
            c(Some(AudioEvent::Violation("x".into())), false),
            PbAudioEnd::Violation {
                message: "x".into()
            }
        );
        assert_eq!(
            c(Some(AudioEvent::WriteFailed("w".into())), false),
            PbAudioEnd::WriteFailed {
                message: "w".into()
            }
        );
    }

    // ---- over a real node pair -------------------------------------------

    #[derive(Default)]
    struct Rec(Mutex<Vec<String>>, Mutex<Vec<PbEvent>>);
    impl crate::EventListener for Rec {
        fn on_event(&self, e: PbEvent) -> Result<(), PbError> {
            let tag = match &e {
                PbEvent::AudioRequest { .. } => "request",
                PbEvent::AudioStarted { .. } => "started",
                PbEvent::AudioEnded { .. } => "ended",
                PbEvent::AudioRefused { .. } => "refused",
                PbEvent::AudioAskEnded { .. } => "askended",
                _ => return Ok(()),
            };
            lock(&self.0).push(tag.into());
            lock(&self.1).push(e);
            Ok(())
        }
    }
    impl Rec {
        async fn wait_for(&self, tag: &str) -> PbEvent {
            for _ in 0..400 {
                {
                    let tags = lock(&self.0);
                    if let Some(i) = tags.iter().position(|t| t == tag) {
                        let evs = lock(&self.1);
                        return clone_event(&evs[i]);
                    }
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
            panic!("no {tag} event, got {:?}", lock(&self.0));
        }
    }

    fn clone_event(e: &PbEvent) -> PbEvent {
        match e {
            PbEvent::AudioRequest {
                id,
                peer_name,
                peer_id,
                source,
                sample_rate,
                channels,
            } => PbEvent::AudioRequest {
                id: *id,
                peer_name: peer_name.clone(),
                peer_id: peer_id.clone(),
                source: *source,
                sample_rate: *sample_rate,
                channels: *channels,
            },
            PbEvent::AudioStarted {
                peer_name,
                peer_id,
                direction,
                source,
                sample_rate,
                channels,
            } => PbEvent::AudioStarted {
                peer_name: peer_name.clone(),
                peer_id: peer_id.clone(),
                direction: *direction,
                source: *source,
                sample_rate: *sample_rate,
                channels: *channels,
            },
            PbEvent::AudioEnded { direction, cause } => PbEvent::AudioEnded {
                direction: *direction,
                cause: cause.clone(),
            },
            PbEvent::AudioRefused {
                peer_name,
                peer_id,
                direction,
                source,
                reason,
            } => PbEvent::AudioRefused {
                peer_name: peer_name.clone(),
                peer_id: peer_id.clone(),
                direction: *direction,
                source: *source,
                reason: *reason,
            },
            PbEvent::AudioAskEnded { id } => PbEvent::AudioAskEnded { id: *id },
            _ => unreachable!(),
        }
    }

    #[derive(Default)]
    struct Frames(Mutex<Vec<Vec<u8>>>);
    impl AudioSink for Frames {
        fn on_frame(&self, f: Vec<u8>) -> Result<(), PbError> {
            lock(&self.0).push(f);
            Ok(())
        }
    }

    struct Rig {
        pc: Node,
        phone: Node,
        hub: Arc<AudioHub>,
        rec: Arc<Rec>,
        sink: Arc<Frames>,
        _pc_ev: pb_transport::NodeEvents,
        sessions: Arc<tokio::sync::Mutex<mpsc::Receiver<Session>>>,
    }

    async fn bind(seed: u8, name: &str) -> (Node, pb_transport::NodeEvents, SharedStore) {
        let store: SharedStore = Arc::new(Mutex::new(MemoryPeerStore::default()));
        let config = NodeConfig {
            device_name: name.into(),
            port: None,
            discovery_port: None,
            tuning: Default::default(),
        };
        let (n, ev) = Node::bind(&Identity::from_seed([seed; 32]), store.clone(), config)
            .await
            .unwrap();
        (n, ev, store)
    }

    async fn rig() -> Rig {
        let (pc, mut pc_ev, pc_store) = bind(41, "PC").await;
        let (phone, phone_ev, _) = bind(42, "Phone").await;
        let code = pc
            .open_pairing(vec![format!("127.0.0.1:{}", pc.port())])
            .unwrap();
        let joining = phone.join(&code).await.unwrap();
        let pending = pc_ev.pairings.recv().await.unwrap();
        let (h, j) = tokio::join!(pending.decide(true), joining.decide(true));
        assert!(h.is_ok() && j.is_ok());
        // The PC dials the phone over loopback (LAN addresses may be firewalled).
        let phone_id = phone.peer_id();
        let mut p: PairedPeer = pc_store.lock().unwrap().get(&phone_id).unwrap();
        p.addrs = vec![format!("127.0.0.1:{}", phone.port())];
        pc_store.lock().unwrap().pin(p);
        let rec = Arc::new(Rec::default());
        let hub = Arc::new(AudioHub::new(rec.clone(), phone.audio_slots().clone()));
        let sink = Arc::new(Frames::default());
        Rig {
            pc,
            phone,
            hub,
            rec,
            sink,
            _pc_ev: pc_ev,
            sessions: Arc::new(tokio::sync::Mutex::new(phone_ev.sessions)),
        }
    }

    fn start(direction: u32, source: u32) -> AudioStart {
        AudioStart {
            id: 0,
            sample_rate: 48000,
            channels: 2,
            direction,
            source,
        }
    }

    impl Rig {
        /// The phone pumps the next incoming `AudioStart` into the hub.
        fn phone_pump(&mut self) -> tokio::task::JoinHandle<String> {
            let hub = self.hub.clone();
            let sessions = self.sessions.clone();
            tokio::spawn(async move {
                let mut s = sessions.lock().await.recv().await.unwrap();
                loop {
                    match s.recv().await {
                        Ok(Some(ControlMsg::AudioStart(st))) => {
                            return hub.serve(s, st, "PC".into()).await
                        }
                        Ok(Some(_)) => continue,
                        _ => return "closed".into(),
                    }
                }
            })
        }
    }

    /// Fits one datagram, so it arrives as it was sent.
    fn frame(i: u8) -> Vec<u8> {
        vec![i; 960]
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn phone_audio_asks_then_streams_to_the_pc_and_the_phone_can_stop_it() {
        let mut r = rig().await;
        let pump = r.phone_pump();
        let pcs = r.phone.peer_id();
        let pc = r.pc.clone();
        let req = tokio::spawn(async move { pc.open_audio(&pcs, start(0, 0), || {}).await });
        let PbEvent::AudioRequest {
            id,
            peer_id,
            source,
            sample_rate,
            channels,
            ..
        } = r.rec.wait_for("request").await
        else {
            unreachable!()
        };
        // The phone's event names the PC by id, not only by name.
        assert_eq!(peer_id, r.pc.peer_id().to_string());
        assert_eq!((source, sample_rate, channels), (0, 48000, 2));
        let out = r.hub.accept(id, 48000, 2, 0).expect("accepted");
        let mut pc_session = req.await.unwrap().expect("pc side");
        assert!(matches!(
            r.rec.wait_for("started").await,
            PbEvent::AudioStarted {
                direction: 0,
                source: 0,
                ..
            }
        ));
        assert!(r.hub.active(0) && !r.hub.active(1));
        for i in 0..3 {
            out.send_frame(frame(i)).unwrap();
        }
        for i in 0..3 {
            let f = pc_session.recv_frame().await.unwrap();
            assert_eq!(f.samples[0], i);
        }
        assert!(
            out.send_frame(vec![0; 3]).is_err(),
            "misaligned frame is rejected"
        );
        assert!(r.hub.stop(0));
        let PbEvent::AudioEnded { direction, cause } = r.rec.wait_for("ended").await else {
            unreachable!()
        };
        assert_eq!((direction, cause), (0, PbAudioEnd::LocalStop));
        assert_eq!(pump.await.unwrap(), "audio ended");
        assert!(!r.hub.active(0));
        assert!(out.send_frame(frame(9)).is_err(), "closed after the end");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn microphone_request_carries_its_source_and_a_refusal_reaches_the_pc() {
        let mut r = rig().await;
        let pump = r.phone_pump();
        let pcs = r.phone.peer_id();
        let pc = r.pc.clone();
        let req =
            tokio::spawn(
                async move { pc.open_audio(&pcs, start(0, AUDIO_SOURCE_MIC), || {}).await },
            );
        let PbEvent::AudioRequest { id, source, .. } = r.rec.wait_for("request").await else {
            unreachable!()
        };
        assert_eq!(source, 1);
        r.hub.refuse(id, AudioReason::NoPermission.code() as u8);
        let err = req.await.unwrap().err().expect("refused");
        assert!(
            matches!(
                err,
                pb_transport::Error::AudioRefused(AudioReason::NoPermission)
            ),
            "{err:?}"
        );
        pump.await.unwrap();
        assert!(!r.hub.active(0));
        assert!(matches!(
            r.rec.wait_for("askended").await,
            PbEvent::AudioAskEnded { .. }
        ));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn pc_audio_plays_without_asking_and_the_pc_can_stop_it() {
        let mut r = rig().await;
        r.hub.set_sink(Some(r.sink.clone()));
        let pump = r.phone_pump();
        let pcs = r.phone.peer_id();
        let pc = r.pc.clone();
        let session = pc.open_audio(&pcs, start(1, 0), || {}).await.unwrap();
        assert!(matches!(
            r.rec.wait_for("started").await,
            PbEvent::AudioStarted {
                direction: 1,
                sample_rate: 48000,
                ..
            }
        ));
        assert!(
            !lock(&r.rec.0).contains(&"request".to_string()),
            "no ask on the phone for PC audio"
        );
        for i in 0..3 {
            session.send_frame(frame(i)).unwrap();
        }
        for _ in 0..200 {
            if lock(&r.sink.0).len() >= 3 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        let got: Vec<u8> = lock(&r.sink.0).iter().map(|f| f[0]).collect();
        assert_eq!(got, vec![0, 1, 2]);
        session.stop();
        let PbEvent::AudioEnded { direction, cause } = r.rec.wait_for("ended").await else {
            unreachable!()
        };
        assert_eq!((direction, cause), (1, PbAudioEnd::PeerStopped));
        pump.await.unwrap();
        assert!(!r.hub.active(1));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn the_phone_can_stop_pc_audio_and_a_link_drop_ends_it() {
        let mut r = rig().await;
        r.hub.set_sink(Some(r.sink.clone()));
        let pump = r.phone_pump();
        let pcs = r.phone.peer_id();
        let session = r.pc.open_audio(&pcs, start(1, 0), || {}).await.unwrap();
        r.rec.wait_for("started").await;
        assert!(r.hub.stop(1));
        let PbEvent::AudioEnded { cause, .. } = r.rec.wait_for("ended").await else {
            unreachable!()
        };
        assert_eq!(cause, PbAudioEnd::LocalStop);
        pump.await.unwrap();
        drop(session);

        // A second session; the PC vanishes without a Stop.
        let mut r2 = rig().await;
        r2.hub.set_sink(Some(r2.sink.clone()));
        let pump = r2.phone_pump();
        let pcs = r2.phone.peer_id();
        let session = r2.pc.open_audio(&pcs, start(1, 0), || {}).await.unwrap();
        r2.rec.wait_for("started").await;
        session.close();
        let PbEvent::AudioEnded { cause, .. } = r2.rec.wait_for("ended").await else {
            unreachable!()
        };
        assert!(
            matches!(cause, PbAudioEnd::LinkLost | PbAudioEnd::PeerStopped),
            "{cause:?}"
        );
        pump.await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_request_in_a_busy_direction_is_refused_busy_and_keeps_the_running_one() {
        let mut r = rig().await;
        r.hub.set_sink(Some(r.sink.clone()));
        let pump = r.phone_pump();
        let pcs = r.phone.peer_id();
        let first = r.pc.open_audio(&pcs, start(1, 0), || {}).await.unwrap();
        r.rec.wait_for("started").await;
        // A second request that this PC node does not know to be a duplicate
        // (its own slots are separate): the phone refuses it by itself.
        let pump2 = r.phone_pump();
        let s = r.pc.connect(&pcs).await.unwrap();
        let err = pb_transport::audio::request_audio(s, start(1, 0), &AudioSlots::default(), || {})
            .await
            .err()
            .expect("busy");
        assert!(
            matches!(err, pb_transport::Error::AudioRefused(AudioReason::Busy)),
            "{err:?}"
        );
        pump2.await.unwrap();
        assert!(matches!(
            r.rec.wait_for("refused").await,
            PbEvent::AudioRefused { reason: 3, .. }
        ));
        assert!(r.hub.active(1), "the running session is untouched");
        first.stop();
        pump.await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn pc_audio_without_a_player_is_unavailable_and_bad_limits_are_refused() {
        let mut r = rig().await;
        let pump = r.phone_pump();
        let pcs = r.phone.peer_id();
        let err =
            r.pc.open_audio(&pcs, start(1, 0), || {})
                .await
                .err()
                .expect("refused");
        assert!(
            matches!(
                err,
                pb_transport::Error::AudioRefused(AudioReason::Unavailable)
            ),
            "{err:?}"
        );
        pump.await.unwrap();
        let PbEvent::AudioRefused { reason, .. } = r.rec.wait_for("refused").await else {
            unreachable!()
        };
        assert_eq!(reason, AudioReason::Unavailable.code() as u8);
        assert!(!r.hub.active(1));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn unanswered_ask_ends_when_the_pc_gives_up_or_stop_is_called() {
        let mut r = rig().await;
        let pump = r.phone_pump();
        let pcs = r.phone.peer_id();
        let pc = r.pc.clone();
        let req = tokio::spawn(async move { pc.open_audio(&pcs, start(0, 0), || {}).await });
        r.rec.wait_for("request").await;
        assert!(r.hub.active(0));
        assert!(r.hub.stop(0), "stop works while the user is being asked");
        pump.await.unwrap();
        assert!(req.await.unwrap().is_err());
        assert!(!r.hub.active(0));
        assert!(matches!(
            r.rec.wait_for("askended").await,
            PbEvent::AudioAskEnded { .. }
        ));
    }
}
