//! File transfers as the user sees them: a registry of live and finished
//! transfers, the lifecycle of each (waiting, active, done, failed,
//! cancelled), smoothed speed and ETA, wording of every failure, and the
//! global store the core feeds and the transfer window draws.
//!
//! Everything here is plain data and maths (no Windows calls), so it is
//! tested without a window. Metadata only: names are shown to the user but
//! never logged beyond what `applog` already redacts.
#![allow(dead_code)]

use std::{path::PathBuf, time::Duration, time::Instant};

use pb_transport::{files::Cancel, Error};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dir {
    Send,
    Receive,
}

/// Who or what stopped a transfer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CancelBy {
    User,
    Pause,
    Quit,
    Peer,
}

/// A failure the user can act on: what happened, what to do next.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Failure {
    pub title: String,
    pub hint: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum State {
    /// Connecting, or waiting for the other device to accept.
    Waiting,
    Active,
    Done,
    Failed(Failure),
    Cancelled(CancelBy),
}

pub struct Lifecycle {
    dir: Dir,
    state: State,
}

impl Lifecycle {
    pub fn new(dir: Dir) -> Self {
        Self {
            dir,
            state: State::Waiting,
        }
    }
    pub fn state(&self) -> &State {
        &self.state
    }
    /// Bytes moved: the transfer is active (ignored once it has ended).
    pub fn progress(&mut self, _done: u64, _total: u64) {
        if self.is_live() {
            self.state = State::Active;
        }
    }
    /// The file is in place. Wins over a cancel that raced it: the file is real.
    pub fn finish(&mut self) {
        if !matches!(self.state, State::Failed(_) | State::Done) {
            self.state = State::Done;
        }
    }
    pub fn fail(&mut self, f: Failure) -> bool {
        if self.is_live() {
            self.state = State::Failed(f);
            true
        } else {
            false
        }
    }
    pub fn cancel(&mut self, by: CancelBy) -> bool {
        if self.is_live() {
            self.state = State::Cancelled(by);
            true
        } else {
            false
        }
    }
    /// Failed or cancelled sends start again; nothing else does.
    pub fn retry(&mut self) -> bool {
        if self.can_retry() {
            self.state = State::Waiting;
            true
        } else {
            false
        }
    }
    pub fn is_live(&self) -> bool {
        matches!(self.state, State::Waiting | State::Active)
    }
    /// A receive cannot be pulled from the PC: the phone sends it again.
    pub fn can_retry(&self) -> bool {
        self.dir == Dir::Send && matches!(self.state, State::Failed(_) | State::Cancelled(_))
    }
}

/// Speed and ETA from progress samples, smoothed (EWMA, 2 s time constant).
pub struct Meter {
    start: Instant,
    last_t: Instant,
    last_done: u64,
    last_progress: Instant,
    ewma: Option<f64>,
}

const TAU: f64 = 2.0;
const WARMUP: Duration = Duration::from_secs(1);
const STALL: Duration = Duration::from_secs(3);

impl Meter {
    pub fn new(now: Instant) -> Self {
        Self {
            start: now,
            last_t: now,
            last_done: 0,
            last_progress: now,
            ewma: None,
        }
    }
    /// Call on a steady timer (about 4 times a second), not per chunk.
    pub fn update(&mut self, now: Instant, done: u64) {
        let dt = now.saturating_duration_since(self.last_t).as_secs_f64();
        if dt < 0.05 {
            return;
        }
        let inst = done.saturating_sub(self.last_done) as f64 / dt;
        self.ewma = Some(match self.ewma {
            None => inst,
            Some(e) => {
                let a = 1.0 - (-dt / TAU).exp();
                a * inst + (1.0 - a) * e
            }
        });
        if done > self.last_done {
            self.last_progress = now;
        }
        self.last_t = now;
        self.last_done = done;
    }
    pub fn speed(&self, now: Instant) -> Option<f64> {
        let e = self.ewma?;
        if now.saturating_duration_since(self.start) < WARMUP {
            return None;
        }
        if now.saturating_duration_since(self.last_progress) > STALL {
            return Some(0.0);
        }
        Some(e)
    }
    pub fn eta(&self, now: Instant, done: u64, total: u64) -> Option<Duration> {
        if total == 0 {
            return None;
        }
        if done >= total {
            return Some(Duration::ZERO);
        }
        let v = self.speed(now)?;
        (v > 0.0).then(|| Duration::from_secs_f64(((total - done) as f64 / v).ceil()))
    }
}

pub fn fmt_bytes(n: u64) -> String {
    const K: f64 = 1024.0;
    let f = n as f64;
    let (v, unit) = if n < 1024 {
        return format!("{n} B");
    } else if f < K * K {
        (f / K, "KB")
    } else if f < K * K * K {
        (f / (K * K), "MB")
    } else {
        (f / (K * K * K), "GB")
    };
    if v >= 10.0 {
        format!("{v:.0} {unit}")
    } else {
        format!("{v:.1} {unit}")
    }
}

pub fn fmt_speed(bps: f64) -> String {
    if bps < 1024.0 {
        format!("{bps:.0} B/s")
    } else {
        format!("{}/s", fmt_bytes(bps as u64))
    }
}

pub fn fmt_eta(d: Duration) -> String {
    let s = d.as_secs();
    match s {
        0 => "almost done".into(),
        1..=59 => format!("{s} s left"),
        60..=3599 => match s % 60 {
            0 => format!("{} min left", s / 60),
            r => format!("{} min {r} s left", s / 60),
        },
        _ => format!("{} h {} min left", s / 3600, s % 3600 / 60),
    }
}

pub fn percent(done: u64, total: u64) -> u32 {
    if total == 0 {
        return 0;
    }
    (done.min(total) as u128 * 100 / total as u128) as u32
}

/// What the error mapper needs to name things.
pub struct Ctx<'a> {
    pub peer: &'a str,
    pub file: &'a str,
    pub folder: &'a str,
    pub refusal: Option<pb_transport::files::Refusal>,
}

/// What to update when two devices can't talk, naming the device.
pub fn update_hint(side: pb_transport::UpdateSide, name: &str) -> String {
    use pb_transport::UpdateSide;
    match side {
        UpdateSide::OtherDevice => {
            format!("{name} runs an older version of Relay. Update Relay on {name}")
        }
        UpdateSide::ThisDevice => format!("Update Relay on this PC: {name} needs a newer version"),
        UpdateSide::Both => {
            format!("{name} runs a different version of Relay. Update Relay on both devices")
        }
    }
}

/// Wording for a session that failed to open when the cause is known and
/// the generic "couldn't reach" would mislead. `None` keeps the caller's own text.
pub fn session_failure(e: &Error, name: &str) -> Option<String> {
    match e {
        Error::VersionMismatch(side) => Some(update_hint(*side, name)),
        Error::Storage(_) => Some("Relay couldn't save its list of devices. Try again".into()),
        _ => None,
    }
}

/// One user-facing wording per error: what happened and what to do next.
pub fn describe(dir: Dir, e: &Error, c: &Ctx) -> Failure {
    use pb_transport::files::Refusal;
    let (peer, file, folder) = (c.peer, c.file, c.folder);
    let f = |t: String, h: &str| Failure {
        title: t,
        hint: h.to_owned(),
    };
    match (dir, e) {
        (_, Error::VersionMismatch(side)) => f(
            format!("{peer} and this PC can't talk: their versions of Relay differ"),
            &format!("{}, then try again.", update_hint(*side, peer)),
        ),
        (_, Error::Storage(_)) => f(
            format!("Relay couldn't save its list of devices for {file}"),
            "Nothing changed. Try again.",
        ),
        (_, Error::Cancelled) => f(format!("Cancelled {file}"), ""),
        (_, Error::PeerCancelled) => f(format!("{peer} cancelled {file}"), ""),
        (Dir::Receive, Error::TooLarge) => match c.refusal {
            Some(Refusal::OverCap) => f(
                format!("{file} is over the 4 GB limit"),
                "Send a smaller file, or split it.",
            ),
            _ => f(
                format!("Not enough space in {folder} for {file}"),
                "Free some space or choose another folder in Settings, then send it again from the phone.",
            ),
        },
        (Dir::Receive, Error::HashMismatch) => f(
            format!("{file} arrived damaged"),
            "Nothing was saved. Send it again from the phone.",
        ),
        (Dir::Receive, Error::Io(_)) => f(
            format!("Couldn't save {file} in {folder}"),
            "Check the folder exists and can be written to, or choose another in Settings, then send it again from the phone.",
        ),
        (Dir::Receive, Error::Declined) => f(format!("Declined {file}"), ""),
        (Dir::Receive, Error::Protocol(_)) => f(
            format!("{peer} sent {file} in a form Relay couldn't read"),
            "Send it again from the phone.",
        ),
        (Dir::Receive, _) => f(
            format!("{peer} stopped sending {file}"),
            "Keep the phone awake on the same Wi-Fi, then send it again from the phone.",
        ),
        (Dir::Send, Error::Declined) => f(
            format!("{peer} declined {file}"),
            "Ask them to accept it on the phone, then send again.",
        ),
        (Dir::Send, Error::TooLarge) => f(
            format!("{file} is too big for {peer}"),
            "It is over the size limit, or the phone is low on space. Free some space or send a smaller file.",
        ),
        (Dir::Send, Error::Timeout) => f(
            format!("{peer} didn't answer"),
            "Open Relay on the phone, check both are on the same Wi-Fi, then retry.",
        ),
        (Dir::Send, Error::PeerChanged) => f(
            format!("Another device answers for {peer}"),
            "If the phone was reinstalled or reset, remove it in Devices and pair again.",
        ),
        (Dir::Send, Error::HashMismatch) => f(
            format!("{file} was damaged on the way to {peer}"),
            "Nothing was saved. Try again.",
        ),
        (Dir::Send, Error::Io(m)) if m.contains("could not save") => f(
            format!("{peer} couldn't save {file}"),
            "Check the phone has free space, then try again.",
        ),
        (Dir::Send, Error::Io(_)) => f(
            format!("Couldn't read {file}"),
            "Check the file still exists and isn't locked by another program, then retry.",
        ),
        (Dir::Send, Error::NotPaired | Error::UnknownPeer | Error::Rejected) => f(
            format!("{peer} is not paired with this PC"),
            "Pair the phone again in Devices.",
        ),
        (Dir::Send, _) => f(
            format!("Couldn't reach {peer}"),
            "Open Relay on the phone and check it is on the same Wi-Fi, then try again.",
        ),
    }
}

/// One transfer to track.
pub struct NewTransfer {
    pub dir: Dir,
    pub peer: String,
    pub name: String,
    pub size: u64,
    /// Send: the source file (kept for Retry). Receive: filled in when saved.
    pub path: Option<PathBuf>,
    pub batch: Option<Batch>,
}

/// "File `index` of `total`" in a multi-file send.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Batch {
    pub id: u64,
    pub index: usize,
    pub total: usize,
}

struct Entry {
    id: u64,
    t: NewTransfer,
    life: Lifecycle,
    meter: Meter,
    done: u64,
    total: u64,
    cancel: Cancel,
    ended: Option<Instant>,
}

/// What a Retry needs.
#[derive(Clone, Debug)]
pub struct RetryInfo {
    pub peer: String,
    pub name: String,
    pub path: PathBuf,
    pub batch: Option<Batch>,
}

/// A snapshot of one transfer for drawing.
#[derive(Clone, Debug)]
pub struct Row {
    pub id: u64,
    pub dir: Dir,
    pub peer: String,
    pub name: String,
    pub state: State,
    pub done: u64,
    pub total: u64,
    pub speed: Option<f64>,
    pub eta: Option<Duration>,
    pub batch: Option<Batch>,
    pub path: Option<PathBuf>,
    pub can_cancel: bool,
    pub can_retry: bool,
    /// Done and the file location is known: Open and Show in folder apply.
    pub can_open: bool,
}

impl Row {
    pub fn percent(&self) -> u32 {
        if self.state == State::Done {
            100
        } else {
            percent(self.done, self.total)
        }
    }

    /// The line under the file name.
    pub fn status_line(&self) -> String {
        match &self.state {
            State::Waiting => match self.dir {
                Dir::Send => format!("Waiting for {} to accept\u{2026}", self.peer),
                Dir::Receive => format!("Starting to receive from {}\u{2026}", self.peer),
            },
            State::Active => {
                let mut s = format!("{} of {}", fmt_bytes(self.done), fmt_bytes(self.total));
                if let Some(v) = self.speed {
                    let v = if v > 0.0 {
                        fmt_speed(v)
                    } else {
                        "stalled".into()
                    };
                    s += &format!(" \u{b7} {v}");
                }
                if let Some(e) = self.eta {
                    s += &format!(" \u{b7} {}", fmt_eta(e));
                }
                s
            }
            State::Done => format!("Done \u{b7} {}", fmt_bytes(self.total)),
            State::Failed(f) if f.hint.is_empty() => f.title.clone(),
            State::Failed(f) => format!("{}. {}", f.title, f.hint),
            State::Cancelled(CancelBy::User) => "Cancelled".into(),
            State::Cancelled(CancelBy::Pause) => "Stopped: Relay is paused".into(),
            State::Cancelled(CancelBy::Quit) => "Stopped: Relay closed".into(),
            State::Cancelled(CancelBy::Peer) => format!("{} cancelled", self.peer),
        }
    }

    /// "2 of 5" for a multi-file send.
    pub fn batch_label(&self) -> Option<String> {
        self.batch
            .filter(|b| b.total > 1)
            .map(|b| format!("{} of {}", b.index, b.total))
    }
}

#[derive(Default)]
pub struct Registry {
    next: u64,
    rows: Vec<Entry>,
    stopped_batches: Vec<u64>,
}

impl Registry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add(&mut self, t: NewTransfer, now: Instant) -> (u64, Cancel) {
        self.next += 1;
        let id = self.next;
        let cancel = Cancel::new();
        self.rows.push(Entry {
            id,
            life: Lifecycle::new(t.dir),
            meter: Meter::new(now),
            done: 0,
            total: t.size,
            cancel: cancel.clone(),
            ended: None,
            t,
        });
        (id, cancel)
    }

    fn get(&mut self, id: u64) -> Option<&mut Entry> {
        self.rows.iter_mut().find(|e| e.id == id)
    }

    pub fn set_progress(&mut self, id: u64, done: u64, total: u64) {
        if let Some(e) = self.get(id) {
            if e.life.is_live() {
                e.done = done;
                e.total = total;
                e.life.progress(done, total);
            }
        }
    }

    /// Feed every live meter (call about 4 times a second).
    pub fn tick(&mut self, now: Instant) {
        for e in self.rows.iter_mut().filter(|e| e.life.is_live()) {
            e.meter.update(now, e.done);
        }
    }

    pub fn finish(&mut self, id: u64, saved: Option<PathBuf>, now: Instant) {
        if let Some(e) = self.get(id) {
            e.life.finish();
            e.done = e.total;
            e.ended = Some(now);
            if saved.is_some() {
                e.t.path = saved;
            }
        }
    }

    pub fn fail(&mut self, id: u64, f: Failure, now: Instant) {
        if let Some(e) = self.get(id) {
            if e.life.fail(f) {
                e.ended = Some(now);
            }
        }
    }

    /// Stop one transfer: fires its cancel handle. The person's Cancel and the other device's cancel
    /// are for this file only; Relay pausing or quitting also stops the rest of its batch.
    pub fn cancel(&mut self, id: u64, by: CancelBy, now: Instant) -> bool {
        let Some(e) = self.get(id) else { return false };
        if !e.life.cancel(by) {
            return false;
        }
        e.ended = Some(now);
        e.cancel.cancel();
        let batch = e.t.batch;
        if let (Some(b), true) = (batch, matches!(by, CancelBy::Pause | CancelBy::Quit)) {
            self.stop_batch(b.id);
        }
        true
    }

    pub fn cancel_all(&mut self, by: CancelBy, now: Instant) -> usize {
        let ids: Vec<u64> = self
            .rows
            .iter()
            .filter(|e| e.life.is_live())
            .map(|e| e.id)
            .collect();
        ids.into_iter()
            .filter(|id| self.cancel(*id, by, now))
            .count()
    }

    /// Stops the outgoing transfers of the file at `path` (Pause on one row). Incoming files are never touched.
    pub fn cancel_sending_path(
        &mut self,
        path: &std::path::Path,
        by: CancelBy,
        now: Instant,
    ) -> usize {
        let ids: Vec<u64> = self
            .rows
            .iter()
            .filter(|e| {
                e.life.is_live() && e.t.dir == Dir::Send && e.t.path.as_deref() == Some(path)
            })
            .map(|e| e.id)
            .collect();
        ids.into_iter()
            .filter(|id| self.cancel(*id, by, now))
            .count()
    }

    /// Stops every outgoing transfer (Pause queue, Cancel all). Incoming files are never touched.
    pub fn cancel_all_sending(&mut self, by: CancelBy, now: Instant) -> usize {
        let ids: Vec<u64> = self
            .rows
            .iter()
            .filter(|e| e.life.is_live() && e.t.dir == Dir::Send)
            .map(|e| e.id)
            .collect();
        ids.into_iter()
            .filter(|id| self.cancel(*id, by, now))
            .count()
    }

    /// Stop the batch itself (also files not started yet).
    pub fn stop_batch(&mut self, batch: u64) {
        if !self.stopped_batches.contains(&batch) {
            self.stopped_batches.push(batch);
        }
    }

    pub fn batch_stopped(&self, batch: u64) -> bool {
        self.stopped_batches.contains(&batch)
    }

    /// Back to waiting with a fresh cancel handle; `None` if not retryable.
    pub fn retry(&mut self, id: u64, now: Instant) -> Option<(Cancel, RetryInfo)> {
        let e = self.get(id)?;
        let path = e.t.path.clone()?;
        if !e.life.retry() {
            return None;
        }
        e.cancel = Cancel::new();
        e.meter = Meter::new(now);
        e.done = 0;
        e.ended = None;
        // A retried file is a fresh single send.
        let batch = e.t.batch.take();
        let info = RetryInfo {
            peer: e.t.peer.clone(),
            name: e.t.name.clone(),
            path,
            batch: None,
        };
        let _ = batch;
        Some((e.cancel.clone(), info))
    }

    pub fn dismiss(&mut self, id: u64) {
        self.rows.retain(|e| e.id != id || e.life.is_live());
    }

    /// Drop successful rows that finished more than `keep` ago.
    pub fn prune_done(&mut self, now: Instant, keep: Duration) {
        self.rows.retain(|e| {
            !(e.life.state() == &State::Done
                && e.ended
                    .is_some_and(|t| now.saturating_duration_since(t) > keep))
        });
    }

    pub fn active_count(&self) -> usize {
        self.rows.iter().filter(|e| e.life.is_live()).count()
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    pub fn rows(&self, now: Instant) -> Vec<Row> {
        self.rows.iter().map(|e| row_of(e, now)).collect()
    }

    pub fn row(&self, id: u64, now: Instant) -> Option<Row> {
        self.rows
            .iter()
            .find(|e| e.id == id)
            .map(|e| row_of(e, now))
    }
}

fn row_of(e: &Entry, now: Instant) -> Row {
    let live = e.life.is_live();
    let active = e.life.state() == &State::Active;
    Row {
        id: e.id,
        dir: e.t.dir,
        peer: e.t.peer.clone(),
        name: e.t.name.clone(),
        state: e.life.state().clone(),
        done: e.done,
        total: e.total,
        speed: if active { e.meter.speed(now) } else { None },
        eta: if active {
            e.meter.eta(now, e.done, e.total)
        } else {
            None
        },
        batch: e.t.batch,
        path: e.t.path.clone(),
        can_cancel: live,
        can_retry: e.life.can_retry() && e.t.path.is_some(),
        can_open: e.life.state() == &State::Done && e.t.path.is_some(),
    }
}

// ---- the process-wide store the core feeds and the window draws ----

static REG: std::sync::Mutex<Option<Registry>> = std::sync::Mutex::new(None);

fn with<T>(f: impl FnOnce(&mut Registry) -> T) -> T {
    let mut g = REG.lock().unwrap_or_else(|p| p.into_inner());
    f(g.get_or_insert_with(Registry::new))
}

fn changed() {
    #[cfg(windows)]
    crate::transfer_window::poke();
}

static NEXT_BATCH: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

pub fn new_batch_id() -> u64 {
    NEXT_BATCH.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

/// Starts tracking a transfer; the window appears now.
pub fn start(t: NewTransfer) -> (u64, Cancel) {
    let r = with(|r| r.add(t, Instant::now()));
    changed();
    r
}

/// The progress callback for the transport: only stores numbers.
pub fn progress_cb(id: u64) -> pb_transport::files::Progress {
    std::sync::Arc::new(move |done, total| with(|r| r.set_progress(id, done, total)))
}

pub fn finish(id: u64, saved: Option<PathBuf>) {
    with(|r| r.finish(id, saved, Instant::now()));
    changed();
}

/// Marks a transfer failed; false if it had already ended (a cancel or a
/// pause that raced the error keeps its own wording).
pub fn fail(id: u64, f: Failure) -> bool {
    let live = with(|r| {
        let was = r.row(id, Instant::now()).is_some_and(|x| x.can_cancel);
        r.fail(id, f, Instant::now());
        was
    });
    changed();
    live
}

/// (done, total) bytes of a transfer.
pub fn progress_of(id: u64) -> Option<(u64, u64)> {
    with(|r| r.row(id, Instant::now()).map(|x| (x.done, x.total)))
}

/// Who cancelled a transfer, for the log.
pub fn cancel_reason(id: u64) -> &'static str {
    match with(|r| r.row(id, Instant::now()).map(|x| x.state)) {
        Some(State::Cancelled(CancelBy::Pause)) => "pause",
        Some(State::Cancelled(CancelBy::Quit)) => "quit",
        Some(State::Cancelled(CancelBy::Peer)) => "peer",
        _ => "user",
    }
}

pub fn cancel(id: u64, by: CancelBy) -> bool {
    let ok = with(|r| r.cancel(id, by, Instant::now()));
    changed();
    ok
}

/// Stops the outgoing transfers of the file at `path` (Pause on one row).
pub fn cancel_sending_path(path: &std::path::Path, by: CancelBy) -> usize {
    let n = with(|r| r.cancel_sending_path(path, by, Instant::now()));
    changed();
    n
}

/// Stops every outgoing transfer (Pause queue, Cancel all); incoming files go on.
pub fn cancel_all_sending(by: CancelBy) -> usize {
    let n = with(|r| r.cancel_all_sending(by, Instant::now()));
    changed();
    n
}

/// Pause and Quit: every live transfer stops (peers are told by the transport).
pub fn cancel_all(by: CancelBy) -> usize {
    let n = with(|r| r.cancel_all(by, Instant::now()));
    changed();
    n
}

pub fn batch_stopped(batch: u64) -> bool {
    with(|r| r.batch_stopped(batch))
}

pub fn active_count() -> usize {
    with(|r| r.active_count())
}

pub fn rows() -> Vec<Row> {
    with(|r| r.rows(Instant::now()))
}

pub fn dismiss(id: u64) {
    with(|r| r.dismiss(id));
    changed();
}

/// Called by the window's timer.
pub fn tick() {
    let now = Instant::now();
    with(|r| {
        r.tick(now);
        r.prune_done(now, Duration::from_secs(12));
    });
}

type RetryFn = Box<dyn Fn(u64, Cancel, RetryInfo) + Send + Sync>;
static RETRY: std::sync::OnceLock<RetryFn> = std::sync::OnceLock::new();

/// The core installs how a send restarts (it owns the node).
pub fn set_retry_handler(f: RetryFn) {
    let _ = RETRY.set(f);
}

pub fn retry(id: u64) {
    let r = with(|r| r.retry(id, Instant::now()));
    if let (Some((cancel, info)), Some(h)) = (r, RETRY.get()) {
        h(id, cancel, info);
    }
    changed();
}

/// The Quit confirmation when `n` transfers are running.
pub fn quit_warning(n: usize) -> (String, String) {
    let heading = if n == 1 {
        "A file transfer is running".to_owned()
    } else {
        format!("{n} file transfers are running")
    };
    (
        heading,
        "If you quit Relay now they stop, and the other device keeps nothing of the unfinished file."
            .to_owned(),
    )
}

/// What a Timeline file row (or a toast) may offer for a saved file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FileActions {
    pub open: bool,
    pub show_in_folder: bool,
}

/// Open and Show in folder both need the file to still exist.
pub fn file_actions(path: Option<&std::path::Path>) -> FileActions {
    let ok = path.is_some_and(|p| p.is_file());
    FileActions {
        open: ok,
        show_in_folder: ok,
    }
}

#[cfg(test)]
#[path = "transfers_tests.rs"]
mod transfers_tests;
