//! Audio sharing on the PC: dials the phone, plays its audio on the speakers
//! or captures this PC's audio for the phone, and ends cleanly on Stop, a
//! peer stop, a link drop or a device failure. The decisions and every
//! sentence are in `audio_state` (pure, tested); this file wires them to the
//! transport and the audio devices.
//!
//! The PC always dials. The microphone is never involved on this side.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use pb_identity::PeerId;
use pb_platform_win::{
    audio_capture::AudioCapture,
    audio_out::{AudioOut, PlayoutTarget, TARGET_WINDOW_S},
    opus::OpusDec,
};
use pb_protocol::control::{AudioStart, AUDIO_SOURCE_PLAYBACK};
use pb_transport::{audio::AudioEvent, Node};
use tokio::sync::{mpsc, oneshot, Notify};

use crate::audio_state::{self as st, Dir, End, Phase, Tracker};
use crate::core::{Log, OfferReply, SharedSettings, UiEvent};

type Ui = Arc<dyn Fn(UiEvent) + Send + Sync>;

/// How long the PC's own question waits before it counts as "no".
const CONFIRM_TIMEOUT: Duration = Duration::from_secs(60);

/// `--test-audio-synth`: no device on this PC. Received audio is measured and
/// thrown away, and a test tone is sent instead of captured sound. Sends
/// nothing of the user's and plays nothing here.
pub static TEST_SYNTH: AtomicBool = AtomicBool::new(false);

fn synth() -> bool {
    TEST_SYNTH.load(Ordering::Relaxed)
}

static TRACK: Mutex<Tracker> = Mutex::new(Tracker::new());
/// The stop signal of each direction's running task.
static STOPS: Mutex<[Option<Arc<Notify>>; 2]> = Mutex::new([None, None]);

fn track<T>(f: impl FnOnce(&mut Tracker) -> T) -> T {
    f(&mut TRACK.lock().unwrap_or_else(|p| p.into_inner()))
}

/// Milliseconds on `CLOCK` when the last phone-audio frame arrived; 0 while no share runs.
static LAST_FRAME_MS: AtomicU64 = AtomicU64::new(0);
static CLOCK: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();

fn clock_ms() -> u64 {
    // +1 so a frame in the first millisecond is not mistaken for "none yet".
    CLOCK.get_or_init(Instant::now).elapsed().as_millis() as u64 + 1
}

/// Whether the running phone-audio share has gone quiet for `STALL_MS` (the flyout shows "Reconnecting…").
pub fn stalled() -> bool {
    let last = LAST_FRAME_MS.load(Ordering::Relaxed);
    last != 0 && st::reconnecting(clock_ms().saturating_sub(last))
}

/// What this direction is doing now (the menu, Home and the flyout read it).
pub fn phase(d: Dir) -> Phase {
    track(|t| t.phase(d))
}

/// Ask a direction to end: a question not yet answered is cancelled, a
/// running session is stopped (the phone is told). Nothing happens when idle.
pub fn stop(d: Dir) {
    if let Some(n) = STOPS.lock().unwrap_or_else(|p| p.into_inner())[d.index()].as_ref() {
        n.notify_one();
    }
}

/// Stop both (Relay paused, tray exiting).
pub fn stop_all() {
    for d in Dir::ALL {
        stop(d);
    }
}

/// Ends the direction when the task ends in any way, including being
/// dropped with the runtime, so the state shown is never left behind.
struct Guard {
    d: Dir,
    ui: Ui,
}

impl Drop for Guard {
    fn drop(&mut self) {
        STOPS.lock().unwrap_or_else(|p| p.into_inner())[self.d.index()] = None;
        track(|t| t.end(self.d));
        (self.ui)(UiEvent::AudioChanged);
    }
}

/// Start (or refuse to start) a session with `peer`. Returns false when that
/// direction is already asking or running.
#[allow(clippy::too_many_arguments)]
pub fn start(
    node: Node,
    peer: PeerId,
    name: String,
    d: Dir,
    log: Log,
    ui: Ui,
    settings: SharedSettings,
) -> bool {
    if !track(|t| t.begin(d)) {
        return false;
    }
    let stop = Arc::new(Notify::new());
    STOPS.lock().unwrap_or_else(|p| p.into_inner())[d.index()] = Some(stop.clone());
    ui(UiEvent::AudioChanged);
    tokio::spawn(run(node, peer, name, d, log, ui, settings, stop));
    true
}

fn timeline_dir(d: Dir) -> crate::timeline::Dir {
    match d {
        Dir::PhoneToPc => crate::timeline::Dir::ToPc,
        Dir::PcToPhone => crate::timeline::Dir::ToPhone,
    }
}

#[allow(clippy::too_many_arguments)]
async fn run(
    node: Node,
    peer: PeerId,
    name: String,
    d: Dir,
    log: Log,
    ui: Ui,
    settings: SharedSettings,
    stop: Arc<Notify>,
) {
    let _guard = Guard { d, ui: ui.clone() };
    let word = d.log_word();
    let skip = settings
        .lock()
        .map(|s| s.audio_skip_confirm_pc_to_phone)
        .unwrap_or(false);

    // 1. The PC's own question (PC audio on the phone only).
    if st::needs_pc_confirm(d, skip) && !synth() {
        let (tx, rx) = oneshot::channel();
        ui(UiEvent::AskAudioShare {
            from: name.clone(),
            reply: OfferReply::new(tx),
        });
        let answer = tokio::select! {
            a = tokio::time::timeout(CONFIRM_TIMEOUT, rx) => a,
            () = stop.notified() => {
                log.line(&format!("AUDIO {word} cancelled before asking"));
                return;
            }
        };
        if !matches!(answer, Ok(Ok(true))) {
            log.line(&format!("AUDIO {word} not confirmed on the PC"));
            crate::applog::emit(crate::applog::audio("declined_here", word, 0, ""));
            return;
        }
    }

    // 2. Ask the phone (it asks its own user for its audio).
    let req = AudioStart {
        id: 0,
        sample_rate: 48_000,
        channels: 2,
        direction: d.wire(),
        source: AUDIO_SOURCE_PLAYBACK,
    };
    let sent = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let asked = {
        let (ui, name, sent) = (ui.clone(), name.clone(), sent.clone());
        move || {
            sent.store(true, std::sync::atomic::Ordering::Relaxed);
            if d == Dir::PhoneToPc {
                ui(UiEvent::AudioAsking(st::asking_text(d, &name)));
            }
        }
    };
    log.line(&format!("AUDIO {word} asking"));
    crate::applog::emit(crate::applog::audio("asked", word, 0, ""));
    let opened = tokio::select! {
        r = crate::core::dial(&node, &peer, node.open_audio(&peer, req, asked)) => r,
        () = stop.notified() => {
            ui(UiEvent::AudioAskEnded);
            log.line(&format!("AUDIO {word} request cancelled by the user"));
            crate::applog::emit(crate::applog::audio("cancelled", word, 0, ""));
            return;
        }
    };
    let mut sess = match opened {
        Ok(s) => s,
        Err(e) => {
            let sent = sent.load(std::sync::atomic::Ordering::Relaxed);
            log.line(&format!("AUDIO {word} refused: {e} (request sent: {sent})"));
            crate::applog::emit(crate::applog::audio("refused", word, 0, &e.to_string()));
            ui(UiEvent::Info(st::request_failed_text(d, &name, &e, sent)));
            return;
        }
    };
    if !track(|t| t.running(d)) {
        sess.stop();
        return;
    }
    crate::core::touch(&peer);
    ui(UiEvent::AudioAskEnded);
    log.line(&format!(
        "AUDIO {word} started ({} Hz, {} ch, codec {})",
        sess.accept.sample_rate, sess.accept.channels, sess.accept.codec
    ));
    crate::applog::emit(crate::applog::audio("started", word, 0, ""));
    ui(UiEvent::AudioChanged);
    let began = Instant::now();

    // 3. Move audio until something ends it.
    let (sess, end) = match d {
        // The output stream is not `Send`: it lives on one thread for the
        // whole session, which drives the async loop from there.
        Dir::PhoneToPc => {
            let (handle, stop, log, ui) = (
                tokio::runtime::Handle::current(),
                stop.clone(),
                log.clone(),
                ui.clone(),
            );
            let joined = tokio::task::spawn_blocking(move || {
                let mut sess = sess;
                let end = handle.block_on(play_phone_audio(&mut sess, &stop, &log, &ui));
                (sess, end)
            })
            .await;
            match joined {
                Ok(r) => r,
                Err(_) => return,
            }
        }
        Dir::PcToPhone => {
            let end = capture_pc_audio(&mut sess, &stop, &log).await;
            (sess, end)
        }
    };
    sess.stop();
    // The last contact is the end of the session, not its start.
    crate::core::touch(&peer);
    let secs = began.elapsed().as_secs();
    log.line(&format!(
        "AUDIO {word} ended after {secs} s: {end:?} ({})",
        sess.end_detail()
    ));
    crate::applog::emit(crate::applog::audio(
        "ended",
        word,
        secs,
        &format!("{end:?}"),
    ));
    crate::core::record(
        &settings,
        crate::timeline::audio_entry(&name, timeline_dir(d), d.title(), secs),
    );
    if let Some(text) = st::ended_text(d, &name, &end) {
        ui(UiEvent::Info(text));
    }
}

fn cause_of(ev: Option<AudioEvent>) -> End {
    match ev {
        Some(AudioEvent::Stopped { by_peer: true }) => End::PeerStopped,
        Some(AudioEvent::Violation(_)) => End::Violation,
        Some(AudioEvent::Stopped { by_peer: false } | AudioEvent::WriteFailed(_)) | None => {
            End::LinkLost
        }
    }
}

/// The phone sends, this PC plays.
async fn play_phone_audio(
    sess: &mut pb_transport::audio::AudioSession,
    stop: &Notify,
    log: &Log,
    ui: &Ui,
) -> End {
    // The session just came up: the first frame has until STALL_MS to arrive.
    LAST_FRAME_MS.store(clock_ms(), Ordering::Relaxed);
    let mut stall_tick = tokio::time::interval(Duration::from_millis(500));
    let mut was_stalled = false;
    // Packets are about 6 ms each, so this bounds a scheduling hiccup, not
    // the latency: the ring behind it keeps that.
    let (tx, rx) = mpsc::channel::<Vec<u8>>(32);
    let mut opus = None;
    if sess.accept.codec == pb_protocol::control::AUDIO_CODEC_OPUS {
        match OpusDec::new(sess.accept.sample_rate, sess.accept.channels) {
            Ok(d) => opus = Some(d),
            Err(e) => {
                log.line(&format!("AUDIO decoder could not start: {e}"));
                return End::DeviceFailed(st::device_text(Dir::PhoneToPc, &e));
            }
        }
    }
    let out = if synth() {
        None
    } else {
        match AudioOut::start(sess.accept.sample_rate, sess.accept.channels, rx) {
            Ok(o) => Some(o),
            Err(e) => {
                log.line(&format!("AUDIO output could not start: {e}"));
                return End::DeviceFailed(st::device_text(Dir::PhoneToPc, &e.to_string()));
            }
        }
    };
    if let Some(o) = out.as_ref() {
        log.line(&format!("AUDIO output device {}", o.device()));
    }
    let (dropped, underruns) = {
        let o = out.as_ref();
        (
            move || o.map_or(0, AudioOut::dropped_ms),
            move || o.map_or(0, AudioOut::underruns),
        )
    };
    // `recv_frame` borrows the whole session: watch its events separately.
    let mut events = std::mem::replace(&mut sess.events, mpsc::unbounded_channel().1);
    let began = Instant::now();
    let mut arrivals = st::Arrivals::default();
    let mut window = Instant::now();
    let (mut decode_us_total, mut decode_us_max) = (0u64, 0u64);
    let mut ages_before = [0u64; pb_transport::audio::AGE_BINS];
    let mut path_before = None;
    let mut playout = PlayoutTarget::default();
    let mut steered = Instant::now();
    let end = loop {
        tokio::select! {
            f = sess.recv_frame() => match f {
                // A full player queue drops the newest chunk; the device
                // side keeps its own latency bound.
                Some(f) => {
                    arrivals.frame(began.elapsed().as_millis() as u64, f.samples.len());
                    LAST_FRAME_MS.store(clock_ms(), Ordering::Relaxed);
                    let t0 = Instant::now();
                    let pcm = match opus.as_mut() {
                        Some(d) => d.decode(&f.samples),
                        None => f.samples,
                    };
                    let spent = t0.elapsed().as_micros() as u64;
                    decode_us_total += spent;
                    decode_us_max = decode_us_max.max(spent);
                    arrivals.level(&pcm);
                    if !pcm.is_empty() {
                        let _ = tx.try_send(pcm);
                    }
                    if steered.elapsed() >= Duration::from_secs(TARGET_WINDOW_S) {
                        steered = Instant::now();
                        let want = playout.note(arrivals.take_recent_gap_ms());
                        if let Some(o) = out.as_ref() {
                            o.set_target_ms(want);
                        }
                    }
                    if window.elapsed() >= Duration::from_secs(10) {
                        let n = sess.stats();
                        log.line(&format!(
                            "AUDIO arrivals {} packets={} lost={} late={} queue_dropped={} decode_avg_us={} decode_max_us={} target_ms={} max_age_ms={} quic_lost={} paths={} rtt_ms={}",
                            arrivals.line(dropped(), underruns()),
                            n.packets, n.lost, n.late, n.queue_dropped,
                            decode_us_total / arrivals.frames.max(1), decode_us_max,
                            out.as_ref().map_or(0, AudioOut::target_ms),
                            n.max_age_ms, n.quic_lost, n.paths, n.rtt_ms
                        ));
                        if let Some(o) = out.as_ref() {
                            let w = o.take_window();
                            let ages: [u64; pb_transport::audio::AGE_BINS] =
                                std::array::from_fn(|i| n.age_bins[i] - ages_before[i]);
                            ages_before = n.age_bins;
                            log.line(&format!(
                                "AUDIO playout window_ms={} consumed_ms={} starved={} max_callback_gap_ms={} queue_low_ms={} queue_peak_ms={} trim_ppm={} device_buffer_ms={} thread_priority={} max_sent_gap_ms={} ages_20_40_60_80_100_150_200_300_more={ages:?}",
                                window.elapsed().as_millis(),
                                w.consumed_ms,
                                w.starved,
                                w.max_callback_gap_ms,
                                w.queue_low_ms,
                                w.queue_peak_ms,
                                w.trim_ppm,
                                w.device_buffer_ms,
                                o.thread_priority(),
                                n.max_sent_gap_ms,
                            ));
                        }
                        let path = sess.path_addr().map(|a| a.ip());
                        if path != path_before {
                            log.line(&format!("AUDIO path {path:?}"));
                            path_before = path;
                        }
                        arrivals.reset_window();
                        decode_us_total = 0;
                        decode_us_max = 0;
                        window = Instant::now();
                    }
                }
                None => break cause_of(events.recv().await),
            },
            ev = events.recv() => break cause_of(ev),
            () = stop.notified() => break End::LocalStop,
            // The flyout only repaints on an event: tell it when the stall starts or ends.
            _ = stall_tick.tick() => {
                let now = stalled();
                if now != was_stalled {
                    was_stalled = now;
                    ui(UiEvent::AudioChanged);
                }
            }
        }
    };
    LAST_FRAME_MS.store(0, Ordering::Relaxed);
    log.line(&format!(
        "AUDIO playback dropped_ms={} underruns={} (counted per sample frame, not per network chunk)",
        dropped(),
        underruns()
    ));
    end
}

/// This PC captures its output, the phone plays.
async fn capture_pc_audio(
    sess: &mut pb_transport::audio::AudioSession,
    stop: &Notify,
    log: &Log,
) -> End {
    if synth() {
        return send_test_tone(sess, stop, log).await;
    }
    let (ftx, mut frx) = mpsc::unbounded_channel::<Vec<u8>>();
    let (etx, mut erx) = mpsc::unbounded_channel::<String>();
    let started = tokio::task::spawn_blocking(move || {
        AudioCapture::start(
            move |f| {
                let _ = ftx.send(f);
            },
            move |e| {
                let _ = etx.send(e.to_string());
            },
        )
    })
    .await;
    let cap = match started {
        Ok(Ok(c)) => c,
        Ok(Err(e)) => {
            log.line(&format!("AUDIO capture could not start: {e}"));
            return End::DeviceFailed(st::device_text(Dir::PcToPhone, &e.to_string()));
        }
        Err(_) => return End::DeviceFailed(st::device_text(Dir::PcToPhone, "capture failed")),
    };
    let end = loop {
        tokio::select! {
            f = frx.recv() => match f {
                Some(f) => {
                    if sess.send_frame(f).is_err() {
                        break cause_of(sess.events.recv().await);
                    }
                }
                None => {
                    // The capture thread reports the cause (default device
                    // changed, device removed) just before it ends the frames.
                    let why = erx.try_recv().unwrap_or_else(|_| "capture ended".into());
                    break End::DeviceFailed(st::device_text(Dir::PcToPhone, &why));
                }
            },
            e = erx.recv() => if let Some(e) = e {
                break End::DeviceFailed(st::device_text(Dir::PcToPhone, &e));
            },
            ev = sess.events.recv() => break cause_of(ev),
            () = stop.notified() => break End::LocalStop,
        }
    };
    // Joining the capture thread may wait on the device: not on the runtime.
    let _ = tokio::task::spawn_blocking(move || drop(cap)).await;
    end
}

/// `--test-audio-synth`, PC to phone: a quiet tone at the real rate.
async fn send_test_tone(
    sess: &mut pb_transport::audio::AudioSession,
    stop: &Notify,
    log: &Log,
) -> End {
    let mut tick = tokio::time::interval(Duration::from_millis(20));
    let (mut n, mut last_log) = (0u64, Instant::now());
    loop {
        tokio::select! {
            _ = tick.tick() => {
                let mut f = Vec::with_capacity(3840);
                for _ in 0..960 {
                    let phase = (n % 109) as f32 / 109.0 * std::f32::consts::TAU;
                    let v = ((phase.sin() * 800.0) as i16).to_le_bytes();
                    f.extend_from_slice(&v);
                    f.extend_from_slice(&v);
                    n += 1;
                }
                if sess.send_frame(f).is_err() {
                    return cause_of(sess.events.recv().await);
                }
                if last_log.elapsed() >= Duration::from_secs(10) {
                    let st = sess.stats();
                    log.line(&format!("AUDIO test tone packets={} quic_lost={} paths={} rtt_ms={}", st.packets, st.quic_lost, st.paths, st.rtt_ms));
                    last_log = Instant::now();
                }
            }
            ev = sess.events.recv() => return cause_of(ev),
            () = stop.notified() => return End::LocalStop,
        }
    }
}
