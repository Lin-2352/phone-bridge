//! Text shared from the phone (its share sheet) waits for the user on the PC:
//! a toast offers Copy, unless "Copy shared text automatically" is on. Any app
//! in front on the phone can share, so the clipboard is never replaced
//! silently. Toasts never show the text itself: Windows keeps them in its
//! notification history.

use std::collections::VecDeque;

/// What to do with text shared from a phone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Plan {
    /// Put it on the clipboard on arrival.
    pub copy_now: bool,
    /// Keep it out of Win+V history and the cloud clipboard (also when the
    /// user copies it later from the toast).
    pub exclude_from_history: bool,
}

pub fn plan(auto_copy: bool, keep_in_history: bool) -> Plan {
    Plan {
        copy_now: auto_copy,
        exclude_from_history: !keep_in_history,
    }
}

pub fn title(from: &str) -> String {
    format!("Text from {from}")
}

pub fn body(copied: bool) -> &'static str {
    if copied {
        "Copied to the clipboard"
    } else {
        "Copy it to the clipboard?"
    }
}

/// Texts waiting for their toast's Copy button, newest last. Only a few are
/// kept: an old toast's Copy then does nothing.
#[derive(Default)]
pub struct Pending {
    items: VecDeque<(String, String)>,
    next: u64,
}

const KEEP: usize = 8;

impl Pending {
    pub const fn new() -> Self {
        Self {
            items: VecDeque::new(),
            next: 0,
        }
    }

    /// Keeps `text` and returns the toast tag that copies it.
    pub fn insert(&mut self, text: String) -> String {
        self.next += 1;
        let tag = format!("txt-{}", self.next);
        self.items.push_back((tag.clone(), text));
        while self.items.len() > KEEP {
            self.items.pop_front();
        }
        tag
    }

    /// The text for `tag`, once.
    pub fn take(&mut self, tag: &str) -> Option<String> {
        let i = self.items.iter().position(|(t, _)| t == tag)?;
        self.items.remove(i).map(|(_, text)| text)
    }
}

/// The toast for clipboard text from the phone. It shows the text only when the user turned
/// "Show shared text in notifications" on, and never for sensitive text: Windows keeps toasts in
/// its notification history on disk.
pub fn clipboard_toast_body(sensitive: bool, show_text: bool, text: &str) -> String {
    if sensitive {
        "Sensitive text copied (hidden, not kept in history)".into()
    } else if show_text {
        text.chars().take(120).collect()
    } else {
        "Copied to the clipboard.".into()
    }
}

/// The toast for a link from the phone: the address only when the user asked to see shared text.
pub fn link_toast_body(show_link: bool, url: &str) -> String {
    if show_link {
        let shown: String = url.chars().take(120).collect();
        format!("{shown}\nClick to open.")
    } else {
        "Click to open.".into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clipboard_toast_hides_text_by_default() {
        let b = clipboard_toast_body(false, false, "my secret plan");
        assert!(!b.contains("secret"));
    }

    #[test]
    fn clipboard_toast_shows_text_only_when_asked() {
        let long = "x".repeat(300);
        assert_eq!(
            clipboard_toast_body(false, true, &long).chars().count(),
            120
        );
    }

    #[test]
    fn clipboard_toast_never_shows_sensitive_text() {
        assert!(!clipboard_toast_body(true, true, "hunter2").contains("hunter2"));
    }

    #[test]
    fn link_toast_hides_url_by_default() {
        let b = link_toast_body(false, "https://example.com/reset?token=abc");
        assert!(!b.contains("example.com"));
        assert!(b.contains("Click to open."));
        assert!(link_toast_body(true, "https://example.com/a").contains("https://example.com/a"));
    }

    #[test]
    fn shared_text_waits_for_copy_by_default() {
        assert!(!plan(false, true).copy_now);
        assert!(!plan(false, false).copy_now);
    }

    #[test]
    fn auto_copy_honours_keep_in_history() {
        assert!(plan(true, false).copy_now);
        assert!(plan(true, false).exclude_from_history);
        assert!(!plan(true, true).exclude_from_history);
        assert!(
            plan(false, false).exclude_from_history,
            "the Copy button too"
        );
    }

    #[test]
    fn toast_lines_come_from_the_sender_only() {
        assert_eq!(title("Pixel 8"), "Text from Pixel 8");
        assert_eq!(body(true), "Copied to the clipboard");
        assert_eq!(body(false), "Copy it to the clipboard?");
    }

    #[test]
    fn pending_texts_keep_only_the_newest() {
        let mut p = Pending::default();
        let tags: Vec<String> = (0..10).map(|i| p.insert(format!("t{i}"))).collect();
        assert_eq!(p.take(&tags[0]), None, "the oldest went");
        assert_eq!(p.take(&tags[1]), None);
        assert_eq!(p.take(&tags[9]).as_deref(), Some("t9"));
        assert_eq!(p.take(&tags[9]), None, "taken once");
        assert_eq!(p.take(&tags[2]).as_deref(), Some("t2"));
    }
}
