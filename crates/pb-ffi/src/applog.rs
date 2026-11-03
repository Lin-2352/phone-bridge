//! The phone's day-by-day structured log (`pb-log`), shared by the Android
//! app (through the `log_*` functions) and the node's own event points.
//! Nothing with user content is accepted: the Kotlin side sends flat
//! metadata fields only, and file names reach a line through `redact_name`.

use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};

use pb_log::{Caps, Logger, Stream, Value};

static LOGGER: Mutex<Option<Logger>> = Mutex::new(None);

fn slot() -> MutexGuard<'static, Option<Logger>> {
    LOGGER.lock().unwrap_or_else(|e| e.into_inner())
}

/// The open logger, if `init` succeeded.
pub fn handle() -> Option<Logger> {
    slot().clone()
}

/// Open the logger at `root` with the phone caps (5 MiB a day, 30 days,
/// `total_cap_bytes` in all; 0 keeps the default 30 MiB). Replaces an
/// earlier one.
pub fn init(root: &str, total_cap_bytes: u64) -> bool {
    let mut caps = Caps::phone();
    if total_cap_bytes > 0 {
        caps.total_bytes = total_cap_bytes;
    }
    match Logger::open(PathBuf::from(root), caps) {
        Ok(l) => {
            let old = slot().replace(l);
            if let Some(old) = old {
                old.flush();
            }
            true
        }
        Err(_) => false,
    }
}

/// One line from the app: `fields_json` is a flat JSON object. Unknown
/// streams and rejected fields (nesting, values over 256 characters) are
/// dropped.
pub fn event(stream: &str, kind: &str, fields_json: &str) {
    let (Some(lg), Some(stream)) = (handle(), Stream::parse(stream)) else {
        return;
    };
    let Ok(fields) = pb_log::parse_flat(fields_json) else {
        return;
    };
    let fields: Vec<(&str, Value)> = fields
        .iter()
        .map(|(k, v)| (k.as_str(), v.clone()))
        .collect();
    lg.event(stream, &kind.chars().take(64).collect::<String>(), &fields);
}

pub fn clear_all() -> bool {
    handle().is_some_and(|l| l.clear_all().is_ok())
}

pub fn size_bytes() -> u64 {
    handle().map_or(0, |l| l.size_bytes())
}

pub fn set_enabled(on: bool) {
    if let Some(l) = handle() {
        l.set_enabled(on);
    }
}

pub fn days() -> Vec<String> {
    handle().map(|l| l.days()).unwrap_or_default()
}

pub fn flush() {
    if let Some(l) = handle() {
        l.flush();
    }
}

pub fn set_tz_offset_secs(secs: i32) {
    if let Some(l) = handle() {
        l.set_tz_offset_secs(secs);
    }
}

/// Drop the logger (tests).
#[cfg(test)]
pub fn reset() {
    *slot() = None;
}

pub fn session_open() {
    if let Some(l) = handle() {
        l.event(Stream::Events, "session_open", &[]);
    }
}

pub fn session_close(why: &str) {
    if let Some(l) = handle() {
        l.event(
            Stream::Events,
            "session_close",
            &[("why", pb_log::scrub(why).into())],
        );
    }
}

/// A finished transfer; `dir` is "sent" or "received".
pub fn file_done(dir: &str, name: &str, bytes: u64, secs: f64) {
    if let Some(l) = handle() {
        l.transfer(dir, name, bytes, secs, true);
    }
}

pub fn file_failed(dir: &str, name: &str, why: &str) {
    if let Some(l) = handle() {
        l.event(
            Stream::Transfers,
            "file_failed",
            &[
                ("dir", dir.into()),
                ("file", pb_log::redact_name(name).into()),
                ("why", pb_log::scrub(why).into()),
            ],
        );
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// The logger is process-global: tests that use it take this lock.
    pub static SERIAL: Mutex<()> = Mutex::new(());

    pub fn fresh(name: &str) -> (PathBuf, MutexGuard<'static, ()>) {
        let g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let root = std::env::temp_dir().join(format!("pb-ffi-log-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        assert!(init(root.to_str().unwrap(), 0));
        (root, g)
    }

    fn today(root: &std::path::Path) -> String {
        let days: Vec<_> = std::fs::read_dir(root).unwrap().flatten().collect();
        assert_eq!(days.len(), 1, "one day folder");
        days[0].file_name().to_string_lossy().into_owned()
    }

    fn read(root: &std::path::Path, file: &str) -> String {
        handle().unwrap().flush();
        std::fs::read_to_string(root.join(today(root)).join(file)).unwrap_or_default()
    }

    #[test]
    fn app_events_become_json_lines() {
        let (root, _g) = fresh("event");
        event(
            "events",
            "app_start",
            r#"{"ver":"1.2","cold":true,"ms":840}"#,
        );
        event("perf", "mem", r#"{"rss_mb":120.5}"#);
        let e = read(&root, "events.jsonl");
        assert!(
            e.contains("\"k\":\"app_start\",\"ver\":\"1.2\",\"cold\":true,\"ms\":840"),
            "{e}"
        );
        assert!(read(&root, "perf.jsonl").contains("\"rss_mb\":120.500"));
    }

    #[test]
    fn nested_long_unknown_and_garbage_are_dropped() {
        let (root, _g) = fresh("reject");
        event("events", "a", r#"{"x":{"y":1}}"#);
        event("events", "b", &format!("{{\"x\":\"{}\"}}", "z".repeat(300)));
        event("nope", "c", "{}");
        event("events", "d", "not json");
        event("events", "ok", "{}");
        let e = read(&root, "events.jsonl");
        assert_eq!(e.lines().count(), 1, "{e}");
        assert!(e.contains("\"k\":\"ok\""));
    }

    #[test]
    fn session_and_transfer_events_use_the_pc_names_and_no_file_names() {
        let (root, _g) = fresh("pump");
        session_open();
        file_done("received", "Salary 2026 final.xlsx", 20_000_000, 4.0);
        file_failed("sent", "private.pdf", "no route to \"My PC\" at C:\\x");
        session_close("connection reset by \"Pixel\"");
        let ev = read(&root, "events.jsonl");
        assert!(ev.contains("\"k\":\"session_open\""), "{ev}");
        assert!(
            ev.contains("\"k\":\"session_close\",\"why\":\"connection reset by <q>\""),
            "{ev}"
        );
        let t = read(&root, "transfers.jsonl");
        assert!(t.contains("\"k\":\"file_done\",\"dir\":\"received\",\"file\":\".xlsx/17-32\",\"bytes\":20000000,\"secs\":4.000,\"mbps\":5.000"), "{t}");
        assert!(
            t.contains("\"k\":\"file_failed\",\"dir\":\"sent\",\"file\":\".pdf/9-16\""),
            "{t}"
        );
        let all = format!("{ev}{t}");
        for secret in ["Salary", "private", "My PC", "Pixel", "C:"] {
            assert!(!all.contains(secret), "{secret} leaked: {all}");
        }
    }

    #[test]
    fn init_failure_is_reported_and_logging_without_init_is_a_no_op() {
        let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let file = std::env::temp_dir().join(format!("pb-ffi-log-file-{}", std::process::id()));
        std::fs::write(&file, "x").unwrap();
        assert!(!init(file.join("sub").to_str().unwrap(), 0));
        *slot() = None;
        event("events", "x", "{}");
        session_open();
        let _ = std::fs::remove_file(file);
    }
}
