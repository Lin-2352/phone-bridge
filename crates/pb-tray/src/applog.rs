//! The structured day-by-day log (`pb-log`) for the tray.
//!
//! `core::Log::line` still writes `tray.log` unchanged; it also hands each
//! line to [`from_line`], which maps the known families to structured
//! events. One constructor per event keeps the mapping readable. Nothing
//! with user content is ever mapped: no clipboard text, notification text,
//! pairing codes, device or file names (files go through `redact_name`).

use pb_log::{Logger, Stream, Value};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::OnceLock;

static LOGGER: OnceLock<Logger> = OnceLock::new();
static SESSIONS: AtomicI64 = AtomicI64::new(0);
static DIR: OnceLock<PathBuf> = OnceLock::new();

/// One structured event, ready to write.
#[derive(Debug, PartialEq)]
pub struct Ev {
    pub stream: Stream,
    pub kind: &'static str,
    pub fields: Vec<(&'static str, Value)>,
}

fn ev(stream: Stream, kind: &'static str, fields: Vec<(&'static str, Value)>) -> Ev {
    Ev {
        stream,
        kind,
        fields,
    }
}

// One constructor per event family.
pub fn tray_start() -> Ev {
    ev(Stream::Events, "tray_start", vec![])
}
pub fn tray_exit() -> Ev {
    ev(Stream::Events, "tray_exit", vec![])
}
pub fn ready(port: u16) -> Ev {
    ev(
        Stream::Events,
        "ready",
        vec![("port", i64::from(port).into())],
    )
}
pub fn session_open() -> Ev {
    ev(Stream::Events, "session_open", vec![])
}
pub fn session_close(why: &str) -> Ev {
    ev(
        Stream::Events,
        "session_close",
        vec![("why", scrub(why).into())],
    )
}
pub fn paired() -> Ev {
    ev(Stream::Events, "paired", vec![])
}
pub fn unpaired() -> Ev {
    ev(Stream::Events, "unpaired", vec![])
}
pub fn pairing_code(round: u32) -> Ev {
    ev(
        Stream::Events,
        "pairing_code",
        vec![("round", i64::from(round).into())],
    )
}
pub fn pairing_end(how: &'static str, why: &str) -> Ev {
    ev(
        Stream::Events,
        "pairing_end",
        vec![("how", how.into()), ("why", scrub(why).into())],
    )
}
pub fn clipsync(dir: &'static str, what: &'static str, size: u64, ok: bool, why: &str) -> Ev {
    let mut f = vec![
        ("dir", dir.into()),
        ("what", what.into()),
        ("size", size.into()),
        ("ok", ok.into()),
    ];
    if !ok {
        f.push(("why", scrub(why).into()));
    }
    ev(Stream::Events, "clipsync", f)
}
pub fn notify(what: &'static str) -> Ev {
    ev(Stream::Events, "notify", vec![("what", what.into())])
}
/// Relay was paused or resumed from the tray menu.
pub fn pause(on: bool) -> Ev {
    ev(
        Stream::Events,
        if on { "paused" } else { "resumed" },
        vec![],
    )
}
pub fn device_released() -> Ev {
    ev(Stream::Events, "device_released", vec![])
}
pub fn error(whence: &str, why: &str) -> Ev {
    ev(
        Stream::Events,
        "error",
        vec![("where", whence.into()), ("why", scrub(why).into())],
    )
}
pub fn file_done(dir: &'static str, name: &str, bytes: u64, mbps: f64) -> Ev {
    let secs = if mbps > 0.0 {
        bytes as f64 / 1e6 / mbps
    } else {
        0.0
    };
    file_event("file_done", dir, name, bytes, secs, mbps)
}
fn file_event(
    kind: &'static str,
    dir: &'static str,
    name: &str,
    bytes: u64,
    secs: f64,
    mbps: f64,
) -> Ev {
    ev(
        Stream::Transfers,
        kind,
        vec![
            ("dir", dir.into()),
            ("file", pb_log::redact_name(name).into()),
            ("bytes", bytes.into()),
            ("secs", secs.into()),
            ("mbps", mbps.into()),
        ],
    )
}
pub fn file_started(dir: &'static str, name: &str, bytes: u64) -> Ev {
    ev(
        Stream::Transfers,
        "file_started",
        vec![
            ("dir", dir.into()),
            ("file", pb_log::redact_name(name).into()),
            ("bytes", bytes.into()),
        ],
    )
}
/// A transfer stopped on purpose (`by`: user, pause, quit or peer).
pub fn file_cancelled(
    dir: &'static str,
    name: &str,
    bytes_done: u64,
    by: &'static str,
    secs: f64,
    mbps: f64,
) -> Ev {
    ev(
        Stream::Transfers,
        "file_cancelled",
        vec![
            ("dir", dir.into()),
            ("file", pb_log::redact_name(name).into()),
            ("bytes", bytes_done.into()),
            ("by", by.into()),
            ("secs", secs.into()),
            ("mbps", mbps.into()),
        ],
    )
}
pub fn file_failed(dir: &'static str, name: &str, why: &str) -> Ev {
    ev(
        Stream::Transfers,
        "file_failed",
        vec![
            ("dir", dir.into()),
            ("file", pb_log::redact_name(name).into()),
            ("why", scrub(why).into()),
        ],
    )
}

pub use pb_log::scrub;

fn file_name_of(path: &str) -> &str {
    path.rsplit(['\\', '/']).next().unwrap_or(path)
}

/// Map one `tray.log` line to a structured event; `None` for lines that are
/// not (or must not be) recorded.
pub fn map_line(msg: &str) -> Option<Ev> {
    match msg {
        "tray started" => return Some(tray_start()),
        "tray exiting" => return Some(tray_exit()),
        "UI device released" => return Some(device_released()),
        _ => {}
    }
    if let Some(r) = msg.strip_prefix("READY ") {
        let port = r.split("port=").nth(1)?.trim().parse().ok()?;
        return Some(ready(port));
    }
    if let Some(r) = msg.strip_prefix("SESSION ") {
        return Some(match r.find("\" ended: ") {
            Some(i) => session_close(&r[i + "\" ended: ".len()..]),
            None => session_open(),
        });
    }
    if msg.starts_with("PAIRED ") {
        return Some(paired());
    }
    if msg.starts_with("UNPAIRED ") {
        return Some(unpaired());
    }
    if let Some(r) = msg.strip_prefix("CODE round=") {
        return Some(pairing_code(r.split(' ').next()?.parse().ok()?));
    }
    if let Some(r) = msg.strip_prefix("PAIRING ") {
        return Some(if r.starts_with("window closed") {
            pairing_end("closed", "")
        } else if r.starts_with("window expired") {
            pairing_end("expired", "")
        } else {
            pairing_end("failed", r.split_once(": ").map_or("", |(_, e)| e))
        });
    }
    if let Some(r) = msg.strip_prefix("CLIPSYNC ") {
        return map_clipsync(r);
    }
    if let Some(r) = msg.strip_prefix("FILE ") {
        return map_file(r);
    }
    if let Some(r) = msg.strip_prefix("NOTIFY ") {
        let what = if r.starts_with("posted") {
            "posted"
        } else if r.starts_with("flood") {
            "flood"
        } else if r.starts_with("action") {
            "action"
        } else if r.starts_with("toast not shown") {
            "not_shown"
        } else if r.starts_with("sync") {
            "sync_skipped"
        } else {
            return None;
        };
        return Some(notify(what));
    }
    let family = msg.split(' ').next().unwrap_or("");
    if family == "FATAL" {
        return Some(error("FATAL", &msg["FATAL".len()..]));
    }
    const BAD: [&str; 6] = [
        "failed",
        "not shown",
        "not started",
        "refused",
        "not received",
        "unavailable",
    ];
    if BAD.iter().any(|b| msg.contains(b)) {
        let why = msg.split_once(": ").map_or("", |(_, e)| e);
        return Some(error(family, why));
    }
    None
}

fn map_clipsync(r: &str) -> Option<Ev> {
    let num = |s: &str, end: &str| -> u64 {
        s.split(end)
            .next()
            .and_then(|n| n.trim().parse().ok())
            .unwrap_or(0)
    };
    let fail = |what: &'static str, r: &str| {
        let why = r.split_once("failed: ").map_or("", |(_, e)| e);
        Some(clipsync("sent", what, 0, false, why))
    };
    if let Some(x) = r.strip_prefix("sent ") {
        return if x.starts_with("a sensitive") {
            Some(clipsync("sent", "sensitive", 0, true, ""))
        } else if let Some(i) = x.strip_prefix("an image (") {
            Some(clipsync("sent", "image", num(i, " bytes"), true, ""))
        } else {
            Some(clipsync("sent", "text", num(x, " chars"), true, ""))
        };
    }
    if r.starts_with("to ") && r.contains("failed: ") {
        return fail("text", r);
    }
    if r.starts_with("image to ") && r.contains("failed: ") {
        return fail("image", r);
    }
    if let Some(x) = r.strip_prefix("received ") {
        return if x.starts_with("a sensitive") {
            Some(clipsync("received", "sensitive", 0, true, ""))
        } else if let Some(i) = x.strip_prefix("an image (") {
            Some(clipsync("received", "image", num(i, " bytes"), true, ""))
        } else {
            Some(clipsync("received", "text", num(x, " chars"), true, ""))
        };
    }
    None
}

fn map_file(r: &str) -> Option<Ev> {
    if let Some(x) = r.strip_prefix("sent \"") {
        // `{name}" {size} bytes to "{device}" in {secs}s ({mbps} MB/s)`
        let (head, tail) = x.rsplit_once(" bytes to \"")?;
        let (name, size) = head.rsplit_once(' ')?;
        let name = name.strip_suffix('"')?;
        let rest = &tail[tail.rfind("\" in ")? + "\" in ".len()..];
        let secs: f64 = rest.split('s').next()?.parse().ok()?;
        let mbps: f64 = rest
            .split('(')
            .nth(1)?
            .split(" MB/s")
            .next()?
            .parse()
            .ok()?;
        return Some(file_event(
            "file_done",
            "sent",
            name,
            size.parse().ok()?,
            secs,
            mbps,
        ));
    }
    if let Some(x) = r.strip_prefix("received ") {
        // `{path} ({size} bytes, {mbps} MB/s) from "{device}"`
        let (head, tail) = x.rsplit_once(" bytes, ")?;
        let (path, size) = head.rsplit_once(" (")?;
        let mbps: f64 = tail.split(' ').next()?.parse().ok()?;
        return Some(file_done(
            "received",
            file_name_of(path),
            size.parse().ok()?,
            mbps,
        ));
    }
    if let Some(x) = r.strip_prefix("send \"") {
        let (name, why) = x.split_once(" failed: ")?;
        return Some(file_failed("sent", name.strip_suffix('"')?, why));
    }
    None
}

// ---- runtime ----

/// The log folder to use: `want`, else `fallback` when `want` cannot be
/// created or written (a missing D: drive, a read-only folder).
pub fn pick_dir(want: &Path, fallback: &Path, writable: impl Fn(&Path) -> bool) -> PathBuf {
    if writable(want) {
        want.to_path_buf()
    } else {
        fallback.to_path_buf()
    }
}

pub fn dir_is_writable(p: &Path) -> bool {
    if std::fs::create_dir_all(p).is_err() {
        return false;
    }
    let probe = p.join(".relay-write-test");
    let ok = std::fs::write(&probe, b"x").is_ok();
    let _ = std::fs::remove_file(probe);
    ok
}

/// `%LOCALAPPDATA%\PhoneBridge\logs`.
pub fn fallback_dir() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("PhoneBridge")
        .join("logs")
}

/// Open the logger once. Returns the folder in use.
pub fn init(want: &Path, keep: bool) -> Option<PathBuf> {
    let dir = pick_dir(want, &fallback_dir(), dir_is_writable);
    let lg = Logger::open(dir.clone(), pb_log::Caps::pc()).ok()?;
    lg.set_tz_offset_secs(crate::applog_sys::tz_offset_secs());
    lg.set_enabled(keep);
    let _ = LOGGER.set(lg);
    let _ = DIR.set(dir.clone());
    Some(dir)
}

/// The folder in use (the fallback when the wanted one was not writable).
pub fn dir() -> PathBuf {
    DIR.get().cloned().unwrap_or_else(fallback_dir)
}

/// Bytes the logs take now.
pub fn size_bytes() -> u64 {
    handle().map_or(0, Logger::size_bytes)
}

/// Delete every log (the folder stays).
pub fn clear_all() {
    if let Some(l) = handle() {
        let _ = l.clear_all();
    }
}

pub fn handle() -> Option<&'static Logger> {
    LOGGER.get()
}

pub fn set_enabled(on: bool) {
    if let Some(l) = handle() {
        l.set_enabled(on);
    }
}

pub fn emit(e: Ev) {
    if let Some(l) = handle() {
        l.event(e.stream, e.kind, &e.fields);
    }
}

/// Called for every `tray.log` line.
pub fn from_line(msg: &str) {
    if handle().is_none() {
        return;
    }
    if let Some(e) = map_line(msg) {
        match e.kind {
            "session_open" => {
                SESSIONS.fetch_add(1, Ordering::Relaxed);
            }
            "session_close" => {
                SESSIONS.fetch_sub(1, Ordering::Relaxed);
            }
            _ => {}
        }
        emit(e);
    }
}

/// Write everything held (at exit).
pub fn flush() {
    if let Some(l) = handle() {
        l.flush();
    }
}

pub fn open_sessions() -> i64 {
    SESSIONS.load(Ordering::Relaxed).max(0)
}

/// Panics are appended to `crash.txt` before the previous hook runs.
pub fn install_panic_hook() {
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if let Some(l) = handle() {
            let loc = info
                .location()
                .map(|l| format!("{}:{}", l.file(), l.line()))
                .unwrap_or_default();
            l.crash(&format!("panic at {loc}"));
        }
        prev(info);
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn field<'a>(e: &'a Ev, k: &str) -> Option<&'a Value> {
        e.fields.iter().find(|(n, _)| *n == k).map(|(_, v)| v)
    }

    #[test]
    fn session_lines_open_and_close() {
        let o = map_line("SESSION \"Pixel 8\"").unwrap();
        assert_eq!((o.stream, o.kind), (Stream::Events, "session_open"));
        let c = map_line("SESSION \"Pixel 8\" ended: connection lost").unwrap();
        assert_eq!(c.kind, "session_close");
        assert_eq!(field(&c, "why"), Some(&Value::s("connection lost")));
        // The device name is never recorded.
        assert!(!format!("{o:?}{c:?}").contains("Pixel"));
    }

    #[test]
    fn pairing_lines_never_carry_the_code_or_names() {
        let p = map_line("PAIRED abc123 \"My Phone\"").unwrap();
        assert_eq!(p.kind, "paired");
        assert_eq!(map_line("UNPAIRED abc123").unwrap().kind, "unpaired");
        let c = map_line("CODE round=2 123456").unwrap();
        assert_eq!(c.kind, "pairing_code");
        assert_eq!(field(&c, "round"), Some(&Value::Int(2)));
        let s = format!("{p:?}{c:?}");
        assert!(!s.contains("123456") && !s.contains("Phone") && !s.contains("abc123"));
        assert!(
            map_line("SAS=482913 peer=\"x\"").is_none(),
            "the SAS is never logged"
        );
        let n = map_line("PAIRING not completed: timed out").unwrap();
        assert_eq!(field(&n, "how"), Some(&Value::s("failed")));
    }

    #[test]
    fn clipsync_records_sizes_not_content() {
        let a = map_line("CLIPSYNC sent 42 chars to \"PC\"").unwrap();
        assert_eq!(field(&a, "size"), Some(&Value::Int(42)));
        assert_eq!(field(&a, "dir"), Some(&Value::s("sent")));
        let b = map_line("CLIPSYNC to \"Pixel\" failed: stream reset").unwrap();
        assert_eq!(field(&b, "ok"), Some(&Value::Bool(false)));
        let c = map_line("CLIPSYNC received 7 chars from \"Pixel\"").unwrap();
        assert_eq!(field(&c, "dir"), Some(&Value::s("received")));
        let d = map_line("CLIPSYNC sent an image (2048 bytes) to \"x\"").unwrap();
        assert_eq!(field(&d, "what"), Some(&Value::s("image")));
        assert_eq!(field(&d, "size"), Some(&Value::Int(2048)));
        let e = map_line("CLIPSYNC sent a sensitive clip to \"x\"").unwrap();
        assert_eq!(field(&e, "what"), Some(&Value::s("sensitive")));
        assert!(!format!("{a:?}{b:?}{c:?}{d:?}{e:?}").contains("Pixel"));
    }

    #[test]
    fn file_sent_goes_to_transfers_with_bytes_seconds_and_rate() {
        let e = map_line(
            "FILE sent \"Q3 salary \"draft\".pdf\" 10485760 bytes to \"My PC\" in 2.50s (4.2 MB/s)",
        )
        .unwrap();
        assert_eq!((e.stream, e.kind), (Stream::Transfers, "file_done"));
        assert_eq!(field(&e, "dir"), Some(&Value::s("sent")));
        assert_eq!(field(&e, "bytes"), Some(&Value::Int(10_485_760)));
        assert_eq!(field(&e, "secs"), Some(&Value::Float(2.5)));
        assert_eq!(field(&e, "mbps"), Some(&Value::Float(4.2)));
        assert_eq!(field(&e, "file"), Some(&Value::s(".pdf/17-32")));
        let s = format!("{e:?}");
        assert!(!s.contains("salary") && !s.contains("My PC"), "{s}");
    }

    #[test]
    fn file_received_and_failed() {
        let e = map_line(
            "FILE received C:\\Users\\me\\Downloads\\Phone Bridge\\secret plans.zip (1000000 bytes, 8.0 MB/s) from \"Pixel\"",
        )
        .unwrap();
        assert_eq!(e.kind, "file_done");
        assert_eq!(field(&e, "dir"), Some(&Value::s("received")));
        assert_eq!(field(&e, "bytes"), Some(&Value::Int(1_000_000)));
        assert_eq!(field(&e, "mbps"), Some(&Value::Float(8.0)));
        assert_eq!(field(&e, "secs"), Some(&Value::Float(0.125)));
        let s = format!("{e:?}");
        assert!(
            !s.contains("secret") && !s.contains("Users") && !s.contains("Pixel"),
            "{s}"
        );
        let f =
            map_line("FILE send \"private.docx\" failed: Access is denied. (os error 5) C:\\x\\y")
                .unwrap();
        assert_eq!((f.stream, f.kind), (Stream::Transfers, "file_failed"));
        let s = format!("{f:?}");
        assert!(!s.contains("private") && !s.contains("C:\\\\x"), "{s}");
    }

    #[test]
    fn notify_lines_carry_no_package_or_key() {
        let n = map_line("NOTIFY posted pkg=com.bank.app key=abcd1234").unwrap();
        assert_eq!(n.kind, "notify");
        assert!(!format!("{n:?}").contains("bank"));
        assert_eq!(
            field(
                &map_line("NOTIFY flood from \"P\": session closed").unwrap(),
                "what"
            ),
            Some(&Value::s("flood"))
        );
        let a = map_line("NOTIFY action Dismiss key=abcd ok=true").unwrap();
        assert_eq!(field(&a, "what"), Some(&Value::s("action")));
    }

    #[test]
    fn lifecycle_and_errors() {
        assert_eq!(map_line("tray started").unwrap().kind, "tray_start");
        assert_eq!(map_line("tray exiting").unwrap().kind, "tray_exit");
        assert_eq!(
            map_line("UI device released").unwrap().kind,
            "device_released"
        );
        let r = map_line("READY id=abcdef port=4455").unwrap();
        assert_eq!(field(&r, "port"), Some(&Value::Int(4455)));
        assert!(!format!("{r:?}").contains("abcdef"));
        let f = map_line("FATAL endpoint closed").unwrap();
        assert_eq!(
            (f.kind, field(&f, "where")),
            ("error", Some(&Value::s("FATAL")))
        );
        let e = map_line("SCREEN \"Pixel\" not shown: no stream").unwrap();
        assert_eq!(e.kind, "error");
        assert_eq!(field(&e, "where"), Some(&Value::s("SCREEN")));
        assert!(map_line("TOAST identity registered").is_none());
        assert!(map_line("CLIP from \"P\" (12 chars)").is_none());
    }

    #[test]
    fn scrub_removes_quotes_and_paths_and_cuts() {
        assert_eq!(
            scrub("cannot open \"my file.txt\" in C:\\a\\b now"),
            "cannot open <q> in <path> now"
        );
        assert_eq!(scrub(&"x".repeat(500)).len(), 120);
    }

    #[test]
    fn log_dir_falls_back_when_not_writable() {
        let want = PathBuf::from(r"D:\Documents\Relay Logs");
        let fb = PathBuf::from(r"C:\fb");
        assert_eq!(pick_dir(&want, &fb, |_| true), want);
        assert_eq!(pick_dir(&want, &fb, |_| false), fb);
        let missing = std::env::temp_dir().join("pb-applog-nonexistent-drive");
        let _ = std::fs::remove_dir_all(&missing);
        assert!(dir_is_writable(&missing), "a creatable folder is writable");
        let _ = std::fs::remove_dir_all(&missing);
        let file = std::env::temp_dir().join(format!("pb-applog-file-{}", std::process::id()));
        std::fs::write(&file, "x").unwrap();
        assert!(!dir_is_writable(&file.join("sub")), "under a file is not");
        let _ = std::fs::remove_file(file);
    }
}
