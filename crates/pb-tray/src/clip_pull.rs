//! "Get phone clipboard": the PC asks the phone for what it has copied. Android lets only the focused app read the
//! clipboard, so the phone opens a see-through window for a moment and answers; this is the PC's side of that,
//! pure and tested. One request at a time, an answer clears it, and a request that nobody answers says so.

use std::time::{Duration, Instant};

/// How long the PC waits for the phone's answer before saying nobody answered.
pub const PULL_WAIT: Duration = Duration::from_secs(6);

/// A second press inside this window is the same request (a held hotkey).
pub const PULL_REPEAT: Duration = Duration::from_millis(1200);

/// The one request in flight, if any.
#[derive(Default)]
pub struct PullState {
    since: Option<Instant>,
}

impl PullState {
    pub const fn new() -> Self {
        Self { since: None }
    }

    /// A request goes out now. False when one is already in flight and was made a moment ago.
    pub fn start(&mut self, now: Instant) -> bool {
        if let Some(at) = self.since {
            if now.saturating_duration_since(at) < PULL_REPEAT {
                return false;
            }
        }
        self.since = Some(now);
        true
    }

    /// Whether the PC asked and has not heard back (a clip that arrives then is wanted even with sharing off).
    pub fn pending(&self, now: Instant) -> bool {
        self.since
            .is_some_and(|at| now.saturating_duration_since(at) < PULL_WAIT)
    }

    /// Something answered (a clip, a picture or a reason). True when it answered a request.
    pub fn answered(&mut self) -> bool {
        self.since.take().is_some()
    }

    /// The wait is over and nothing came: clears the request and says so once.
    pub fn timed_out(&mut self, now: Instant) -> bool {
        match self.since {
            Some(at) if now.saturating_duration_since(at) >= PULL_WAIT => {
                self.since = None;
                true
            }
            _ => false,
        }
    }
}

/// The words shown at the PC when the phone did not answer in time.
pub const NO_ANSWER: &str = "The phone did not answer. It must be awake with Relay on";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_request_is_pending_until_it_is_answered() {
        let t0 = Instant::now();
        let mut p = PullState::default();
        assert!(!p.pending(t0));
        assert!(p.start(t0));
        assert!(p.pending(t0 + Duration::from_secs(1)));
        assert!(p.answered());
        assert!(
            !p.pending(t0 + Duration::from_secs(1)),
            "answered requests are done"
        );
        assert!(!p.answered(), "an answer with no request asks nothing");
    }

    #[test]
    fn a_held_hotkey_is_one_request() {
        let t0 = Instant::now();
        let mut p = PullState::default();
        assert!(p.start(t0));
        assert!(!p.start(t0 + PULL_REPEAT - Duration::from_millis(1)));
        assert!(
            p.start(t0 + PULL_REPEAT),
            "a deliberate second press goes out again"
        );
    }

    #[test]
    fn silence_times_out_once_and_only_after_the_wait() {
        let t0 = Instant::now();
        let mut p = PullState::default();
        assert!(!p.timed_out(t0), "nothing was asked");
        p.start(t0);
        assert!(!p.timed_out(t0 + PULL_WAIT - Duration::from_millis(1)));
        assert!(p.timed_out(t0 + PULL_WAIT));
        assert!(
            !p.timed_out(t0 + PULL_WAIT + Duration::from_secs(1)),
            "said once"
        );
    }

    #[test]
    fn a_clip_arriving_after_the_wait_is_not_wanted_any_more() {
        let t0 = Instant::now();
        let mut p = PullState::default();
        p.start(t0);
        assert!(!p.pending(t0 + PULL_WAIT));
    }
}
