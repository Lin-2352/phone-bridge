//! What the settings window shows, as plain data (tested on every
//! platform); `settings_window` paints it.

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Key {
    Share,
    Passwords,
    Keep,
    Auto,
    SendTo,
    Startup,
    Control,
    Notify,
    NotifyLock,
    NotifyClear,
    NotifyOpen,
    NotifyOtp,
    NotifyQuietMirror,
}

/// Three-way choices, painted as segmented buttons.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum ChoiceKey {
    /// Full / hidden while locked / sender only.
    Content,
    /// Follow the phone / always alert / always quiet.
    Sound,
}

#[derive(Clone, Default)]
pub struct Model {
    /// 0 system, 1 light, 2 dark.
    pub theme: u8,
    pub share: bool,
    pub passwords: bool,
    pub keep: bool,
    pub auto: bool,
    pub save_dir: String,
    pub sendto: bool,
    pub startup: bool,
    pub control: bool,
    /// (name, fingerprint)
    pub devices: Vec<(String, String)>,
    pub my_fingerprint: String,
    pub notify: bool,
    pub notify_lock: bool,
    pub notify_clear: bool,
    pub notify_open: bool,
    pub notify_otp: bool,
    pub notify_quiet_mirror: bool,
    pub content: u8,
    pub sound: u8,
    /// Phone apps seen so far: (name, shown). Off = muted.
    pub apps: Vec<(String, bool)>,
}

impl Model {
    pub fn get(&self, k: Key) -> bool {
        match k {
            Key::Share => self.share,
            Key::Passwords => self.passwords,
            Key::Keep => self.keep,
            Key::Auto => self.auto,
            Key::SendTo => self.sendto,
            Key::Startup => self.startup,
            Key::Control => self.control,
            Key::Notify => self.notify,
            Key::NotifyLock => self.notify_lock,
            Key::NotifyClear => self.notify_clear,
            Key::NotifyOpen => self.notify_open,
            Key::NotifyOtp => self.notify_otp,
            Key::NotifyQuietMirror => self.notify_quiet_mirror,
        }
    }

    pub fn choice(&self, k: ChoiceKey) -> u8 {
        match k {
            ChoiceKey::Content => self.content,
            ChoiceKey::Sound => self.sound,
        }
    }
}

pub enum Action {
    Theme(u8),
    Toggle(Key),
    ChangeFolder,
    OpenFolder,
    Remove(usize),
    Pair,
    Choice(ChoiceKey, u8),
    /// Flip the phone app at this index between shown and muted.
    Mute(usize),
}

#[derive(Clone)]
pub enum Row {
    Header(&'static str),
    Theme,
    Toggle(Key, &'static str, &'static str),
    Choice(ChoiceKey, &'static str, [&'static str; 3]),
    /// A phone app, by index into `Model::apps`.
    App(usize),
    Folder,
    Device(usize),
    Pair,
    About,
}

pub fn rows(m: &Model) -> Vec<Row> {
    let mut v = vec![
        Row::Header("Appearance"),
        Row::Theme,
        Row::Header("Clipboard"),
        Row::Toggle(
            Key::Share,
            "Share clipboard",
            "Copy on one device, paste on the other",
        ),
        Row::Toggle(
            Key::Passwords,
            "Passwords and codes",
            "Also share text a password manager marks private",
        ),
        Row::Toggle(
            Key::Keep,
            "Keep received text in clipboard history",
            "Show it in Win+V history (never for passwords)",
        ),
        Row::Header("Phone notifications"),
        Row::Toggle(
            Key::Notify,
            "Show phone notifications",
            "Notifications from your phone appear here",
        ),
        Row::Choice(
            ChoiceKey::Content,
            "Content",
            ["Full", "Hide when locked", "Sender only"],
        ),
        Row::Toggle(
            Key::NotifyLock,
            "Show on the lock screen",
            "Otherwise they go straight to the notification centre",
        ),
        Row::Choice(
            ChoiceKey::Sound,
            "Sound and pop-ups",
            ["Follow phone", "Always alert", "Always quiet"],
        ),
        Row::Toggle(
            Key::NotifyClear,
            "Clearing here clears on the phone",
            "Off: clears only on this PC",
        ),
        Row::Toggle(
            Key::NotifyOpen,
            "Clicking opens it on the phone",
            "Also shows the phone screen when available",
        ),
        Row::Toggle(
            Key::NotifyOtp,
            "Copy one-time codes automatically",
            "Cleared after 60 s if unchanged; never while locked",
        ),
        Row::Toggle(
            Key::NotifyQuietMirror,
            "Quiet while the phone screen is open",
            "Toasts still go to the notification centre",
        ),
    ];
    if !m.apps.is_empty() {
        v.push(Row::Header("Apps"));
        v.extend((0..m.apps.len()).map(Row::App));
    }
    v.extend([
        Row::Header("Files"),
        Row::Toggle(
            Key::Auto,
            "Accept files automatically",
            "From your paired devices only",
        ),
        Row::Folder,
        Row::Toggle(
            Key::SendTo,
            "Show in Explorer \u{201c}Send to\u{201d}",
            "Right-click files, Send to, Phone Bridge",
        ),
        Row::Header("General"),
        Row::Toggle(
            Key::Startup,
            "Start with Windows",
            "Phone Bridge waits quietly in the tray",
        ),
        Row::Toggle(
            Key::Control,
            "Let my phone control this PC",
            "Use the phone as a touchpad and keyboard",
        ),
        Row::Header("Devices"),
    ]);
    v.extend((0..m.devices.len()).map(Row::Device));
    v.push(Row::Pair);
    v.push(Row::Header("About"));
    v.push(Row::About);
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    fn title(r: &Row) -> String {
        match r {
            Row::Header(t) => format!("# {t}"),
            Row::Toggle(_, t, _) => (*t).to_owned(),
            Row::Choice(_, t, _) => (*t).to_owned(),
            Row::App(i) => format!("app {i}"),
            _ => "?".into(),
        }
    }

    #[test]
    fn rows_include_notification_section_in_order() {
        let m = Model {
            apps: vec![("WhatsApp".into(), true)],
            ..Default::default()
        };
        let all: Vec<String> = rows(&m).iter().map(title).collect();
        let from = all
            .iter()
            .position(|t| t == "# Phone notifications")
            .unwrap();
        let to = all.iter().position(|t| t == "# Files").unwrap();
        assert_eq!(
            all[from + 1..to],
            [
                "Show phone notifications",
                "Content",
                "Show on the lock screen",
                "Sound and pop-ups",
                "Clearing here clears on the phone",
                "Clicking opens it on the phone",
                "Copy one-time codes automatically",
                "Quiet while the phone screen is open",
                "# Apps",
                "app 0",
            ]
        );
    }
}
