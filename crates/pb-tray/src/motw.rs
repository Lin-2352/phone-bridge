//! Received files are marked as coming from the internet (Mark-of-the-Web),
//! so SmartScreen, Office Protected View and Smart App Control treat them as
//! they treat downloads. Risky types always ask before saving, and shortcut
//! types (which can leak the Windows password hash just by being shown in a
//! folder) are refused unless allowed.

use pb_transport::files::FileRisk;

/// What happens to an offered file before anything is saved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OfferAction {
    Accept,
    Ask,
    Refuse,
}

/// Auto-accept covers ordinary files only: risky ones always ask, and
/// shortcuts are refused unless the user allowed them.
pub fn offer_action(auto_accept: bool, risk: FileRisk, allow_shortcuts: bool) -> OfferAction {
    match risk {
        FileRisk::Shortcut if !allow_shortcuts => OfferAction::Refuse,
        FileRisk::Normal if auto_accept => OfferAction::Accept,
        _ => OfferAction::Ask,
    }
}

/// The `Zone.Identifier` stream of a file from the internet zone.
pub const ZONE_IDENTIFIER: &str = "[ZoneTransfer]\r\nZoneId=3\r\n";

/// Our list, plus any extension Windows itself calls dangerous.
#[cfg(windows)]
#[allow(unsafe_code)]
pub fn risk(name: &str) -> FileRisk {
    let r = pb_transport::files::classify(name);
    if r != FileRisk::Normal {
        return r;
    }
    let Some((_, ext)) = name.trim_end_matches(['.', ' ']).rsplit_once('.') else {
        return r;
    };
    let dotted = windows::core::HSTRING::from(format!(".{ext}"));
    // SAFETY: a plain query with a valid, NUL-terminated string.
    if unsafe { windows::Win32::UI::Shell::AssocIsDangerous(&dotted) }.as_bool() {
        FileRisk::Risky
    } else {
        r
    }
}

#[cfg(not(windows))]
pub fn risk(name: &str) -> FileRisk {
    pb_transport::files::classify(name)
}

/// Marks `path` as from the internet zone. Fails on file systems without
/// alternate streams (FAT, exFAT): callers only log that.
#[cfg(windows)]
pub fn mark(path: &std::path::Path) -> std::io::Result<()> {
    let mut ads = path.as_os_str().to_owned();
    ads.push(":Zone.Identifier");
    std::fs::write(ads, ZONE_IDENTIFIER)
}

#[cfg(not(windows))]
pub fn mark(_: &std::path::Path) -> std::io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use pb_transport::files::FileRisk;

    #[test]
    fn risky_files_always_ask_even_with_auto_accept() {
        assert_eq!(offer_action(true, FileRisk::Risky, false), OfferAction::Ask);
        assert_eq!(
            offer_action(true, FileRisk::Normal, false),
            OfferAction::Accept
        );
        assert_eq!(
            offer_action(false, FileRisk::Normal, false),
            OfferAction::Ask
        );
    }

    #[test]
    fn shortcuts_are_refused_unless_allowed() {
        assert_eq!(
            offer_action(true, FileRisk::Shortcut, false),
            OfferAction::Refuse
        );
        assert_eq!(
            offer_action(false, FileRisk::Shortcut, false),
            OfferAction::Refuse
        );
        assert_eq!(
            offer_action(true, FileRisk::Shortcut, true),
            OfferAction::Ask
        );
    }

    #[test]
    fn the_mark_is_the_internet_zone() {
        assert_eq!(ZONE_IDENTIFIER, "[ZoneTransfer]\r\nZoneId=3\r\n");
    }

    #[cfg(windows)]
    #[test]
    fn zone_identifier_is_written_and_readable() {
        let dir = std::env::temp_dir().join(format!("relay motw {}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("a.txt");
        std::fs::write(&f, b"x").unwrap();
        mark(&f).unwrap();
        let ads = format!("{}:Zone.Identifier", f.display());
        assert_eq!(std::fs::read_to_string(ads).unwrap(), ZONE_IDENTIFIER);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
