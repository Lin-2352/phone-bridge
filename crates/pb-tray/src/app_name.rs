//! The product name people see (window titles, tray, toasts, messages).
//! Crate names, wire/IPC identifiers, window class names and data folders
//! keep their historical names on purpose.

pub const APP_NAME: &str = "Relay";

/// The name apps list for the virtual camera (a display string only: Windows
/// finds the camera by its class id, not by this name).
pub const CAMERA_NAME: &str = "Relay Camera";

/// What older builds called the camera; still recognised so a leftover
/// device from one of them is swept.
pub const LEGACY_CAMERA_NAME: &str = "Phone Bridge Camera";

/// Name of the Start menu and Send to shortcuts (the toast header shows it).
pub const LINK_NAME: &str = "Relay.lnk";

/// What older builds called those shortcuts; removed when the new one is made.
pub const LEGACY_LINK_NAME: &str = "Phone Bridge.lnk";

#[cfg(test)]
mod tests {
    /// Every user-visible string says "Relay". The old name stays only where
    /// it is an identity on disk or in the registry (listed here).
    #[test]
    fn no_user_visible_string_still_says_phone_bridge() {
        let allowed = [
            ".join(\"Phone Bridge\")",   // data folder
            "Downloads\\\\Phone Bridge", // folder received files go to
            "LEGACY_",                   // migration of old names
            "RUN_KEY",                   // registry value name
            "an older build",            // the test of the legacy camera name
        ];
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        for e in std::fs::read_dir(dir).unwrap() {
            let p = e.unwrap().path();
            if p.extension().is_none_or(|x| x != "rs") || p.ends_with("app_name.rs") {
                continue;
            }
            let text = std::fs::read_to_string(&p).unwrap();
            let lines: Vec<&str> = text.lines().collect();
            for (i, l) in lines.iter().enumerate() {
                let near_run = i > 0 && lines[i - 1].contains("RUN_KEY");
                if l.contains("Phone Bridge")
                    && !l.trim_start().starts_with("//")
                    && !near_run
                    && !allowed.iter().any(|a| l.contains(a))
                {
                    panic!("{}:{}: {}", p.display(), i + 1, l.trim());
                }
            }
        }
    }
}
