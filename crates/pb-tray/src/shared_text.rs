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

#[cfg(test)]
mod tests {
    use super::*;

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
