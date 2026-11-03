//! The PC timeline: a metadata-only record of what moved between this PC and
//! its phones. Never stored: clipboard text, picture bytes, file contents,
//! notification titles or bodies. Notifications keep the app name only.
//!
//! The file is `timeline.sealed` in the tray dir, `wrap(encode(entries))` with
//! the same sealing the identity uses. Rewrites are atomic and at most once a
//! second, plus on exit. `encode` / `parse` / `prune` / `summary` are pure.
#![allow(unsafe_code)] // only `replace`'s MoveFileExW, below.

use std::{
    io,
    path::{Path, PathBuf},
    sync::Mutex,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use pb_identity::vault::SeedWrapper;

pub const FILE: &str = "timeline.sealed";
const HEADER: &str = "tl1";
pub const MAX_ENTRIES: usize = 1000;
pub const MAX_AGE_SECS: u64 = 30 * 24 * 3600;
const WRITE_EVERY: Duration = Duration::from_secs(1);

/// How long to wait before the next write after `failures` failed ones
/// in a row.
pub fn retry_after(failures: u32) -> Duration {
    const CAP: Duration = Duration::from_secs(60);
    match failures {
        0 => WRITE_EVERY,
        n => WRITE_EVERY
            .checked_mul(1u32.checked_shl(n - 1).unwrap_or(u32::MAX))
            .map_or(CAP, |d| d.min(CAP)),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    File,
    ClipText,
    ClipImage,
    Notification,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dir {
    ToPhone,
    ToPc,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub kind: Kind,
    pub dir: Dir,
    pub peer: String,
    /// Unix seconds.
    pub at: u64,
    pub name: Option<String>,
    pub size: Option<u64>,
    pub path: Option<String>,
    pub chars: Option<u32>,
    pub dims: Option<(u32, u32)>,
    pub app: Option<String>,
}

pub fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn blank(kind: Kind, dir: Dir, peer: &str) -> Entry {
    Entry {
        kind,
        dir,
        peer: peer.to_owned(),
        at: now_unix(),
        name: None,
        size: None,
        path: None,
        chars: None,
        dims: None,
        app: None,
    }
}

pub fn file_entry(peer: &str, dir: Dir, name: &str, size: u64, path: Option<&Path>) -> Entry {
    Entry {
        name: Some(name.to_owned()),
        size: Some(size),
        path: path.map(|p| p.to_string_lossy().into_owned()),
        ..blank(Kind::File, dir, peer)
    }
}

/// Takes the text only to count its characters; the text itself is dropped.
/// A sensitive clip (a password) keeps not even its length.
pub fn clip_text_entry(peer: &str, dir: Dir, text: &str, sensitive: bool) -> Entry {
    Entry {
        chars: (!sensitive).then(|| text.chars().count().min(u32::MAX as usize) as u32),
        ..blank(Kind::ClipText, dir, peer)
    }
}

/// Keeps the PNG's size and dimensions (IHDR), never its bytes.
pub fn clip_image_entry(peer: &str, dir: Dir, png: &[u8]) -> Entry {
    let dims = (png.len() >= 24 && png.starts_with(b"\x89PNG\r\n\x1a\n"))
        .then(|| {
            let be = |i: usize| u32::from_be_bytes([png[i], png[i + 1], png[i + 2], png[i + 3]]);
            (be(16), be(20))
        })
        .filter(|&(w, h)| w > 0 && h > 0);
    Entry {
        size: Some(png.len() as u64),
        dims,
        ..blank(Kind::ClipImage, dir, peer)
    }
}

/// Who a notification row names: the session's device name.
pub fn notification_peer(name: &str) -> &str {
    if name.trim().is_empty() {
        "your phone"
    } else {
        name
    }
}

/// The source app's name only: never the title or text.
pub fn notification_entry(peer: &str, app: &str) -> Entry {
    Entry {
        app: Some(app.to_owned()),
        ..blank(Kind::Notification, Dir::ToPc, peer)
    }
}

fn esc(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => o.push_str("\\\\"),
            '\t' => o.push_str("\\t"),
            '\n' => o.push_str("\\n"),
            '\r' => o.push_str("\\r"),
            c => o.push(c),
        }
    }
    o
}

fn unesc(s: &str) -> Result<String, String> {
    let mut o = String::with_capacity(s.len());
    let mut it = s.chars();
    while let Some(c) = it.next() {
        if c != '\\' {
            o.push(c);
            continue;
        }
        match it.next() {
            Some('\\') => o.push('\\'),
            Some('t') => o.push('\t'),
            Some('n') => o.push('\n'),
            Some('r') => o.push('\r'),
            _ => return Err("bad escape".into()),
        }
    }
    Ok(o)
}

/// A string field: empty = absent, `=text` = present (so "" survives).
fn opt_s(v: &Option<String>) -> String {
    v.as_deref()
        .map_or(String::new(), |s| format!("={}", esc(s)))
}

fn opt_n<T: ToString>(v: Option<T>) -> String {
    v.map_or(String::new(), |n| n.to_string())
}

fn parse_opt_s(f: &str) -> Result<Option<String>, String> {
    match f.strip_prefix('=') {
        Some(s) => unesc(s).map(Some),
        None if f.is_empty() => Ok(None),
        None => Err("bad text field".into()),
    }
}

fn parse_opt_n<T: std::str::FromStr>(f: &str) -> Result<Option<T>, String> {
    if f.is_empty() {
        Ok(None)
    } else {
        f.parse().map(Some).map_err(|_| "bad number".to_string())
    }
}

/// `tl1` header, then one tab-separated line per entry:
/// kind, dir, at, peer, name, size, path, chars, width, height, app.
pub fn encode(v: &[Entry]) -> Vec<u8> {
    let mut o = String::from(HEADER);
    o.push('\n');
    for e in v {
        let kind = match e.kind {
            Kind::File => "F",
            Kind::ClipText => "T",
            Kind::ClipImage => "I",
            Kind::Notification => "N",
        };
        let dir = match e.dir {
            Dir::ToPhone => "p",
            Dir::ToPc => "c",
        };
        let cols = [
            kind.to_owned(),
            dir.to_owned(),
            e.at.to_string(),
            esc(&e.peer),
            opt_s(&e.name),
            opt_n(e.size),
            opt_s(&e.path),
            opt_n(e.chars),
            opt_n(e.dims.map(|d| d.0)),
            opt_n(e.dims.map(|d| d.1)),
            opt_s(&e.app),
        ];
        o.push_str(&cols.join("\t"));
        o.push('\n');
    }
    o.into_bytes()
}

pub fn parse(b: &[u8]) -> Result<Vec<Entry>, String> {
    let text = std::str::from_utf8(b).map_err(|_| "not UTF-8".to_string())?;
    let mut lines = text.split('\n');
    if lines.next() != Some(HEADER) {
        return Err("unknown version".into());
    }
    let mut out = Vec::new();
    for line in lines.filter(|l| !l.is_empty()) {
        let f: Vec<&str> = line.split('\t').collect();
        if f.len() != 11 {
            return Err("wrong field count".into());
        }
        let kind = match f[0] {
            "F" => Kind::File,
            "T" => Kind::ClipText,
            "I" => Kind::ClipImage,
            "N" => Kind::Notification,
            _ => return Err("bad kind".into()),
        };
        let dir = match f[1] {
            "p" => Dir::ToPhone,
            "c" => Dir::ToPc,
            _ => return Err("bad direction".into()),
        };
        let dims = match (parse_opt_n::<u32>(f[8])?, parse_opt_n::<u32>(f[9])?) {
            (Some(w), Some(h)) => Some((w, h)),
            (None, None) => None,
            _ => return Err("half a size".into()),
        };
        out.push(Entry {
            kind,
            dir,
            at: f[2].parse().map_err(|_| "bad time".to_string())?,
            peer: unesc(f[3])?,
            name: parse_opt_s(f[4])?,
            size: parse_opt_n(f[5])?,
            path: parse_opt_s(f[6])?,
            chars: parse_opt_n(f[7])?,
            dims,
            app: parse_opt_s(f[10])?,
        });
    }
    Ok(out)
}

/// Drops entries older than 30 days, then keeps the newest 1,000 (newest first).
pub fn prune(v: &mut Vec<Entry>, now: u64) {
    let oldest = now.saturating_sub(MAX_AGE_SECS);
    v.retain(|e| e.at >= oldest);
    v.sort_by_key(|e| std::cmp::Reverse(e.at)); // stable: ties keep their order
    v.truncate(MAX_ENTRIES);
}

fn human_size(n: u64) -> String {
    let f = n as f64;
    if n < 1000 {
        format!("{n} B")
    } else if n < 1_000_000 {
        format!("{:.1} KB", f / 1e3)
    } else if n < 1_000_000_000 {
        format!("{:.1} MB", f / 1e6)
    } else {
        format!("{:.1} GB", f / 1e9)
    }
}

pub fn summary(e: &Entry) -> String {
    match e.kind {
        Kind::File => {
            let name = e.name.as_deref().unwrap_or("File");
            match e.size {
                Some(s) => format!("{name}, {}", human_size(s)),
                None => name.to_owned(),
            }
        }
        Kind::ClipText => match e.chars {
            Some(1) => "Text, 1 character".into(),
            Some(n) => format!("Text, {n} characters"),
            None => "Text".into(),
        },
        Kind::ClipImage => {
            let mut s = String::from("Picture");
            if let Some((w, h)) = e.dims {
                s += &format!(", {w} \u{d7} {h}");
            }
            if let Some(n) = e.size {
                s += &format!(", {}", human_size(n));
            }
            s
        }
        Kind::Notification => format!("{} notification", e.app.as_deref().unwrap_or("App")),
    }
}

/// `.tmp` beside the target, flushed to disk, then swapped in so a crash
/// leaves the old file or the new one, never half of one.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    use std::io::Write;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = tmp_path(path);
    {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?; // FlushFileBuffers on Windows
    }
    let r = replace(&tmp, path);
    if r.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    r
}

fn tmp_path(path: &Path) -> PathBuf {
    let mut s = path.as_os_str().to_owned();
    s.push(".tmp");
    PathBuf::from(s)
}

#[cfg(windows)]
fn replace(from: &Path, to: &Path) -> io::Result<()> {
    use windows::{
        core::HSTRING,
        Win32::Storage::FileSystem::{
            MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
        },
    };
    // SAFETY: both HSTRINGs are valid NUL-terminated wide strings that outlive the call.
    unsafe {
        MoveFileExW(
            &HSTRING::from(from.as_os_str()),
            &HSTRING::from(to.as_os_str()),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    }
    .map_err(|_| io::Error::last_os_error())
}

#[cfg(not(windows))]
fn replace(from: &Path, to: &Path) -> io::Result<()> {
    std::fs::rename(from, to)
}

pub struct Store {
    path: PathBuf,
    wrapper: Box<dyn SeedWrapper>,
    entries: Vec<Entry>,
    enabled: bool,
    /// The file could not be read (not just unsealed): never overwrite it this session.
    write_locked: bool,
    dirty: bool,
    /// Bumped whenever `entries` changes, so readers copy them only then.
    version: u64,
    last_write: Option<Instant>,
    /// Failed writes in a row (0 after a success); spaces out retries.
    pub(crate) write_failures: u32,
    log: Box<dyn Fn(&str) + Send>,
    pending_log: Vec<String>,
}

impl Store {
    pub fn open(dir: &Path, w: Box<dyn SeedWrapper>) -> Store {
        let path = dir.join(FILE);
        let _ = std::fs::remove_file(tmp_path(&path)); // a crash mid-write
        let mut s = Store {
            path,
            wrapper: w,
            entries: Vec::new(),
            enabled: true,
            write_locked: false,
            dirty: false,
            version: 0,
            last_write: None,
            write_failures: 0,
            log: Box::new(|_| {}),
            pending_log: Vec::new(),
        };
        match std::fs::read(&s.path) {
            Ok(sealed) => {
                let parsed = s
                    .wrapper
                    .unwrap(&sealed)
                    .map_err(|e| e.to_string())
                    .and_then(|plain| parse(&plain));
                match parsed {
                    Ok(v) => {
                        s.entries = v;
                        prune(&mut s.entries, now_unix());
                    }
                    Err(e) => {
                        s.pending_log
                            .push(format!("TIMELINE unreadable, starting empty: {e}"));
                        let _ = std::fs::remove_file(&s.path);
                    }
                }
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            // A sharing violation or similar: the history may be fine on disk.
            Err(e) => {
                s.write_locked = true;
                s.pending_log.push(format!("TIMELINE read failed: {e}"));
            }
        }
        s
    }

    /// Where `TIMELINE ...` lines go; messages queued during `open` come out now.
    pub fn set_logger(&mut self, f: impl Fn(&str) + Send + 'static) {
        for m in self.pending_log.drain(..) {
            f(&m);
        }
        self.log = Box::new(f);
    }

    /// Newest first.
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// Changes whenever `entries` does (add, prune, clear, history off).
    pub fn version(&self) -> u64 {
        self.version
    }

    pub fn add(&mut self, e: Entry) {
        if !self.enabled {
            return;
        }
        self.entries.insert(0, e);
        prune(&mut self.entries, now_unix());
        self.dirty = true;
        self.version += 1;
    }

    /// `add` only marks the entry; the file is written by this (from the
    /// tray's 1 s timer) or by `flush`, never on the caller's thread.
    /// Writes if something changed and the last write was over a second ago.
    pub fn flush_if_due(&mut self, now: Instant) {
        if self.dirty
            && self.last_write.is_none_or(|t| {
                now.saturating_duration_since(t) >= retry_after(self.write_failures)
            })
        {
            self.write_at(now);
        }
    }

    /// An add is not written yet and could be (the tray's timer runs
    /// only while this or a window needs it).
    pub fn wants_flush(&self) -> bool {
        self.dirty && self.enabled && !self.write_locked
    }

    pub fn flush(&mut self) {
        self.write_at(Instant::now());
    }

    /// Writes now; `now` is when (the back-off counts from it).
    fn write_at(&mut self, now: Instant) {
        if !self.dirty || !self.enabled || self.write_locked {
            return;
        }
        self.last_write = Some(now);
        let r = self
            .wrapper
            .wrap(&encode(&self.entries))
            .and_then(|sealed| write_atomic(&self.path, &sealed));
        match r {
            Ok(()) => {
                self.dirty = false;
                self.write_failures = 0;
            }
            // The data stays in memory; `flush_if_due` tries again later
            // and later (`retry_after`). One line per failure streak.
            Err(e) => {
                if self.write_failures == 0 {
                    (self.log)(&format!("TIMELINE write failed: {e}"));
                }
                self.write_failures = self.write_failures.saturating_add(1);
            }
        }
    }

    /// Removes the entries `gone` says yes to (the Timeline's delete);
    /// returns how many.
    pub fn remove_where(&mut self, gone: impl Fn(&Entry) -> bool) -> usize {
        let before = self.entries.len();
        self.entries.retain(|e| !gone(e));
        let n = before - self.entries.len();
        if n > 0 {
            self.dirty = true;
            self.version += 1;
        }
        n
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.version += 1;
        self.dirty = false;
        self.write_locked = false; // the user asked for it gone
        let _ = std::fs::remove_file(&self.path);
        let _ = std::fs::remove_file(tmp_path(&self.path));
    }

    /// Off clears and stops recording; on starts again.
    pub fn set_enabled(&mut self, on: bool) {
        self.enabled = on;
        if !on {
            self.clear();
        }
    }
}

static STORE: Mutex<Option<Store>> = Mutex::new(None);

/// Opens the global store. Call once at startup.
pub fn init(
    dir: &Path,
    w: Box<dyn SeedWrapper>,
    keep_history: bool,
    log: impl Fn(&str) + Send + 'static,
) {
    let mut s = Store::open(dir, w);
    s.set_logger(log);
    s.set_enabled(keep_history);
    *STORE.lock().unwrap_or_else(|p| p.into_inner()) = Some(s);
}

/// Runs `f` on the global store; does nothing before `init`.
pub fn with(f: impl FnOnce(&mut Store)) {
    if let Some(s) = STORE.lock().unwrap_or_else(|p| p.into_inner()).as_mut() {
        f(s);
    }
}

#[cfg(test)]
#[path = "timeline_tests.rs"]
mod tests;
