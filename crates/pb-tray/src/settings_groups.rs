//! Which group caption each settings option sits under (pure, tested);
//! `settings_window` lays its rows out by it.

use crate::settings_rows::Row;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Group {
    Connection,
    Camera,
    Screen,
    Files,
    Settings,
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

/// Display order and caption titles.
pub fn groups() -> [(Group, &'static str); 5] {
    [
        (Group::Connection, "Connection"),
        (Group::Camera, "Camera"),
        (Group::Screen, "Screen"),
        (Group::Files, "Files"),
        (Group::Settings, "Settings"),
    ]
}

/// Group of an option label; anything unknown lands under Settings.
pub fn group_of(option: &str) -> Group {
    match option {
        "Paired device" | "Pair device" => Group::Connection,
        "Quiet while the phone screen is open" => Group::Screen,
        "Accept files automatically"
        | "Save received files to"
        | "Show in Explorer \u{201c}Send to\u{201d}" => Group::Files,
        _ => Group::Settings,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings_rows::{rows, Model};

    #[test]
    fn groups_are_in_display_order() {
        let g = groups();
        assert_eq!(
            g.map(|(g, _)| g),
            [
                Group::Connection,
                Group::Camera,
                Group::Screen,
                Group::Files,
                Group::Settings
            ]
        );
        assert_eq!(
            g.map(|(_, t)| t),
            ["Connection", "Camera", "Screen", "Files", "Settings"]
        );
    }

    #[test]
    fn every_existing_option_has_a_group() {
        // OPTIONS has no duplicates and covers every labelled row the
        // window builds today (so none can be dropped or drawn twice).
        for (i, o) in OPTIONS.iter().enumerate() {
            assert!(!OPTIONS[..i].contains(o), "duplicate option {o}");
            let n = groups().iter().filter(|(g, _)| group_of(o) == *g).count();
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
        assert_eq!(group_of("Accept files automatically"), Group::Files);
        assert_eq!(group_of("Pair device"), Group::Connection);
        assert_eq!(
            group_of("Quiet while the phone screen is open"),
            Group::Screen
        );
        assert_eq!(group_of("Start with Windows"), Group::Settings);
    }

    #[test]
    fn unknown_options_fall_back_to_settings() {
        assert_eq!(group_of("no such option"), Group::Settings);
        assert_eq!(group_of(""), Group::Settings);
    }
}
