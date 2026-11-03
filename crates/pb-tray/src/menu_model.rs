//! What the tray's right-click menu lists: a few urgent actions, a group of
//! quick settings that apply at once, then Settings and Quit. Pure: the
//! caller reads the real state each time the menu opens and draws the rows.
//! Everything that needs more than a couple of clicks stays in Settings.

use crate::{fluent::glyph, menu::Item, settings::Settings};

pub const ID_PAUSE: usize = 26;
pub const ID_TOGGLE_NOTIFY: usize = 27;
pub const ID_TOGGLE_ALLOW_CONTROL: usize = 28;
pub const ID_TOGGLE_LOGS: usize = 29;
pub const ID_AUDIO_PHONE: usize = 30;
pub const ID_AUDIO_PC: usize = 31;

/// The ids the actions already had in the tray's command table.
pub struct Ids {
    pub open: usize,
    pub send_files: usize,
    pub send_clipboard: usize,
    pub screen: usize,
    pub camera: usize,
    pub clipboard_sync: usize,
    pub settings: usize,
    pub quit: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Camera {
    Idle,
    /// Asked, the phone has not answered.
    Pending,
    Running,
}

pub struct Inputs<'a> {
    pub settings: &'a Settings,
    pub has_peer: bool,
    pub screen_open: bool,
    pub camera: Camera,
    /// What each audio direction is doing (phone audio on this PC, PC audio
    /// on the phone).
    pub audio: [crate::audio_state::Phase; 2],
    /// The log writer could open its folder (otherwise the toggle cannot apply).
    pub logs_available: bool,
}

use crate::strings::{NO_PEER, PAUSED};
const NO_LOGS: &str = "Log folder is not available";

pub fn build(i: &Inputs, ids: &Ids) -> Vec<Item> {
    let s = i.settings;
    // Starting anything needs a phone and an unpaused Relay.
    let start_block = if s.paused {
        Some(PAUSED)
    } else if !i.has_peer {
        Some(NO_PEER)
    } else {
        None
    };
    let action = |g, label: &str, id, block: Option<&str>| {
        let it = Item::new(g, label, id);
        match block {
            Some(r) => it.disabled(r),
            None => it,
        }
    };
    let toggle = |label: &str, id, on: bool| Item::new(' ', label, id).checked(on);
    let screen = if i.screen_open {
        action(glyph::PHONE, "Hide phone screen", ids.screen, None)
    } else {
        action(glyph::PHONE, "Show phone screen", ids.screen, start_block)
    };
    let camera = match i.camera {
        Camera::Pending => action(
            glyph::PHONE,
            "Cancel phone camera request",
            ids.camera,
            None,
        ),
        Camera::Running => action(glyph::PHONE, "Stop phone camera", ids.camera, None),
        Camera::Idle => action(glyph::PHONE, "Use phone as camera", ids.camera, start_block),
    };
    let audio = |d: crate::audio_state::Dir, id| {
        use crate::audio_state::{Dir, Phase};
        match (i.audio[d.index()], d) {
            (Phase::Asking, Dir::PhoneToPc) => {
                action(glyph::PHONE, "Cancel phone audio request", id, None)
            }
            (Phase::Asking, Dir::PcToPhone) => {
                action(glyph::PHONE, "Cancel PC audio request", id, None)
            }
            (Phase::Running, Dir::PhoneToPc) => {
                action(glyph::PHONE, "Stop phone audio on this PC", id, None)
            }
            (Phase::Running, Dir::PcToPhone) => {
                action(glyph::PHONE, "Stop PC audio on the phone", id, None)
            }
            (Phase::Idle, d) => action(glyph::PHONE, d.title(), id, start_block),
        }
    };
    let mut logs = toggle(crate::strings::KEEP_LOGS, ID_TOGGLE_LOGS, s.keep_logs);
    if !i.logs_available {
        logs = logs.disabled(NO_LOGS);
    }
    vec![
        Item::new(glyph::PHONE, "Open Relay", ids.open),
        action(
            glyph::SEND,
            "Send files\u{2026}",
            ids.send_files,
            start_block,
        ),
        action(
            glyph::CLIPBOARD,
            "Send clipboard now",
            ids.send_clipboard,
            start_block,
        ),
        screen,
        camera,
        audio(crate::audio_state::Dir::PhoneToPc, ID_AUDIO_PHONE),
        audio(crate::audio_state::Dir::PcToPhone, ID_AUDIO_PC),
        Item::separator(),
        Item::heading("Quick settings"),
        toggle("Pause Relay", ID_PAUSE, s.paused),
        toggle(
            crate::strings::SHARE_CLIPBOARD,
            ids.clipboard_sync,
            s.universal_clipboard,
        ),
        toggle(
            crate::strings::PHONE_NOTIFICATIONS,
            ID_TOGGLE_NOTIFY,
            s.notify_enabled,
        ),
        toggle(
            crate::strings::PHONE_CONTROL,
            ID_TOGGLE_ALLOW_CONTROL,
            s.allow_control,
        ),
        logs,
        Item::separator(),
        Item::new(glyph::SETTINGS, "Settings", ids.settings),
        Item::new(glyph::POWER, "Quit Relay", ids.quit),
    ]
}

/// The labels of the menu's setting toggles other than Pause (the ones
/// Settings also shows); for the wording test.
#[cfg(test)]
pub fn toggle_labels() -> Vec<String> {
    let ids = Ids {
        open: 0,
        send_files: 0,
        send_clipboard: 0,
        screen: 0,
        camera: 0,
        clipboard_sync: 0,
        settings: 0,
        quit: 0,
    };
    let settings = Settings::default();
    let inputs = Inputs {
        settings: &settings,
        has_peer: true,
        screen_open: false,
        camera: Camera::Idle,
        audio: Default::default(),
        logs_available: true,
    };
    build(&inputs, &ids)
        .into_iter()
        .filter(|i| i.checked.is_some() && i.label != "Pause Relay")
        .map(|i| i.label)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const IDS: Ids = Ids {
        open: 24,
        send_files: 4,
        send_clipboard: 2,
        screen: 14,
        camera: 17,
        clipboard_sync: 5,
        settings: 11,
        quit: 3,
    };

    fn inputs(s: &Settings) -> Inputs<'_> {
        Inputs {
            settings: s,
            has_peer: true,
            screen_open: false,
            camera: Camera::Idle,
            audio: Default::default(),
            logs_available: true,
        }
    }

    fn labels(items: &[Item]) -> Vec<&str> {
        items
            .iter()
            .filter(|i| i.id != 0 || i.heading)
            .map(|i| i.label.as_str())
            .collect()
    }

    fn find<'a>(items: &'a [Item], label: &str) -> &'a Item {
        items.iter().find(|i| i.label == label).expect(label)
    }

    #[test]
    fn lists_the_decided_rows_in_order() {
        let s = Settings::default();
        let items = build(&inputs(&s), &IDS);
        assert_eq!(
            labels(&items),
            [
                "Open Relay",
                "Send files\u{2026}",
                "Send clipboard now",
                "Show phone screen",
                "Use phone as camera",
                "Phone audio on this PC",
                "PC audio on the phone",
                "Quick settings",
                "Pause Relay",
                "Share clipboard",
                "Show phone notifications",
                "Let my phone control this PC",
                "Keep logs",
                "Settings",
                "Quit Relay",
            ]
        );
    }

    #[test]
    fn settings_only_items_are_not_in_the_menu() {
        let s = Settings::default();
        let items = build(&inputs(&s), &IDS);
        for l in labels(&items) {
            let l = l.to_lowercase();
            for banned in [
                "accept",
                "folder",
                "theme",
                "history",
                "remove",
                "unpair",
                "permission",
                "retention",
            ] {
                assert!(!l.contains(banned), "{l} must stay in Settings");
            }
        }
    }

    #[test]
    fn toggle_checks_follow_the_settings() {
        let off = Settings {
            paused: false,
            universal_clipboard: false,
            notify_enabled: false,
            allow_control: false,
            keep_logs: false,
            ..Settings::default()
        };
        let on = Settings {
            paused: true,
            universal_clipboard: true,
            notify_enabled: true,
            allow_control: true,
            keep_logs: true,
            ..Settings::default()
        };
        let toggles = [
            "Pause Relay",
            "Share clipboard",
            "Show phone notifications",
            "Let my phone control this PC",
            "Keep logs",
        ];
        for t in toggles {
            assert_eq!(
                find(&build(&inputs(&off), &IDS), t).checked,
                Some(false),
                "{t}"
            );
            assert_eq!(
                find(&build(&inputs(&on), &IDS), t).checked,
                Some(true),
                "{t}"
            );
        }
        // Actions are never checkable.
        assert_eq!(find(&build(&inputs(&on), &IDS), "Open Relay").checked, None);
    }

    #[test]
    fn ids_are_unique_and_reuse_the_existing_command() {
        let s = Settings::default();
        let items = build(&inputs(&s), &IDS);
        assert_eq!(find(&items, "Share clipboard").id, IDS.clipboard_sync);
        let ids: Vec<usize> = items.iter().filter(|i| i.id != 0).map(|i| i.id).collect();
        let mut uniq = ids.clone();
        uniq.sort_unstable();
        uniq.dedup();
        assert_eq!(uniq.len(), ids.len(), "ids are unique");
    }

    #[test]
    fn without_a_phone_the_senders_are_disabled_with_a_reason() {
        let s = Settings::default();
        let mut i = inputs(&s);
        i.has_peer = false;
        let items = build(&i, &IDS);
        for l in [
            "Send files\u{2026}",
            "Send clipboard now",
            "Show phone screen",
            "Use phone as camera",
        ] {
            assert_eq!(
                find(&items, l).disabled.as_deref(),
                Some("Pair a phone first")
            );
        }
        assert_eq!(find(&items, "Open Relay").disabled, None);
        assert_eq!(find(&items, "Pause Relay").disabled, None);
    }

    #[test]
    fn paused_disables_starting_but_not_stopping() {
        let s = Settings {
            paused: true,
            ..Settings::default()
        };
        let mut i = inputs(&s);
        i.screen_open = true;
        i.camera = Camera::Running;
        let items = build(&i, &IDS);
        assert_eq!(find(&items, "Hide phone screen").disabled, None);
        assert_eq!(find(&items, "Stop phone camera").disabled, None);
        let mut i = inputs(&s);
        i.has_peer = false;
        let items = build(&i, &IDS);
        for l in [
            "Send files\u{2026}",
            "Send clipboard now",
            "Show phone screen",
        ] {
            assert_eq!(find(&items, l).disabled.as_deref(), Some("Relay is paused"));
        }
        // Pause itself stays clickable: it is how to resume.
        assert_eq!(find(&items, "Pause Relay").disabled, None);
    }

    #[test]
    fn keep_logs_is_disabled_when_the_log_folder_cannot_open() {
        let s = Settings::default();
        let mut i = inputs(&s);
        i.logs_available = false;
        let items = build(&i, &IDS);
        assert_eq!(
            find(&items, "Keep logs").disabled.as_deref(),
            Some("Log folder is not available")
        );
    }

    #[test]
    fn camera_and_screen_labels_follow_their_state() {
        let s = Settings::default();
        let mut i = inputs(&s);
        i.camera = Camera::Pending;
        i.screen_open = true;
        let items = build(&i, &IDS);
        find(&items, "Cancel phone camera request");
        find(&items, "Hide phone screen");
    }

    #[test]
    fn headings_and_disabled_rows_are_not_clickable() {
        let s = Settings::default();
        let mut i = inputs(&s);
        i.has_peer = false;
        let items = build(&i, &IDS);
        assert!(!find(&items, "Quick settings").clickable());
        assert!(!find(&items, "Send files\u{2026}").clickable());
        assert!(find(&items, "Open Relay").clickable());
        assert!(!Item::separator().clickable());
    }
}
