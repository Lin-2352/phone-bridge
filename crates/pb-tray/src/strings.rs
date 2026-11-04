//! One table for wording shared by several surfaces (tray menu, flyout,
//! Settings). The Settings phrasing is the source of truth.

pub const SHARE_CLIPBOARD: &str = "Share clipboard";
#[cfg(all(test, windows))]
pub const PHONE_NOTIFICATIONS: &str = "Show phone notifications";
pub const PHONE_CONTROL: &str = "Let my phone control this PC";
#[cfg(test)]
pub const KEEP_LOGS: &str = "Keep logs";
pub const NO_PEER: &str = "Pair a phone first";
pub const PAIR_DEVICE: &str = "Pair device";
pub const PAUSED: &str = "Relay is paused";

#[cfg(test)]
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
