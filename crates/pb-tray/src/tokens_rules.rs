//! The two rules the glass and motion tokens come with. The generator emits data only; these are written
//! here, next to the code that calls them, and tested here (the Kotlin side has its own copy in `TokenRules.kt`).
#![allow(dead_code)] // The M5 screens are the first callers.

use crate::tokens::{Material, Spring};

/// What the user and the system have asked for.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Modes {
    pub reduced_transparency: bool,
    pub reduced_motion: bool,
    /// Relay's own lite mode: no effects at all.
    pub lite: bool,
    pub high_contrast: bool,
}

/// The material to draw. Reduced transparency, lite and high contrast make it fully opaque with no blur,
/// grain or highlight, so text keeps its contrast whatever is behind the window.
pub fn resolve(m: Material, modes: Modes) -> Material {
    if modes.reduced_transparency || modes.lite || modes.high_contrast {
        Material {
            opacity: 1.0,
            blur: 0.0,
            grain: 0.0,
            highlight: 0.0,
            ..m
        }
    } else {
        m
    }
}

/// The spring to animate with, or `None` to cut: reduced motion replaces travel with a cross-fade at
/// `tokens::motion::FAST` and keeps colour and focus feedback, so no spring runs.
pub fn spring_or_cut(s: Spring, modes: Modes) -> Option<Spring> {
    (!modes.reduced_motion).then_some(s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tokens;

    #[test]
    fn reduced_transparency_is_opaque() {
        for modes in [
            Modes {
                reduced_transparency: true,
                ..Modes::default()
            },
            Modes {
                lite: true,
                ..Modes::default()
            },
            Modes {
                high_contrast: true,
                ..Modes::default()
            },
        ] {
            for m in [
                tokens::NATIVE_LIGHT,
                tokens::NATIVE_DARK,
                tokens::RELAY_LIGHT,
                tokens::RELAY_DARK,
            ] {
                let r = resolve(m, modes);
                assert_eq!(r.opacity, 1.0, "{modes:?}");
                assert_eq!((r.blur, r.grain, r.highlight), (0.0, 0.0, 0.0), "{modes:?}");
                assert_eq!(r.tint, m.tint, "the colour is kept");
            }
        }
    }

    #[test]
    fn normal_modes_leave_the_material_alone() {
        let m = tokens::RELAY_DARK;
        assert_eq!(resolve(m, Modes::default()), m);
        // Reduced motion alone does not touch transparency.
        assert_eq!(
            resolve(
                m,
                Modes {
                    reduced_motion: true,
                    ..Modes::default()
                }
            ),
            m
        );
    }

    #[test]
    fn reduced_motion_cuts_springs() {
        let on = Modes {
            reduced_motion: true,
            ..Modes::default()
        };
        for s in [
            tokens::spring::STANDARD,
            tokens::spring::EMPHASISED,
            tokens::spring::QUICK,
            tokens::spring::EFFECTS,
        ] {
            assert_eq!(spring_or_cut(s, on), None);
            assert_eq!(spring_or_cut(s, Modes::default()), Some(s));
        }
    }

    #[test]
    fn effects_springs_never_overshoot() {
        const { assert!(tokens::spring::EFFECTS.damping_ratio >= 1.0) };
    }
}
