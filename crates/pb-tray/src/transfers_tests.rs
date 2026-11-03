use super::*;
use pb_transport::files::Refusal;

fn s(n: u64) -> Duration {
    Duration::from_secs(n)
}

fn fail(t: &str) -> Failure {
    Failure {
        title: t.into(),
        hint: String::new(),
    }
}

// ---- lifecycle ----

#[test]
fn a_transfer_goes_waiting_active_done() {
    let mut l = Lifecycle::new(Dir::Send);
    assert_eq!(l.state(), &State::Waiting);
    assert!(l.is_live());
    l.progress(10, 100);
    assert_eq!(l.state(), &State::Active);
    l.finish();
    assert_eq!(l.state(), &State::Done);
    assert!(!l.is_live());
}

#[test]
fn an_empty_file_can_finish_without_any_progress() {
    let mut l = Lifecycle::new(Dir::Send);
    l.finish();
    assert_eq!(l.state(), &State::Done);
}

#[test]
fn cancel_is_final_and_a_second_cancel_changes_nothing() {
    let mut l = Lifecycle::new(Dir::Send);
    l.progress(1, 10);
    assert!(l.cancel(CancelBy::User));
    assert_eq!(l.state(), &State::Cancelled(CancelBy::User));
    assert!(!l.cancel(CancelBy::Pause));
    assert_eq!(l.state(), &State::Cancelled(CancelBy::User));
    // Late progress or failure from the dying transfer is ignored.
    l.progress(5, 10);
    assert!(!l.fail(fail("late")));
    assert_eq!(l.state(), &State::Cancelled(CancelBy::User));
}

#[test]
fn cancel_after_done_changes_nothing() {
    let mut l = Lifecycle::new(Dir::Receive);
    l.finish();
    assert!(!l.cancel(CancelBy::User));
    assert_eq!(l.state(), &State::Done);
}

#[test]
fn a_file_that_arrived_is_done_even_if_the_cancel_raced_it() {
    let mut l = Lifecycle::new(Dir::Receive);
    l.cancel(CancelBy::User);
    l.finish();
    assert_eq!(l.state(), &State::Done);
}

#[test]
fn failure_is_final_and_keeps_its_text() {
    let mut l = Lifecycle::new(Dir::Send);
    assert!(l.fail(fail("Couldn't reach Pixel")));
    assert_eq!(l.state(), &State::Failed(fail("Couldn't reach Pixel")));
    l.progress(1, 2);
    assert!(!l.cancel(CancelBy::User));
    assert_eq!(l.state(), &State::Failed(fail("Couldn't reach Pixel")));
}

#[test]
fn a_failed_or_cancelled_send_can_be_retried_but_not_a_done_one() {
    let mut l = Lifecycle::new(Dir::Send);
    l.fail(fail("x"));
    assert!(l.can_retry());
    assert!(l.retry());
    assert_eq!(l.state(), &State::Waiting);
    l.cancel(CancelBy::Pause);
    assert!(l.can_retry());
    assert!(l.retry());
    l.finish();
    assert!(!l.can_retry());
    assert!(!l.retry());
    assert_eq!(l.state(), &State::Done);
}

#[test]
fn a_live_transfer_cannot_be_retried() {
    let mut l = Lifecycle::new(Dir::Send);
    assert!(!l.retry());
    l.progress(1, 2);
    assert!(!l.retry());
    assert_eq!(l.state(), &State::Active);
}

#[test]
fn a_receive_is_never_retried_from_the_pc() {
    let mut l = Lifecycle::new(Dir::Receive);
    l.fail(fail("x"));
    assert!(!l.can_retry());
    assert!(!l.retry());
}

// ---- speed, ETA ----

#[test]
fn no_speed_or_eta_in_the_first_second() {
    let t0 = Instant::now();
    let mut m = Meter::new(t0);
    m.update(t0 + Duration::from_millis(500), 5_000_000);
    assert_eq!(m.speed(t0 + Duration::from_millis(500)), None);
    assert_eq!(
        m.eta(t0 + Duration::from_millis(500), 5_000_000, 100_000_000),
        None
    );
}

#[test]
fn steady_progress_reads_as_its_rate() {
    let t0 = Instant::now();
    let mut m = Meter::new(t0);
    for i in 1..=20u64 {
        m.update(t0 + Duration::from_millis(250 * i), 1_000_000 * i);
    }
    let now = t0 + s(5);
    let v = m.speed(now).expect("speed");
    assert!((v - 4_000_000.0).abs() < 200_000.0, "{v}");
    let eta = m.eta(now, 20_000_000, 60_000_000).expect("eta");
    assert!((9..=11).contains(&eta.as_secs()), "{eta:?}");
}

#[test]
fn one_burst_does_not_make_the_speed_jump() {
    let t0 = Instant::now();
    let mut m = Meter::new(t0);
    for i in 1..=8u64 {
        m.update(t0 + Duration::from_millis(250 * i), 1_000_000 * i);
    }
    // A single sample 50 times faster.
    m.update(t0 + Duration::from_millis(2250), 8_000_000 + 12_500_000);
    let v = m.speed(t0 + Duration::from_millis(2250)).unwrap();
    assert!(v < 12_000_000.0, "smoothed: {v}");
}

#[test]
fn a_stalled_transfer_reads_zero_and_has_no_eta() {
    let t0 = Instant::now();
    let mut m = Meter::new(t0);
    for i in 1..=8u64 {
        m.update(t0 + Duration::from_millis(250 * i), 1_000_000 * i);
    }
    let later = t0 + s(10);
    m.update(later, 8_000_000);
    assert_eq!(m.speed(later), Some(0.0));
    assert_eq!(m.eta(later, 8_000_000, 50_000_000), None);
}

#[test]
fn eta_is_zero_when_complete_and_none_for_an_empty_total() {
    let t0 = Instant::now();
    let mut m = Meter::new(t0);
    for i in 1..=8u64 {
        m.update(t0 + Duration::from_millis(250 * i), 1_000_000 * i);
    }
    let now = t0 + s(2);
    assert_eq!(m.eta(now, 8_000_000, 8_000_000), Some(Duration::ZERO));
    assert_eq!(m.eta(now, 0, 0), None);
}

// ---- formatting ----

#[test]
fn bytes_use_the_right_unit() {
    assert_eq!(fmt_bytes(0), "0 B");
    assert_eq!(fmt_bytes(512), "512 B");
    assert_eq!(fmt_bytes(1536), "1.5 KB");
    assert_eq!(fmt_bytes(100 * 1024), "100 KB");
    assert_eq!(fmt_bytes(5_452_595), "5.2 MB");
    assert_eq!(fmt_bytes(3 * 1024 * 1024 * 1024), "3.0 GB");
}

#[test]
fn a_tiny_file_does_not_read_as_zero_megabytes() {
    assert_ne!(fmt_bytes(40_000), "0.0 MB");
    assert_eq!(fmt_bytes(40_000), "39 KB");
}

#[test]
fn speed_and_eta_read_naturally() {
    assert_eq!(fmt_speed(5_452_595.0), "5.2 MB/s");
    assert_eq!(fmt_speed(800.0), "800 B/s");
    assert_eq!(fmt_eta(s(8)), "8 s left");
    assert_eq!(fmt_eta(Duration::ZERO), "almost done");
    assert_eq!(fmt_eta(s(125)), "2 min 5 s left");
    assert_eq!(fmt_eta(s(3 * 3600 + 120)), "3 h 2 min left");
}

#[test]
fn percent_is_clamped_and_safe_on_empty() {
    assert_eq!(percent(0, 0), 0);
    assert_eq!(percent(50, 200), 25);
    assert_eq!(percent(300, 200), 100);
    assert_eq!(percent(199, 200), 99);
}

// ---- error wording ----

fn ctx(refusal: Option<Refusal>) -> Ctx<'static> {
    Ctx {
        peer: "Pixel",
        file: "a.zip",
        folder: r"D:\Inbox",
        refusal,
    }
}

#[test]
fn a_full_disk_names_the_folder_and_the_next_step() {
    let f = describe(Dir::Receive, &Error::TooLarge, &ctx(Some(Refusal::NoSpace)));
    assert!(f.title.contains(r"D:\Inbox"), "{f:?}");
    assert!(f.title.to_lowercase().contains("space"), "{f:?}");
    assert!(f.hint.contains("Settings"), "{f:?}");
}

#[test]
fn a_file_over_the_cap_says_so() {
    let f = describe(Dir::Receive, &Error::TooLarge, &ctx(Some(Refusal::OverCap)));
    assert!(
        f.title.contains("a.zip") && f.title.contains("4 GB"),
        "{f:?}"
    );
}

#[test]
fn a_hash_mismatch_says_nothing_was_saved_and_to_resend() {
    let f = describe(Dir::Receive, &Error::HashMismatch, &ctx(None));
    assert!(f.title.contains("a.zip"), "{f:?}");
    assert!(f.hint.contains("again"), "{f:?}");
}

#[test]
fn a_vanished_sender_is_named() {
    for e in [Error::Closed, Error::Timeout, Error::Transport("x".into())] {
        let f = describe(Dir::Receive, &e, &ctx(None));
        assert!(
            f.title.contains("Pixel") && f.title.contains("a.zip"),
            "{f:?}"
        );
        assert!(!f.title.contains("transport error"), "{f:?}");
    }
}

#[test]
fn a_save_error_points_at_the_folder() {
    let f = describe(Dir::Receive, &Error::Io("denied".into()), &ctx(None));
    assert!(f.title.contains(r"D:\Inbox"), "{f:?}");
    assert!(
        !f.title.contains("denied"),
        "raw error text stays out: {f:?}"
    );
}

#[test]
fn an_unreachable_phone_says_what_to_check() {
    for e in [Error::Closed, Error::Transport("x".into())] {
        let f = describe(Dir::Send, &e, &ctx(None));
        assert!(f.title.contains("Pixel"), "{f:?}");
        assert!(
            f.hint.contains("Wi-Fi") && f.hint.contains("Relay"),
            "{f:?}"
        );
    }
    let t = describe(Dir::Send, &Error::Timeout, &ctx(None));
    assert!(t.title.contains("didn't answer"), "{t:?}");
}

#[test]
fn a_declined_send_is_not_phrased_as_a_fault() {
    let f = describe(Dir::Send, &Error::Declined, &ctx(None));
    assert!(
        f.title.contains("Pixel") && f.title.contains("declined"),
        "{f:?}"
    );
}

#[test]
fn a_peer_that_could_not_save_is_told_apart_from_a_local_read_error() {
    let saved = Error::Io("the other device could not save the file".into());
    let f = describe(Dir::Send, &saved, &ctx(None));
    assert!(f.title.contains("couldn't save"), "{f:?}");
    let local = describe(Dir::Send, &Error::Io("denied".into()), &ctx(None));
    assert!(local.title.contains("read"), "{local:?}");
}

#[test]
fn cancels_are_worded_by_who_cancelled() {
    let me = describe(Dir::Send, &Error::Cancelled, &ctx(None));
    assert_eq!(me.title, "Cancelled a.zip");
    let them = describe(Dir::Send, &Error::PeerCancelled, &ctx(None));
    assert!(
        them.title.contains("Pixel") && them.title.contains("cancelled"),
        "{them:?}"
    );
}

#[test]
fn every_error_gets_a_non_empty_title() {
    let all = [
        Error::NotPaired,
        Error::UnknownPeer,
        Error::Rejected,
        Error::Timeout,
        Error::Closed,
        Error::Declined,
        Error::TooLarge,
        Error::PeerChanged,
        Error::HashMismatch,
        Error::Cancelled,
        Error::PeerCancelled,
        Error::Io("x".into()),
        Error::Protocol("x".into()),
        Error::Transport("x".into()),
    ];
    for dir in [Dir::Send, Dir::Receive] {
        for e in &all {
            assert!(
                !describe(dir, e, &ctx(None)).title.is_empty(),
                "{dir:?} {e:?}"
            );
        }
    }
}

// ---- registry ----

fn nt(dir: Dir, name: &str, batch: Option<Batch>) -> NewTransfer {
    NewTransfer {
        dir,
        peer: "Pixel".into(),
        name: name.into(),
        size: 100,
        path: Some(PathBuf::from(format!(r"C:\{name}"))),
        batch,
    }
}

#[test]
fn pause_cancels_every_live_transfer_fires_the_handles_and_leaves_done_ones() {
    let now = Instant::now();
    let mut r = Registry::new();
    let (a, ca) = r.add(nt(Dir::Send, "a", None), now);
    let (b, cb) = r.add(nt(Dir::Receive, "b", None), now);
    let (c, cc) = r.add(nt(Dir::Send, "c", None), now);
    r.finish(c, None, now);
    assert_eq!(r.active_count(), 2);
    assert_eq!(r.cancel_all(CancelBy::Pause, now), 2);
    assert!(ca.is_cancelled() && cb.is_cancelled());
    assert!(!cc.is_cancelled());
    assert_eq!(r.active_count(), 0);
    assert_eq!(
        r.row(a, now).unwrap().state,
        State::Cancelled(CancelBy::Pause)
    );
    assert_eq!(
        r.row(b, now).unwrap().state,
        State::Cancelled(CancelBy::Pause)
    );
    assert_eq!(r.row(c, now).unwrap().state, State::Done);
    // A late error from the dying transfer cannot overwrite the pause.
    r.fail(a, fail("Couldn't reach Pixel"), now);
    assert_eq!(
        r.row(a, now).unwrap().state,
        State::Cancelled(CancelBy::Pause)
    );
    // Pausing again does nothing.
    assert_eq!(r.cancel_all(CancelBy::Pause, now), 0);
}

#[test]
fn pause_refuses_new_sends_and_quiet_commands_stay_quiet() {
    use crate::core::Cmd;
    assert!(crate::pause::needs_network(&Cmd::SendFiles(vec![])));
    assert!(!crate::pause::is_quiet(&Cmd::SendFiles(vec![])));
}

#[test]
fn cancelling_one_file_of_a_batch_stops_the_rest_but_a_peer_cancel_does_not() {
    let now = Instant::now();
    let mut r = Registry::new();
    let b1 = Batch {
        id: 7,
        index: 1,
        total: 3,
    };
    let (a, _) = r.add(nt(Dir::Send, "a", Some(b1)), now);
    assert!(!r.batch_stopped(7));
    r.cancel(a, CancelBy::Peer, now);
    assert!(!r.batch_stopped(7));
    let b2 = Batch {
        id: 8,
        index: 2,
        total: 3,
    };
    let (c, _) = r.add(nt(Dir::Send, "c", Some(b2)), now);
    r.cancel(c, CancelBy::User, now);
    assert!(r.batch_stopped(8));
}

#[test]
fn a_multi_file_row_reads_n_of_m_and_a_single_one_does_not() {
    let now = Instant::now();
    let mut r = Registry::new();
    let (a, _) = r.add(
        nt(
            Dir::Send,
            "a",
            Some(Batch {
                id: 1,
                index: 2,
                total: 5,
            }),
        ),
        now,
    );
    let (b, _) = r.add(
        nt(
            Dir::Send,
            "b",
            Some(Batch {
                id: 2,
                index: 1,
                total: 1,
            }),
        ),
        now,
    );
    assert_eq!(
        r.row(a, now).unwrap().batch_label().as_deref(),
        Some("2 of 5")
    );
    assert_eq!(r.row(b, now).unwrap().batch_label(), None);
}

#[test]
fn retry_gives_a_fresh_handle_and_resets_progress() {
    let now = Instant::now();
    let mut r = Registry::new();
    let (a, old) = r.add(nt(Dir::Send, "a", None), now);
    r.set_progress(a, 50, 100);
    r.fail(a, fail("x"), now);
    let (new, info) = r.retry(a, now).expect("retry");
    assert!(!new.is_cancelled() && !old.is_cancelled());
    assert_eq!(info.path, PathBuf::from(r"C:\a"));
    let row = r.row(a, now).unwrap();
    assert_eq!((row.state.clone(), row.done), (State::Waiting, 0));
    // Not twice while live, and never for a receive.
    assert!(r.retry(a, now).is_none());
    let (b, _) = r.add(nt(Dir::Receive, "b", None), now);
    r.fail(b, fail("x"), now);
    assert!(r.retry(b, now).is_none());
}

#[test]
fn dismiss_removes_finished_rows_only_and_done_rows_expire() {
    let now = Instant::now();
    let mut r = Registry::new();
    let (a, _) = r.add(nt(Dir::Send, "a", None), now);
    r.dismiss(a);
    assert_eq!(r.rows(now).len(), 1);
    r.finish(a, None, now);
    r.prune_done(now + s(5), s(12));
    assert_eq!(r.rows(now).len(), 1);
    r.prune_done(now + s(13), s(12));
    assert!(r.is_empty());
}

#[test]
fn a_received_file_row_offers_open_once_saved_and_the_status_reads_done() {
    let now = Instant::now();
    let mut r = Registry::new();
    let mut t = nt(Dir::Receive, "b", None);
    t.path = None;
    let (b, _) = r.add(t, now);
    assert!(!r.row(b, now).unwrap().can_open);
    r.finish(b, Some(PathBuf::from(r"D:\Inbox\b")), now);
    let row = r.row(b, now).unwrap();
    assert!(row.can_open);
    assert_eq!(row.percent(), 100);
    assert!(row.status_line().starts_with("Done"));
}

#[test]
fn the_status_line_shows_bytes_speed_and_eta() {
    let t0 = Instant::now();
    let mut r = Registry::new();
    let (a, _) = r.add(nt(Dir::Send, "a", None), t0);
    for i in 1..=8u64 {
        r.set_progress(a, i * 10, 100);
        r.tick(t0 + Duration::from_millis(250 * i));
    }
    let line = r.row(a, t0 + s(2)).unwrap().status_line();
    assert!(
        line.contains(" of ") && line.contains("/s") && line.contains("left"),
        "{line}"
    );
}

#[test]
fn file_actions_need_an_existing_file() {
    assert_eq!(
        file_actions(None),
        FileActions {
            open: false,
            show_in_folder: false
        }
    );
    let f = std::env::temp_dir().join(format!("pb-act-{}", std::process::id()));
    std::fs::write(&f, b"x").unwrap();
    assert!(file_actions(Some(&f)).open);
    std::fs::remove_file(&f).unwrap();
    assert!(!file_actions(Some(&f)).show_in_folder);
}
