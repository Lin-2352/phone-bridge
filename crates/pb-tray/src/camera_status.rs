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
    /// The window read pictures from the camera the way an app does:
    /// `None` not tried, `Some(frames per second)` after.
    pub preview_fps: Option<u32>,
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
        row(
            "Camera installed",
            Mark::Ok,
            "Phone Bridge Camera is set up",
        )
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
    out.push(if i.streaming {
        if i.fps > 0 {
            row(
                "Phone camera",
                Mark::Ok,
                format!("{} frames a second from the phone", i.fps),
            )
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
            "Apps and browsers can pick Phone Bridge Camera",
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
    out
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
        }
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
    fn streaming_without_a_first_picture_waits() {
        let r = rows(&Inputs { fps: 0, ..base() });
        assert_eq!(mark(&r, "Phone camera"), Mark::Wait);
    }
}
