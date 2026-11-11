//! "You came back to this PC": the moment to fetch the phone's clipboard by itself. A person can't type on
//! both devices at once, so when input resumes after a while away, the phone is not in use and Relay can ask
//! it for what was copied there. Pure and tested; the Windows side only reports how long the PC has been idle.

use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Away at least this long before coming back counts as "went to the phone".
pub const AWAY: Duration = Duration::from_secs(30);

/// Two automatic requests are never closer than this.
pub const MIN_GAP: Duration = Duration::from_secs(5);

/// Watches the idle time, sampled about once a second.
#[derive(Default)]
pub struct Away {
    was_away: bool,
    last_ask: Option<Instant>,
}

impl Away {
    pub const fn new() -> Self {
        Self {
            was_away: false,
            last_ask: None,
        }
    }

    /// One sample: `idle` is how long since the last key or mouse input. True when the phone should be asked now.
    pub fn tick(&mut self, idle: Duration, now: Instant) -> bool {
        if idle >= AWAY {
            self.was_away = true;
            return false;
        }
        if !self.was_away {
            return false;
        }
        self.was_away = false;
        if self
            .last_ask
            .is_some_and(|at| now.saturating_duration_since(at) < MIN_GAP)
        {
            return false;
        }
        self.last_ask = Some(now);
        true
    }
}

static AWAY_STATE: Mutex<Away> = Mutex::new(Away::new());
static LOCAL_CHANGE: Mutex<Option<Instant>> = Mutex::new(None);

/// Something other than Relay changed this PC's clipboard just now.
pub fn note_local_copy() {
    *LOCAL_CHANGE.lock().unwrap_or_else(|p| p.into_inner()) = Some(Instant::now());
}

/// One sample of the idle time. `Some(age)` when the phone should be asked now: `age` is what goes in the
/// request as `pc_changed_ago_ms`.
pub fn poll(idle: Duration) -> Option<u64> {
    let now = Instant::now();
    let ask = AWAY_STATE
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .tick(idle, now);
    ask.then(|| changed_ago_ms(*LOCAL_CHANGE.lock().unwrap_or_else(|p| p.into_inner()), now))
}

/// What the PC tells the phone about its own clipboard: how long ago it last changed, in ms. 0 means the PC
/// has not seen it change since Relay started, so the phone has nothing to compare with.
pub fn changed_ago_ms(last_change: Option<Instant>, now: Instant) -> u64 {
    last_change.map_or(0, |at| {
        u64::try_from(now.saturating_duration_since(at).as_millis())
            .unwrap_or(u64::MAX)
            .max(1)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn secs(n: u64) -> Duration {
        Duration::from_secs(n)
    }

    #[test]
    fn nothing_is_asked_while_the_user_stays_at_the_pc() {
        let t0 = Instant::now();
        let mut a = Away::default();
        for i in 0..20 {
            assert!(!a.tick(secs(1), t0 + secs(i)));
        }
    }

    #[test]
    fn coming_back_after_a_long_away_asks_once() {
        let t0 = Instant::now();
        let mut a = Away::default();
        assert!(!a.tick(AWAY + secs(5), t0));
        assert!(a.tick(secs(0), t0 + secs(1)), "first input after the away");
        assert!(
            !a.tick(secs(1), t0 + secs(2)),
            "still typing: no second ask"
        );
    }

    #[test]
    fn a_short_pause_is_not_an_away() {
        let t0 = Instant::now();
        let mut a = Away::default();
        assert!(!a.tick(AWAY - secs(1), t0));
        assert!(!a.tick(secs(0), t0 + secs(1)));
    }

    #[test]
    fn a_second_away_is_asked_again() {
        let t0 = Instant::now();
        let mut a = Away::default();
        a.tick(AWAY, t0);
        assert!(a.tick(secs(0), t0 + secs(31)));
        a.tick(AWAY, t0 + secs(62));
        assert!(a.tick(secs(0), t0 + secs(63)));
    }

    #[test]
    fn two_asks_are_never_closer_than_the_gap() {
        let t0 = Instant::now();
        let mut a = Away {
            was_away: true,
            last_ask: Some(t0),
        };
        assert!(!a.tick(secs(0), t0 + MIN_GAP - Duration::from_millis(1)));
        a.was_away = true;
        assert!(a.tick(secs(0), t0 + MIN_GAP));
    }

    #[test]
    fn the_pcs_own_copy_age_is_never_zero_unless_unknown() {
        let t0 = Instant::now();
        assert_eq!(changed_ago_ms(None, t0), 0);
        assert_eq!(
            changed_ago_ms(Some(t0), t0),
            1,
            "a copy this instant is 1 ms old, not unknown"
        );
        assert_eq!(changed_ago_ms(Some(t0), t0 + secs(4)), 4000);
    }
}
