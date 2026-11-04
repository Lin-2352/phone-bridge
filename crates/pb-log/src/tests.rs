use super::*;
use std::sync::atomic::AtomicU64;

const DAY_MS: u64 = 86_400_000;
/// 2026-11-03T00:00:00Z
const T0: u64 = 1_793_664_000_000;

fn tmp(name: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!(
        "pb-log-{name}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = fs::remove_dir_all(&p);
    p
}

fn logger(name: &str, caps: Caps, start: u64) -> (Logger, Arc<AtomicU64>, PathBuf) {
    let root = tmp(name);
    let now = Arc::new(AtomicU64::new(start));
    let n2 = now.clone();
    let lg = Logger::open_with(
        root.clone(),
        caps,
        Arc::new(move || n2.load(Ordering::SeqCst)),
    )
    .unwrap();
    (lg, now, root)
}

fn read(lg: &Logger, day: &str, file: &str) -> String {
    lg.flush();
    fs::read_to_string(lg.day_dir(day).join(file)).unwrap_or_default()
}

fn small() -> Caps {
    Caps {
        per_day_bytes: 4000,
        total_bytes: 1 << 30,
        keep_days: 30,
        compress_after_days: 2,
    }
}

#[test]
fn day_for_handles_offsets_and_midnight() {
    assert_eq!(day_for(0, 0), "1970-01-01");
    // 18:29:59Z is 23:59:59 at +05:30; one second later it is the next day.
    let base = (T0 / 1000) as i64 - 86_400 + 18 * 3600 + 29 * 60 + 59;
    assert_eq!(day_for(base, 19_800), "2026-11-02");
    assert_eq!(day_for(base + 1, 19_800), "2026-11-03");
    assert_eq!(day_for(base + 1, 0), "2026-11-02");
    assert_eq!(day_for(-1, 0), "1969-12-31");
    assert_eq!(day_for(951_782_400, 0), "2000-02-29");
    assert_eq!(timefmt::iso_utc(T0 + 42), "2026-11-03T00:00:00.042Z");
}

#[test]
fn rolls_to_a_new_folder_at_local_midnight() {
    let (lg, now, _r) = logger("roll", Caps::pc(), T0 - 3_600_000 * 6 + 5_000);
    lg.set_tz_offset_secs(19_800);
    // 18:00:05Z of 11-02 is 23:30:05 local: still 11-02.
    now.store(T0 - 6 * 3_600_000 + 5_000, Ordering::SeqCst);
    lg.event(Stream::Events, "a", &[]);
    now.store(T0 - 6 * 3_600_000 + 40 * 60_000, Ordering::SeqCst); // 00:10 local
    lg.event(Stream::Events, "b", &[]);
    lg.flush();
    assert_eq!(lg.days(), ["2026-11-02", "2026-11-03"]);
    assert!(read(&lg, "2026-11-02", "events.jsonl").contains("\"k\":\"a\""));
    assert!(read(&lg, "2026-11-03", "events.jsonl").contains("\"k\":\"b\""));
}

#[test]
fn line_shape_is_one_json_object_with_t_and_k() {
    let (lg, _n, _r) = logger("shape", Caps::pc(), T0 + 1234);
    lg.event(
        Stream::Events,
        "session_open",
        &[
            ("peer", "a\"b\n".into()),
            ("n", 3i64.into()),
            ("ok", true.into()),
            ("x", 1.5f64.into()),
        ],
    );
    let s = read(&lg, "2026-11-03", "events.jsonl");
    assert_eq!(
        s,
        "{\"t\":\"2026-11-03T00:00:01.234Z\",\"k\":\"session_open\",\"peer\":\"a\\\"b\\n\",\"n\":3,\"ok\":true,\"x\":1.500}\n"
    );
}

#[test]
fn per_day_cap_drops_low_priority_and_writes_one_summary_per_minute() {
    let (lg, now, _r) = logger("cap", small(), T0 + 10_000);
    // Distinct lines, 10 per second so the rate limiter stays out of it.
    for i in 0..400u64 {
        now.store(T0 + 10_000 + i * 100, Ordering::SeqCst);
        lg.event(
            Stream::Perf,
            "p",
            &[("i", i.into()), ("pad", "x".repeat(40).into())],
        );
    }
    let s = read(&lg, "2026-11-03", "perf.jsonl");
    let size = s.len() as u64;
    assert!(size < 4000 + 500, "day stays near the cap, was {size}");
    let drops: Vec<&str> = s
        .lines()
        .filter(|l| l.contains("\"k\":\"dropped\""))
        .collect();
    assert_eq!(
        drops.len(),
        2,
        "first drop summarises at once, the rest at flush: {drops:?}"
    );
    // Within one minute no further summary appears, after a minute one does.
    now.store(T0 + 10_000 + 70_000, Ordering::SeqCst);
    lg.event(Stream::Perf, "p", &[("i", 9999i64.into())]);
    lg.event(Stream::Perf, "p", &[("i", 9998i64.into())]);
    let s = read(&lg, "2026-11-03", "perf.jsonl");
    assert!(!s.contains("9999"), "still full: dropped");
    // High priority lines still get through until the hard limit.
    lg.event(Stream::Events, "important", &[]);
    assert!(read(&lg, "2026-11-03", "events.jsonl").contains("important"));
}

#[test]
fn identical_consecutive_lines_collapse_with_a_count() {
    let (lg, _n, _r) = logger("dedupe", Caps::pc(), T0);
    for _ in 0..5 {
        lg.event(Stream::Events, "retry", &[("why", "timeout".into())]);
    }
    lg.event(Stream::Events, "other", &[]);
    lg.event(Stream::Events, "retry", &[("why", "timeout".into())]);
    let s = read(&lg, "2026-11-03", "events.jsonl");
    let lines: Vec<&str> = s.lines().collect();
    assert_eq!(lines.len(), 3, "{s}");
    assert!(lines[0].contains("\"n\":5"));
    assert!(!lines[2].contains("\"n\""));
}

#[test]
fn rate_limiter_caps_a_stream_and_counts_the_excess() {
    let (lg, _n, _r) = logger("rate", Caps::pc(), T0);
    for i in 0..200i64 {
        lg.event(Stream::Network, "x", &[("i", i.into())]);
    }
    let s = read(&lg, "2026-11-03", "network.jsonl");
    let (mut kept, mut dropped) = (0u64, 0u64);
    for l in s.lines() {
        if l.contains("\"k\":\"dropped\"") {
            assert!(l.contains("\"why\":\"rate\""));
            let n = l
                .split("\"n\":")
                .nth(1)
                .unwrap()
                .split([',', '}'])
                .next()
                .unwrap();
            dropped += n.parse::<u64>().unwrap();
        } else {
            kept += 1;
        }
    }
    assert_eq!(kept, u64::from(RATE_PER_SEC));
    assert_eq!(dropped, 150);
    // Other streams have their own budget.
    lg.event(Stream::Events, "fine", &[]);
    assert!(read(&lg, "2026-11-03", "events.jsonl").contains("fine"));
}

#[test]
fn sample_lets_one_through_per_interval() {
    let (lg, now, _r) = logger("sample", Caps::pc(), T0);
    assert!(lg.sample(Stream::Perf, "rssi", 10));
    now.store(T0 + 5_000, Ordering::SeqCst);
    assert!(!lg.sample(Stream::Perf, "rssi", 10));
    assert!(lg.sample(Stream::Perf, "other", 10));
    now.store(T0 + 10_000, Ordering::SeqCst);
    assert!(lg.sample(Stream::Perf, "rssi", 10));
}

fn mkday(root: &std::path::Path, day: &str, bytes: usize) {
    let d = root.join(day);
    fs::create_dir_all(&d).unwrap();
    fs::write(d.join("events.jsonl"), "x".repeat(bytes)).unwrap();
}

#[test]
fn retention_deletes_days_older_than_keep_days() {
    let (lg, _n, root) = logger("keep", Caps::pc(), T0);
    mkday(&root, "2026-09-01", 10);
    mkday(&root, "2026-10-03", 10); // exactly 31 days old
    mkday(&root, "2026-10-04", 10); // 30 days: kept
    mkday(&root, "2026-11-03", 10);
    fs::create_dir_all(root.join("not-a-day")).unwrap();
    lg.prune_at(T0 + 3_600_000);
    assert_eq!(lg.days(), ["2026-10-04", "2026-11-03"]);
    assert!(
        root.join("not-a-day").exists(),
        "foreign folders are left alone"
    );
}

#[test]
fn total_cap_deletes_oldest_days_first_but_never_today() {
    let caps = Caps {
        total_bytes: 250,
        compress_after_days: 100,
        ..Caps::pc()
    };
    let (lg, _n, root) = logger("total", caps, T0);
    mkday(&root, "2026-10-30", 100);
    mkday(&root, "2026-10-31", 100);
    mkday(&root, "2026-11-01", 100);
    mkday(&root, "2026-11-03", 100);
    lg.prune_at(T0);
    assert_eq!(lg.days(), ["2026-11-01", "2026-11-03"]);
    assert!(lg.size_bytes() <= 250);
}

#[test]
fn old_days_are_gzipped_and_today_is_not() {
    let (lg, _n, root) = logger("gzip", Caps::pc(), T0);
    mkday(&root, "2026-10-31", 5000);
    mkday(&root, "2026-11-02", 5000); // 1 day old: stays plain
    fs::write(root.join("2026-10-31").join("crash.txt"), "boom").unwrap();
    lg.prune_at(T0);
    let d = root.join("2026-10-31");
    assert!(d.join("events.jsonl.gz").exists());
    assert!(d.join("crash.txt.gz").exists());
    assert!(!d.join("events.jsonl").exists());
    assert!(
        fs::metadata(d.join("events.jsonl.gz")).unwrap().len() < 200,
        "5000 x's compress well"
    );
    assert!(root.join("2026-11-02").join("events.jsonl").exists());
    // Pruning again does not re-compress or fail.
    lg.prune_at(T0);
    assert!(d.join("events.jsonl.gz").exists());
}

#[test]
fn clear_all_removes_day_folders_and_keeps_the_root() {
    let (lg, _n, root) = logger("clear", Caps::pc(), T0);
    lg.event(Stream::Events, "a", &[]);
    mkday(&root, "2026-10-01", 10);
    assert!(lg.size_bytes() > 0);
    lg.clear_all().unwrap();
    assert!(root.exists());
    assert!(lg.days().is_empty());
    assert_eq!(lg.size_bytes(), 0);
    // Logging resumes into a fresh folder.
    lg.event(Stream::Events, "b", &[]);
    assert!(read(&lg, "2026-11-03", "events.jsonl").contains("\"k\":\"b\""));
}

#[test]
fn size_bytes_counts_what_is_on_disk_including_buffered_lines() {
    let (lg, _n, _r) = logger("size", Caps::pc(), T0);
    assert_eq!(lg.size_bytes(), 0);
    lg.event(Stream::Events, "a", &[]);
    let one = lg.size_bytes();
    assert!(one > 20, "{one}");
    lg.event(Stream::Events, "b", &[]);
    assert!(lg.size_bytes() > one);
}

#[test]
fn a_torn_last_line_does_not_corrupt_the_next_one() {
    let (lg, _n, root) = logger("torn", Caps::pc(), T0);
    let d = root.join("2026-11-03");
    fs::create_dir_all(&d).unwrap();
    fs::write(
        d.join("events.jsonl"),
        "{\"t\":\"x\",\"k\":\"ok\"}\n{\"t\":\"x\",\"k\":\"to",
    )
    .unwrap();
    lg.event(Stream::Events, "after", &[]);
    let s = read(&lg, "2026-11-03", "events.jsonl");
    let lines: Vec<&str> = s.lines().collect();
    assert_eq!(lines.len(), 3, "{s}");
    assert!(lines[2].starts_with('{') && lines[2].ends_with('}') && lines[2].contains("after"));
}

#[test]
fn redact_name_keeps_only_extension_and_a_length_bucket() {
    assert_eq!(redact_name("Q3 secret plan.PDF"), ".pdf/17-32");
    assert_eq!(redact_name("a.b"), ".b/1-8");
    assert_eq!(redact_name("README"), "/1-8");
    assert_eq!(redact_name("weird.ex t"), "/9-16");
    assert_eq!(redact_name(&"n".repeat(80)), "/65+");
    assert_eq!(redact_name("passw0rd.hunter2hunter2"), "/17-32");
}

#[test]
fn transfer_lines_never_contain_the_file_name() {
    let (lg, _n, _r) = logger("priv", Caps::pc(), T0);
    let secret = "hunter2-salary-2026";
    lg.transfer(
        "sent",
        &format!("{secret}.pdf"),
        10 * 1024 * 1024,
        2.0,
        true,
    );
    lg.transfer("received", secret, 1, 0.0, false);
    let s = read(&lg, "2026-11-03", "transfers.jsonl");
    assert!(!s.contains(secret) && !s.contains("salary"), "{s}");
    assert!(s.contains(".pdf/17-32"), "{s}");
    assert!(s.contains("\"mbps\":5.243"), "{s}");
}

#[test]
fn crash_notes_go_to_crash_txt_even_without_flush() {
    let (lg, _n, _r) = logger("crash", Caps::pc(), T0);
    lg.crash("panic at core.rs:1\nsecond line");
    let s = fs::read_to_string(lg.day_dir("2026-11-03").join("crash.txt")).unwrap();
    assert_eq!(
        s,
        "2026-11-03T00:00:00.000Z panic at core.rs:1 | second line\n"
    );
    lg.event(Stream::Crash, "fatal", &[("why", "x".into())]);
    assert!(read(&lg, "2026-11-03", "crash.txt").contains("fatal why=x"));
}

#[test]
fn disabled_logger_writes_nothing() {
    let (lg, _n, _r) = logger("off", Caps::pc(), T0);
    lg.set_enabled(false);
    lg.event(Stream::Events, "a", &[]);
    lg.crash("x");
    assert!(lg.days().is_empty());
    lg.set_enabled(true);
    lg.event(Stream::Events, "b", &[]);
    lg.flush();
    assert_eq!(lg.days().len(), 1);
}

#[test]
fn long_strings_are_cut() {
    let (lg, _n, _r) = logger("long", Caps::pc(), T0);
    lg.event(Stream::Events, "e", &[("s", "y".repeat(1000).into())]);
    let s = read(&lg, "2026-11-03", "events.jsonl");
    assert!(s.len() < 400, "{}", s.len());
}

#[test]
fn four_writers_lose_nothing_and_never_tear_lines() {
    let (lg, now, _r) = logger("threads", Caps::pc(), T0);
    let mut hs = Vec::new();
    for t in 0..4u64 {
        let lg = lg.clone();
        let now = now.clone();
        hs.push(std::thread::spawn(move || {
            for i in 0..40u64 {
                // Different second for each batch so the rate limiter allows all.
                now.fetch_add(100, Ordering::SeqCst);
                lg.event(Stream::Events, "w", &[("w", t.into()), ("i", i.into())]);
            }
        }));
    }
    for h in hs {
        h.join().unwrap();
    }
    now.store(T0 + DAY_MS / 2, Ordering::SeqCst);
    let s = read(&lg, &day_for((T0 / 1000) as i64, 0), "events.jsonl");
    let lines: Vec<&str> = s.lines().collect();
    assert_eq!(
        lines.len(),
        160,
        "{}
{s}",
        lines.len()
    );
    for l in lines {
        assert!(l.starts_with("{\"t\":") && l.ends_with('}'), "{l}");
    }
}

#[test]
fn flat_json_parses_scalars_and_skips_null() {
    let f =
        parse_flat(r#" {"a":"x\"y\u00e9","n":-3,"f":1.5,"ok":true,"z":null,"e":false} "#).unwrap();
    assert_eq!(
        f,
        vec![
            ("a".to_owned(), Value::s("x\"yé")),
            ("n".to_owned(), Value::Int(-3)),
            ("f".to_owned(), Value::Float(1.5)),
            ("ok".to_owned(), Value::Bool(true)),
            ("e".to_owned(), Value::Bool(false)),
        ]
    );
    assert_eq!(parse_flat("{}").unwrap(), vec![]);
}

#[test]
fn flat_json_rejects_nesting_long_values_and_garbage() {
    assert_eq!(parse_flat(r#"{"a":{"b":1}}"#), Err(FieldsError::Nested));
    assert_eq!(parse_flat(r#"{"a":[1]}"#), Err(FieldsError::Nested));
    let long = format!("{{\"a\":\"{}\"}}", "x".repeat(257));
    assert_eq!(parse_flat(&long), Err(FieldsError::TooLong));
    let ok = format!("{{\"a\":\"{}\"}}", "x".repeat(256));
    assert!(parse_flat(&ok).is_ok());
    for bad in [
        "",
        "[]",
        "{",
        r#"{"a"}"#,
        r#"{"a":1,}"#,
        r#"{"a":1} x"#,
        r#"{"a":tru}"#,
        r#"{a:1}"#,
    ] {
        assert_eq!(parse_flat(bad), Err(FieldsError::Syntax), "{bad}");
    }
    let many: String = (0..33)
        .map(|i| format!("\"k{i}\":1"))
        .collect::<Vec<_>>()
        .join(",");
    assert_eq!(
        parse_flat(&format!("{{{many}}}")),
        Err(FieldsError::TooMany)
    );
    assert_eq!(
        parse_flat(&format!("{{\"{}\":1}}", "k".repeat(65))),
        Err(FieldsError::TooLong)
    );
}

#[test]
fn scrub_removes_quotes_and_paths_and_cuts() {
    assert_eq!(
        scrub("cannot open \"my file.txt\" in C:\\a\\b now"),
        "cannot open <q> in <path>"
    );
    assert_eq!(scrub(&"x".repeat(500)).len(), 120);
}

#[test]
fn scrub_drops_the_tail_of_unquoted_paths_with_spaces() {
    let a = scrub("java.io.FileNotFoundException: /storage/emulated/0/Download/My Report.pdf: open failed: ENOENT");
    assert_eq!(
        a,
        "java.io.FileNotFoundException: <path> open failed: ENOENT"
    );
    assert!(!a.contains("Report"));
    let b = scrub(r"Access denied: C:\Users\Alex Kim");
    assert_eq!(b, "Access denied: <path>");
    assert!(!b.contains("Raj"));
}

#[test]
fn crash_notes_stop_at_a_size_cap_and_flush_held_lines() {
    let (lg, _n, root) = logger("crashcap", Caps::pc(), T0);
    // A held line reaches the disk when a crash note is written (a hard
    // kill right after would otherwise lose it).
    lg.event(Stream::Events, "last_thing", &[]);
    lg.crash("panic at a:1");
    let raw = fs::read_to_string(root.join("2026-11-03").join("events.jsonl")).unwrap_or_default();
    assert!(raw.contains("last_thing"), "held line not flushed: {raw:?}");
    // A panic loop cannot grow crash.txt without limit.
    for _ in 0..5000 {
        lg.crash(&"x".repeat(2000));
    }
    let len = fs::metadata(root.join("2026-11-03").join("crash.txt"))
        .unwrap()
        .len();
    assert!(len < 400 * 1024, "crash.txt grew to {len}");
}

#[test]
fn gzip_never_overwrites_an_existing_archive() {
    let (lg, _n, root) = logger("gzkeep", Caps::pc(), T0);
    mkday(&root, "2026-10-31", 5000);
    lg.prune_at(T0);
    let d = root.join("2026-10-31");
    let first = fs::read(d.join("events.jsonl.gz")).unwrap();
    // The same day gets new lines again (clock moved back), then is pruned.
    fs::write(d.join("events.jsonl"), "y".repeat(3000)).unwrap();
    lg.prune_at(T0);
    assert!(!d.join("events.jsonl").exists());
    assert_eq!(fs::read(d.join("events.jsonl.gz")).unwrap(), first);
    assert!(d.join("events.jsonl.1.gz").exists());
}
