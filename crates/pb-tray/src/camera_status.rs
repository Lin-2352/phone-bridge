//! What the phone-camera window shows: one row per thing that has to be true
//! for an app to see "Phone Bridge Camera", each with a mark and, when it
//! fails, the one-line fix. Pure: the window only draws these.

/// Windows' machine-wide "Camera access" switch (and the user's own).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Access {
    Allow,
    Deny,
    Unknown,
}

/// Everything the rows are made from.
#[derive(Clone, Debug)]
pub struct Inputs {
    pub phone: Option<String>,
    pub installed: bool,
    pub update_ready: bool,
    pub access: Access,
    pub streaming: bool,
    pub fps: u32,
    /// `None` until measured; `Some(listed)` after.
    pub listed: Option<bool>,
    /// The picture the phone sends is (nearly) black.
    pub dark: bool,
    /// The window read pictures from the camera the way an app does:
    /// `None` not tried, `Some(frames per second)` after.
    pub preview_fps: Option<u32>,
    /// The phone has been asked and has not answered yet (its consent window
    /// is open).
    pub pending: bool,
    /// Why the last start failed, until a start works or the camera is repaired.
    pub start_error: Option<String>,
    /// The lens in use while streaming (0 back, 1 front); `None` when unknown.
    pub lens: Option<u8>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mark {
    Ok,
    Bad,
    /// Not applicable yet (nothing is streaming).
    Idle,
    /// In progress or an update is waiting.
    Wait,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Row {
    pub label: &'static str,
    pub mark: Mark,
    pub detail: String,
}

fn row(label: &'static str, mark: Mark, detail: impl Into<String>) -> Row {
    Row {
        label,
        mark,
        detail: detail.into(),
    }
}

/// "30 frames a second from the phone (front lens)"; no lens when unknown.
pub fn streaming_detail(fps: u32, lens: Option<u8>) -> String {
    let lens = match lens {
        Some(0) => " (back lens)",
        Some(1) => " (front lens)",
        _ => "",
    };
    format!("{fps} frames a second from the phone{lens}")
}

pub fn rows(i: &Inputs) -> Vec<Row> {
    let mut out = Vec::new();
    out.push(match &i.phone {
        Some(n) => row("Phone", Mark::Ok, format!("{n} is connected")),
        None => row("Phone", Mark::Bad, "Pair a phone first"),
    });
    out.push(if !i.installed {
        row(
            "Camera installed",
            Mark::Bad,
            "Installs on first use (Windows asks once for permission)",
        )
    } else if i.update_ready {
        row(
            "Camera installed",
            Mark::Wait,
            "An update installs next time the camera starts",
        )
    } else {
        row("Camera installed", Mark::Ok, "Relay Camera is set up")
    });
    out.push(match i.access {
        Access::Allow => row("Windows camera access", Mark::Ok, "On"),
        Access::Deny => row(
            "Windows camera access",
            Mark::Bad,
            "Off: Windows blocks every camera. Turn on Settings, Privacy & security, Camera",
        ),
        Access::Unknown => row("Windows camera access", Mark::Wait, "Checking"),
    });
    out.push(if i.pending && !i.streaming {
        row(
            "Phone camera",
            Mark::Wait,
            "Waiting for the phone: accept on the phone",
        )
    } else if i.streaming {
        if i.fps > 0 && i.dark {
            row(
                "Phone camera",
                Mark::Bad,
                "Pictures arrive but they are black: is the phone's lens covered?",
            )
        } else if i.fps > 0 {
            row("Phone camera", Mark::Ok, streaming_detail(i.fps, i.lens))
        } else {
            row("Phone camera", Mark::Wait, "Waiting for the first picture")
        }
    } else {
        row("Phone camera", Mark::Idle, "Not started")
    });
    out.push(match (i.streaming, i.listed) {
        (false, _) => row(
            "Listed by Windows",
            Mark::Idle,
            "Appears while the camera is on",
        ),
        (true, None) => row("Listed by Windows", Mark::Wait, "Checking"),
        (true, Some(true)) => row(
            "Listed by Windows",
            Mark::Ok,
            "Apps and browsers can pick Relay Camera",
        ),
        (true, Some(false)) => row(
            "Listed by Windows",
            Mark::Bad,
            "Windows does not list it; apps cannot see it",
        ),
    });
    out.push(match (i.streaming, i.preview_fps) {
        (false, _) => row("Apps can read pictures", Mark::Idle, "Shown below while on"),
        (true, None) => row("Apps can read pictures", Mark::Wait, "Opening the camera"),
        (true, Some(0)) => row(
            "Apps can read pictures",
            Mark::Bad,
            "Listed, but no pictures arrive",
        ),
        (true, Some(n)) => row(
            "Apps can read pictures",
            Mark::Ok,
            format!("{n} frames a second read the way an app does"),
        ),
    });
    if let (Some(msg), false) = (&i.start_error, i.streaming) {
        out.push(row("Camera could not start", Mark::Bad, msg.as_str()));
    }
    out
}

/// "Repair camera" is offered when the virtual camera is missing, outdated
/// (the shipped source differs) or the last start failed. `playback_error`:
/// the failure was in decoding or drawing the phone's picture, which the
/// installer cannot fix.
pub fn offer_repair(
    installed: bool,
    update_ready: bool,
    start_error: &Option<String>,
    playback_error: bool,
) -> bool {
    !installed || update_ready || (start_error.is_some() && !playback_error)
}

/// What the Start/Stop button (and the tray entry) does right now.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CameraButton {
    Start,
    /// Withdraw a request the phone has not answered yet.
    Cancel,
    Stop,
}

pub fn button(streaming: bool, pending: bool) -> CameraButton {
    if pending {
        CameraButton::Cancel
    } else if streaming {
        CameraButton::Stop
    } else {
        CameraButton::Start
    }
}

/// One line for the settings page: what the camera is doing right now, from
/// the same rows the camera window draws (so they never disagree), and what
/// the buttons offer. The slow checks the window measures ("listed", "apps
/// can read pictures") are left to the window.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Summary {
    pub mark: Mark,
    pub title: String,
    pub detail: String,
    pub button: CameraButton,
    pub repair: bool,
}

pub fn summary(i: &Inputs, playback_error: bool) -> Summary {
    const FAST: [&str; 5] = [
        "Phone",
        "Camera installed",
        "Windows camera access",
        "Phone camera",
        "Camera could not start",
    ];
    let all = rows(i);
    let fast: Vec<&Row> = all.iter().filter(|r| FAST.contains(&r.label)).collect();
    let button = button(i.streaming, i.pending);
    let repair = !i.streaming
        && !i.pending
        && offer_repair(i.installed, i.update_ready, &i.start_error, playback_error);
    let find = |label: &str| {
        fast.iter()
            .find(|r| r.label == label)
            .map(|r| (**r).clone())
    };
    let from = |r: &Row, title: &str| Summary {
        mark: r.mark,
        title: title.to_owned(),
        detail: r.detail.clone(),
        button,
        repair,
    };
    let phone_camera = find("Phone camera");
    // The first thing that is wrong, in the order the window lists them.
    let bad = fast.iter().find(|r| r.mark == Mark::Bad);
    if i.streaming || i.pending {
        if let Some(r) = bad.filter(|r| r.label == "Phone camera") {
            return from(r, "Camera has a problem");
        }
        if let Some(r) = phone_camera {
            return match r.mark {
                Mark::Ok => from(&r, "Camera is on"),
                _ => from(&r, "Starting the camera"),
            };
        }
    }
    if let Some(r) = bad {
        let title = if r.label == "Camera could not start" {
            r.label
        } else {
            "Camera cannot start yet"
        };
        return from(r, title);
    }
    let detail = match find("Camera installed") {
        Some(r) if r.mark != Mark::Ok => r.detail,
        _ => "Use your phone as a webcam in any app".to_owned(),
    };
    Summary {
        mark: Mark::Idle,
        title: "Camera is off".to_owned(),
        detail,
        button,
        repair,
    }
}

/// The whole camera works for apps when every row is ok.
pub fn all_ok(rows: &[Row]) -> bool {
    rows.iter().all(|r| r.mark == Mark::Ok)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> Inputs {
        Inputs {
            phone: Some("RMX3561".into()),
            installed: true,
            update_ready: false,
            access: Access::Allow,
            streaming: true,
            fps: 29,
            listed: Some(true),
            preview_fps: Some(28),
            dark: false,
            pending: false,
            start_error: None,
            lens: None,
        }
    }

    fn detail(rows: &[Row], label: &str) -> String {
        rows.iter()
            .find(|r| r.label == label)
            .unwrap()
            .detail
            .clone()
    }

    #[test]
    fn names_the_lens_when_known() {
        let front = rows(&Inputs {
            fps: 30,
            lens: Some(1),
            ..base()
        });
        assert_eq!(
            detail(&front, "Phone camera"),
            "30 frames a second from the phone (front lens)"
        );
        assert_eq!(mark(&front, "Phone camera"), Mark::Ok);
        assert_eq!(
            streaming_detail(29, Some(0)),
            "29 frames a second from the phone (back lens)"
        );
    }

    #[test]
    fn omits_the_lens_when_unknown() {
        let r = rows(&Inputs {
            fps: 30,
            lens: None,
            ..base()
        });
        assert_eq!(
            detail(&r, "Phone camera"),
            "30 frames a second from the phone"
        );
        assert_eq!(
            streaming_detail(29, Some(9)),
            "29 frames a second from the phone"
        );
    }

    fn mark(rows: &[Row], label: &str) -> Mark {
        rows.iter().find(|r| r.label == label).unwrap().mark
    }

    #[test]
    fn everything_working_is_all_ok() {
        let r = rows(&base());
        assert_eq!(r.len(), 6);
        assert!(all_ok(&r), "{r:?}");
    }

    #[test]
    fn blocked_camera_access_is_named_with_its_fix() {
        let r = rows(&Inputs {
            access: Access::Deny,
            ..base()
        });
        assert_eq!(mark(&r, "Windows camera access"), Mark::Bad);
        let d = &r
            .iter()
            .find(|x| x.label == "Windows camera access")
            .unwrap()
            .detail;
        assert!(d.contains("Camera"), "{d}");
        assert!(!all_ok(&r));
    }

    #[test]
    fn not_listed_while_streaming_is_a_failure() {
        let r = rows(&Inputs {
            listed: Some(false),
            preview_fps: None,
            ..base()
        });
        assert_eq!(mark(&r, "Listed by Windows"), Mark::Bad);
        assert_eq!(mark(&r, "Apps can read pictures"), Mark::Wait);
    }

    #[test]
    fn listed_without_pictures_is_called_out() {
        let r = rows(&Inputs {
            preview_fps: Some(0),
            ..base()
        });
        assert_eq!(mark(&r, "Apps can read pictures"), Mark::Bad);
    }

    #[test]
    fn nothing_is_expected_before_the_camera_starts() {
        let r = rows(&Inputs {
            streaming: false,
            fps: 0,
            listed: None,
            preview_fps: None,
            ..base()
        });
        assert_eq!(mark(&r, "Phone camera"), Mark::Idle);
        assert_eq!(mark(&r, "Listed by Windows"), Mark::Idle);
        assert_eq!(mark(&r, "Apps can read pictures"), Mark::Idle);
        assert!(!all_ok(&r), "idle is not working");
    }

    #[test]
    fn missing_phone_and_missing_install_are_reported() {
        let r = rows(&Inputs {
            phone: None,
            installed: false,
            ..base()
        });
        assert_eq!(mark(&r, "Phone"), Mark::Bad);
        assert_eq!(mark(&r, "Camera installed"), Mark::Bad);
        let u = rows(&Inputs {
            update_ready: true,
            ..base()
        });
        assert_eq!(mark(&u, "Camera installed"), Mark::Wait);
    }

    #[test]
    fn frames_that_are_black_say_so_and_do_not_count_as_working() {
        let r = rows(&Inputs {
            dark: true,
            ..base()
        });
        assert_eq!(mark(&r, "Phone camera"), Mark::Bad);
        let d = &r.iter().find(|x| x.label == "Phone camera").unwrap().detail;
        assert!(d.to_lowercase().contains("black"), "{d}");
        assert!(d.to_lowercase().contains("lens"), "{d}");
        assert!(!all_ok(&r));
    }

    #[test]
    fn streaming_without_a_first_picture_waits() {
        let r = rows(&Inputs { fps: 0, ..base() });
        assert_eq!(mark(&r, "Phone camera"), Mark::Wait);
    }

    #[test]
    fn a_pending_request_shows_waiting_for_the_phone() {
        let r = rows(&Inputs {
            streaming: false,
            fps: 0,
            listed: None,
            preview_fps: None,
            pending: true,
            ..base()
        });
        assert_eq!(mark(&r, "Phone camera"), Mark::Wait);
        let d = &r.iter().find(|x| x.label == "Phone camera").unwrap().detail;
        assert_eq!(d, "Waiting for the phone: accept on the phone");
        assert_eq!(r.len(), 6);
    }

    #[test]
    fn button_is_cancel_while_pending() {
        assert_eq!(button(false, true), CameraButton::Cancel);
    }

    #[test]
    fn button_is_stop_while_streaming() {
        assert_eq!(button(true, false), CameraButton::Stop);
    }

    #[test]
    fn button_is_start_when_idle() {
        assert_eq!(button(false, false), CameraButton::Start);
    }

    #[test]
    fn streaming_wins_over_pending_only_when_not_pending() {
        assert_eq!(button(true, true), CameraButton::Cancel);
    }

    #[test]
    fn an_error_row_appears_when_starting_failed() {
        let r = rows(&Inputs {
            streaming: false,
            fps: 0,
            listed: None,
            preview_fps: None,
            start_error: Some("the camera is not installed".into()),
            ..base()
        });
        let e = r
            .iter()
            .find(|x| x.label == "Camera could not start")
            .expect("error row");
        assert_eq!(e.mark, Mark::Bad);
        assert_eq!(e.detail, "the camera is not installed");
        assert!(!all_ok(&r));
    }

    #[test]
    fn no_error_row_while_streaming() {
        let r = rows(&Inputs {
            start_error: Some("old failure".into()),
            ..base()
        });
        assert!(r.iter().all(|x| x.label != "Camera could not start"));
        assert!(all_ok(&r), "{r:?}");
    }

    #[test]
    fn repair_is_offered_when_not_installed() {
        assert!(offer_repair(false, false, &None, false));
    }

    #[test]
    fn repair_is_offered_when_an_update_is_ready() {
        assert!(offer_repair(true, true, &None, false));
    }

    #[test]
    fn repair_is_offered_after_a_start_error() {
        assert!(offer_repair(true, false, &Some("broken".into()), false));
    }

    #[test]
    fn repair_is_not_offered_when_everything_is_fine() {
        assert!(!offer_repair(true, false, &None, false));
    }

    #[test]
    fn repair_is_not_offered_for_a_decoder_or_gpu_error() {
        // The installer cannot fix the phone's stream or this PC's graphics.
        assert!(!offer_repair(
            true,
            false,
            &Some("decode failed".into()),
            true
        ));
    }

    #[test]
    fn repair_is_still_offered_when_missing_even_after_a_playback_error() {
        assert!(offer_repair(
            false,
            false,
            &Some("decode failed".into()),
            true
        ));
    }

    #[test]
    fn summary_follows_the_same_rows_as_the_window() {
        let on = summary(&base(), false);
        assert_eq!(on.mark, Mark::Ok);
        assert_eq!(on.title, "Camera is on");
        assert_eq!(on.detail, "29 frames a second from the phone");
        assert_eq!(on.button, CameraButton::Stop);
        assert!(!on.repair);

        let off = summary(
            &Inputs {
                streaming: false,
                fps: 0,
                listed: None,
                preview_fps: None,
                ..base()
            },
            false,
        );
        assert_eq!(off.mark, Mark::Idle);
        assert_eq!(off.title, "Camera is off");
        assert_eq!(off.button, CameraButton::Start);

        let wait = summary(
            &Inputs {
                streaming: false,
                pending: true,
                ..base()
            },
            false,
        );
        assert_eq!(wait.mark, Mark::Wait);
        assert_eq!(wait.button, CameraButton::Cancel);
        assert!(wait.detail.contains("accept on the phone"));
    }

    #[test]
    fn summary_reports_what_blocks_the_camera() {
        let no_phone = summary(
            &Inputs {
                phone: None,
                streaming: false,
                ..base()
            },
            false,
        );
        assert_eq!(no_phone.mark, Mark::Bad);
        assert_eq!(no_phone.detail, "Pair a phone first");

        let denied = summary(
            &Inputs {
                access: Access::Deny,
                streaming: false,
                ..base()
            },
            false,
        );
        assert_eq!(denied.mark, Mark::Bad);
        assert!(denied.detail.contains("Camera"));

        let failed = summary(
            &Inputs {
                streaming: false,
                start_error: Some("no frames".into()),
                ..base()
            },
            false,
        );
        assert_eq!(failed.mark, Mark::Bad);
        assert_eq!(failed.title, "Camera could not start");
        assert_eq!(failed.detail, "no frames");
        assert!(failed.repair);

        let dark = summary(
            &Inputs {
                dark: true,
                ..base()
            },
            false,
        );
        assert_eq!(dark.mark, Mark::Bad);
    }

    #[test]
    fn summary_offers_repair_only_when_idle_and_needed() {
        let missing = summary(
            &Inputs {
                installed: false,
                streaming: false,
                ..base()
            },
            false,
        );
        assert!(missing.repair);
        let streaming_outdated = summary(
            &Inputs {
                update_ready: true,
                ..base()
            },
            false,
        );
        assert!(!streaming_outdated.repair);
    }

    #[test]
    fn summary_ignores_the_slow_checks_that_may_not_have_run() {
        // The settings page never measures "listed" or the preview.
        let s = summary(
            &Inputs {
                listed: None,
                preview_fps: None,
                ..base()
            },
            false,
        );
        assert_eq!(s.mark, Mark::Ok);
    }
}
