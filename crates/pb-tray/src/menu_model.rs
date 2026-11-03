//! What the tray's right-click menu lists: only what is urgent. Whatever is
//! running comes first so stopping it is one click, then the two senders, the
//! off switches (Pause and phone control), then Settings and Quit. Starting
//! the screen, camera or audio is on Home; other toggles are in Settings.
//! Pure: the caller reads the real state each time the menu opens.

use crate::{fluent::glyph, menu::Item, settings::Settings};

pub const ID_PAUSE: usize = 26;
pub const ID_TOGGLE_ALLOW_CONTROL: usize = 28;
pub const ID_AUDIO_PHONE: usize = 30;
pub const ID_AUDIO_PC: usize = 31;

/// The ids the actions already had in the tray's command table.
pub struct Ids {
    pub open: usize,
    pub send_files: usize,
    pub send_clipboard: usize,
    pub screen: usize,
    pub camera: usize,
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
}

use crate::strings::{NO_PEER, PAUSED};

pub fn build(i: &Inputs, ids: &Ids) -> Vec<Item> {
    use crate::audio_state::{Dir, Phase};
    let s = i.settings;
    // Sending needs a phone and an unpaused Relay.
    let start_block = if s.paused {
        Some(PAUSED)
    } else if !i.has_peer {
        Some(NO_PEER)
    } else {
        None
    };
    let toggle = |label: &str, id, on: bool| Item::new(' ', label, id).checked(on);
    let send = |g, label: &str, id| {
        let it = Item::new(g, label, id);
        match start_block {
            Some(r) => it.disabled(r),
            None => it,
        }
    };
    // What is running, so it can be stopped in one click even when paused.
    let mut items = Vec::new();
    if i.screen_open {
        items.push(Item::new(glyph::PHONE, "Hide phone screen", ids.screen));
    }
    match i.camera {
        Camera::Pending => items.push(Item::new(
            glyph::PHONE,
            "Cancel phone camera request",
            ids.camera,
        )),
        Camera::Running => items.push(Item::new(glyph::PHONE, "Stop phone camera", ids.camera)),
        Camera::Idle => {}
    }
    for (d, id, asking, running) in [
        (
            Dir::PhoneToPc,
            ID_AUDIO_PHONE,
            "Cancel phone audio request",
            "Stop phone audio on this PC",
        ),
        (
            Dir::PcToPhone,
            ID_AUDIO_PC,
            "Cancel PC audio request",
            "Stop PC audio on the phone",
        ),
    ] {
        match i.audio[d.index()] {
            Phase::Asking => items.push(Item::new(glyph::PHONE, asking, id)),
            Phase::Running => items.push(Item::new(glyph::PHONE, running, id)),
            Phase::Idle => {}
        }
    }
    if !items.is_empty() {
        items.push(Item::separator());
    }
    items.extend([
        Item::new(glyph::PHONE, "Open Relay", ids.open),
        send(glyph::SEND, "Send files\u{2026}", ids.send_files),
        send(glyph::CLIPBOARD, "Send clipboard now", ids.send_clipboard),
        Item::separator(),
        toggle("Pause Relay", ID_PAUSE, s.paused),
        toggle(
            crate::strings::PHONE_CONTROL,
            ID_TOGGLE_ALLOW_CONTROL,
            s.allow_control,
        ),
        Item::separator(),
        Item::new(glyph::SETTINGS, "Settings", ids.settings),
        Item::new(glyph::POWER, "Quit Relay", ids.quit),
    ]);
    items
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
    use crate::audio_state::Phase;

    const IDS: Ids = Ids {
        open: 24,
        send_files: 4,
        send_clipboard: 2,
        screen: 14,
        camera: 17,
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

    const IDLE: [&str; 7] = [
        "Open Relay",
        "Send files\u{2026}",
        "Send clipboard now",
        "Pause Relay",
        "Let my phone control this PC",
        "Settings",
        "Quit Relay",
    ];

    #[test]
    fn idle_lists_only_the_urgent_rows() {
        let s = Settings::default();
        assert_eq!(labels(&build(&inputs(&s), &IDS)), IDLE);
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
                "clipboard sync",
                "share clipboard",
                "notifications",
                "logs",
            ] {
                assert!(!l.contains(banned), "{l} must stay in Settings");
            }
        }
    }

    #[test]
    fn whatever_is_running_comes_first_with_its_stop() {
        let s = Settings::default();
        let mut i = inputs(&s);
        i.screen_open = true;
        i.camera = Camera::Running;
        i.audio = [Phase::Running, Phase::Asking];
        let items = build(&i, &IDS);
        assert_eq!(
            labels(&items)[..4],
            [
                "Hide phone screen",
                "Stop phone camera",
                "Stop phone audio on this PC",
                "Cancel PC audio request",
            ]
        );
        assert_eq!(
            find(&items, "Stop phone audio on this PC").id,
            ID_AUDIO_PHONE
        );
        assert_eq!(find(&items, "Cancel PC audio request").id, ID_AUDIO_PC);
        assert_eq!(labels(&items)[4..], IDLE);
        assert!(items[4].label.is_empty(), "a separator ends the live rows");
    }

    #[test]
    fn toggle_checks_follow_the_settings() {
        for on in [false, true] {
            let s = Settings {
                paused: on,
                allow_control: on,
                ..Settings::default()
            };
            let items = build(&inputs(&s), &IDS);
            for t in ["Pause Relay", "Let my phone control this PC"] {
                assert_eq!(find(&items, t).checked, Some(on), "{t}");
            }
            assert_eq!(find(&items, "Open Relay").checked, None);
        }
    }

    #[test]
    fn ids_are_unique() {
        let s = Settings::default();
        let mut i = inputs(&s);
        i.screen_open = true;
        i.camera = Camera::Running;
        i.audio = [Phase::Running, Phase::Running];
        let ids: Vec<usize> = build(&i, &IDS)
            .iter()
            .filter(|i| i.id != 0)
            .map(|i| i.id)
            .collect();
        let mut uniq = ids.clone();
        uniq.sort_unstable();
        uniq.dedup();
        assert_eq!(uniq.len(), ids.len());
    }

    #[test]
    fn without_a_phone_the_senders_are_disabled_with_a_reason() {
        let s = Settings::default();
        let mut i = inputs(&s);
        i.has_peer = false;
        let items = build(&i, &IDS);
        for l in ["Send files\u{2026}", "Send clipboard now"] {
            assert_eq!(
                find(&items, l).disabled.as_deref(),
                Some("Pair a phone first")
            );
        }
        assert_eq!(find(&items, "Open Relay").disabled, None);
        assert_eq!(find(&items, "Pause Relay").disabled, None);
    }

    #[test]
    fn paused_disables_sending_but_not_stopping() {
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
        assert_eq!(
            find(&items, "Send files\u{2026}").disabled.as_deref(),
            Some("Relay is paused")
        );
        // Pause itself stays clickable: it is how to resume.
        assert_eq!(find(&items, "Pause Relay").disabled, None);
    }

    #[test]
    fn disabled_rows_and_separators_are_not_clickable() {
        let s = Settings::default();
        let mut i = inputs(&s);
        i.has_peer = false;
        let items = build(&i, &IDS);
        assert!(!find(&items, "Send files\u{2026}").clickable());
        assert!(find(&items, "Open Relay").clickable());
        assert!(!Item::separator().clickable());
    }
}
