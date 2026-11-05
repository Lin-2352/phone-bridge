//! Why a phone-screen session ended, as the PC tells it. A plain end retries
//! quietly for 30 s (maybe only the link dropped); a phone that locked does
//! not come back by itself, so the PC says so instead.

/// A plain end: core.rs retries for 30 s.
pub const ENDED: &str = "Phone stopped sharing its screen";
/// The phone locked (Android ends sharing then): nothing to retry.
pub const LOCKED: &str =
    "Phone locked: screen sharing stopped. Unlock it, then open the screen again";

/// The end reason, from the last notice the phone sent in that session.
pub fn reason(last_notice: &str) -> &'static str {
    if last_notice == pb_protocol::SCREEN_LOCKED_NOTICE {
        LOCKED
    } else {
        ENDED
    }
}

/// Whether the PC tries again by itself.
pub fn retries(why: &str) -> bool {
    why == ENDED
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_lock_notice_ends_without_retrying() {
        let why = reason(pb_protocol::SCREEN_LOCKED_NOTICE);
        assert!(!retries(why));
        assert!(why.starts_with("Phone locked"));
    }

    #[test]
    fn a_plain_end_retries() {
        assert!(retries(reason("")));
        assert!(retries(reason("Weak Wi-Fi: lower quality")));
    }
}
