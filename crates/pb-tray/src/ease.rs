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
/// from (0, 0) to (1, 1); the value is the design token `easing.standard`.
#[allow(dead_code)] // the widgets animate with it (pages use them from Task 8 on)
pub const STANDARD: (f32, f32, f32, f32) = (
    crate::tokens::easing::STANDARD[0],
    crate::tokens::easing::STANDARD[1],
    crate::tokens::easing::STANDARD[2],
    crate::tokens::easing::STANDARD[3],
);

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

/// A frame timer is needed only while something animates; a window kills its timer when this turns false.
pub fn needs_frame_timer(anim_running: bool) -> bool {
    anim_running
}

/// A spring animation for motion that follows the user's hand (a released drag, a fling settling).
/// Transitions use `Anim` with the token curve and durations instead. Units are the caller's own (pixels,
/// 0..1): the spring is linear, so how long it takes does not depend on the distance.
#[allow(dead_code)] // the nav track and scrollbar thumb use it when drags are released
#[derive(Clone, Copy, Debug)]
pub struct SpringAnim {
    x: f32,
    v: f32,
    target: f32,
    span: f32,
    spring: crate::tokens::Spring,
}

#[allow(dead_code)] // the nav track and scrollbar thumb use it when drags are released
impl SpringAnim {
    pub fn new(from: f32, to: f32, spring: crate::tokens::Spring) -> Self {
        Self {
            x: from,
            v: 0.0,
            target: to,
            span: (to - from).abs().max(1.0),
            spring,
        }
    }

    /// Advances by `dt_ms` (at most 64: a stall while a window is dragged must not blow the velocity up)
    /// and returns the new value. Semi-implicit Euler in sub-steps of at most 4 ms, with
    /// omega = sqrt(stiffness) and the damping ratio from the token.
    pub fn step(&mut self, dt_ms: u32) -> f32 {
        if self.settled() {
            self.x = self.target;
            self.v = 0.0;
            return self.x;
        }
        let w = self.spring.stiffness.sqrt();
        let damping = 2.0 * self.spring.damping_ratio * w;
        let mut left = dt_ms.min(64) as f32 / 1000.0;
        while left > 0.0 {
            let h = left.min(0.004);
            let a = -w * w * (self.x - self.target) - damping * self.v;
            self.v += a * h;
            self.x += self.v * h;
            left -= h;
        }
        if self.settled() {
            self.x = self.target;
            self.v = 0.0;
        }
        self.x
    }

    /// Within a thousandth of the distance of the target, and nearly still.
    pub fn settled(&self) -> bool {
        (self.x - self.target).abs() < 0.001 * self.span && self.v.abs() < 0.01 * self.span
    }

    pub fn value(&self) -> f32 {
        self.x
    }
}

/// The spring for a released drag, or `None` when reduced motion asks for a cut.
#[allow(dead_code)] // the nav track and scrollbar thumb use it when drags are released
pub fn spring_anim(
    from: f32,
    to: f32,
    spring: crate::tokens::Spring,
    modes: crate::tokens_rules::Modes,
) -> Option<SpringAnim> {
    crate::tokens_rules::spring_or_cut(spring, modes).map(|s| SpringAnim::new(from, to, s))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn moves_toward_the_target_without_overshoot() {
        let next = approach_with(false, 0.0, 1.0, 16);
        assert!(next > 0.0 && next < 1.0);
        let down = approach_with(false, 1.0, 0.0, 16);
        assert!(down > 0.0 && down < 1.0);
    }

    #[test]
    fn a_longer_step_moves_further() {
        assert!(approach_with(false, 0.0, 1.0, 50) > approach_with(false, 0.0, 1.0, 10));
    }

    #[test]
    fn snaps_when_close() {
        assert_eq!(approach_with(false, 0.995, 1.0, 16), 1.0);
        assert_eq!(approach_with(false, 0.005, 0.0, 16), 0.0);
    }

    #[test]
    fn anim_eases_from_start_to_end_and_stops() {
        use crate::tokens::motion;
        use std::time::{Duration, Instant};
        let t0 = Instant::now();
        let a = Anim::new_with(false, 0.0, 1.0, t0, motion::NORMAL_MS);
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

    /// Steps `a` in 16 ms frames until it settles; the time it took, in ms (capped at 5 s).
    fn settle_ms(a: &mut SpringAnim) -> u32 {
        let mut t = 0;
        while !a.settled() && t < 5_000 {
            a.step(16);
            t += 16;
        }
        t
    }

    #[test]
    fn a_spring_reaches_its_target_and_settles() {
        use crate::tokens::spring;
        for s in [
            spring::STANDARD,
            spring::EMPHASISED,
            spring::QUICK,
            spring::EFFECTS,
        ] {
            let mut a = SpringAnim::new(0.0, 1.0, s);
            let t = settle_ms(&mut a);
            assert!(t < 1_500, "{s:?} took {t} ms");
            assert_eq!(a.step(16), 1.0, "settled means exactly on target");
        }
    }

    #[test]
    fn pixel_range_spring_settles_without_tail_burn() {
        use crate::tokens::spring;
        let mut a = SpringAnim::new(0.0, 240.0, spring::STANDARD);
        assert!(settle_ms(&mut a) < 600);
    }

    #[test]
    fn a_long_stall_dt_is_clamped_and_stays_stable() {
        use crate::tokens::spring;
        let mut a = SpringAnim::new(0.0, 1.0, spring::EMPHASISED);
        let x = a.step(2_000);
        assert!(x <= 1.15, "a window-drag stall must not explode: {x}");
        assert!(x.is_finite());
    }

    #[test]
    fn the_effects_spring_never_overshoots() {
        use crate::tokens::spring;
        let mut a = SpringAnim::new(0.0, 1.0, spring::EFFECTS);
        for _ in 0..2_000 {
            assert!(a.step(1) <= 1.0 + 1e-4, "effects must not overshoot");
        }
    }

    #[test]
    fn an_emphasised_spring_overshoots_a_little() {
        use crate::tokens::spring;
        let mut a = SpringAnim::new(0.0, 1.0, spring::EMPHASISED);
        let mut peak = 0.0f32;
        for _ in 0..1_000 {
            peak = peak.max(a.step(4));
        }
        assert!(peak > 1.0, "it passes the target");
        assert!(peak < 1.10, "by under 10%: {peak}");
    }

    #[test]
    fn standard_matches_the_token_curve() {
        let [x1, y1, x2, y2] = crate::tokens::easing::STANDARD;
        assert_eq!(STANDARD, (x1, y1, x2, y2));
        assert!(standard(0.0).abs() < 1e-3 && (standard(1.0) - 1.0).abs() < 1e-3);
    }

    #[test]
    fn reduced_motion_makes_no_spring() {
        use crate::{tokens::spring, tokens_rules::Modes};
        let reduced = Modes {
            reduced_motion: true,
            ..Modes::default()
        };
        assert!(spring_anim(0.0, 1.0, spring::STANDARD, reduced).is_none());
        assert!(spring_anim(0.0, 1.0, spring::STANDARD, Modes::default()).is_some());
    }

    #[test]
    fn a_frame_timer_runs_only_while_something_animates() {
        assert!(needs_frame_timer(true));
        assert!(!needs_frame_timer(false));
    }
}
