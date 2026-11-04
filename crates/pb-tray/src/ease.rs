//! Easing for PC-side widgets: exponential approach for hover and press, and
//! timed animations on the standard curve.

use std::time::{Duration, Instant};

/// Time constant of the ease: about 63% of the gap closes in this long.
const TIME_CONSTANT_MS: f32 = 90.0;
/// Within this distance of the target the value snaps onto it.
const SNAP: f32 = 0.01;

/// Moves `current` toward `target` by an exponential step of `dt_ms`.
/// Never overshoots; snaps to `target` once within `SNAP`.
#[cfg(windows)]
pub fn approach(current: f32, target: f32, dt_ms: u32) -> f32 {
    approach_with(crate::fluent::reduced_motion(), current, target, dt_ms)
}

/// `approach`, with reduced motion given: then the value jumps to the target.
pub fn approach_with(reduced: bool, current: f32, target: f32, dt_ms: u32) -> f32 {
    if reduced || (target - current).abs() <= SNAP {
        return target;
    }
    let k = 1.0 - (-(dt_ms as f32) / TIME_CONSTANT_MS).exp();
    let next = current + (target - current) * k;
    if (target - next).abs() <= SNAP {
        target
    } else {
        next
    }
}

/// True when `current` has reached `target`, so no animation timer is needed.
pub fn settled(current: f32, target: f32) -> bool {
    (target - current).abs() <= SNAP
}

/// Control points of the standard easing curve, cubic-bezier(x1, y1, x2, y2)
/// from (0, 0) to (1, 1): quick start, long gentle settle.
/// `design/tokens.toml` names no curve yet, so it lives here.
#[allow(dead_code)] // the widgets animate with it (pages use them from Task 8 on)
pub const STANDARD: (f32, f32, f32, f32) = (0.2, 0.0, 0.0, 1.0);

/// One coordinate of a cubic bezier from 0 to 1 with inner controls `a`, `b`.
#[allow(dead_code)] // the widgets animate with it (pages use them from Task 8 on)
fn bezier(a: f32, b: f32, t: f32) -> f32 {
    let u = 1.0 - t;
    3.0 * u * u * t * a + 3.0 * u * t * t * b + t * t * t
}

/// The standard curve at progress `p` in 0..=1 (time in, value out).
#[allow(dead_code)] // the widgets animate with it (pages use them from Task 8 on)
pub fn standard(p: f32) -> f32 {
    let (x1, y1, x2, y2) = STANDARD;
    // x(t) rises monotonically, so bisection finds the t with x(t) = p.
    let (mut lo, mut hi) = (0.0f32, 1.0f32);
    for _ in 0..24 {
        let mid = (lo + hi) / 2.0;
        if bezier(x1, x2, mid) < p {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    bezier(y1, y2, (lo + hi) / 2.0)
}

/// A timed animation from `from` to `to` over `dur_ms`, eased with the
/// standard curve. A window keeps its frame timer only while one runs.
#[allow(dead_code)] // the widgets animate with it (pages use them from Task 8 on)
#[derive(Clone, Copy, Debug)]
pub struct Anim {
    pub from: f32,
    pub to: f32,
    pub start: Instant,
    pub dur_ms: u32,
}

#[allow(dead_code)] // the widgets animate with it (pages use them from Task 8 on)
impl Anim {
    #[cfg(windows)]
    pub fn new(from: f32, to: f32, start: Instant, dur_ms: u32) -> Self {
        Self::new_with(crate::fluent::reduced_motion(), from, to, start, dur_ms)
    }

    /// `new`, with reduced motion given: then it is over at once.
    pub fn new_with(reduced: bool, from: f32, to: f32, start: Instant, dur_ms: u32) -> Self {
        Self {
            from,
            to,
            start,
            dur_ms: if reduced { 0 } else { dur_ms },
        }
    }

    /// The value at `now`: exactly `from` at the start, exactly `to` at the end.
    pub fn value(&self, now: Instant) -> f32 {
        let elapsed = now.saturating_duration_since(self.start).as_secs_f32() * 1000.0;
        if elapsed <= 0.0 {
            return self.from;
        }
        if self.dur_ms == 0 || elapsed >= self.dur_ms as f32 {
            return self.to;
        }
        self.from + (self.to - self.from) * standard(elapsed / self.dur_ms as f32)
    }

    /// True until the animation has run its full duration.
    pub fn running(&self, now: Instant) -> bool {
        now < self.start + Duration::from_millis(self.dur_ms as u64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn moves_toward_the_target_without_overshoot() {
        let next = approach(0.0, 1.0, 16);
        assert!(next > 0.0 && next < 1.0);
        let down = approach(1.0, 0.0, 16);
        assert!(down > 0.0 && down < 1.0);
    }

    #[test]
    fn a_longer_step_moves_further() {
        assert!(approach(0.0, 1.0, 50) > approach(0.0, 1.0, 10));
    }

    #[test]
    fn snaps_when_close() {
        assert_eq!(approach(0.995, 1.0, 16), 1.0);
        assert_eq!(approach(0.005, 0.0, 16), 0.0);
    }

    #[test]
    fn anim_eases_from_start_to_end_and_stops() {
        use crate::tokens::motion;
        use std::time::{Duration, Instant};
        let t0 = Instant::now();
        let a = Anim::new(0.0, 1.0, t0, motion::NORMAL_MS);
        assert_eq!(a.value(t0), 0.0);
        assert_eq!(a.value(t0 + Duration::from_millis(200)), 1.0);
        assert!(a.running(t0 + Duration::from_millis(100)));
        assert!(!a.running(t0 + Duration::from_millis(201)));
        // Eased: in between, and still moving.
        let mid = a.value(t0 + Duration::from_millis(100));
        assert!(mid > 0.0 && mid < 1.0);
        assert!(a.value(t0 + Duration::from_millis(150)) > mid);
    }

    #[test]
    fn reduced_motion_jumps_to_the_end() {
        assert_eq!(approach_with(true, 0.0, 1.0, 16), 1.0);
        assert!(approach_with(false, 0.0, 1.0, 16) < 1.0);
        let t0 = Instant::now();
        let a = Anim::new_with(true, 0.0, 1.0, t0, 200);
        assert_eq!(a.value(t0 + Duration::from_millis(1)), 1.0);
        assert!(!a.running(t0 + Duration::from_millis(1)));
        assert!(Anim::new_with(false, 0.0, 1.0, t0, 200).running(t0));
    }

    #[test]
    fn a_settled_value_needs_no_timer() {
        assert!(settled(1.0, 1.0));
        assert!(!settled(0.5, 1.0));
    }
}
