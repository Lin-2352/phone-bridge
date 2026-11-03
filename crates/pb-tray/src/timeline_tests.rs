use super::*;

struct Plain;
impl SeedWrapper for Plain {
    fn wrap(&self, plain: &[u8]) -> io::Result<Vec<u8>> {
        let mut v = vec![0xA5];
        v.extend_from_slice(plain);
        Ok(v)
    }
    fn unwrap(&self, sealed: &[u8]) -> io::Result<Vec<u8>> {
        match sealed.split_first() {
            Some((0xA5, rest)) => Ok(rest.to_vec()),
            _ => Err(io::Error::new(io::ErrorKind::InvalidData, "not sealed")),
        }
    }
}

fn tmpdir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("pb-timeline-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn pic(w: u32, h: u32, size: u64) -> Entry {
    Entry {
        dims: Some((w, h)),
        size: Some(size),
        ..blank(Kind::ClipImage, Dir::ToPc, "Phone")
    }
}

fn collect_log(s: &mut Store) -> std::sync::Arc<Mutex<Vec<String>>> {
    let msgs = std::sync::Arc::new(Mutex::new(Vec::<String>::new()));
    let m = msgs.clone();
    s.set_logger(move |l| m.lock().unwrap().push(l.to_owned()));
    msgs
}

#[test]
fn round_trips_all_kinds_with_tabs_and_newlines_in_names() {
    let mut f = file_entry(
        "Pixel\t8",
        Dir::ToPhone,
        "a\tb\nc\\d.txt",
        42,
        Some(Path::new("C:\\x\\a.txt")),
    );
    f.at = 100;
    let mut t = clip_text_entry("Phone", Dir::ToPc, "hello", false);
    t.at = 101;
    let mut i = clip_image_entry("Phone", Dir::ToPhone, &[]);
    i.dims = Some((1080, 2412));
    i.at = 102;
    let mut n = notification_entry("Phone", "Chat \u{1f4ac}\r\n");
    n.at = 103;
    let mut empty_name = f.clone();
    empty_name.name = Some(String::new());
    let mut a = audio_entry("Phone", Dir::ToPc, "Phone audio on this PC", 252);
    a.at = 104;
    assert_eq!(summary(&a), "Phone audio on this PC, 4 min 12 s");
    assert_eq!(a.size, None);
    assert_eq!(a.path, None);
    let v = vec![f, t, i, n, empty_name, a];
    assert_eq!(parse(&encode(&v)).unwrap(), v);
    assert!(parse(b"tl2\n").is_err());
    assert!(parse(b"tl1\nF\tp\n").is_err());
}

#[test]
fn keeps_newest_1000_and_drops_older_than_30_days() {
    let now = 100 * 86400;
    let mk = |at| Entry {
        at,
        ..clip_text_entry("p", Dir::ToPc, "x", false)
    };
    let mut v: Vec<Entry> = (0..1150).map(|i| mk(now - 10 - i)).collect();
    v.extend((0..50).map(|i| mk(now - 31 * 86400 - i)));
    prune(&mut v, now);
    assert_eq!(v.len(), 1000);
    assert_eq!(v[0].at, now - 10);
    assert_eq!(v[999].at, now - 10 - 999);
    assert!(v.iter().all(|e| e.at >= now - MAX_AGE_SECS));
}

#[test]
fn never_stores_text_or_titles() {
    let text = format!("secret words{}", "x".repeat(130));
    let mut e = clip_text_entry("pc", Dir::ToPc, &text, false);
    e.at = 5;
    assert_eq!(e.chars, Some(142));
    let s = String::from_utf8(encode(&[e])).unwrap();
    assert!(s.contains("142"));
    assert!(!s.contains("secret"));
    // A notification keeps the app name and nothing else.
    let n = notification_entry("pc", "WhatsApp");
    assert!(n.name.is_none() && n.path.is_none() && n.chars.is_none());
}

#[test]
fn summaries_read_well() {
    assert_eq!(
        summary(&pic(1080, 2412, 1_200_000)),
        "Picture, 1080 \u{d7} 2412, 1.2 MB"
    );
    let mut t = clip_text_entry("p", Dir::ToPc, &"a".repeat(142), false);
    assert_eq!(summary(&t), "Text, 142 characters");
    t.chars = Some(1);
    assert_eq!(summary(&t), "Text, 1 character");
    assert_eq!(
        summary(&file_entry("p", Dir::ToPc, "photo.jpg", 3_400_000, None)),
        "photo.jpg, 3.4 MB"
    );
    assert_eq!(
        summary(&notification_entry("p", "WhatsApp")),
        "WhatsApp notification"
    );
}

#[test]
fn png_dimensions_come_from_the_header() {
    let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
    png.extend_from_slice(&1080u32.to_be_bytes());
    png.extend_from_slice(&2412u32.to_be_bytes());
    let e = clip_image_entry("p", Dir::ToPc, &png);
    assert_eq!(e.dims, Some((1080, 2412)));
    assert_eq!(e.size, Some(png.len() as u64));
    assert_eq!(clip_image_entry("p", Dir::ToPc, b"junk").dims, None);
}

#[test]
fn corrupt_file_is_replaced_but_a_missing_one_is_just_empty() {
    let d = tmpdir("corrupt");
    let mut s = Store::open(&d, Box::new(Plain));
    let msgs = collect_log(&mut s);
    assert!(s.entries().is_empty());
    assert!(
        msgs.lock().unwrap().is_empty(),
        "missing file is not an error"
    );
    std::fs::write(d.join(FILE), b"garbage").unwrap();
    let mut s = Store::open(&d, Box::new(Plain));
    let msgs = collect_log(&mut s);
    assert!(s.entries().is_empty());
    assert_eq!(msgs.lock().unwrap().len(), 1);
    assert!(!d.join(FILE).exists());
    s.add(clip_text_entry("p", Dir::ToPc, "x", false));
    s.flush();
    assert_eq!(Store::open(&d, Box::new(Plain)).entries().len(), 1);
}

#[test]
fn leftover_tmp_is_removed_on_open() {
    let d = tmpdir("tmp");
    let tmp = d.join("timeline.sealed.tmp");
    std::fs::write(&tmp, b"half").unwrap();
    let _ = Store::open(&d, Box::new(Plain));
    assert!(!tmp.exists());
}

#[test]
fn exit_inside_the_debounce_still_saves_the_last_entry() {
    let d = tmpdir("debounce");
    let mut s = Store::open(&d, Box::new(Plain));
    s.add(clip_text_entry("p", Dir::ToPc, "one", false));
    s.flush_if_due(Instant::now()); // written now
    let mut last = clip_text_entry("p", Dir::ToPc, "two", false);
    last.chars = Some(777);
    s.add(last);
    s.flush_if_due(Instant::now()); // inside the second: held back
    assert_eq!(Store::open(&d, Box::new(Plain)).entries().len(), 1);
    s.flush();
    let r = Store::open(&d, Box::new(Plain));
    assert_eq!(r.entries().len(), 2);
    assert_eq!(r.entries()[0].chars, Some(777));
}

#[test]
fn at_most_one_write_per_second() {
    let d = tmpdir("rate");
    let mut s = Store::open(&d, Box::new(Plain));
    let t0 = Instant::now();
    s.add(clip_text_entry("p", Dir::ToPc, "a", false));
    assert!(!d.join(FILE).exists(), "add does no file I/O");
    s.flush_if_due(t0);
    assert_eq!(Store::open(&d, Box::new(Plain)).entries().len(), 1);
    s.add(clip_text_entry("p", Dir::ToPc, "b", false));
    std::thread::sleep(Duration::from_millis(10));
    s.add(clip_text_entry("p", Dir::ToPc, "c", false));
    s.flush_if_due(t0 + Duration::from_millis(500));
    assert_eq!(Store::open(&d, Box::new(Plain)).entries().len(), 1);
    // The trailing write: held-back entries land once a second has passed.
    s.flush_if_due(t0 + Duration::from_millis(1100));
    assert_eq!(Store::open(&d, Box::new(Plain)).entries().len(), 3);
}

#[test]
fn flush_when_clean_does_not_touch_the_file() {
    let d = tmpdir("clean");
    let mut s = Store::open(&d, Box::new(Plain));
    s.add(clip_text_entry("p", Dir::ToPc, "a", false));
    s.flush();
    std::fs::remove_file(d.join(FILE)).unwrap();
    s.flush_if_due(Instant::now() + Duration::from_secs(5));
    s.flush();
    assert!(!d.join(FILE).exists());
}

#[test]
fn a_read_error_keeps_the_file_and_never_overwrites_it() {
    let d = tmpdir("readerr");
    // A directory where the file should be: reading it fails, not NotFound.
    std::fs::create_dir_all(d.join(FILE)).unwrap();
    let mut s = Store::open(&d, Box::new(Plain));
    let msgs = collect_log(&mut s);
    assert!(msgs.lock().unwrap()[0].starts_with("TIMELINE read failed: "));
    s.add(clip_text_entry("p", Dir::ToPc, "a", false));
    s.flush();
    assert_eq!(s.entries().len(), 1, "still recorded in memory");
    assert!(d.join(FILE).is_dir(), "not replaced");
    assert_eq!(msgs.lock().unwrap().len(), 1, "no write attempted");
}

#[test]
fn switching_history_off_deletes_it() {
    let d = tmpdir("off");
    let mut s = Store::open(&d, Box::new(Plain));
    s.add(clip_text_entry("p", Dir::ToPc, "a", false));
    s.flush();
    assert!(d.join(FILE).exists());
    s.set_enabled(false);
    assert!(!d.join(FILE).exists());
    assert!(s.entries().is_empty());
    s.add(clip_text_entry("p", Dir::ToPc, "b", false));
    s.flush();
    assert!(s.entries().is_empty() && !d.join(FILE).exists());
    s.set_enabled(true);
    s.add(clip_text_entry("p", Dir::ToPc, "c", false));
    assert_eq!(s.entries().len(), 1);
}

#[test]
fn failed_write_logs_and_keeps_data() {
    let d = tmpdir("fail");
    let mut s = Store::open(&d, Box::new(Plain));
    let msgs = collect_log(&mut s);
    // A directory where the file should be makes the swap fail.
    std::fs::create_dir_all(d.join(FILE).join("blocker")).unwrap();
    s.add(clip_text_entry("p", Dir::ToPc, "a", false));
    s.flush();
    assert!(msgs.lock().unwrap()[0].starts_with("TIMELINE write failed: "));
    assert_eq!(s.entries().len(), 1);
}

#[test]
fn version_changes_with_the_entries_only() {
    let d = tmpdir("version");
    let mut s = Store::open(&d, Box::new(Plain));
    let v0 = s.version();
    s.add(clip_text_entry("Pixel", Dir::ToPc, "hi", false));
    let v1 = s.version();
    assert_ne!(v1, v0, "add");
    // Writing to disk does not change what is shown.
    s.flush();
    s.flush_if_due(Instant::now());
    assert_eq!(s.version(), v1);
    s.clear();
    let v2 = s.version();
    assert_ne!(v2, v1, "clear");
    s.add(clip_text_entry("Pixel", Dir::ToPc, "hi", false));
    let v3 = s.version();
    s.set_enabled(false);
    assert_ne!(s.version(), v3, "history off clears");
    // Off: adds are dropped, nothing changes.
    let v4 = s.version();
    s.add(clip_text_entry("Pixel", Dir::ToPc, "hi", false));
    assert_eq!(s.version(), v4);
}

/// The tray's 1 s timer runs for a write only while one is owed and possible.
#[test]
fn wants_a_flush_only_while_an_add_is_unwritten() {
    let d = tmpdir("wants");
    let mut s = Store::open(&d, Box::new(Plain));
    s.set_enabled(true);
    assert!(!s.wants_flush(), "nothing added yet");
    s.add(pic(10, 10, 100));
    assert!(s.wants_flush());
    s.flush();
    assert!(!s.wants_flush(), "written");
    s.add(pic(10, 10, 100));
    s.set_enabled(false);
    assert!(!s.wants_flush(), "history off: nothing to write");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn a_sensitive_clip_keeps_no_length() {
    let e = clip_text_entry("Pixel", Dir::ToPc, "hunter2", true);
    assert_eq!(e.chars, None);
    assert_eq!(summary(&e), "Text");
    assert!(parse(&encode(std::slice::from_ref(&e))).unwrap()[0]
        .chars
        .is_none());
    // An ordinary one still counts.
    let e = clip_text_entry("Pixel", Dir::ToPhone, "hello", false);
    assert_eq!(summary(&e), "Text, 5 characters");
}

#[test]
fn failed_writes_back_off_to_a_minute() {
    let secs: Vec<u64> = (0..10).map(|n| retry_after(n).as_secs()).collect();
    assert_eq!(secs, [1, 1, 2, 4, 8, 16, 32, 60, 60, 60]);
    assert_eq!(retry_after(u32::MAX), Duration::from_secs(60));
}

#[test]
fn a_failing_write_is_retried_on_the_back_off_and_logged_once() {
    let d = tmpdir("backoff");
    let mut s = Store::open(&d, Box::new(Plain));
    let msgs = collect_log(&mut s);
    std::fs::create_dir_all(d.join(FILE).join("blocker")).unwrap();
    s.add(clip_text_entry("p", Dir::ToPc, "a", false));
    let t0 = Instant::now();
    s.flush_if_due(t0); // failure 1: next try after 1 s
    s.flush_if_due(t0 + Duration::from_millis(1100)); // failure 2: then 2 s
    s.flush_if_due(t0 + Duration::from_millis(2200)); // too early
    s.flush_if_due(t0 + Duration::from_millis(3200)); // failure 3: then 4 s
    s.flush_if_due(t0 + Duration::from_millis(6000)); // too early
    assert_eq!(s.write_failures, 3);
    assert_eq!(msgs.lock().unwrap().len(), 1, "{:?}", msgs.lock().unwrap());
    assert!(s.wants_flush(), "the data is still unwritten");
    // The blocker goes: the next due write succeeds and the streak ends.
    std::fs::remove_dir_all(d.join(FILE)).unwrap();
    s.flush_if_due(t0 + Duration::from_millis(7300));
    assert!(!s.wants_flush());
    assert_eq!(s.write_failures, 0);
    assert_eq!(Store::open(&d, Box::new(Plain)).entries().len(), 1);
}

#[test]
fn a_notification_row_always_names_a_device() {
    assert_eq!(notification_peer("Pixel 8"), "Pixel 8");
    assert_eq!(notification_peer(""), "your phone");
    assert_eq!(notification_peer("  "), "your phone");
}
