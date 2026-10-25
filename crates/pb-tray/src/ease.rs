//! Exponential easing for the hover and press animation of PC-side widgets.

/// Time constant of the ease: about 63% of the gap closes in this long.
const TIME_CONSTANT_MS: f32 = 90.0;
/// Within this distance of the target the value snaps onto it.
const SNAP: f32 = 0.01;

/// Moves `current` toward `target` by an exponential step of `dt_ms`.
/// Never overshoots; snaps to `target` once within `SNAP`.
pub fn approach(current: f32, target: f32, dt_ms: u32) -> f32 {
    if (target - current).abs() <= SNAP {
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
    fn a_settled_value_needs_no_timer() {
        assert!(settled(1.0, 1.0));
        assert!(!settled(0.5, 1.0));
    }
}
