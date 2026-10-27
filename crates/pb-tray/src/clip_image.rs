//! Rules for syncing clipboard images (pure, tested): when a clipboard image
//! may be sent or applied, and the echo guard that keeps two devices from
//! bouncing one picture back and forth.

use pb_protocol::control::MAX_CLIPBOARD_IMAGE;
use sha2::{Digest, Sha256};

/// Images sync only when the user turned on clipboard sharing and the
/// separate image switch (both default off).
pub fn enabled(universal_clipboard: bool, share_images: bool) -> bool {
    universal_clipboard && share_images
}

/// Echo guard, like the text one: the hash of the last PNG received and the
/// last sent, so neither side sends the same picture again.
#[derive(Default)]
pub struct ImageGuard {
    last_remote: [u8; 32],
    last_sent: [u8; 32],
}

fn hash(png: &[u8]) -> [u8; 32] {
    Sha256::digest(png).into()
}

impl ImageGuard {
    /// `Some(hash)` if this PNG should go out now (and is remembered as
    /// sent); `None` if it is empty, too large, just arrived or was just sent.
    pub fn should_send(&mut self, png: &[u8]) -> Option<[u8; 32]> {
        if png.is_empty() || png.len() > MAX_CLIPBOARD_IMAGE {
            return None;
        }
        let h = hash(png);
        if h == self.last_remote || h == self.last_sent {
            return None;
        }
        self.last_sent = h;
        Some(h)
    }

    /// The send for `h` failed: copying the same picture again retries.
    pub fn send_failed(&mut self, h: [u8; 32]) {
        if self.last_sent == h {
            self.last_sent = [0; 32];
        }
    }

    /// A picture arrived from the other device.
    pub fn received(&mut self, png: &[u8]) {
        self.last_remote = hash(png);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn images_need_clipboard_sharing_and_the_image_switch() {
        assert!(enabled(true, true));
        assert!(!enabled(true, false));
        assert!(!enabled(false, true));
        assert!(!enabled(false, false));
    }

    #[test]
    fn a_new_image_is_sent_once() {
        let mut g = ImageGuard::default();
        assert!(g.should_send(&[1, 2, 3]).is_some());
        // The same picture copied again is not sent again.
        assert!(g.should_send(&[1, 2, 3]).is_none());
        assert!(g.should_send(&[4, 5, 6]).is_some());
    }

    #[test]
    fn an_image_that_just_arrived_is_not_sent_back() {
        let mut g = ImageGuard::default();
        g.received(&[9, 9, 9]);
        assert!(g.should_send(&[9, 9, 9]).is_none());
    }

    #[test]
    fn a_failed_send_can_be_retried() {
        let mut g = ImageGuard::default();
        let h = g.should_send(&[7, 7]).unwrap();
        g.send_failed(h);
        assert!(g.should_send(&[7, 7]).is_some());
    }

    #[test]
    fn a_late_failure_does_not_clear_a_newer_image() {
        let mut g = ImageGuard::default();
        let old = g.should_send(&[1]).unwrap();
        let _new = g.should_send(&[2]).unwrap();
        g.send_failed(old);
        // [2] is still the last sent one: it must not go out twice.
        assert!(g.should_send(&[2]).is_none());
    }

    #[test]
    fn empty_and_oversized_images_are_never_sent() {
        let mut g = ImageGuard::default();
        assert!(g.should_send(&[]).is_none());
        let big = vec![0u8; pb_protocol::control::MAX_CLIPBOARD_IMAGE + 1];
        assert!(g.should_send(&big).is_none());
    }
}
