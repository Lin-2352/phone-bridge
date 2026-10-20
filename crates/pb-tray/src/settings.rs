//! User settings, persisted as `key=value` lines in `<data dir>/settings.txt`.
//! Unknown or malformed lines are ignored so old/new versions coexist.

use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    /// Share what is copied on either device automatically. Off by default:
    /// clipboard content is sensitive, so syncing is the user's choice.
    pub universal_clipboard: bool,
    /// Never sync content a password manager marked private. On by default;
    /// when off it is sent flagged sensitive (kept out of history on arrival).
    pub skip_private: bool,
    /// Keep received clipboard items in Win+V history / cloud clipboard.
    pub keep_in_history: bool,
    /// Accept files from paired devices without asking.
    pub auto_accept: bool,
    pub save_dir: PathBuf,
    /// 0 = follow Windows, 1 = light, 2 = dark.
    pub theme: u8,
    /// Let a paired phone drive this PC's pointer and keyboard.
    pub allow_control: bool,
    /// Show the phone's notifications as Windows toasts.
    pub notify_enabled: bool,
    /// 0 = full, 1 = full but hidden while the PC is locked, 2 = sender only.
    pub notify_content: u8,
    /// Toasts may appear on the Windows lock screen.
    pub notify_lock_screen: bool,
    /// 0 = follow the phone, 1 = always alert, 2 = always quiet.
    pub notify_sound: u8,
    /// Clearing a toast clears the notification on the phone.
    pub notify_clear_on_phone: bool,
    /// Clicking a toast opens the notification on the phone.
    pub notify_open_on_phone: bool,
    /// Copy one-time codes to the clipboard on arrival.
    pub notify_otp_autocopy: bool,
    /// No pop-ups while the phone-screen window is focused.
    pub notify_quiet_mirroring: bool,
    /// Phone apps seen so far: (package, name, muted).
    pub notify_apps: Vec<(String, String, bool)>,
    /// The phone-screen window stays on top.
    pub screen_pinned: bool,
    /// Dim the phone's backlight while its screen is shown here.
    pub screen_dim: bool,
    /// Native resolution instead of a 1920 long side.
    pub screen_sharp: bool,
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
            theme: 0,
            allow_control: false,
            notify_enabled: true,
            notify_content: 1,
            notify_lock_screen: false,
            notify_sound: 0,
            notify_clear_on_phone: true,
            notify_open_on_phone: true,
            notify_otp_autocopy: false,
            notify_quiet_mirroring: true,
            notify_apps: Vec::new(),
            screen_pinned: false,
            screen_dim: false,
            screen_sharp: false,
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
                "theme" => {
                    s.theme = match v {
                        "light" => 1,
                        "dark" => 2,
                        _ => 0,
                    }
                }
                "allow_control" => s.allow_control = parse_bool(v).unwrap_or(s.allow_control),
                "skip_private" => s.skip_private = parse_bool(v).unwrap_or(s.skip_private),
                "keep_in_history" => s.keep_in_history = parse_bool(v).unwrap_or(s.keep_in_history),
                "auto_accept" => s.auto_accept = parse_bool(v).unwrap_or(s.auto_accept),
                "save_dir" if !v.is_empty() => s.save_dir = PathBuf::from(v),
                "notify_enabled" => s.notify_enabled = parse_bool(v).unwrap_or(s.notify_enabled),
                "notify_content" => {
                    s.notify_content = v
                        .parse()
                        .ok()
                        .filter(|n| *n <= 2)
                        .unwrap_or(s.notify_content)
                }
                "notify_lock_screen" => {
                    s.notify_lock_screen = parse_bool(v).unwrap_or(s.notify_lock_screen)
                }
                "notify_sound" => {
                    s.notify_sound = v.parse().ok().filter(|n| *n <= 2).unwrap_or(s.notify_sound)
                }
                "notify_clear_on_phone" => {
                    s.notify_clear_on_phone = parse_bool(v).unwrap_or(s.notify_clear_on_phone)
                }
                "notify_open_on_phone" => {
                    s.notify_open_on_phone = parse_bool(v).unwrap_or(s.notify_open_on_phone)
                }
                "notify_otp_autocopy" => {
                    s.notify_otp_autocopy = parse_bool(v).unwrap_or(s.notify_otp_autocopy)
                }
                "notify_quiet_mirroring" => {
                    s.notify_quiet_mirroring = parse_bool(v).unwrap_or(s.notify_quiet_mirroring)
                }
                "screen_pinned" => s.screen_pinned = parse_bool(v).unwrap_or(s.screen_pinned),
                "screen_dim" => s.screen_dim = parse_bool(v).unwrap_or(s.screen_dim),
                "screen_sharp" => s.screen_sharp = parse_bool(v).unwrap_or(s.screen_sharp),
                "notify_app" => {
                    let mut f = v.splitn(3, '\t');
                    if let (Some(m), Some(pkg), Some(name)) = (f.next(), f.next(), f.next()) {
                        if !pkg.is_empty() {
                            s.notify_apps.push((pkg.into(), name.into(), m == "1"));
                        }
                    }
                }
                _ => {}
            }
        }
        s
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let mut text = format!(
            "universal_clipboard={}\nskip_private={}\nkeep_in_history={}\nauto_accept={}\ntheme={}\nallow_control={}\nsave_dir={}\n",
            self.universal_clipboard,
            self.skip_private,
            self.keep_in_history,
            self.auto_accept,
            ["system", "light", "dark"][self.theme.min(2) as usize],
            self.allow_control,
            self.save_dir.display()
        );
        text += &format!(
            "notify_enabled={}\nnotify_content={}\nnotify_lock_screen={}\nnotify_sound={}\nnotify_clear_on_phone={}\nnotify_open_on_phone={}\nnotify_otp_autocopy={}\nnotify_quiet_mirroring={}\n",
            self.notify_enabled,
            self.notify_content,
            self.notify_lock_screen,
            self.notify_sound,
            self.notify_clear_on_phone,
            self.notify_open_on_phone,
            self.notify_otp_autocopy,
            self.notify_quiet_mirroring
        );
        text += &format!(
            "screen_pinned={}\nscreen_dim={}\nscreen_sharp={}\n",
            self.screen_pinned, self.screen_dim, self.screen_sharp
        );
        let clean = |s: &str| s.replace(['\t', '\n', '\r'], " ");
        for (pkg, name, muted) in &self.notify_apps {
            text += &format!(
                "notify_app={}\t{}\t{}\n",
                u8::from(*muted),
                clean(pkg),
                clean(name)
            );
        }
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
            skip_private: false,
            keep_in_history: true,
            auto_accept: false,
            save_dir: PathBuf::from(r"D:\Inbox"),
            theme: 2,
            allow_control: false,
            notify_enabled: false,
            notify_content: 2,
            notify_lock_screen: true,
            notify_sound: 1,
            notify_clear_on_phone: false,
            notify_open_on_phone: false,
            notify_otp_autocopy: true,
            notify_quiet_mirroring: false,
            notify_apps: vec![
                ("com.whatsapp".into(), "WhatsApp".into(), false),
                ("com.noisy".into(), "Noisy Game".into(), true),
            ],
            screen_pinned: true,
            screen_dim: true,
            screen_sharp: true,
        };
        s.save(&path).unwrap();
        assert_eq!(Settings::load(&path), s);
        // Separators inside an app name do not break the file.
        let odd = Settings {
            notify_apps: vec![(
                "com.x".into(),
                "Tab	here
new"
                .into(),
                true,
            )],
            ..Settings::default()
        };
        odd.save(&path).unwrap();
        assert_eq!(
            Settings::load(&path).notify_apps,
            [("com.x".to_owned(), "Tab here new".to_owned(), true)]
        );
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
