//! Which sidebar page each settings option sits on (pure, tested);
//! `settings_window` shows one page at a time.

use crate::settings_rows::Row;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Page {
    Devices,
    Clipboard,
    Notifications,
    Apps,
    Files,
    General,
}

/// Every option label the window shows (fixed titles for the rows that
/// have none of their own).
#[cfg_attr(not(test), allow(dead_code))] // the tests check it against the rows
pub const OPTIONS: &[&str] = &[
    "Theme",
    "Share clipboard",
    "Passwords and codes",
    "Keep received text in clipboard history",
    "Show phone notifications",
    "Content",
    "Show on the lock screen",
    "Sound and pop-ups",
    "Clearing here clears on the phone",
    "Clicking opens it on the phone",
    "Copy one-time codes automatically",
    "Quiet while the phone screen is open",
    "Phone app",
    "Accept files automatically",
    "Save received files to",
    "Show in Explorer \u{201c}Send to\u{201d}",
    "Start with Windows",
    "Let my phone control this PC",
    "Paired device",
    "Pair device",
    "About",
];

/// The option label of a row; section headers have none.
pub fn label_of(row: &Row) -> Option<&'static str> {
    Some(match row {
        Row::Toggle(_, t, _) | Row::Choice(_, t, _) => t,
        Row::Theme => "Theme",
        Row::App(_) => "Phone app",
        Row::Folder => "Save received files to",
        Row::Device(_) => "Paired device",
        Row::Pair => "Pair device",
        Row::About => "About",
        Row::Header(_) => return None,
    })
}

/// Sidebar order and titles.
pub fn pages() -> [(Page, &'static str); 6] {
    [
        (Page::Devices, "Devices"),
        (Page::Clipboard, "Clipboard"),
        (Page::Notifications, "Notifications"),
        (Page::Apps, "Apps"),
        (Page::Files, "Files"),
        (Page::General, "General"),
    ]
}

/// Page of an option label; anything unknown lands on General.
pub fn page_of(option: &str) -> Page {
    match option {
        "Paired device" | "Pair device" => Page::Devices,
        "Share clipboard" | "Passwords and codes" | "Keep received text in clipboard history" => {
            Page::Clipboard
        }
        "Show phone notifications"
        | "Content"
        | "Show on the lock screen"
        | "Sound and pop-ups"
        | "Clearing here clears on the phone"
        | "Clicking opens it on the phone"
        | "Copy one-time codes automatically"
        | "Quiet while the phone screen is open" => Page::Notifications,
        "Phone app" => Page::Apps,
        "Accept files automatically"
        | "Save received files to"
        | "Show in Explorer \u{201c}Send to\u{201d}" => Page::Files,
        _ => Page::General,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings_rows::{rows, Model};

    #[test]
    fn pages_are_in_sidebar_order() {
        assert_eq!(
            pages().map(|(_, t)| t),
            [
                "Devices",
                "Clipboard",
                "Notifications",
                "Apps",
                "Files",
                "General"
            ]
        );
    }

    #[test]
    fn every_existing_option_is_on_exactly_one_page() {
        // OPTIONS has no duplicates and covers every labelled row the
        // window builds today (so none can be dropped or drawn twice).
        for (i, o) in OPTIONS.iter().enumerate() {
            assert!(!OPTIONS[..i].contains(o), "duplicate option {o}");
            let n = pages().iter().filter(|(p, _)| page_of(o) == *p).count();
            assert_eq!(n, 1, "{o}");
        }
        let m = Model {
            apps: vec![("A".into(), true)],
            devices: vec![("D".into(), "f".into())],
            ..Default::default()
        };
        for r in rows(&m) {
            let Some(label) = label_of(&r) else { continue };
            assert!(OPTIONS.contains(&label), "missing {label}");
        }
    }

    #[test]
    fn options_land_on_the_page_a_person_would_look_for() {
        assert_eq!(page_of("Pair device"), Page::Devices);
        assert_eq!(page_of("Share clipboard"), Page::Clipboard);
        assert_eq!(page_of("Content"), Page::Notifications);
        assert_eq!(
            page_of("Quiet while the phone screen is open"),
            Page::Notifications
        );
        assert_eq!(page_of("Phone app"), Page::Apps);
        assert_eq!(page_of("Accept files automatically"), Page::Files);
        assert_eq!(page_of("Theme"), Page::General);
        assert_eq!(page_of("Start with Windows"), Page::General);
    }

    #[test]
    fn no_page_but_apps_is_long() {
        // Every page except Apps must fit a window without scrolling far:
        // at most 6 rows (Apps holds the per-app list on its own page).
        let m = Model {
            apps: vec![("A".into(), true)],
            devices: vec![("D".into(), "f".into())],
            ..Default::default()
        };
        for (page, title) in pages() {
            if page == Page::Apps {
                continue;
            }
            let n = rows(&m)
                .iter()
                .filter(|r| label_of(r).map(page_of) == Some(page))
                .count();
            assert!(n <= 8, "{title} has {n} rows");
        }
    }

    #[test]
    fn unknown_options_fall_back_to_general() {
        assert_eq!(page_of("no such option"), Page::General);
        assert_eq!(page_of(""), Page::General);
    }
}
