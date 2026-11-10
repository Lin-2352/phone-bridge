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

use std::time::{Duration, Instant};

/// A clip that just arrived from the other device is not sent straight back for this long.
pub const ECHO: Duration = Duration::from_secs(15);
/// The same clip seen again inside this window is one copy (a clipboard change can fire twice).
pub const REPEAT: Duration = Duration::from_secs(2);

/// Echo guard for text and pictures: the last clip received and the last sent, each only for a short
/// while. Copying the same thing again later, or after something else arrived, must send it again.
#[derive(Default)]
pub struct EchoGuard {
    remote: Option<([u8; 32], Instant)>,
    sent: Option<([u8; 32], Instant)>,
}

impl EchoGuard {
    /// Whether the clip with hash `h` goes out now; if so it is remembered as sent.
    pub fn should_send(&mut self, h: [u8; 32], now: Instant) -> bool {
        let echo = self
            .remote
            .is_some_and(|(r, at)| r == h && now.saturating_duration_since(at) < ECHO);
        let repeat = self
            .sent
            .is_some_and(|(s, at)| s == h && now.saturating_duration_since(at) < REPEAT);
        if echo || repeat {
            return false;
        }
        self.sent = Some((h, now));
        true
    }

    /// The send for `h` failed: copying the same clip again retries at once.
    pub fn send_failed(&mut self, h: [u8; 32]) {
        if self.sent.is_some_and(|(s, _)| s == h) {
            self.sent = None;
        }
    }

    /// A clip arrived from the other device. What we sent before no longer blocks a re-copy.
    pub fn received(&mut self, h: [u8; 32], now: Instant) {
        self.remote = Some((h, now));
        self.sent = None;
    }
}

/// Echo guard for pictures: hashes the PNG and defers to [`EchoGuard`].
#[derive(Default)]
pub struct ImageGuard {
    inner: EchoGuard,
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
        self.inner.should_send(h, Instant::now()).then_some(h)
    }

    /// The send for `h` failed: copying the same picture again retries.
    pub fn send_failed(&mut self, h: [u8; 32]) {
        self.inner.send_failed(h);
    }

    /// A picture arrived from the other device.
    pub fn received(&mut self, png: &[u8]) {
        self.inner.received(hash(png), Instant::now());
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

    // ---- EchoGuard (text and pictures share it) ----

    fn h(b: u8) -> [u8; 32] {
        [b; 32]
    }

    #[test]
    fn copying_the_same_text_again_right_away_sends_it_once() {
        let t0 = std::time::Instant::now();
        let mut g = EchoGuard::default();
        assert!(g.should_send(h(1), t0));
        assert!(!g.should_send(h(1), t0 + std::time::Duration::from_millis(300)));
    }

    #[test]
    fn the_same_text_copied_much_later_is_sent_again() {
        let t0 = std::time::Instant::now();
        let mut g = EchoGuard::default();
        assert!(g.should_send(h(1), t0));
        assert!(g.should_send(h(1), t0 + REPEAT + std::time::Duration::from_millis(1)));
    }

    #[test]
    fn a_clip_that_just_arrived_is_not_echoed_but_is_after_the_window() {
        let t0 = std::time::Instant::now();
        let mut g = EchoGuard::default();
        g.received(h(7), t0);
        assert!(!g.should_send(h(7), t0 + std::time::Duration::from_secs(2)));
        assert!(g.should_send(h(7), t0 + ECHO + std::time::Duration::from_secs(1)));
    }

    #[test]
    fn copying_a_again_after_b_arrived_from_the_phone_is_sent() {
        // A sent, B arrives from the phone, the user copies A again: nothing may swallow it.
        let t0 = std::time::Instant::now();
        let mut g = EchoGuard::default();
        assert!(g.should_send(h(1), t0));
        g.received(h(2), t0 + std::time::Duration::from_secs(1));
        assert!(g.should_send(h(1), t0 + std::time::Duration::from_millis(1500)));
    }

    #[test]
    fn a_failed_send_lets_the_same_text_retry_at_once() {
        let t0 = std::time::Instant::now();
        let mut g = EchoGuard::default();
        assert!(g.should_send(h(1), t0));
        g.send_failed(h(1));
        assert!(g.should_send(h(1), t0 + std::time::Duration::from_millis(100)));
    }
}
