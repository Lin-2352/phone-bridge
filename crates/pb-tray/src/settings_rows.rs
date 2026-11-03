//! What the Settings page shows, as plain data (tested on every
//! platform); `pages::settings` draws it.

use crate::camera_status::Summary;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Key {
    Share,
    ShareImages,
    Passwords,
    Keep,
    Auto,
    SendTo,
    Startup,
    Control,
    Notify,
    NotifyLock,
    NotifyClear,
    NotifyOpen,
    NotifyOtp,
    NotifyQuietMirror,
    /// Record the Timeline; off deletes it.
    KeepHistory,
    /// Write the day-by-day structured logs.
    KeepLogs,
}

/// Three-way choices, painted as segmented buttons.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ChoiceKey {
    /// Full / hidden while locked / sender only.
    Content,
    /// Follow the phone / always alert / always quiet.
    Sound,
}

#[derive(Clone, Default, Debug, PartialEq)]
pub struct Model {
    /// 0 system, 1 light, 2 dark.
    pub theme: u8,
    pub share: bool,
    pub share_images: bool,
    pub passwords: bool,
    pub keep: bool,
    pub auto: bool,
    pub save_dir: String,
    pub sendto: bool,
    pub startup: bool,
    pub control: bool,
    pub keep_history: bool,
    pub keep_logs: bool,
    /// Where the logs go, and how big they are now (bytes).
    pub log_dir: String,
    pub log_bytes: u64,
    /// (name, fingerprint, answers as a different device now)
    pub devices: Vec<(String, String, bool)>,
    pub my_fingerprint: String,
    pub notify: bool,
    pub notify_lock: bool,
    pub notify_clear: bool,
    pub notify_open: bool,
    pub notify_otp: bool,
    pub notify_quiet_mirror: bool,
    pub content: u8,
    pub sound: u8,
    /// Phone apps seen so far: (name, shown). Off = muted.
    pub apps: Vec<(String, bool)>,
    /// What the camera is doing now (`None` before the first reading).
    pub camera: Option<Summary>,
}

impl Model {
    pub fn get(&self, k: Key) -> bool {
        match k {
            Key::Share => self.share,
            Key::ShareImages => self.share_images,
            Key::Passwords => self.passwords,
            Key::Keep => self.keep,
            Key::Auto => self.auto,
            Key::SendTo => self.sendto,
            Key::Startup => self.startup,
            Key::Control => self.control,
            Key::Notify => self.notify,
            Key::NotifyLock => self.notify_lock,
            Key::NotifyClear => self.notify_clear,
            Key::NotifyOpen => self.notify_open,
            Key::NotifyOtp => self.notify_otp,
            Key::NotifyQuietMirror => self.notify_quiet_mirror,
            Key::KeepHistory => self.keep_history,
            Key::KeepLogs => self.keep_logs,
        }
    }

    pub fn choice(&self, k: ChoiceKey) -> u8 {
        match k {
            ChoiceKey::Content => self.content,
            ChoiceKey::Sound => self.sound,
        }
    }
}

/// What the Camera page asks for; each is what the camera window and the
/// tray entry already do.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CameraAct {
    /// Start, Cancel or Stop, whichever the status card offers.
    Toggle,
    Repair,
    OpenWindow,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    Camera(CameraAct),
    Theme(u8),
    Toggle(Key),
    ChangeFolder,
    OpenFolder,
    Remove(usize),
    Pair,
    Choice(ChoiceKey, u8),
    /// Flip the phone app at this index between shown and muted.
    Mute(usize),
    /// Delete the Timeline (the tray asks first).
    ClearHistory,
    /// Show the log folder in Explorer.
    OpenLogs,
    /// Delete every log (the tray asks first).
    ClearLogs,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Row {
    Header(&'static str),
    Theme,
    Toggle(Key, &'static str, &'static str),
    Choice(ChoiceKey, &'static str, [&'static str; 3]),
    /// A phone app, by index into `Model::apps`.
    App(usize),
    Folder,
    Device(usize),
    Pair,
    About,
    /// The camera status card with its buttons.
    CameraStatus,
    /// Opens the camera window (preview and the full checklist).
    CameraWindow,
    /// Deletes the Timeline.
    ClearHistory,
    /// The log folder, with an Open button.
    LogFolder,
    /// Deletes every log, showing their size.
    ClearLogs,
}

pub fn rows(m: &Model) -> Vec<Row> {
    let mut v = vec![
        Row::Header("Appearance"),
        Row::Theme,
        Row::Header("Clipboard"),
        Row::Toggle(
            Key::Share,
            crate::strings::SHARE_CLIPBOARD,
            "Copy on one device, paste on the other",
        ),
        Row::Toggle(
            Key::ShareImages,
            "Share images",
            "Copied pictures and screenshots; never saved as files",
        ),
        Row::Toggle(
            Key::Passwords,
            "Passwords and codes",
            "Also share text a password manager marks private",
        ),
        Row::Toggle(
            Key::Keep,
            "Keep received text in clipboard history",
            "Show it in Win+V history (never for passwords)",
        ),
        Row::Header("Phone notifications"),
        Row::Toggle(
            Key::Notify,
            "Show phone notifications",
            "Notifications from your phone appear here",
        ),
        Row::Choice(
            ChoiceKey::Content,
            "Content",
            ["Full", "Hide when locked", "Sender only"],
        ),
        Row::Toggle(
            Key::NotifyLock,
            "Show on the lock screen",
            "Otherwise they go straight to the notification centre",
        ),
        Row::Choice(
            ChoiceKey::Sound,
            "Sound and pop-ups",
            ["Follow phone", "Always alert", "Always quiet"],
        ),
        Row::Toggle(
            Key::NotifyClear,
            "Clearing here clears on the phone",
            "Off: clears only on this PC",
        ),
        Row::Toggle(
            Key::NotifyOpen,
            "Clicking opens it on the phone",
            "Also shows the phone screen when available",
        ),
        Row::Toggle(
            Key::NotifyOtp,
            "Copy one-time codes automatically",
            "Cleared after 60 s if unchanged; never while locked",
        ),
        Row::Toggle(
            Key::NotifyQuietMirror,
            "Quiet while the phone screen is open",
            "Toasts still go to the notification centre",
        ),
    ];
    v.extend([Row::Header("Camera"), Row::CameraStatus, Row::CameraWindow]);
    if !m.apps.is_empty() {
        v.push(Row::Header("Apps"));
        v.extend((0..m.apps.len()).map(Row::App));
    }
    v.extend([
        Row::Header("Files"),
        Row::Toggle(
            Key::Auto,
            "Accept files automatically",
            "From your paired devices only",
        ),
        Row::Folder,
        Row::Toggle(
            Key::SendTo,
            "Show in Explorer \u{201c}Send to\u{201d}",
            "Right-click files, Send to, Relay",
        ),
        Row::Header("General"),
        Row::Toggle(
            Key::Startup,
            "Start with Windows",
            "Relay waits quietly in the tray",
        ),
        Row::Toggle(
            Key::Control,
            "Let my phone control this PC",
            "Use the phone as a touchpad and keyboard",
        ),
        Row::Header("History"),
        Row::Toggle(
            Key::KeepHistory,
            "Keep history",
            "Remember what moved in the Timeline; off deletes it",
        ),
        Row::ClearHistory,
        Row::Header("Logs"),
        Row::Toggle(
            Key::KeepLogs,
            "Keep logs",
            "Day-by-day records of what Relay did; never your content",
        ),
        Row::LogFolder,
        Row::ClearLogs,
        Row::Header("Devices"),
    ]);
    v.extend((0..m.devices.len()).map(Row::Device));
    v.push(Row::Pair);
    v.push(Row::Header("About"));
    v.push(Row::About);
    v
}

/// "12.3 MB" for a byte count.
pub fn log_size_text(bytes: u64) -> String {
    format!("{:.1} MB", bytes as f64 / 1_048_576.0)
}

/// The small line under a paired device: its fingerprint, or why it is no
/// longer reachable when it answers with a new identity.
pub fn device_note(fingerprint: &str, changed: bool) -> String {
    if changed {
        // One line even in the narrowest window (about 57 characters fit);
        // it does not claim a reinstall (another device can cause this).
        "Other device answers. If reinstalled, remove & re-pair.".into()
    } else {
        format!("Fingerprint {fingerprint}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_device_that_was_reinstalled_says_so_instead_of_showing_a_fingerprint() {
        assert_eq!(device_note("18CF-F6D8", false), "Fingerprint 18CF-F6D8");
        let n = device_note("18CF-F6D8", true);
        assert_eq!(n, "Other device answers. If reinstalled, remove & re-pair.");
        // Fits one line in the narrowest window (about 57 characters).
        assert!(n.chars().count() <= 55, "{} chars", n.chars().count());
        assert!(n.contains("reinstalled"));
        assert!(!n.contains("18CF"));
    }

    fn title(r: &Row) -> String {
        match r {
            Row::Header(t) => format!("# {t}"),
            Row::Toggle(_, t, _) => (*t).to_owned(),
            Row::Choice(_, t, _) => (*t).to_owned(),
            Row::App(i) => format!("app {i}"),
            _ => "?".into(),
        }
    }

    #[test]
    fn rows_include_notification_section_in_order() {
        let m = Model {
            apps: vec![("WhatsApp".into(), true)],
            ..Default::default()
        };
        let all: Vec<String> = rows(&m).iter().map(title).collect();
        let from = all
            .iter()
            .position(|t| t == "# Phone notifications")
            .unwrap();
        let to = all.iter().position(|t| t == "# Files").unwrap();
        assert_eq!(
            all[from + 1..to],
            [
                "Show phone notifications",
                "Content",
                "Show on the lock screen",
                "Sound and pop-ups",
                "Clearing here clears on the phone",
                "Clicking opens it on the phone",
                "Copy one-time codes automatically",
                "Quiet while the phone screen is open",
                "# Camera",
                "?",
                "?",
                "# Apps",
                "app 0",
            ]
        );
    }

    #[test]
    fn the_image_switch_sits_under_share_clipboard_and_mirrors_its_setting() {
        let m = Model {
            share_images: true,
            ..Default::default()
        };
        assert!(m.get(Key::ShareImages));
        assert!(!Model::default().get(Key::ShareImages));
        let t: Vec<String> = rows(&Model::default()).iter().map(title).collect();
        let at = t.iter().position(|x| x == "Share clipboard").unwrap();
        assert_eq!(t[at + 1], "Share images");
    }
}

#[cfg(test)]
mod log_rows_tests {
    use super::*;

    #[test]
    fn log_rows_sit_after_history_on_the_general_page() {
        let r = rows(&Model::default());
        let at = |f: &dyn Fn(&Row) -> bool| r.iter().position(f).unwrap();
        let clear = at(&|x| *x == Row::ClearHistory);
        let keep = at(&|x| matches!(x, Row::Toggle(Key::KeepLogs, ..)));
        let folder = at(&|x| *x == Row::LogFolder);
        let wipe = at(&|x| *x == Row::ClearLogs);
        assert!(clear < keep && keep < folder && folder < wipe);
        assert_eq!(r[keep - 1], Row::Header("Logs"));
        let m = Model {
            keep_logs: true,
            ..Default::default()
        };
        assert!(m.get(Key::KeepLogs));
        assert!(!Model::default().get(Key::KeepLogs));
    }

    #[test]
    fn log_size_reads_in_megabytes() {
        assert_eq!(log_size_text(0), "0.0 MB");
        assert_eq!(log_size_text(1_572_864), "1.5 MB");
        assert_eq!(log_size_text(200), "0.0 MB");
    }
}
