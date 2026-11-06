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
    /// Put text shared from the phone's share sheet on the clipboard at
    /// once. Off by default: any app on the phone can share, so the user
    /// copies it from the toast instead.
    pub auto_copy_shared_text: bool,
    /// Toasts about text and links from the phone show their content (off: only who sent it).
    pub toast_show_text: bool,
    /// Also share copied pictures and screenshots (needs `universal_clipboard`).
    /// Off by default: pictures are as sensitive as text.
    pub share_images: bool,
    /// Accept files from paired devices without asking.
    pub auto_accept: bool,
    /// Accept shortcut files (.lnk, .url, .library-ms, ...). Off: they are
    /// refused, because Explorer reads them by itself (settings.txt only).
    pub allow_shortcut_files: bool,
    pub save_dir: PathBuf,
    /// 0 = follow Windows, 1 = light, 2 = dark.
    pub theme: u8,
    /// The glass look: 0 = native (Mica and Acrylic), 1 = Relay glass.
    pub look: u8,
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
    /// Record what moved in the PC timeline (metadata only).
    pub keep_history: bool,
    /// Write the day-by-day structured logs (metadata only).
    pub keep_logs: bool,
    /// Do not ask on this PC before playing its audio on the phone. Off by
    /// default; the phone asks for its own audio, so there is no twin toggle.
    pub audio_skip_confirm_pc_to_phone: bool,
    /// Relay is paused from the tray menu: no sessions, nothing sent.
    /// Survives a restart, so a pause is never lost silently.
    pub paused: bool,
    /// Where the day folders go.
    pub log_dir: PathBuf,
    /// Where the main window was last (x, y, width, height in screen
    /// pixels); None until it has been closed once.
    pub main_window_rect: Option<(i32, i32, i32, i32)>,
    /// The Timeline days the user opened or closed ("day:1,day:0").
    pub timeline_days: String,
    /// The settings file's format version (`version=` on its first line).
    pub version: u32,
    /// Keys this build doesn't know (from a newer or older build), written back unchanged.
    pub extra: Vec<(String, String)>,
}

impl Default for Settings {
    fn default() -> Self {
        let downloads = std::env::var_os("USERPROFILE")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir)
            .join("Downloads")
            .join("Relay");
        Self {
            universal_clipboard: false,
            skip_private: true,
            keep_in_history: false,
            auto_copy_shared_text: false,
            toast_show_text: false,
            share_images: false,
            auto_accept: true,
            allow_shortcut_files: false,
            save_dir: downloads,
            theme: 0,
            look: 0,
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
            keep_history: true,
            audio_skip_confirm_pc_to_phone: false,
            keep_logs: true,
            paused: false,
            log_dir: PathBuf::new(),
            main_window_rect: None,
            timeline_days: String::new(),
            version: SETTINGS_VERSION,
            extra: Vec::new(),
        }
    }
}

/// The settings file's format version. 1 introduced `version=` and pinned the save folder of
/// installs that existed before the default changed. 2 added `look=` (nothing to rewrite: an
/// install that never chose gets the native look, and the file is saved at version 2).
pub const SETTINGS_VERSION: u32 = 2;

/// The folder name received files went to before "Relay" names (installs keep it).
const LEGACY_SAVE_FOLDER: &str = "Phone Bridge";

/// What a migration needs to know about the install.
pub struct MigrateCtx {
    /// `peers.txt` or `identity.sealed` exists: Relay ran here before.
    pub existing_install: bool,
    /// The user's profile folder (where `Downloads` lives).
    pub home: PathBuf,
}

/// Brings the file's `key=value` pairs from version `from` to [SETTINGS_VERSION], one step at a
/// time. A file from a newer build is returned as it is.
pub fn migrate(
    from: u32,
    mut pairs: Vec<(String, String)>,
    cx: &MigrateCtx,
) -> (u32, Vec<(String, String)>) {
    let mut v = from;
    while v < SETTINGS_VERSION {
        if v == 0 && cx.existing_install && !pairs.iter().any(|(k, _)| k == "save_dir") {
            // Before 1 the default folder was never written: pin it, so a new default for new installs can't move it.
            let old = cx.home.join("Downloads").join(LEGACY_SAVE_FOLDER);
            pairs.push(("save_dir".into(), old.display().to_string()));
        }
        v += 1;
    }
    (v.max(from), pairs)
}

/// `key=value` lines in order, repeats kept (`notify_app` repeats); lines without `=` are skipped.
fn parse_pairs(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter_map(|l| l.split_once('='))
        .map(|(k, v)| (k.trim().to_owned(), v.trim().to_owned()))
        .collect()
}

/// Logs live with the rest of the data, unless the user picks a folder.
fn default_log_dir(settings_file: &Path) -> PathBuf {
    settings_file
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("logs")
}

fn parse_bool(v: &str) -> Option<bool> {
    match v {
        "true" | "1" | "on" => Some(true),
        "false" | "0" | "off" => Some(false),
        _ => None,
    }
}

impl Settings {
    /// Reads the file, migrates it and saves it back if a migration changed it, so each step
    /// runs once. A missing file is written at once: with today's defaults on a new install (so its
    /// next start, which already has an identity, isn't taken for an old one), and migrated from
    /// version 0 on an old install that never saved its settings (Relay 0.2.0 wrote the file only
    /// when something changed).
    pub fn load_and_migrate(path: &Path, cx: &MigrateCtx) -> Self {
        let Ok(text) = std::fs::read_to_string(path) else {
            if !cx.existing_install {
                let s = Self::load(path);
                let _ = s.save(path);
                return s;
            }
            let (to, migrated) = migrate(0, Vec::new(), cx);
            let mut s = Self::from_pairs(&migrated, path);
            s.version = to;
            let _ = s.save(path);
            return s;
        };
        let pairs = parse_pairs(&text);
        let from = pairs
            .iter()
            .find(|(k, _)| k == "version")
            .and_then(|(_, v)| v.parse::<u32>().ok())
            .unwrap_or(0);
        let (to, migrated) = migrate(from, pairs.clone(), cx);
        if to == from {
            return Self::from_pairs(&pairs, path);
        }
        let mut s = Self::from_pairs(&migrated, path);
        s.version = to;
        let _ = s.save(path);
        s
    }

    pub fn load(path: &Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(text) => Self::from_pairs(&parse_pairs(&text), path),
            Err(_) => Self {
                log_dir: default_log_dir(path),
                ..Self::default()
            },
        }
    }

    fn from_pairs(pairs: &[(String, String)], path: &Path) -> Self {
        let mut s = Self::default();
        // A file without a version is from before versions: not migrated yet until load_and_migrate says so.
        if !pairs.iter().any(|(k, _)| k == "version") {
            s.version = 0;
        }
        for (k, v) in pairs {
            let (k, v) = (k.as_str(), v.as_str());
            match k {
                "version" => s.version = v.parse().unwrap_or(s.version),
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
                "look" => s.look = u8::from(v == "relay"),
                "allow_control" => s.allow_control = parse_bool(v).unwrap_or(s.allow_control),
                "skip_private" => s.skip_private = parse_bool(v).unwrap_or(s.skip_private),
                "keep_in_history" => s.keep_in_history = parse_bool(v).unwrap_or(s.keep_in_history),
                "toast_show_text" => s.toast_show_text = parse_bool(v).unwrap_or(s.toast_show_text),
                "auto_copy_shared_text" => {
                    s.auto_copy_shared_text = parse_bool(v).unwrap_or(s.auto_copy_shared_text)
                }
                "share_images" => s.share_images = parse_bool(v).unwrap_or(s.share_images),
                "auto_accept" => s.auto_accept = parse_bool(v).unwrap_or(s.auto_accept),
                "allow_shortcut_files" => {
                    s.allow_shortcut_files = parse_bool(v).unwrap_or(s.allow_shortcut_files)
                }
                "save_dir" => {
                    if !v.is_empty() {
                        s.save_dir = PathBuf::from(v)
                    }
                }
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
                "keep_history" => s.keep_history = parse_bool(v).unwrap_or(s.keep_history),
                "keep_logs" => s.keep_logs = parse_bool(v).unwrap_or(s.keep_logs),
                "audio_skip_confirm_pc_to_phone" => {
                    s.audio_skip_confirm_pc_to_phone =
                        parse_bool(v).unwrap_or(s.audio_skip_confirm_pc_to_phone)
                }
                "paused" => s.paused = parse_bool(v).unwrap_or(s.paused),
                "timeline_days" => s.timeline_days = v.chars().take(4096).collect(),
                "log_dir" => {
                    if !v.is_empty() {
                        s.log_dir = PathBuf::from(v)
                    }
                }
                "main_window_rect" => {
                    let n: Vec<i32> = v.split(',').filter_map(|f| f.trim().parse().ok()).collect();
                    if let [x, y, w, h] = n[..] {
                        s.main_window_rect = Some((x, y, w, h));
                    }
                }
                "notify_app" => {
                    let mut f = v.splitn(3, '\t');
                    if let (Some(m), Some(pkg), Some(name)) = (f.next(), f.next(), f.next()) {
                        if !pkg.is_empty() {
                            s.notify_apps.push((pkg.into(), name.into(), m == "1"));
                        }
                    }
                }
                _ => s.extra.push((k.to_owned(), v.to_owned())),
            }
        }
        if s.log_dir.as_os_str().is_empty() {
            s.log_dir = default_log_dir(path);
        }
        s
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let mut text = format!("version={}\n", self.version);
        text += &format!(
            "universal_clipboard={}\nskip_private={}\nkeep_in_history={}\nshare_images={}\nauto_accept={}\ntheme={}\nlook={}\nallow_control={}\nsave_dir={}\n",
            self.universal_clipboard,
            self.skip_private,
            self.keep_in_history,
            self.share_images,
            self.auto_accept,
            ["system", "light", "dark"][self.theme.min(2) as usize],
            ["native", "relay"][self.look.min(1) as usize],
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
            "screen_pinned={}\nscreen_dim={}\nscreen_sharp={}\nkeep_history={}\n",
            self.screen_pinned, self.screen_dim, self.screen_sharp, self.keep_history
        );
        text += &format!(
            "keep_logs={}\naudio_skip_confirm_pc_to_phone={}\npaused={}\nauto_copy_shared_text={}\ntoast_show_text={}\nallow_shortcut_files={}\nlog_dir={}\n",
            self.keep_logs,
            self.audio_skip_confirm_pc_to_phone,
            self.paused,
            self.auto_copy_shared_text,
            self.toast_show_text,
            self.allow_shortcut_files,
            // Not written when it is the default, so a moved folder keeps its logs with it.
            if self.log_dir == default_log_dir(path) {
                PathBuf::new()
            } else {
                self.log_dir.clone()
            }
            .display()
        );
        if !self.timeline_days.is_empty() {
            text += &format!("timeline_days={}\n", self.timeline_days);
        }
        if let Some((x, y, w, h)) = self.main_window_rect {
            text += &format!("main_window_rect={x},{y},{w},{h}\n");
        }
        let clean = |s: &str| s.replace(['\t', '\n', '\r'], " ");
        for (pkg, name, muted) in &self.notify_apps {
            text += &format!(
                "notify_app={}\t{}\t{}\n",
                u8::from(*muted),
                clean(pkg),
                clean(name)
            );
        }
        for (k, v) in &self.extra {
            text += &format!("{}={}\n", clean(k), clean(v));
        }
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, text)?;
        std::fs::rename(tmp, path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mig_dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("relay-mig-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn cx(existing_install: bool) -> MigrateCtx {
        MigrateCtx {
            existing_install,
            home: PathBuf::from("/home/u"),
        }
    }

    fn phone_bridge_dir() -> PathBuf {
        PathBuf::from("/home/u")
            .join("Downloads")
            .join("Phone Bridge")
    }

    #[test]
    fn old_install_without_save_dir_keeps_phone_bridge() {
        let d = mig_dir("old");
        let f = d.join("settings.txt");
        std::fs::write(&f, "theme=dark\n").unwrap();
        let s = Settings::load_and_migrate(&f, &cx(true));
        assert_eq!(s.save_dir, phone_bridge_dir());
        assert_eq!(s.version, SETTINGS_VERSION);
    }

    #[test]
    fn old_install_with_save_dir_is_unchanged() {
        let d = mig_dir("pinned");
        let f = d.join("settings.txt");
        std::fs::write(&f, "save_dir=D:\\Inbox\n").unwrap();
        let s = Settings::load_and_migrate(&f, &cx(true));
        assert_eq!(s.save_dir, PathBuf::from("D:\\Inbox"));
    }

    #[test]
    fn new_install_has_no_pin() {
        let d = mig_dir("new");
        let f = d.join("settings.txt");
        // No file at all, and a file on a install that has no identity yet.
        let none = Settings::load_and_migrate(&f, &cx(false));
        assert_ne!(none.save_dir, phone_bridge_dir());
        let text =
            std::fs::read_to_string(&f).expect("a new install's first start writes the file");
        assert!(
            text.starts_with(&format!("version={SETTINGS_VERSION}\n")),
            "{text}"
        );
        std::fs::remove_file(&f).unwrap();
        std::fs::write(&f, "theme=dark\n").unwrap();
        let s = Settings::load_and_migrate(&f, &cx(false));
        assert_ne!(s.save_dir, phone_bridge_dir());
    }

    #[test]
    fn new_install_keeps_relay_on_the_second_start() {
        let d = mig_dir("new2");
        let f = d.join("settings.txt");
        let first = Settings::load_and_migrate(&f, &cx(false));
        // By the second start the first one made an identity, so the install looks existing.
        let second = Settings::load_and_migrate(&f, &cx(true));
        assert_ne!(second.save_dir, phone_bridge_dir());
        assert_eq!(first.save_dir, second.save_dir);
    }

    #[test]
    fn old_install_that_never_saved_keeps_phone_bridge() {
        // Relay 0.2.0 wrote settings.txt only on a settings change or when the main window moved.
        let d = mig_dir("never");
        let f = d.join("settings.txt");
        let first = Settings::load_and_migrate(&f, &cx(true));
        assert_eq!(first.save_dir, phone_bridge_dir());
        let text = std::fs::read_to_string(&f).expect("the pin is saved");
        assert!(
            text.starts_with(&format!("version={SETTINGS_VERSION}\n")),
            "{text}"
        );
        let second = Settings::load_and_migrate(&f, &cx(true));
        assert_eq!(second.save_dir, phone_bridge_dir());
    }

    #[test]
    fn v1_settings_gain_native_look() {
        let d = mig_dir("v1look");
        let f = d.join("settings.txt");
        std::fs::write(&f, "version=1\ntheme=dark\n").unwrap();
        let s = Settings::load_and_migrate(&f, &cx(true));
        assert_eq!(
            s.look, 0,
            "an install that never chose keeps the native look"
        );
        assert_eq!(s.version, 2);
        assert_eq!(s.theme, 2, "the other settings are untouched");
        let text = std::fs::read_to_string(&f).unwrap();
        assert!(text.starts_with("version=2\n"), "{text}");
        assert!(text.contains("look=native\n"), "{text}");
    }

    #[test]
    fn look_round_trips_and_unknown_values_read_as_native() {
        let d = mig_dir("look");
        let f = d.join("settings.txt");
        let s = Settings {
            look: 1,
            ..Settings::default()
        };
        s.save(&f).unwrap();
        let text = std::fs::read_to_string(&f).unwrap();
        assert!(text.contains("look=relay\n"), "{text}");
        assert_eq!(Settings::load(&f).look, 1);
        std::fs::write(&f, format!("version={SETTINGS_VERSION}\nlook=sparkly\n")).unwrap();
        assert_eq!(Settings::load(&f).look, 0);
    }

    #[test]
    fn current_version_is_unchanged() {
        let d = mig_dir("cur");
        let f = d.join("settings.txt");
        std::fs::write(&f, format!("version={SETTINGS_VERSION}\ntheme=dark\n")).unwrap();
        let before = std::fs::read_to_string(&f).unwrap();
        let s = Settings::load_and_migrate(&f, &cx(true));
        assert_ne!(
            s.save_dir,
            phone_bridge_dir(),
            "no pin once at the current version"
        );
        assert_eq!(
            std::fs::read_to_string(&f).unwrap(),
            before,
            "nothing rewritten"
        );
    }

    #[test]
    fn newer_file_is_left_alone() {
        let d = mig_dir("newer");
        let f = d.join("settings.txt");
        std::fs::write(&f, "version=9\nfuture_key=1\n").unwrap();
        let before = std::fs::read_to_string(&f).unwrap();
        let s = Settings::load_and_migrate(&f, &cx(true));
        assert_eq!(s.version, 9);
        assert_eq!(std::fs::read_to_string(&f).unwrap(), before);
        // A later save keeps the newer version and its keys.
        s.save(&f).unwrap();
        let after = std::fs::read_to_string(&f).unwrap();
        assert!(after.starts_with("version=9\n"), "{after}");
        assert!(after.contains("future_key=1"), "{after}");
    }

    #[test]
    fn migration_is_saved_so_it_runs_once() {
        let d = mig_dir("once");
        let f = d.join("settings.txt");
        std::fs::write(&f, "theme=dark\n").unwrap();
        let first = Settings::load_and_migrate(&f, &cx(true));
        let text = std::fs::read_to_string(&f).unwrap();
        assert!(
            text.starts_with(&format!("version={SETTINGS_VERSION}\n")),
            "{text}"
        );
        assert!(text.contains(LEGACY_SAVE_FOLDER), "{text}");
        // Later, with the default folder changed for new installs, this install keeps its pin.
        let second = Settings::load_and_migrate(&f, &cx(true));
        assert_eq!(first.save_dir, second.save_dir);
    }

    #[test]
    fn unknown_keys_survive_a_save() {
        let d = mig_dir("extra");
        let f = d.join("settings.txt");
        std::fs::write(&f, "version=1\nfuture_key=abc\ntheme=light\n").unwrap();
        let s = Settings::load(&f);
        s.save(&f).unwrap();
        let text = std::fs::read_to_string(&f).unwrap();
        assert!(text.contains("future_key=abc"), "{text}");
    }

    #[test]
    fn settings_load_survives_random_files() {
        // Pseudo-random settings files (keys Relay knows, garbage values,
        // stray characters, lines without "="): loading never panics, and
        // after one save what loads is stable.
        let keys = [
            "universal_clipboard",
            "auto_accept",
            "theme",
            "look",
            "save_dir",
            "notify_content",
            "notify_app",
            "version",
            "main_window_rect",
            "timeline_days",
            "log_dir",
            "paused",
            "x",
            "",
        ];
        let alphabet: Vec<char> = "=,:;019- truefalsx\\/\r\t\u{0}\u{e9}\u{202e}"
            .chars()
            .collect();
        let dir =
            std::env::temp_dir().join(format!("relay settings random {}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("settings.txt");
        let mut seed = 0x9e37_79b9_7f4a_7c15u64;
        let mut next = || {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (seed >> 33) as usize
        };
        for _ in 0..2000 {
            let mut text = String::new();
            for _ in 0..next() % 12 {
                text.push_str(keys[next() % keys.len()]);
                if next() % 4 != 0 {
                    text.push('=');
                }
                for _ in 0..next() % 20 {
                    text.push(alphabet[next() % alphabet.len()]);
                }
                text.push('\n');
            }
            std::fs::write(&path, &text).unwrap();
            Settings::load(&path).save(&path).unwrap();
            let once = Settings::load(&path);
            once.save(&path).unwrap();
            let twice = Settings::load(&path);
            assert_eq!(format!("{once:?}"), format!("{twice:?}"), "{text:?}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn new_installs_save_to_relay() {
        let s = Settings::default();
        assert!(s.save_dir.ends_with("Relay"), "{:?}", s.save_dir);
    }

    #[test]
    fn defaults_are_private_by_default() {
        let s = Settings::default();
        assert!(!s.universal_clipboard, "clipboard sync must be opt-in");
        assert!(!s.share_images, "image sync must be opt-in");
        assert!(s.skip_private);
        assert!(!s.keep_in_history);
        assert!(!s.auto_copy_shared_text, "shared text waits for Copy");
        assert!(
            !s.toast_show_text,
            "toasts never show shared text by default"
        );
        assert!(!s.allow_shortcut_files, "shortcut files are refused");
        assert!(s.keep_history, "the timeline records by default");
        assert_eq!(s.main_window_rect, None);
        assert!(s.keep_logs, "logs are kept by default");
        assert!(!s.paused, "Relay starts unpaused");
        assert_eq!(s.log_dir, PathBuf::new(), "logs follow the data folder");
    }

    #[test]
    fn logs_live_beside_the_settings_unless_the_user_chose_a_folder() {
        let dir = std::env::temp_dir().join(format!("relay logs {}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("settings.txt");
        let _ = std::fs::remove_file(&path);
        assert_eq!(Settings::load(&path).log_dir, dir.join("logs"));
        Settings::load(&path).save(&path).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(
            text.contains(
                "log_dir=
"
            ),
            "a moved folder must not keep the old path"
        );
        let mut chosen = Settings::load(&path);
        chosen.log_dir = PathBuf::from(r"E:\Logs");
        chosen.save(&path).unwrap();
        assert_eq!(Settings::load(&path).log_dir, PathBuf::from(r"E:\Logs"));
        let _ = std::fs::remove_dir_all(dir);
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
            auto_copy_shared_text: true,
            toast_show_text: true,
            allow_shortcut_files: true,
            share_images: true,
            auto_accept: false,
            save_dir: PathBuf::from(r"D:\Inbox"),
            theme: 2,
            look: 1,
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
            keep_history: false,
            keep_logs: false,
            audio_skip_confirm_pc_to_phone: true,
            paused: true,
            log_dir: PathBuf::from(r"E:\Logs"),
            main_window_rect: Some((-1200, 40, 1000, 700)),
            timeline_days: "20000:1,19999:0".into(),
            version: SETTINGS_VERSION,
            extra: vec![("future_key".into(), "7".into())],
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
            "garbage\nuniversal_clipboard=maybe\nauto_accept=off\nfuture_key=1\nmain_window_rect=1,2,x,4\n",
        )
        .unwrap();
        let t = Settings::load(&path);
        assert_eq!(t.main_window_rect, None, "a malformed rect is ignored");
        assert!(!t.universal_clipboard);
        assert!(!t.auto_accept);
        assert_eq!(
            Settings::load(&dir.join("missing.txt")),
            Settings {
                log_dir: dir.join("logs"),
                ..Settings::default()
            }
        );
    }
}
