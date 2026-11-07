//! One table for wording shared by several surfaces (tray menu, flyout,
//! Settings). The Settings phrasing is the source of truth.

pub const SHARE_CLIPBOARD: &str = "Share clipboard";
#[cfg(all(test, windows))]
pub const PHONE_NOTIFICATIONS: &str = "Show phone notifications";
pub const PHONE_CONTROL: &str = "Let my phone control this PC";
#[cfg(all(test, windows))]
pub const KEEP_LOGS: &str = "Keep logs";
pub const NO_PEER: &str = "Pair a phone first";
pub const PAIR_DEVICE: &str = "Pair device";
pub const PAUSED: &str = "Relay is paused";
pub const TRANSFERS_EMPTY: &str = "No transfers yet";
pub const TRANSFERS_EMPTY_HINT: &str =
    "Files you send to or receive from a paired phone show up here while they move.";

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use crate::{menu_model, settings_groups::OPTIONS};

    #[test]
    fn shared_labels_are_the_settings_labels() {
        for s in [
            SHARE_CLIPBOARD,
            PHONE_NOTIFICATIONS,
            PHONE_CONTROL,
            KEEP_LOGS,
        ] {
            assert!(OPTIONS.contains(&s), "{s} is not a Settings label");
        }
    }

    #[test]
    fn the_tray_menu_uses_the_settings_wording() {
        let labels = menu_model::toggle_labels();
        assert_eq!(labels.len(), 1);
        for l in labels {
            assert!(
                OPTIONS.contains(&l.as_str()),
                "menu says {l:?}, Settings does not"
            );
        }
    }
}

/// Said once at start when the discovery port is taken, so finding a moved phone is off.
pub fn discovery_note(active: bool) -> Option<&'static str> {
    (!active).then_some(
        "Relay can't look for your phone on a new network: another app uses UDP port 47199. Pairing and saved addresses still work.",
    )
}

#[cfg(test)]
mod discovery_note_tests {
    use super::discovery_note;

    #[test]
    fn only_a_taken_port_gets_a_note() {
        assert_eq!(discovery_note(true), None);
        assert!(discovery_note(false).unwrap().contains("47199"));
    }
}
