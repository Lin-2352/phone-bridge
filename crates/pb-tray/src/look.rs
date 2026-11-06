//! Which material a surface gets. Pure: no Win32, so it builds and tests everywhere. The drawing code
//! (`d2d.rs`, `fluent.rs`) only calls these functions.

#![allow(dead_code)] // The M5 surfaces are the first callers.

use crate::tokens;
use crate::tokens_rules::{self, Modes};

/// Which of the two looks the user chose (settings key `look`; Native is the default).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Look {
    Native,
    Relay,
}

/// A window or layer the tray draws.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SurfaceKind {
    MainWindow,
    Flyout,
    Menu,
    Dialog,
    Qr,
    Transfers,
    StatusPill,
    ScreenBars,
    Camera,
}

/// What the system says right now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SysState {
    pub dark: bool,
    /// "Transparency effects" in Windows settings.
    pub transparency_on: bool,
    pub high_contrast: bool,
    /// Windows animations (client area animations).
    pub animations_on: bool,
    /// DWM accepts system backdrops (Windows 11 22H2 and later, a local session).
    pub dwm_backdrop_ok: bool,
}

/// What a surface is drawn on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Material {
    Mica,
    Acrylic,
    /// DWM Acrylic as the base, with the tint, a top highlight and a grain tile drawn in Direct2D.
    RelayGlass,
    Solid,
}

/// Text that sits directly on glass needs at least this alpha; icon-only glass needs `ICON_GLASS_MIN_ALPHA`.
pub const TEXT_GLASS_MIN_ALPHA: f32 = 0.70;
pub const ICON_GLASS_MIN_ALPHA: f32 = 0.50;

/// The material for a surface. Reduced transparency, high contrast and a DWM that refuses backdrops all give
/// `Solid`; so does the QR window, which needs a white quiet zone.
pub fn material_for(kind: SurfaceKind, look: Look, sys: SysState) -> Material {
    if !sys.transparency_on || sys.high_contrast || !sys.dwm_backdrop_ok {
        return Material::Solid;
    }
    match (kind, look) {
        (SurfaceKind::MainWindow, Look::Native) => Material::Mica,
        (SurfaceKind::Flyout | SurfaceKind::Menu | SurfaceKind::Dialog, Look::Native) => {
            Material::Acrylic
        }
        (
            SurfaceKind::MainWindow | SurfaceKind::Flyout | SurfaceKind::Menu | SurfaceKind::Dialog,
            Look::Relay,
        ) => Material::RelayGlass,
        _ => Material::Solid,
    }
}

/// The modes the token rules take. Lite mode is Android only.
pub fn modes(sys: SysState) -> Modes {
    Modes {
        reduced_transparency: !sys.transparency_on,
        reduced_motion: !sys.animations_on,
        lite: false,
        high_contrast: sys.high_contrast,
    }
}

/// The glass values to draw with, after the reduced modes have made them opaque where they must.
pub fn glass(look: Look, dark: bool, sys: SysState) -> tokens::Material {
    let base = match (look, dark) {
        (Look::Native, false) => tokens::NATIVE_LIGHT,
        (Look::Native, true) => tokens::NATIVE_DARK,
        (Look::Relay, false) => tokens::RELAY_LIGHT,
        (Look::Relay, true) => tokens::RELAY_DARK,
    };
    // A DWM that refuses backdrops cannot blur, so the surface must be opaque too.
    let modes = Modes {
        reduced_transparency: !sys.transparency_on || !sys.dwm_backdrop_ok,
        ..modes(sys)
    };
    tokens_rules::resolve(base, modes)
}

/// `SystemParametersInfo` actions that announce a high-contrast or animation change.
pub const SPI_SETHIGHCONTRAST: usize = 0x0043;
pub const SPI_SETCLIENTAREAANIMATION: usize = 0x1043;

/// Whether a `WM_SETTINGCHANGE` (its `wParam` action and its `lParam` area name) changes anything the
/// surfaces draw from: the colour theme or transparency ("ImmersiveColorSet"), the high-contrast theme
/// or "Show animations".
pub fn setting_change_needs_refresh(action: usize, area: Option<&str>) -> bool {
    matches!(action, SPI_SETHIGHCONTRAST | SPI_SETCLIENTAREAANIMATION)
        || matches!(area, Some("ImmersiveColorSet" | "WindowsThemeElement"))
}

/// The glass values for one surface: opaque, with no blur, grain or highlight, whenever the surface is `Solid`
/// (a Transfers card or the QR window must never be drawn with a translucent fill).
pub fn surface_glass(kind: SurfaceKind, look: Look, sys: SysState) -> tokens::Material {
    if material_for(kind, look, sys) == Material::Solid {
        let base = glass(look, sys.dark, sys);
        tokens::Material {
            opacity: 1.0,
            blur: 0.0,
            grain: 0.0,
            highlight: 0.0,
            ..base
        }
    } else {
        glass(look, sys.dark, sys)
    }
}

/// Whether a theme, transparency, contrast or backdrop change means the surfaces must be redrawn and their
/// backdrop reset. It never means a window is destroyed.
pub fn rebuild_needed(old: SysState, new: SysState) -> bool {
    old.dark != new.dark
        || old.transparency_on != new.transparency_on
        || old.high_contrast != new.high_contrast
        || old.dwm_backdrop_ok != new.dwm_backdrop_ok
}

/// ClearType assumes an opaque background; over a translucent backdrop it fringes, so text there uses
/// grayscale antialiasing. Opaque cards inside glass and solid surfaces keep ClearType.
pub fn uses_cleartype(material: Material, on_opaque_card: bool) -> bool {
    material == Material::Solid || on_opaque_card
}

/// A 1 px stroke lands on pixel centres, so it is one crisp line instead of two blurred ones.
pub fn snap_hairline(px: f32) -> f32 {
    px.floor() + 0.5
}

/// The grain tile: 64 by 64 monochrome bytes, the same on every run (seeded xorshift32), mean about 128.
/// It is drawn tiled at a very low alpha, so it costs nothing per frame.
pub fn grain_tile_64() -> [u8; 4096] {
    let mut state: u32 = 0x9E37_79B9;
    let mut tile = [0u8; 4096];
    for b in tile.iter_mut() {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        *b = (state >> 24) as u8;
    }
    tile
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tokens;

    const KINDS: [SurfaceKind; 9] = [
        SurfaceKind::MainWindow,
        SurfaceKind::Flyout,
        SurfaceKind::Menu,
        SurfaceKind::Dialog,
        SurfaceKind::Qr,
        SurfaceKind::Transfers,
        SurfaceKind::StatusPill,
        SurfaceKind::ScreenBars,
        SurfaceKind::Camera,
    ];

    /// Every combination of the four system flags, for both themes.
    fn all_states() -> Vec<SysState> {
        let mut v = Vec::new();
        for bits in 0..32u32 {
            v.push(SysState {
                dark: bits & 1 != 0,
                transparency_on: bits & 2 != 0,
                high_contrast: bits & 4 != 0,
                animations_on: bits & 8 != 0,
                dwm_backdrop_ok: bits & 16 != 0,
            });
        }
        v
    }

    fn normal() -> SysState {
        SysState {
            dark: false,
            transparency_on: true,
            high_contrast: false,
            animations_on: true,
            dwm_backdrop_ok: true,
        }
    }

    #[test]
    fn native_main_window_is_mica() {
        assert_eq!(
            material_for(SurfaceKind::MainWindow, Look::Native, normal()),
            Material::Mica
        );
    }

    #[test]
    fn native_flyout_menu_dialog_are_acrylic() {
        for k in [SurfaceKind::Flyout, SurfaceKind::Menu, SurfaceKind::Dialog] {
            assert_eq!(material_for(k, Look::Native, normal()), Material::Acrylic);
        }
    }

    #[test]
    fn relay_layers_are_relay_glass() {
        for k in [
            SurfaceKind::MainWindow,
            SurfaceKind::Flyout,
            SurfaceKind::Menu,
            SurfaceKind::Dialog,
        ] {
            assert_eq!(material_for(k, Look::Relay, normal()), Material::RelayGlass);
        }
        for k in [
            SurfaceKind::Transfers,
            SurfaceKind::StatusPill,
            SurfaceKind::ScreenBars,
            SurfaceKind::Camera,
        ] {
            assert_eq!(material_for(k, Look::Relay, normal()), Material::Solid);
        }
    }

    #[test]
    fn transparency_off_is_solid_everywhere() {
        let sys = SysState {
            transparency_on: false,
            ..normal()
        };
        for look in [Look::Native, Look::Relay] {
            for k in KINDS {
                assert_eq!(material_for(k, look, sys), Material::Solid, "{k:?}");
            }
        }
    }

    #[test]
    fn high_contrast_is_solid_everywhere() {
        let sys = SysState {
            high_contrast: true,
            ..normal()
        };
        for look in [Look::Native, Look::Relay] {
            for k in KINDS {
                assert_eq!(material_for(k, look, sys), Material::Solid, "{k:?}");
            }
        }
    }

    #[test]
    fn old_windows_without_backdrops_is_solid() {
        let sys = SysState {
            dwm_backdrop_ok: false,
            ..normal()
        };
        for look in [Look::Native, Look::Relay] {
            for k in KINDS {
                assert_eq!(material_for(k, look, sys), Material::Solid, "{k:?}");
            }
        }
    }

    #[test]
    fn qr_is_always_solid() {
        for look in [Look::Native, Look::Relay] {
            for sys in all_states() {
                assert_eq!(material_for(SurfaceKind::Qr, look, sys), Material::Solid);
            }
        }
    }

    fn contrast(a: u32, b: u32) -> f64 {
        fn lin(c: u32) -> f64 {
            let c = c as f64 / 255.0;
            if c <= 0.03928 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        }
        fn lum(rgb: u32) -> f64 {
            0.2126 * lin((rgb >> 16) & 0xFF)
                + 0.7152 * lin((rgb >> 8) & 0xFF)
                + 0.0722 * lin(rgb & 0xFF)
        }
        let (x, y) = (lum(a), lum(b));
        (x.max(y) + 0.05) / (x.min(y) + 0.05)
    }

    #[test]
    fn focus_ring_is_3_to_1() {
        // The focus ring is drawn in `primary` on the opaque card and page colours (M4.9 R-3).
        for colors in [tokens::LIGHT, tokens::DARK] {
            for surface in [colors.surface, colors.surface_variant] {
                let c = contrast(colors.primary, surface);
                assert!(c >= 3.0, "primary on {surface:06X}: {c:.2}");
            }
        }
    }

    #[test]
    fn text_layers_meet_the_glass_floor() {
        for look in [Look::Native, Look::Relay] {
            for sys in all_states() {
                for k in [
                    SurfaceKind::MainWindow,
                    SurfaceKind::Flyout,
                    SurfaceKind::Menu,
                    SurfaceKind::Dialog,
                ] {
                    let m = material_for(k, look, sys);
                    let alpha = glass(look, sys.dark, sys).opacity;
                    if m == Material::Solid {
                        assert_eq!(alpha, 1.0, "{k:?} {look:?} {sys:?}");
                    } else {
                        assert!(
                            alpha >= TEXT_GLASS_MIN_ALPHA,
                            "{k:?} {look:?} {sys:?}: {alpha}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn on_theme_change_rebuilds_backdrops() {
        let a = normal();
        assert!(!rebuild_needed(a, a));
        assert!(rebuild_needed(a, SysState { dark: true, ..a }));
        assert!(rebuild_needed(
            a,
            SysState {
                transparency_on: false,
                ..a
            }
        ));
        assert!(rebuild_needed(
            a,
            SysState {
                high_contrast: true,
                ..a
            }
        ));
        assert!(rebuild_needed(
            a,
            SysState {
                dwm_backdrop_ok: false,
                ..a
            }
        ));
        // Animations do not change the backdrop.
        assert!(!rebuild_needed(
            a,
            SysState {
                animations_on: false,
                ..a
            }
        ));
    }

    #[test]
    fn text_aa_mode_matches_surface_opacity() {
        for m in [Material::Mica, Material::Acrylic, Material::RelayGlass] {
            assert!(!uses_cleartype(m, false), "{m:?} bare");
            assert!(uses_cleartype(m, true), "{m:?} on an opaque card");
        }
        assert!(uses_cleartype(Material::Solid, false));
        assert!(uses_cleartype(Material::Solid, true));
    }

    #[test]
    fn stroke_rect_snaps_to_half_pixel() {
        assert_eq!(snap_hairline(10.0), 10.5);
        assert_eq!(snap_hairline(10.9), 10.5);
        assert_eq!(snap_hairline(0.0), 0.5);
    }

    #[test]
    fn grain_tile_is_deterministic_and_balanced() {
        let a = grain_tile_64();
        assert_eq!(a, grain_tile_64());
        let mean = a.iter().map(|b| *b as u32).sum::<u32>() / a.len() as u32;
        assert!((120..=136).contains(&mean), "mean {mean}");
    }

    #[test]
    fn these_setting_changes_refresh_the_surfaces() {
        // The theme, accent and transparency toggles all arrive as "ImmersiveColorSet".
        assert!(setting_change_needs_refresh(0, Some("ImmersiveColorSet")));
        assert!(setting_change_needs_refresh(0, Some("WindowsThemeElement")));
        assert!(setting_change_needs_refresh(SPI_SETHIGHCONTRAST, None));
        assert!(setting_change_needs_refresh(
            SPI_SETCLIENTAREAANIMATION,
            None
        ));
        assert!(!setting_change_needs_refresh(0x002A, None));
        assert!(!setting_change_needs_refresh(0, Some("intl")));
        assert!(!setting_change_needs_refresh(0, None));
    }

    #[test]
    fn solid_surfaces_get_an_opaque_material() {
        for look in [Look::Native, Look::Relay] {
            for sys in all_states() {
                for k in KINDS {
                    let m = surface_glass(k, look, sys);
                    if material_for(k, look, sys) == Material::Solid {
                        assert_eq!(m.opacity, 1.0, "{k:?} {look:?} {sys:?}");
                        assert_eq!((m.blur, m.grain, m.highlight), (0.0, 0.0, 0.0));
                    } else {
                        assert_eq!(m, glass(look, sys.dark, sys), "{k:?} {look:?}");
                    }
                }
            }
        }
    }

    #[test]
    fn modes_follow_the_system_state() {
        let m = modes(SysState {
            transparency_on: false,
            high_contrast: true,
            animations_on: false,
            ..normal()
        });
        assert!(m.reduced_transparency && m.high_contrast && m.reduced_motion);
        assert!(!m.lite, "lite is Android only");
        assert_eq!(modes(normal()), crate::tokens_rules::Modes::default());
    }
}
