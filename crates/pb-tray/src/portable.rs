//! A portable copy keeps everything in a `data` folder beside the exe, however
//! it is started: double-click, Start menu, a toast, or Windows at sign-in.

use std::path::{Path, PathBuf};

/// The `data` folder beside the exe, if there is one.
pub fn data_dir(exe_dir: &Path) -> Option<PathBuf> {
    let dir = exe_dir.join("data");
    dir.is_dir().then_some(dir)
}

/// The data folder when Relay isn't portable. An install that already uses
/// `PhoneBridge\tray` keeps it (start-up entries, Send to and toasts all start
/// the exe without `--dir` and must land in the same folder); a new install
/// gets `Relay\tray`. `exists` says whether a folder is there.
pub fn pick_default_dir(local_app_data: &Path, exists: impl Fn(&Path) -> bool) -> PathBuf {
    let old = local_app_data.join("PhoneBridge").join("tray");
    if exists(&old) {
        old
    } else {
        local_app_data.join("Relay").join("tray")
    }
}

/// The data folder holds the identity key and the trusted devices: this user,
/// SYSTEM and Administrators only, nothing inherited. A folder unpacked where
/// other accounts may write (say C:\Relay) would otherwise let them add a
/// trusted device or swap settings.
pub fn data_dir_sddl(user_sid: &str) -> String {
    format!("D:PAI(A;OICI;FA;;;{user_sid})(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn existing_folder_is_kept() {
        let base = Path::new("/local");
        let old = base.join("PhoneBridge").join("tray");
        let got = pick_default_dir(base, |p| p == old);
        assert_eq!(got, old);
    }

    #[test]
    fn new_install_uses_relay() {
        let base = Path::new("/local");
        assert_eq!(
            pick_default_dir(base, |_| false),
            base.join("Relay").join("tray")
        );
    }

    #[test]
    fn once_created_the_relay_folder_is_found_again() {
        // Start-up entries pass no --dir: the second start must land in the same folder.
        let base = Path::new("/local");
        let new = base.join("Relay").join("tray");
        assert_eq!(pick_default_dir(base, |p| p == new), new);
    }

    #[test]
    fn data_dir_sddl_has_no_inheritance_and_three_principals() {
        let sid = "S-1-5-21-1-2-3-1001";
        let sddl = data_dir_sddl(sid);
        // Protected ("P"): nothing comes from a parent other users can write.
        assert!(sddl.starts_with("D:PAI"), "{sddl}");
        assert_eq!(sddl.matches("(A;").count(), 3, "{sddl}");
        assert!(sddl.contains(&format!(";;;{sid})")));
        assert!(sddl.contains(";;;SY)") && sddl.contains(";;;BA)"));
    }

    #[test]
    fn a_data_folder_beside_the_exe_makes_it_portable() {
        let root = std::env::temp_dir().join(format!("relay portable {}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        assert_eq!(data_dir(&root), None);
        std::fs::create_dir_all(root.join("data")).unwrap();
        assert_eq!(data_dir(&root), Some(root.join("data")));
        let _ = std::fs::remove_dir_all(root);
    }
}
