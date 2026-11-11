//! Phone-side text echo suppression; hashes and monotonic times stay in RAM.

use std::time::{Duration, Instant};

pub(crate) const ECHO_WINDOW: Duration = Duration::from_secs(15);

#[derive(Default)]
pub(crate) struct RemoteEcho {
    recent: Option<([u8; 32], Instant)>,
}

impl RemoteEcho {
    pub(crate) fn received(&mut self, hash: [u8; 32], now: Instant) {
        self.recent = Some((hash, now));
    }

    pub(crate) fn suppresses(&self, hash: [u8; 32], now: Instant) -> bool {
        self.recent.is_some_and(|(remote, at)| {
            remote == hash
                && now
                    .checked_duration_since(at)
                    .is_some_and(|age| age < ECHO_WINDOW)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_remote_copy_is_suppressed_only_during_the_echo_window() {
        let now = Instant::now();
        let mut echo = RemoteEcho::default();
        assert!(!echo.suppresses([1; 32], now));
        echo.received([1; 32], now);
        assert!(echo.suppresses([1; 32], now));
        assert!(echo.suppresses([1; 32], now + ECHO_WINDOW - Duration::from_nanos(1)));
        assert!(
            !echo.suppresses([1; 32], now + ECHO_WINDOW),
            "re-copying the same text later must be allowed"
        );
        assert!(!echo.suppresses([1; 32], now + Duration::from_secs(60)));
    }

    #[test]
    fn another_received_clip_replaces_the_stamp_and_never_blocks_other_text() {
        let now = Instant::now();
        let mut echo = RemoteEcho::default();
        echo.received([1; 32], now);
        assert!(!echo.suppresses([2; 32], now));
        echo.received([2; 32], now + ECHO_WINDOW);
        assert!(!echo.suppresses([1; 32], now + ECHO_WINDOW));
        assert!(echo.suppresses([2; 32], now + ECHO_WINDOW));
    }

    #[test]
    fn an_earlier_clock_sample_cannot_create_an_unbounded_echo() {
        let now = Instant::now();
        let mut echo = RemoteEcho::default();
        echo.received([1; 32], now + Duration::from_secs(1));
        assert!(!echo.suppresses([1; 32], now));
    }
}
