//! Which sidebar page each settings option sits on (pure, tested);
//! `settings_window` shows one page at a time.

use crate::settings_rows::{Model, Row};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Page {
    Devices,
    Clipboard,
    Notifications,
    Camera,
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
    "Share images",
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
    "Camera status",
    "Camera window",
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
        Row::CameraStatus => "Camera status",
        Row::CameraWindow => "Camera window",
        Row::Header(_) => return None,
    })
}

/// Sidebar order and titles.
pub fn pages() -> [(Page, &'static str); 7] {
    [
        (Page::Devices, "Devices"),
        (Page::Clipboard, "Clipboard"),
        (Page::Notifications, "Notifications"),
        (Page::Camera, "Camera"),
        (Page::Apps, "Apps"),
        (Page::Files, "Files"),
        (Page::General, "General"),
    ]
}

/// Page of an option label; anything unknown lands on General.
pub fn page_of(option: &str) -> Page {
    match option {
        "Paired device" | "Pair device" => Page::Devices,
        "Share clipboard"
        | "Share images"
        | "Passwords and codes"
        | "Keep received text in clipboard history" => Page::Clipboard,
        "Show phone notifications"
        | "Content"
        | "Show on the lock screen"
        | "Sound and pop-ups"
        | "Clearing here clears on the phone"
        | "Clicking opens it on the phone"
        | "Copy one-time codes automatically"
        | "Quiet while the phone screen is open" => Page::Notifications,
        "Camera status" | "Camera window" => Page::Camera,
        "Phone app" => Page::Apps,
        "Accept files automatically"
        | "Save received files to"
        | "Show in Explorer \u{201c}Send to\u{201d}" => Page::Files,
        _ => Page::General,
    }
}

/// Everything a person can read on a row, for search: its title and
/// subtitle (an app row is just the app name).
fn texts(row: &Row, m: &Model) -> Vec<String> {
    match row {
        Row::Header(_) => vec![],
        Row::Toggle(_, t, sub) => vec![(*t).into(), (*sub).into()],
        Row::Choice(_, t, labels) => {
            let mut v = vec![(*t).to_owned()];
            v.extend(labels.iter().map(|l| (*l).to_owned()));
            v
        }
        Row::Theme => vec!["Theme".into(), "System Light Dark".into()],
        Row::App(i) => m.apps.get(*i).map(|(n, _)| n.clone()).into_iter().collect(),
        Row::Folder => vec!["Save received files to".into(), m.save_dir.clone()],
        Row::Device(i) => m
            .devices
            .get(*i)
            .map(|(n, _)| n.clone())
            .into_iter()
            .collect(),
        Row::Pair => vec!["Pair device".into(), "Add another device".into()],
        Row::About => vec![format!(
            "{} {}",
            crate::app_name::APP_NAME,
            env!("CARGO_PKG_VERSION")
        )],
        Row::CameraStatus => {
            let mut v = vec!["Camera status".to_owned()];
            if let Some(c) = &m.camera {
                v.push(c.title.clone());
                v.push(c.detail.clone());
            }
            v
        }
        Row::CameraWindow => vec![
            "Camera window".into(),
            "Preview and the full checklist".into(),
        ],
    }
}

/// Indexes into `rows` of the rows matching `query` (case-insensitive, on
/// title and subtitle; an app on its name), ordered by sidebar page and then
/// as built. Empty or blank query matches nothing.
pub fn search(rows: &[Row], m: &Model, query: &str) -> Vec<usize> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return Vec::new();
    }
    let mut hits: Vec<(usize, usize)> = rows
        .iter()
        .enumerate()
        .filter(|(_, r)| texts(r, m).iter().any(|t| t.to_lowercase().contains(&q)))
        .map(|(i, r)| {
            let page = label_of(r).map_or(Page::General, page_of);
            let order = pages().iter().position(|(p, _)| *p == page).unwrap_or(0);
            (order, i)
        })
        .collect();
    hits.sort();
    hits.into_iter().map(|(_, i)| i).collect()
}

/// Title of the page a row sits on.
pub fn page_title_of(row: &Row) -> &'static str {
    let page = label_of(row).map_or(Page::General, page_of);
    pages()
        .iter()
        .find(|(p, _)| *p == page)
        .map_or("", |(_, t)| *t)
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
                "Camera",
                "Apps",
                "Files",
                "General"
            ]
        );
    }

    fn model() -> Model {
        Model {
            apps: vec![("WhatsApp".into(), true), ("Gmail".into(), true)],
            devices: vec![("Pixel".into(), "f".into())],
            ..Default::default()
        }
    }

    fn found(m: &Model, q: &str) -> Vec<Option<&'static str>> {
        let all = rows(m);
        search(&all, m, q)
            .iter()
            .map(|i| label_of(&all[*i]))
            .collect()
    }

    #[test]
    fn search_empty_query_matches_nothing() {
        assert!(found(&model(), "").is_empty());
        assert!(found(&model(), "   ").is_empty());
    }

    #[test]
    fn search_is_case_insensitive_and_matches_titles() {
        assert_eq!(
            found(&model(), "LOCK SCREEN"),
            [Some("Show on the lock screen")]
        );
    }

    #[test]
    fn search_matches_subtitles() {
        // "win+v" appears only in the subtitle of the history toggle.
        assert_eq!(
            found(&model(), "win+v"),
            [Some("Keep received text in clipboard history")]
        );
    }

    #[test]
    fn search_matches_an_app_by_name() {
        assert_eq!(found(&model(), "whats"), [Some("Phone app")]);
    }

    #[test]
    fn search_with_no_match_is_empty() {
        assert!(found(&model(), "zzzz").is_empty());
    }

    #[test]
    fn search_orders_results_by_page() {
        let hits = found(&model(), "phone");
        let m = model();
        let all = rows(&m);
        let idx = search(&all, &m, "phone");
        let pages_of: Vec<usize> = idx
            .iter()
            .map(|i| {
                let p = page_of(label_of(&all[*i]).unwrap());
                pages().iter().position(|(q, _)| *q == p).unwrap()
            })
            .collect();
        assert!(!hits.is_empty());
        assert!(pages_of.windows(2).all(|w| w[0] <= w[1]));
    }

    #[test]
    fn camera_rows_are_on_the_camera_page() {
        assert_eq!(page_of("Camera status"), Page::Camera);
        assert_eq!(page_of("Camera window"), Page::Camera);
        let m = model();
        let all = rows(&m);
        assert!(all
            .iter()
            .filter(|r| matches!(r, Row::CameraStatus | Row::CameraWindow))
            .all(|r| page_title_of(r) == "Camera"));
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
        assert_eq!(page_of("Share images"), Page::Clipboard);
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
