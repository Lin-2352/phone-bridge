//! Phone notifications as toasts: what to show for a notification under the
//! user's settings, and which toasts belong to which phone notification.
//! Pure (no Win32), so it is tested on every platform.

use std::collections::{HashMap, HashSet};

use pb_identity::PeerId;
use pb_protocol::control::Notification;
use pb_transport::notify::key_hash;

use crate::settings::Settings;

pub struct Ctx {
    pub locked: bool,
    pub mirroring_focused: bool,
}

#[derive(Debug, PartialEq)]
pub struct Presentation {
    pub title: String,
    pub lines: Vec<String>,
    pub popup: bool,
    pub silent: bool,
    pub otp_label: Option<String>,
    pub show_picture: bool,
}

/// What a toast shows for `n`, or `None` when it must not appear at all.
pub fn present(n: &Notification, s: &Settings, ctx: &Ctx) -> Option<Presentation> {
    let muted = s
        .notify_apps
        .iter()
        .any(|(pkg, _, muted)| *muted && *pkg == n.package);
    if !s.notify_enabled || muted {
        return None;
    }
    let code_digits = (!n.otp.is_empty()).then(|| format!("Copy {}", n.otp));
    let code_hidden = (!n.otp.is_empty()).then(|| "Copy code".to_owned());
    let hidden = s.notify_content == 1 && ctx.locked;
    let (title, lines, otp_label, show_picture) = if hidden {
        let app = if n.app_name.is_empty() {
            &n.package
        } else {
            &n.app_name
        };
        (
            app.clone(),
            vec!["New notification".to_owned()],
            code_hidden,
            false,
        )
    } else if s.notify_content == 2 {
        (n.title.clone(), Vec::new(), code_hidden, false)
    } else {
        let lines = if n.lines.is_empty() {
            vec![n.text.clone()]
        } else {
            n.lines.clone()
        };
        (n.title.clone(), lines, code_digits, true)
    };
    let (mut popup, mut silent) = match s.notify_sound {
        1 => (true, false),
        2 => (false, true),
        _ => (!n.quiet, n.quiet),
    };
    popup &= !(ctx.locked && !s.notify_lock_screen);
    popup &= !(ctx.mirroring_focused && s.notify_quiet_mirroring);
    if n.call && popup {
        silent = false;
    }
    Some(Presentation {
        title,
        lines,
        popup,
        silent,
        otp_label,
        show_picture,
    })
}

/// The toast tag for one phone notification (Windows allows 64 characters).
pub fn tag(peer: &PeerId, key: &str) -> String {
    key_hash(&format!("{peer}|{key}"))
}

/// Toasts of one conversation share a group; other notifications get their own.
pub fn group(peer: &PeerId, n: &Notification) -> String {
    if n.conversation.is_empty() {
        key_hash(&format!("{peer}|key|{}", n.key))
    } else {
        key_hash(&format!("{peer}|chat|{}", n.conversation))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub peer: PeerId,
    pub key: String,
    pub package: String,
    pub otp: String,
}

/// Toasts currently shown for phone notifications, by tag.
#[derive(Default)]
pub struct Mirror {
    by_tag: HashMap<String, Entry>,
}

impl Mirror {
    pub fn insert(&mut self, tag: String, e: Entry) {
        self.by_tag.insert(tag, e);
    }

    pub fn get(&self, tag: &str) -> Option<&Entry> {
        self.by_tag.get(tag)
    }

    pub fn remove_key(&mut self, peer: &PeerId, key: &str) -> Option<String> {
        let t = tag(peer, key);
        self.by_tag.remove(&t).map(|_| t)
    }

    /// Keep `peer`'s toasts whose keys are in `live`; returns the tags removed.
    pub fn retain_live(&mut self, peer: &PeerId, live: &[String]) -> Vec<String> {
        let live: HashSet<&str> = live.iter().map(String::as_str).collect();
        self.take(|e| e.peer == *peer && !live.contains(e.key.as_str()))
    }

    pub fn remove_peer(&mut self, peer: &PeerId) -> Vec<String> {
        self.take(|e| e.peer == *peer)
    }

    pub fn remove_package(&mut self, package: &str) -> Vec<String> {
        self.take(|e| e.package == package)
    }

    /// Entries whose toast is no longer in Windows' history (the user
    /// cleared it there); each is reported once.
    pub fn vanished(&mut self, history: &HashSet<String>) -> Vec<Entry> {
        let gone: Vec<String> = self
            .by_tag
            .keys()
            .filter(|t| !history.contains(*t))
            .cloned()
            .collect();
        gone.iter().filter_map(|t| self.by_tag.remove(t)).collect()
    }

    pub fn is_empty(&self) -> bool {
        self.by_tag.is_empty()
    }

    fn take(&mut self, drop: impl Fn(&Entry) -> bool) -> Vec<String> {
        let tags: Vec<String> = self
            .by_tag
            .iter()
            .filter(|(_, e)| drop(e))
            .map(|(t, _)| t.clone())
            .collect();
        for t in &tags {
            self.by_tag.remove(t);
        }
        tags
    }
}

/// Clear a copied one-time code only if the clipboard still holds exactly
/// it (nothing was copied since), so newer text is never wiped.
pub fn otp_should_clear(set_seq: u32, now_seq: u32, now_text: Option<&str>, code: &str) -> bool {
    set_seq == now_seq && now_text == Some(code)
}

#[cfg(test)]
mod tests {
    use pb_identity::Identity;

    use super::*;

    fn peer(seed: u8) -> PeerId {
        Identity::from_seed([seed; 32]).peer_id()
    }

    fn note() -> Notification {
        Notification {
            key: "k1".into(),
            package: "com.whatsapp".into(),
            app_name: "WhatsApp".into(),
            title: "Mom".into(),
            text: "Your code is 482913".into(),
            conversation: "Mom".into(),
            lines: vec!["Mom: hi".into(), "Mom: Your code is 482913".into()],
            otp: "482913".into(),
            picture: vec![1],
            ..Default::default()
        }
    }

    const OPEN: Ctx = Ctx {
        locked: false,
        mirroring_focused: false,
    };
    const LOCKED: Ctx = Ctx {
        locked: true,
        mirroring_focused: false,
    };

    #[test]
    fn muted_app_is_not_shown() {
        let s = Settings {
            notify_apps: vec![("com.whatsapp".into(), "WhatsApp".into(), true)],
            ..Default::default()
        };
        assert_eq!(present(&note(), &s, &OPEN), None);
    }

    #[test]
    fn disabled_shows_nothing() {
        let s = Settings {
            notify_enabled: false,
            ..Default::default()
        };
        assert_eq!(present(&note(), &s, &OPEN), None);
    }

    #[test]
    fn default_unlocked_shows_content() {
        let p = present(&note(), &Settings::default(), &OPEN).unwrap();
        assert_eq!(p.title, "Mom");
        assert_eq!(p.lines, ["Mom: hi", "Mom: Your code is 482913"]);
        assert_eq!(p.otp_label.as_deref(), Some("Copy 482913"));
        assert!(p.show_picture && p.popup && !p.silent);
    }

    #[test]
    fn locked_hides_content_in_default_mode() {
        let p = present(&note(), &Settings::default(), &LOCKED).unwrap();
        assert_eq!(p.title, "WhatsApp");
        assert_eq!(p.lines, ["New notification"]);
        assert_eq!(p.otp_label.as_deref(), Some("Copy code"));
        assert!(!p.show_picture);
    }

    #[test]
    fn full_mode_shows_content_even_when_locked() {
        let s = Settings {
            notify_content: 0,
            notify_lock_screen: true,
            ..Default::default()
        };
        let p = present(&note(), &s, &LOCKED).unwrap();
        assert_eq!(p.title, "Mom");
        assert!(p.show_picture && p.popup);
    }

    #[test]
    fn sender_only_never_shows_text_or_code_digits() {
        let s = Settings {
            notify_content: 2,
            ..Default::default()
        };
        let p = present(&note(), &s, &OPEN).unwrap();
        assert_eq!(p.title, "Mom");
        assert!(p.lines.is_empty());
        assert!(!p.show_picture);
        assert_eq!(p.otp_label.as_deref(), Some("Copy code"));
    }

    #[test]
    fn text_is_used_when_there_are_no_lines() {
        let n = Notification {
            lines: vec![],
            ..note()
        };
        let p = present(&n, &Settings::default(), &OPEN).unwrap();
        assert_eq!(p.lines, ["Your code is 482913"]);
    }

    #[test]
    fn follow_phone_keeps_quiet_ones_quiet() {
        let n = Notification {
            quiet: true,
            ..note()
        };
        let p = present(&n, &Settings::default(), &OPEN).unwrap();
        assert!(!p.popup && p.silent);
    }

    #[test]
    fn always_alert_overrides_phone_quiet() {
        let n = Notification {
            quiet: true,
            ..note()
        };
        let s = Settings {
            notify_sound: 1,
            ..Default::default()
        };
        let p = present(&n, &s, &OPEN).unwrap();
        assert!(p.popup && !p.silent);
    }

    #[test]
    fn always_quiet_never_pops_up() {
        let s = Settings {
            notify_sound: 2,
            ..Default::default()
        };
        let p = present(&note(), &s, &OPEN).unwrap();
        assert!(!p.popup && p.silent);
    }

    #[test]
    fn lock_screen_off_suppresses_popup_while_locked() {
        let p = present(&note(), &Settings::default(), &LOCKED).unwrap();
        assert!(!p.popup);
        let s = Settings {
            notify_lock_screen: true,
            ..Default::default()
        };
        assert!(present(&note(), &s, &LOCKED).unwrap().popup);
    }

    #[test]
    fn mirroring_focus_suppresses_popup() {
        let ctx = Ctx {
            locked: false,
            mirroring_focused: true,
        };
        assert!(!present(&note(), &Settings::default(), &ctx).unwrap().popup);
        let s = Settings {
            notify_quiet_mirroring: false,
            ..Default::default()
        };
        assert!(present(&note(), &s, &ctx).unwrap().popup);
    }

    #[test]
    fn a_popping_call_is_never_silent() {
        let n = Notification {
            call: true,
            quiet: false,
            ..note()
        };
        let p = present(&n, &Settings::default(), &OPEN).unwrap();
        assert!(p.popup && !p.silent);
    }

    #[test]
    fn tags_are_stable_and_16_hex() {
        let t = tag(&peer(1), "k1");
        assert_eq!(t, tag(&peer(1), "k1"));
        assert_eq!(t.len(), 16);
        assert!(t.bytes().all(|b| b.is_ascii_hexdigit()));
        assert_ne!(t, tag(&peer(2), "k1"));
        assert_ne!(t, tag(&peer(1), "k2"));
    }

    #[test]
    fn same_conversation_shares_a_group() {
        let a = note();
        let b = Notification {
            key: "k2".into(),
            ..note()
        };
        assert_eq!(group(&peer(1), &a), group(&peer(1), &b));
        let c = Notification {
            key: "k3".into(),
            conversation: String::new(),
            ..note()
        };
        assert_ne!(group(&peer(1), &a), group(&peer(1), &c));
        assert_eq!(group(&peer(1), &c).len(), 16);
    }

    fn entry(p: PeerId, key: &str, package: &str) -> Entry {
        Entry {
            peer: p,
            key: key.into(),
            package: package.into(),
            otp: String::new(),
        }
    }

    #[test]
    fn retain_live_returns_stale_tags() {
        let mut m = Mirror::default();
        for k in ["a", "b", "c"] {
            m.insert(tag(&peer(1), k), entry(peer(1), k, "p"));
        }
        m.insert(tag(&peer(2), "a"), entry(peer(2), "a", "p"));
        let mut gone = m.retain_live(&peer(1), &["b".into()]);
        gone.sort();
        let mut want = vec![tag(&peer(1), "a"), tag(&peer(1), "c")];
        want.sort();
        assert_eq!(gone, want);
        assert!(m.get(&tag(&peer(1), "b")).is_some());
        assert!(
            m.get(&tag(&peer(2), "a")).is_some(),
            "other phones untouched"
        );
    }

    #[test]
    fn remove_key_returns_its_tag() {
        let mut m = Mirror::default();
        m.insert(tag(&peer(1), "a"), entry(peer(1), "a", "p"));
        assert_eq!(m.remove_key(&peer(1), "a"), Some(tag(&peer(1), "a")));
        assert_eq!(m.remove_key(&peer(1), "a"), None);
        assert!(m.is_empty());
    }

    #[test]
    fn unpair_removes_that_peers_toasts() {
        let mut m = Mirror::default();
        m.insert(tag(&peer(1), "a"), entry(peer(1), "a", "p"));
        m.insert(tag(&peer(2), "b"), entry(peer(2), "b", "p"));
        assert_eq!(m.remove_peer(&peer(1)), vec![tag(&peer(1), "a")]);
        assert!(m.get(&tag(&peer(1), "a")).is_none());
        assert!(m.get(&tag(&peer(2), "b")).is_some());
    }

    #[test]
    fn remove_package_removes_only_that_app() {
        let mut m = Mirror::default();
        m.insert(tag(&peer(1), "a"), entry(peer(1), "a", "com.noisy"));
        m.insert(tag(&peer(1), "b"), entry(peer(1), "b", "com.quiet"));
        assert_eq!(m.remove_package("com.noisy"), vec![tag(&peer(1), "a")]);
        assert!(m.get(&tag(&peer(1), "b")).is_some());
    }

    #[test]
    fn vanished_reports_cleared_toasts_once() {
        let mut m = Mirror::default();
        m.insert(tag(&peer(1), "a"), entry(peer(1), "a", "p"));
        m.insert(tag(&peer(1), "b"), entry(peer(1), "b", "p"));
        let history: HashSet<String> = [tag(&peer(1), "b")].into_iter().collect();
        assert_eq!(m.vanished(&history), vec![entry(peer(1), "a", "p")]);
        assert!(m.vanished(&history).is_empty());
    }

    #[test]
    fn otp_clear_only_if_unchanged() {
        assert!(otp_should_clear(7, 7, Some("482913"), "482913"));
        assert!(!otp_should_clear(7, 8, Some("482913"), "482913"));
        assert!(!otp_should_clear(7, 7, Some("other"), "482913"));
        assert!(!otp_should_clear(7, 7, None, "482913"));
    }
}
