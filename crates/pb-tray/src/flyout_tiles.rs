//! What the tray flyout's Quick Settings tiles show, kept free of Win32 so it
//! can be tested: one tile per feature and the status line under the grid.

use crate::flyout::Model;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TileKind {
    Camera,
    Screen,
    Clipboard,
    SendFile,
    Pair,
}

pub struct Tile {
    pub kind: TileKind,
    pub label: String,
    /// Drawn with the accent fill.
    pub on: bool,
    /// Drawn dimmed and ignores clicks when false.
    pub enabled: bool,
}

/// The five tiles, in grid order: Camera, Screen, Clipboard, SendFile, Pair.
/// Camera, Screen and SendFile need a paired phone, except that a running
/// camera or an open screen window stays clickable so it can be stopped.
pub fn tiles(m: &Model) -> Vec<Tile> {
    let phone = m.peer.is_some();
    let camera = if !m.camera_on {
        "Camera".to_owned()
    } else if m.camera_fps > 0 {
        format!("Camera, {} fps", m.camera_fps)
    } else {
        "Camera on".to_owned()
    };
    vec![
        Tile {
            kind: TileKind::Camera,
            label: camera,
            on: m.camera_on,
            enabled: phone || m.camera_on,
        },
        Tile {
            kind: TileKind::Screen,
            label: "Phone screen".to_owned(),
            on: m.screen_on,
            enabled: phone || m.screen_on,
        },
        Tile {
            kind: TileKind::Clipboard,
            label: "Clipboard".to_owned(),
            on: m.clipboard_on,
            enabled: true,
        },
        Tile {
            kind: TileKind::SendFile,
            label: "Send files".to_owned(),
            on: false,
            enabled: phone,
        },
        Tile {
            kind: TileKind::Pair,
            label: if phone {
                "Control phone"
            } else {
                "Pair device"
            }
            .to_owned(),
            on: false,
            enabled: true,
        },
    ]
}

/// One line under the grid: the paired phone and the last activity, clipped
/// to at most `max_chars` characters with a trailing ellipsis.
pub fn status_line(m: &Model, max_chars: usize) -> String {
    let line = match (&m.peer, m.last.is_empty()) {
        (Some(n), true) => format!("Paired with {n}"),
        (Some(n), false) => format!("{n} \u{b7} {}", m.last),
        (None, true) => "No phone paired".to_owned(),
        (None, false) => m.last.clone(),
    };
    if line.chars().count() <= max_chars {
        return line;
    }
    if max_chars == 0 {
        return String::new();
    }
    let mut out: String = line.chars().take(max_chars - 1).collect();
    out.truncate(out.trim_end().len());
    out.push('…');
    out
}

/// What a left-button release should act on: the thing under the cursor, but
/// only when the press started on that same thing.
pub fn release_target<T: PartialEq>(pressed: Option<T>, under: Option<T>) -> Option<T> {
    match (pressed, under) {
        (Some(p), Some(u)) if p == u => Some(u),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paired() -> Model {
        Model {
            peer: Some("Pixel 8".to_owned()),
            ..Default::default()
        }
    }

    #[test]
    fn five_tiles_in_order() {
        let kinds: Vec<TileKind> = tiles(&paired()).iter().map(|t| t.kind).collect();
        assert_eq!(
            kinds,
            [
                TileKind::Camera,
                TileKind::Screen,
                TileKind::Clipboard,
                TileKind::SendFile,
                TileKind::Pair,
            ]
        );
    }

    #[test]
    fn camera_tile_is_on_while_streaming() {
        let mut m = paired();
        assert!(!tiles(&m)[0].on);
        m.camera_on = true;
        m.camera_fps = 30;
        let cam = &tiles(&m)[0];
        assert_eq!(cam.kind, TileKind::Camera);
        assert!(cam.on);
        assert!(cam.enabled);
    }

    #[test]
    fn tiles_that_need_a_phone_are_disabled_without_one() {
        for t in tiles(&Model::default()) {
            let needs_phone = matches!(
                t.kind,
                TileKind::Camera | TileKind::Screen | TileKind::SendFile
            );
            assert_eq!(t.enabled, !needs_phone, "{:?}", t.kind);
        }
    }

    #[test]
    fn status_line_names_the_phone() {
        assert!(status_line(&paired(), 60).contains("Pixel 8"));
        let mut m = paired();
        m.last = "Sent photo.jpg".to_owned();
        let line = status_line(&m, 60);
        assert!(line.contains("Pixel 8") && line.contains("Sent photo.jpg"));
    }

    #[test]
    fn long_text_is_clipped() {
        let m = Model {
            last: "x".repeat(200),
            ..Default::default()
        };
        let line = status_line(&m, 40);
        assert!(line.chars().count() <= 40);
        assert!(line.ends_with('…'));
    }

    #[test]
    fn release_on_the_pressed_tile_acts() {
        assert_eq!(release_target(Some(2), Some(2)), Some(2));
    }

    #[test]
    fn release_on_another_tile_does_nothing() {
        assert_eq!(release_target(Some(2), Some(3)), None);
        assert_eq!(release_target(Some(2), None), None);
    }

    #[test]
    fn release_without_a_press_does_nothing() {
        assert_eq!(release_target(None, Some(2)), None);
        assert_eq!(release_target::<usize>(None, None), None);
    }
}
