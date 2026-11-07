//! The offscreen snapshot job's pure parts: which renders it makes, which fonts it needs, and what a
//! translucent surface composites over in the simulated backdrop. The renderer itself (Direct2D on WARP)
//! is in `d2d`; nothing here touches Windows, so the tests run everywhere.

#![allow(dead_code)] // the --snapshot renderer uses these (the next M5.10 commit)

use crate::look::{Material, SurfaceKind};

/// Every surface the snapshot job draws.
pub const SURFACES: [(SurfaceKind, &str); 9] = [
    (SurfaceKind::MainWindow, "main"),
    (SurfaceKind::Flyout, "flyout"),
    (SurfaceKind::Menu, "menu"),
    (SurfaceKind::Dialog, "dialog"),
    (SurfaceKind::Qr, "qr"),
    (SurfaceKind::Transfers, "transfers"),
    (SurfaceKind::StatusPill, "status"),
    (SurfaceKind::ScreenBars, "screen"),
    (SurfaceKind::Camera, "camera"),
];

/// The name of each render: `<surface>-<look>-<mode>`, for the 9 surfaces, 2 looks and 2 modes.
pub fn snapshot_names() -> Vec<String> {
    let mut v = Vec::new();
    for (_, surface) in SURFACES {
        for look in ["native", "relay"] {
            for mode in ["light", "dark"] {
                v.push(format!("{surface}-{look}-{mode}"));
            }
        }
    }
    v
}

/// The fonts the renders need. Neither text face means the renders would be wrong, so the job stops.
/// `have` answers whether a family is installed.
pub fn check_fonts(have: &dyn Fn(&str) -> bool) -> Result<(), String> {
    if !have("Segoe UI Variable Text") && !have("Segoe UI") {
        return Err("missing font: Segoe UI Variable Text".into());
    }
    if !have("Segoe Fluent Icons") && !have("Segoe MDL2 Assets") {
        return Err("missing font: Segoe Fluent Icons".into());
    }
    Ok(())
}

/// What a translucent surface composites over the simulated backdrop: a tint and its opacity. Solid is an
/// opaque fill, so every material gives a different pair and the renders tell them apart.
pub fn simulated_backdrop(material: Material, dark: bool) -> (u32, f32) {
    let (page, relay_tint) = if dark {
        (0x202020, 0x14172A)
    } else {
        (0xF3F3F3, 0xE6ECFF)
    };
    match material {
        Material::Mica => (page, 0.85),
        Material::Acrylic => (page, 0.70),
        Material::RelayGlass => (relay_tint, 0.88),
        Material::Solid => (page, 1.0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn snapshot_names_cover_every_surface_look_and_mode() {
        let names = snapshot_names();
        assert_eq!(names.len(), SURFACES.len() * 2 * 2);
        let unique: HashSet<_> = names.iter().collect();
        assert_eq!(unique.len(), names.len(), "no duplicates");
        assert!(names.contains(&"flyout-relay-dark".to_owned()));
        assert!(names.contains(&"qr-native-light".to_owned()));
    }

    #[test]
    fn simulated_backdrops_differ_across_mica_acrylic_relay_and_solid() {
        for dark in [false, true] {
            let fills: HashSet<(u32, u32)> = [
                Material::Mica,
                Material::Acrylic,
                Material::RelayGlass,
                Material::Solid,
            ]
            .into_iter()
            .map(|m| {
                let (c, a) = simulated_backdrop(m, dark);
                (c, (a * 100.0) as u32)
            })
            .collect();
            assert_eq!(fills.len(), 4, "dark {dark}");
        }
    }

    #[test]
    fn missing_fonts_fail_fast() {
        assert!(check_fonts(&|_| true).is_ok());
        let none = |_: &str| false;
        assert_eq!(
            check_fonts(&none),
            Err("missing font: Segoe UI Variable Text".into())
        );
        // The Windows 10 fallback face is enough for text.
        let segoe_ui_only = |f: &str| f == "Segoe UI" || f == "Segoe MDL2 Assets";
        assert!(check_fonts(&segoe_ui_only).is_ok());
        let no_icons = |f: &str| f.starts_with("Segoe UI");
        assert_eq!(
            check_fonts(&no_icons),
            Err("missing font: Segoe Fluent Icons".into())
        );
    }
}
