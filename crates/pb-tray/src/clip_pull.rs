//! "Get phone clipboard": wait briefly for an answer, then let an accepted picture
//! finish under the transport's existing idle/size limits. Peer and generation
//! checks keep another phone or an old image completion from finishing this request.

use std::time::{Duration, Instant};

/// Time to receive an answer, not a deadline for transferring an accepted picture.
pub const PULL_WAIT: Duration = Duration::from_secs(6);
/// Suppress repeated presses while the hotkey is held.
pub const PULL_REPEAT: Duration = Duration::from_millis(1200);

#[derive(Default)]
pub struct PullState {
    since: Option<Instant>,
    receiving_image: bool,
    generation: u64,
    peer: String,
}

impl PullState {
    pub const fn new() -> Self {
        Self {
            since: None,
            receiving_image: false,
            generation: 0,
            peer: String::new(),
        }
    }

    /// Start one request; do not overlap an image already being received.
    pub fn start(&mut self, now: Instant, peer: &str) -> bool {
        if self.receiving_image
            || self
                .since
                .is_some_and(|at| now.saturating_duration_since(at) < PULL_REPEAT)
        {
            return false;
        }
        self.generation = self.generation.wrapping_add(1);
        self.peer = peer.to_owned();
        self.since = Some(now);
        true
    }

    pub fn pending(&self, now: Instant) -> bool {
        self.since
            .is_some_and(|at| self.receiving_image || now.saturating_duration_since(at) < PULL_WAIT)
    }

    pub fn pending_from(&self, peer: &str, now: Instant) -> bool {
        self.peer == peer && self.pending(now)
    }

    /// An accepted image offer counts as the answer; retain its request token.
    pub fn image_started(&mut self, now: Instant, peer: &str) -> Option<u64> {
        if !self.pending_from(peer, now) {
            return None;
        }
        self.receiving_image = true;
        Some(self.generation)
    }

    /// Only that image may complete its generation, even if another reply raced it.
    pub fn image_finished(&mut self, generation: u64) -> bool {
        self.receiving_image && self.generation == generation && self.answered()
    }

    pub fn answered_from(&mut self, peer: &str) -> bool {
        self.peer == peer && self.answered()
    }

    pub fn answered(&mut self) -> bool {
        self.receiving_image = false;
        self.peer.clear();
        self.since.take().is_some()
    }

    /// An image offer already answered; transfer failures have their own truthful result.
    pub fn timed_out(&mut self, now: Instant) -> bool {
        if !self.receiving_image
            && self
                .since
                .is_some_and(|at| now.saturating_duration_since(at) >= PULL_WAIT)
        {
            self.answered()
        } else {
            false
        }
    }
}

pub const NO_ANSWER: &str = "The phone did not answer. It must be awake with Relay on";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_request_is_pending_until_it_is_answered() {
        let t0 = Instant::now();
        let mut p = PullState::default();
        assert!(!p.pending(t0));
        assert!(p.start(t0, "phone"));
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
        assert!(p.start(t0, "phone"));
        assert!(!p.start(t0 + PULL_REPEAT - Duration::from_millis(1), "phone"));
        assert!(
            p.start(t0 + PULL_REPEAT, "phone"),
            "a deliberate second press goes out again"
        );
    }

    #[test]
    fn silence_times_out_once_and_only_after_the_wait() {
        let t0 = Instant::now();
        let mut p = PullState::default();
        assert!(!p.timed_out(t0), "nothing was asked");
        p.start(t0, "phone");
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
        p.start(t0, "phone");
        assert!(!p.pending(t0 + PULL_WAIT));
    }
    #[test]
    fn a_picture_in_flight_is_an_answer_even_when_the_bytes_take_longer_than_six_seconds() {
        let now = Instant::now();
        let mut p = PullState::new();
        p.start(now, "phone");
        let generation = p
            .image_started(now + Duration::from_secs(1), "phone")
            .unwrap();
        assert!(
            !p.timed_out(now + PULL_WAIT),
            "a picture offer already answered"
        );
        assert!(p.pending(now + Duration::from_secs(30)));
        assert!(p.image_finished(generation));
        assert!(!p.pending(now + Duration::from_secs(30)));
    }

    #[test]
    fn another_pull_cannot_overlap_the_picture_being_received() {
        let now = Instant::now();
        let mut p = PullState::new();
        p.start(now, "phone");
        p.image_started(now, "phone");
        assert!(!p.start(now + PULL_REPEAT, "phone"));
    }

    #[test]
    fn a_late_image_completion_cannot_finish_a_newer_request() {
        let now = Instant::now();
        let mut p = PullState::new();
        p.start(now, "phone");
        let old = p.image_started(now, "phone").unwrap();
        p.answered();
        assert!(p.start(now + PULL_REPEAT, "phone"));
        let new = p.image_started(now + PULL_REPEAT, "phone").unwrap();
        assert!(!p.image_finished(old));
        assert!(p.pending(now + PULL_REPEAT));
        assert!(p.image_finished(new));
    }

    #[test]
    fn a_late_image_offer_cannot_revive_a_timed_out_request() {
        let now = Instant::now();
        let mut p = PullState::new();
        assert!(p.image_started(now, "phone").is_none());
        p.start(now, "phone");
        assert!(p.timed_out(now + PULL_WAIT));
        assert!(p.image_started(now + PULL_WAIT, "phone").is_none());
    }

    #[test]
    fn another_phone_cannot_answer_or_extend_the_requested_phones_transfer() {
        let now = Instant::now();
        let mut p = PullState::new();
        assert!(p.start(now, "phone-a"));
        assert!(!p.pending_from("phone-b", now));
        assert!(p.image_started(now, "phone-b").is_none());
        assert!(!p.answered_from("phone-b"));
        assert!(p.pending_from("phone-a", now));
    }
}
