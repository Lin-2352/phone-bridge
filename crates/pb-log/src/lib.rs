//! Day-by-day structured logs for Relay.
//!
//! Layout: `<root>/YYYY-MM-DD/{events,transfers,network,perf}.jsonl` plus
//! `crash.txt`; one flat JSON object per line with `t` (UTC ISO ms), `k`
//! (kind) and the caller's fields. The API has no field for clipboard text,
//! notification text or file contents, and file names only ever enter a line
//! through [`redact_name`]. Writes never panic and never block on anything
//! but a short lock; I/O errors are swallowed after one `eprintln`.

mod json;
mod retention;
mod timefmt;

#[cfg(test)]
mod tests;

pub use json::{parse_flat, scrub, FieldsError, Value};
pub use timefmt::day_for;

use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{BufWriter, Read, Seek, SeekFrom, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, Weak};
use std::time::Duration;

const MIB: u64 = 1024 * 1024;
/// Lines per second per stream before the rate limiter drops.
pub const RATE_PER_SEC: u32 = 50;
/// An identical line is held (and counted) at most this long.
const DEDUPE_HOLD_MS: u64 = 5_000;
const SUMMARY_EVERY_MS: u64 = 60_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Stream {
    Events,
    Transfers,
    Network,
    Perf,
    Crash,
}

impl Stream {
    const ALL: [Stream; 5] = [
        Stream::Events,
        Stream::Transfers,
        Stream::Network,
        Stream::Perf,
        Stream::Crash,
    ];

    fn idx(self) -> usize {
        self as usize
    }

    pub fn file_name(self) -> &'static str {
        match self {
            Stream::Events => "events.jsonl",
            Stream::Transfers => "transfers.jsonl",
            Stream::Network => "network.jsonl",
            Stream::Perf => "perf.jsonl",
            Stream::Crash => "crash.txt",
        }
    }

    /// Dropped first when the day is full.
    fn low_priority(self) -> bool {
        matches!(self, Stream::Network | Stream::Perf)
    }

    pub fn parse(s: &str) -> Option<Stream> {
        Some(match s {
            "events" => Stream::Events,
            "transfers" => Stream::Transfers,
            "network" => Stream::Network,
            "perf" => Stream::Perf,
            "crash" => Stream::Crash,
            _ => return None,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Caps {
    pub per_day_bytes: u64,
    pub total_bytes: u64,
    pub keep_days: u32,
    pub compress_after_days: u32,
}

impl Caps {
    /// PC defaults: 5 MiB a day, 100 MiB in all, 30 days, gzip after 2.
    pub fn pc() -> Self {
        Self {
            per_day_bytes: 5 * MIB,
            total_bytes: 100 * MIB,
            keep_days: 30,
            compress_after_days: 2,
        }
    }

    /// Phone defaults: as the PC but 30 MiB in all.
    pub fn phone() -> Self {
        Self {
            total_bytes: 30 * MIB,
            ..Self::pc()
        }
    }
}

impl Default for Caps {
    fn default() -> Self {
        Self::pc()
    }
}

/// Only the extension (letters/digits, at most 5) and a length bucket of a
/// file name: `"Q3 secret plan.pdf"` becomes `".pdf/17-32"`.
pub fn redact_name(name: &str) -> String {
    let ext = name
        .rsplit_once('.')
        .map(|(_, e)| e)
        .filter(|e| !e.is_empty() && e.len() <= 5 && e.chars().all(|c| c.is_ascii_alphanumeric()))
        .map(|e| format!(".{}", e.to_ascii_lowercase()))
        .unwrap_or_default();
    let bucket = match name.chars().count() {
        0..=8 => "1-8",
        9..=16 => "9-16",
        17..=32 => "17-32",
        33..=64 => "33-64",
        _ => "65+",
    };
    format!("{ext}/{bucket}")
}

type Clock = Arc<dyn Fn() -> u64 + Send + Sync>;

struct Pending {
    body: String,
    t_first: u64,
    count: u64,
}

#[derive(Default)]
struct StreamState {
    writer: Option<BufWriter<File>>,
    pending: Option<Pending>,
    sec: u64,
    sec_n: u32,
    cap_drops: u64,
    rate_drops: u64,
    last_summary: u64,
}

struct Inner {
    streams: [StreamState; 5],
    day: String,
    day_bytes: u64,
    last_flush: u64,
    warned: bool,
    sampled: HashMap<(Stream, String), u64>,
}

struct Shared {
    root: PathBuf,
    caps: Caps,
    clock: Clock,
    offset: AtomicI32,
    enabled: AtomicBool,
    inner: Mutex<Inner>,
}

/// A cheap, cloneable handle to one log root.
#[derive(Clone)]
pub struct Logger(Arc<Shared>);

fn system_clock() -> Clock {
    Arc::new(|| {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0)
    })
}

impl Inner {
    fn warn(&mut self, what: &str, e: &std::io::Error) {
        if !self.warned {
            self.warned = true;
            eprintln!("pb-log: {what}: {e} (further log errors are silent)");
        }
    }

    fn roll(&mut self, sh: &Shared, now: u64) {
        let day = day_for((now / 1000) as i64, sh.offset.load(Ordering::Relaxed));
        if day == self.day {
            return;
        }
        self.flush_all(sh, now, true);
        for s in &mut self.streams {
            s.writer = None;
        }
        self.day = day;
        self.day_bytes = retention::dir_size(&sh.root.join(&self.day));
    }

    fn write_raw(&mut self, sh: &Shared, stream: Stream, line: &str) {
        if self.streams[stream.idx()].writer.is_none() {
            match open_append(sh, &self.day, stream) {
                Ok(w) => self.streams[stream.idx()].writer = Some(w),
                Err(e) => return self.warn("open", &e),
            }
        }
        let res = match self.streams[stream.idx()].writer.as_mut() {
            Some(w) => w
                .write_all(line.as_bytes())
                .and_then(|()| w.write_all(b"\n")),
            None => return,
        };
        match res {
            Ok(()) => self.day_bytes += line.len() as u64 + 1,
            Err(e) => self.warn("write", &e),
        }
    }

    fn emit_pending(&mut self, sh: &Shared, stream: Stream) {
        let Some(p) = self.streams[stream.idx()].pending.take() else {
            return;
        };
        let mut line = format!("{{\"t\":\"{}\",{}", timefmt::iso_utc(p.t_first), p.body);
        if p.count > 1 {
            line.push_str(&format!(",\"n\":{}", p.count));
        }
        line.push('}');
        self.write_raw(sh, stream, &line);
    }

    fn summary(&mut self, sh: &Shared, stream: Stream, now: u64, force: bool) {
        let st = &mut self.streams[stream.idx()];
        let (cap, rate) = (st.cap_drops, st.rate_drops);
        if cap + rate == 0 || (!force && now.saturating_sub(st.last_summary) < SUMMARY_EVERY_MS) {
            return;
        }
        st.cap_drops = 0;
        st.rate_drops = 0;
        st.last_summary = now;
        let why = match (cap > 0, rate > 0) {
            (true, true) => "cap+rate",
            (true, false) => "cap",
            _ => "rate",
        };
        let line = format!(
            "{{\"t\":\"{}\",\"k\":\"dropped\",\"n\":{},\"why\":\"{why}\"}}",
            timefmt::iso_utc(now),
            cap + rate
        );
        self.write_raw(sh, stream, &line);
    }

    /// Write held lines (all, or only those held longer than the dedupe
    /// window) and flush the buffers.
    fn flush_all(&mut self, sh: &Shared, now: u64, everything: bool) {
        for s in Stream::ALL {
            if s == Stream::Crash {
                continue;
            }
            let due = self.streams[s.idx()]
                .pending
                .as_ref()
                .is_some_and(|p| everything || now.saturating_sub(p.t_first) >= DEDUPE_HOLD_MS);
            if due {
                self.emit_pending(sh, s);
            }
            self.summary(sh, s, now, everything);
        }
        for i in 0..self.streams.len() {
            let err = self.streams[i]
                .writer
                .as_mut()
                .and_then(|w| w.flush().err());
            if let Some(e) = err {
                self.warn("flush", &e);
            }
        }
        self.last_flush = now;
    }
}

fn open_append(sh: &Shared, day: &str, stream: Stream) -> std::io::Result<BufWriter<File>> {
    let dir = sh.root.join(day);
    fs::create_dir_all(&dir)?;
    let path = dir.join(stream.file_name());
    let mut f = OpenOptions::new()
        .create(true)
        .read(true)
        .append(true)
        .open(path)?;
    // A crash can leave a torn last line: start on a fresh line.
    let len = f.metadata()?.len();
    if len > 0 {
        let mut last = [0u8; 1];
        f.seek(SeekFrom::Start(len - 1))?;
        f.read_exact(&mut last)?;
        if last[0] != b'\n' {
            f.write_all(b"\n")?;
        }
    }
    Ok(BufWriter::with_capacity(8 * 1024, f))
}

/// Most a day's `crash.txt` may hold through [`Logger::crash`].
const CRASH_FILE_CAP: u64 = 256 * 1024;

impl Logger {
    /// Open (creating `root` if needed) with the system clock; a background
    /// thread flushes once a second and prunes at start and hourly.
    pub fn open(root: PathBuf, caps: Caps) -> std::io::Result<Logger> {
        let lg = Self::open_with(root, caps, system_clock())?;
        let weak: Weak<Shared> = Arc::downgrade(&lg.0);
        let _ = std::thread::Builder::new()
            .name("pb-log".into())
            .spawn(move || {
                let mut ticks = 0u32;
                loop {
                    let Some(sh) = weak.upgrade() else { return };
                    let lg = Logger(sh);
                    if ticks.is_multiple_of(3600) {
                        lg.prune();
                    }
                    lg.tick();
                    ticks = ticks.wrapping_add(1);
                    drop(lg);
                    std::thread::sleep(Duration::from_secs(1));
                }
            });
        Ok(lg)
    }

    /// As [`Logger::open`] but with an injected clock (Unix ms) and no
    /// background thread: tests and embedders drive `tick`/`prune` themselves.
    pub fn open_with(
        root: PathBuf,
        caps: Caps,
        clock: Arc<dyn Fn() -> u64 + Send + Sync>,
    ) -> std::io::Result<Logger> {
        fs::create_dir_all(&root)?;
        Ok(Logger(Arc::new(Shared {
            root,
            caps,
            clock,
            offset: AtomicI32::new(0),
            enabled: AtomicBool::new(true),
            inner: Mutex::new(Inner {
                streams: Default::default(),
                day: String::new(),
                day_bytes: 0,
                last_flush: 0,
                warned: false,
                sampled: HashMap::new(),
            }),
        })))
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.0.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Local UTC offset in seconds; the day boundary follows it. Callers
    /// refresh it now and then so daylight-saving changes are followed.
    pub fn set_tz_offset_secs(&self, secs: i32) {
        self.0.offset.store(secs, Ordering::Relaxed);
    }

    pub fn set_enabled(&self, on: bool) {
        self.0.enabled.store(on, Ordering::Relaxed);
        if !on {
            self.flush();
        }
    }

    pub fn enabled(&self) -> bool {
        self.0.enabled.load(Ordering::Relaxed)
    }

    /// Append one line. `fields` are flat values; string values are cut to
    /// 256 characters.
    pub fn event(&self, stream: Stream, kind: &str, fields: &[(&str, Value)]) {
        if !self.enabled() {
            return;
        }
        let sh = &*self.0;
        let now = (sh.clock)();
        let mut g = self.lock();
        g.roll(sh, now);
        if stream == Stream::Crash {
            let mut line = format!("{} {kind}", timefmt::iso_utc(now));
            for (n, v) in fields {
                line.push_str(&format!(" {n}="));
                match v {
                    Value::Str(x) => line.push_str(&x.replace(['\n', '\r'], " ")),
                    Value::Int(i) => line.push_str(&i.to_string()),
                    Value::Float(f) => line.push_str(&format!("{f:.3}")),
                    Value::Bool(b) => line.push_str(&b.to_string()),
                }
            }
            g.write_raw(sh, stream, &line);
            if let Some(w) = g.streams[stream.idx()].writer.as_mut() {
                let _ = w.flush();
            }
            return;
        }
        let i = stream.idx();
        let sec = now / 1000;
        if g.streams[i].sec != sec {
            g.streams[i].sec = sec;
            g.streams[i].sec_n = 0;
        }
        g.streams[i].sec_n += 1;
        let cap = sh.caps.per_day_bytes;
        let hard_full = g.day_bytes >= cap.saturating_add(cap / 2);
        if g.streams[i].sec_n > RATE_PER_SEC {
            g.streams[i].rate_drops += 1;
            g.summary(sh, stream, now, false);
            return;
        }
        if hard_full || (stream.low_priority() && g.day_bytes >= cap) {
            g.streams[i].cap_drops += 1;
            g.summary(sh, stream, now, false);
            return;
        }
        let body = json::body(kind, fields);
        match g.streams[i].pending.as_mut() {
            Some(p) if p.body == body => p.count += 1,
            _ => {
                g.emit_pending(sh, stream);
                g.streams[i].pending = Some(Pending {
                    body,
                    t_first: now,
                    count: 1,
                });
            }
        }
        if now.saturating_sub(g.last_flush) >= 1000 {
            g.flush_all(sh, now, false);
        }
    }

    /// Append a crash/panic note straight away, bypassing the buffer and the
    /// lock: safe from a panic hook even if the panicking thread holds the
    /// logger lock.
    pub fn crash(&self, text: &str) {
        if !self.enabled() {
            return;
        }
        let sh = &*self.0;
        let now = (sh.clock)();
        let text: String = text.chars().take(2000).collect();
        let line = format!(
            "{} {}",
            timefmt::iso_utc(now),
            text.replace('\r', "").replace('\n', " | ")
        );
        let day = day_for((now / 1000) as i64, sh.offset.load(Ordering::Relaxed));
        let dir = sh.root.join(day);
        // Write what is still held first: a kill right after a crash note
        // would lose it. try_lock: the panicking thread may hold the lock.
        let held = match sh.inner.try_lock() {
            Ok(g) => Some(g),
            Err(std::sync::TryLockError::Poisoned(e)) => Some(e.into_inner()),
            Err(std::sync::TryLockError::WouldBlock) => None,
        };
        if let Some(mut g) = held {
            g.flush_all(sh, now, true);
        }
        // A panic loop must not grow the note file without limit.
        if fs::metadata(dir.join("crash.txt")).is_ok_and(|m| m.len() >= CRASH_FILE_CAP) {
            return;
        }
        let append = || -> std::io::Result<()> {
            fs::create_dir_all(&dir)?;
            let mut f = OpenOptions::new()
                .create(true)
                .append(true)
                .open(dir.join("crash.txt"))?;
            writeln!(f, "{line}")
        };
        if let Err(e) = append() {
            eprintln!("pb-log: crash note not written: {e}");
        }
    }

    /// True at most once per `every_secs` for this (stream, key): wrap
    /// high-frequency lines as `if log.sample(..) { log.event(..) }`.
    pub fn sample(&self, stream: Stream, key: &str, every_secs: u64) -> bool {
        let now = (self.0.clock)();
        let mut g = self.lock();
        let slot = g.sampled.entry((stream, key.to_owned())).or_insert(0);
        if *slot == 0 || now.saturating_sub(*slot) >= every_secs * 1000 {
            *slot = now.max(1);
            true
        } else {
            false
        }
    }

    /// Write everything held and flush the buffers.
    pub fn flush(&self) {
        let sh = &*self.0;
        let now = (sh.clock)();
        self.lock().flush_all(sh, now, true);
    }

    /// Once-a-second housekeeping (the background thread's job).
    pub fn tick(&self) {
        let sh = &*self.0;
        let now = (sh.clock)();
        self.lock().flush_all(sh, now, false);
    }

    /// Apply retention (delete, gzip, total cap) at the clock's now.
    pub fn prune(&self) {
        self.prune_at((self.0.clock)());
    }

    pub fn prune_at(&self, now_ms: u64) {
        retention::prune(
            &self.0.root,
            &self.0.caps,
            now_ms,
            self.0.offset.load(Ordering::Relaxed),
        );
    }

    /// Delete every day folder; the root stays.
    pub fn clear_all(&self) -> std::io::Result<()> {
        let sh = &*self.0;
        let mut g = self.lock();
        for s in &mut g.streams {
            s.writer = None;
            s.pending = None;
            s.cap_drops = 0;
            s.rate_drops = 0;
        }
        g.day = String::new();
        g.day_bytes = 0;
        drop(g);
        let mut first_err = None;
        for (name, _) in retention::day_folders(&sh.root) {
            if let Err(e) = fs::remove_dir_all(sh.root.join(name)) {
                first_err.get_or_insert(e);
            }
        }
        first_err.map_or(Ok(()), Err)
    }

    /// Bytes on disk under the root (buffers are flushed first).
    pub fn size_bytes(&self) -> u64 {
        self.flush();
        retention::dir_size(&self.0.root)
    }

    /// Day folder names, oldest first.
    pub fn days(&self) -> Vec<String> {
        retention::day_folders(&self.0.root)
            .into_iter()
            .map(|(n, _)| n)
            .collect()
    }

    pub fn day_dir(&self, day: &str) -> PathBuf {
        self.0.root.join(day)
    }

    pub fn root(&self) -> &std::path::Path {
        &self.0.root
    }

    /// A finished file transfer. The name is reduced by [`redact_name`];
    /// nothing else about the file is recorded.
    pub fn transfer(&self, dir: &str, name: &str, bytes: u64, secs: f64, ok: bool) {
        let mbps = if secs > 0.0 {
            bytes as f64 / secs / 1e6
        } else {
            0.0
        };
        self.event(
            Stream::Transfers,
            if ok { "file_done" } else { "file_failed" },
            &[
                ("dir", dir.into()),
                ("file", redact_name(name).into()),
                ("bytes", bytes.into()),
                ("secs", secs.into()),
                ("mbps", mbps.into()),
            ],
        );
    }
}
