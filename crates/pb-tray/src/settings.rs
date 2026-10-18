//! User settings, persisted as `key=value` lines in `<data dir>/settings.txt`.
//! Unknown or malformed lines are ignored so old/new versions coexist.

use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    /// Share what is copied on either device automatically. Off by default:
    /// clipboard content is sensitive, so syncing is the user's choice.
    pub universal_clipboard: bool,
    /// Never sync content a password manager marked private (always on; kept
    /// as a setting so the UI can show it, but the code ignores `false`).
    pub skip_private: bool,
    /// Keep received clipboard items in Win+V history / cloud clipboard.
    pub keep_in_history: bool,
    /// Accept files from paired devices without asking.
    pub auto_accept: bool,
    pub save_dir: PathBuf,
}

impl Default for Settings {
    fn default() -> Self {
        let downloads = std::env::var_os("USERPROFILE")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir)
            .join("Downloads")
            .join("Phone Bridge");
        Self {
            universal_clipboard: false,
            skip_private: true,
            keep_in_history: false,
            auto_accept: true,
            save_dir: downloads,
        }
    }
}

fn parse_bool(v: &str) -> Option<bool> {
    match v {
        "true" | "1" | "on" => Some(true),
        "false" | "0" | "off" => Some(false),
        _ => None,
    }
}

impl Settings {
    pub fn load(path: &Path) -> Self {
        let mut s = Self::default();
        let Ok(text) = std::fs::read_to_string(path) else {
            return s;
        };
        for line in text.lines() {
            let Some((k, v)) = line.split_once('=') else {
                continue;
            };
            let (k, v) = (k.trim(), v.trim());
            match k {
                "universal_clipboard" => {
                    s.universal_clipboard = parse_bool(v).unwrap_or(s.universal_clipboard)
                }
                "keep_in_history" => s.keep_in_history = parse_bool(v).unwrap_or(s.keep_in_history),
                "auto_accept" => s.auto_accept = parse_bool(v).unwrap_or(s.auto_accept),
                "save_dir" if !v.is_empty() => s.save_dir = PathBuf::from(v),
                _ => {}
            }
        }
        s
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let text = format!(
            "universal_clipboard={}\nkeep_in_history={}\nauto_accept={}\nsave_dir={}\n",
            self.universal_clipboard,
            self.keep_in_history,
            self.auto_accept,
            self.save_dir.display()
        );
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, text)?;
        std::fs::rename(tmp, path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_private_by_default() {
        let s = Settings::default();
        assert!(!s.universal_clipboard, "clipboard sync must be opt-in");
        assert!(s.skip_private);
        assert!(!s.keep_in_history);
    }

    #[test]
    fn round_trip_and_tolerant_parsing() {
        let dir = std::env::temp_dir().join(format!("pb-settings-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("settings.txt");
        let s = Settings {
            universal_clipboard: true,
            skip_private: true,
            keep_in_history: true,
            auto_accept: false,
            save_dir: PathBuf::from(r"D:\Inbox"),
        };
        s.save(&path).unwrap();
        assert_eq!(Settings::load(&path), s);
        std::fs::write(
            &path,
            "garbage\nuniversal_clipboard=maybe\nauto_accept=off\nfuture_key=1\n",
        )
        .unwrap();
        let t = Settings::load(&path);
        assert!(!t.universal_clipboard);
        assert!(!t.auto_accept);
        assert_eq!(
            Settings::load(&dir.join("missing.txt")),
            Settings::default()
        );
    }
}
