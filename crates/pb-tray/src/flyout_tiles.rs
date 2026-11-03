//! What the tray flyout's Quick Settings tiles show, kept free of Win32 so it
//! can be tested: one tile per feature and the status line under the grid.

use crate::flyout::Model;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TileKind {
    Camera,
    Screen,
    Audio,
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

/// The six tiles, in grid order: Camera, Screen, Audio, Clipboard, SendFile,
/// Pair. All but Clipboard and Pair need a paired phone, except that a running
/// camera, screen window or audio share stays clickable so it can be stopped.
pub fn tiles(m: &Model) -> Vec<Tile> {
    // Paused: the core refuses everything that needs the phone, so say so up
    // front instead of after the click (a live session may still be stopped).
    let phone = m.peer.is_some() && !m.paused && !m.stopped;
    let free = !m.paused && !m.stopped;
    // A request the phone has not answered: the tile says Cancel, so a second
    // click is a decision, and it works while paused (cancelling needs no network).
    let camera_wait = m.camera_pending && !m.camera_on;
    let screen_wait = m.screen_pending && !m.screen_on;
    let camera = if camera_wait {
        "Cancel start".to_owned()
    } else if !m.camera_on {
        "Camera".to_owned()
    } else if m.camera_fps > 0 {
        format!("Camera, {} fps", m.camera_fps)
    } else {
        "Camera on".to_owned()
    };
    let audio = m.audio[crate::audio_state::Dir::PhoneToPc.index()];
    let audio_live = audio != crate::audio_state::Phase::Idle;
    vec![
        Tile {
            kind: TileKind::Camera,
            label: camera,
            on: m.camera_on || camera_wait,
            enabled: phone || m.camera_on || camera_wait,
        },
        Tile {
            kind: TileKind::Screen,
            label: if screen_wait {
                "Cancel start"
            } else {
                "Phone screen"
            }
            .to_owned(),
            on: m.screen_on || screen_wait,
            enabled: phone || m.screen_on || screen_wait,
        },
        Tile {
            kind: TileKind::Audio,
            label: match audio {
                crate::audio_state::Phase::Idle => "Phone audio",
                crate::audio_state::Phase::Asking => "Cancel start",
                crate::audio_state::Phase::Running => "Stop audio",
            }
            .to_owned(),
            on: audio_live,
            enabled: phone || audio_live,
        },
        Tile {
            kind: TileKind::Clipboard,
            label: "Clipboard".to_owned(),
            on: m.clipboard_on,
            enabled: free,
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
            enabled: free,
        },
    ]
}

/// One line under the grid: the paired phone and the last activity, clipped
/// to at most `max_chars` characters with a trailing ellipsis.
pub fn status_line(m: &Model, max_chars: usize) -> String {
    if m.stopped {
        return "Relay stopped. Restart it from Home.".to_owned();
    }
    if m.paused {
        return "Paused. Resume in the tray menu.".to_owned();
    }
    if let Some(a) = audio_line(m) {
        return clip(a, max_chars);
    }
    let line = match (&m.peer, m.last.is_empty()) {
        (Some(n), true) => format!("Paired with {n}"),
        (Some(n), false) => format!("{n} \u{b7} {}", m.last),
        (None, true) => "No phone paired".to_owned(),
        (None, false) => m.last.clone(),
    };
    clip(line, max_chars)
}

/// Running audio is the most sensitive thing Relay does, so the flyout names it
/// instead of the last activity (the Stop is on Home and in the tray menu).
fn audio_line(m: &Model) -> Option<String> {
    use crate::audio_state::{Dir, Phase};
    let on: Vec<&str> = Dir::ALL
        .iter()
        .filter(|d| m.audio[d.index()] == Phase::Running)
        .map(|d| d.title())
        .collect();
    (!on.is_empty()).then(|| format!("{} · Stop in the tray menu", on.join(" and ")))
}

fn clip(line: String, max_chars: usize) -> String {
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
    fn six_tiles_in_order() {
        let kinds: Vec<TileKind> = tiles(&paired()).iter().map(|t| t.kind).collect();
        assert_eq!(
            kinds,
            [
                TileKind::Camera,
                TileKind::Screen,
                TileKind::Audio,
                TileKind::Clipboard,
                TileKind::SendFile,
                TileKind::Pair,
            ]
        );
    }

    fn audio_tile(m: &Model) -> Tile {
        tiles(m)
            .into_iter()
            .find(|t| t.kind == TileKind::Audio)
            .expect("the audio tile")
    }

    #[test]
    fn the_audio_tile_follows_the_phone_to_pc_session() {
        use crate::audio_state::{Dir, Phase};
        let mut m = paired();
        let idle = audio_tile(&m);
        assert_eq!(idle.label, "Phone audio");
        assert!(!idle.on && idle.enabled);
        m.audio[Dir::PhoneToPc.index()] = Phase::Asking;
        let asking = audio_tile(&m);
        assert_eq!(asking.label, "Cancel start");
        assert!(asking.on && asking.enabled);
        m.audio[Dir::PhoneToPc.index()] = Phase::Running;
        let running = audio_tile(&m);
        assert_eq!(running.label, "Stop audio");
        assert!(running.on && running.enabled);
    }

    #[test]
    fn pc_audio_on_the_phone_does_not_light_the_phone_audio_tile() {
        use crate::audio_state::{Dir, Phase};
        let mut m = paired();
        m.audio[Dir::PcToPhone.index()] = Phase::Running;
        let t = audio_tile(&m);
        assert_eq!(t.label, "Phone audio");
        assert!(!t.on);
    }

    #[test]
    fn a_running_share_can_be_stopped_while_paused_but_not_started() {
        use crate::audio_state::{Dir, Phase};
        let mut m = Model {
            paused: true,
            ..paired()
        };
        assert!(!audio_tile(&m).enabled);
        m.audio[Dir::PhoneToPc.index()] = Phase::Running;
        assert!(audio_tile(&m).enabled);
    }

    #[test]
    fn a_waiting_request_says_cancel_and_stays_clickable_when_paused() {
        let m = Model {
            paused: true,
            camera_pending: true,
            screen_pending: true,
            ..paired()
        };
        let t = tiles(&m);
        for i in [0, 1] {
            assert_eq!(t[i].label, "Cancel start");
            assert!(t[i].on && t[i].enabled);
        }
        let plain = tiles(&paired());
        assert_eq!(plain[0].label, "Camera");
        assert_eq!(plain[1].label, "Phone screen");
    }

    #[test]
    fn running_audio_is_named_under_the_tiles() {
        use crate::audio_state::Phase;
        let mut m = paired();
        m.audio[1] = Phase::Asking;
        assert!(status_line(&m, 80).contains("Pixel 8"));
        m.audio[1] = Phase::Running;
        let line = status_line(&m, 80);
        assert!(line.contains("PC audio on the phone"), "{line}");
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
                TileKind::Camera | TileKind::Screen | TileKind::Audio | TileKind::SendFile
            );
            assert_eq!(t.enabled, !needs_phone, "{:?}", t.kind);
        }
    }

    #[test]
    fn paused_disables_every_tile_that_needs_the_network() {
        let m = Model {
            paused: true,
            ..paired()
        };
        for t in tiles(&m) {
            assert!(!t.enabled, "{:?} must be disabled while paused", t.kind);
        }
        assert_eq!(status_line(&m, 80), "Paused. Resume in the tray menu.");
    }

    #[test]
    fn a_stopped_core_disables_every_tile() {
        let m = Model {
            stopped: true,
            ..paired()
        };
        assert!(tiles(&m).iter().all(|t| !t.enabled));
        assert!(status_line(&m, 80).contains("stopped"));
    }

    #[test]
    fn a_running_session_can_still_be_stopped_while_paused() {
        let m = Model {
            paused: true,
            camera_on: true,
            screen_on: true,
            ..paired()
        };
        let t = tiles(&m);
        assert!(t[0].enabled && t[1].enabled);
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
