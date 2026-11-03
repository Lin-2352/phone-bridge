//! "Pause Relay": while paused nothing is accepted or sent. The state lives
//! in `Settings::paused` (one source of truth, saved with the other
//! settings); this module holds the pure rules around it and the registry
//! of open sessions that a pause closes.

use std::sync::Mutex;

use pb_transport::SessionCloser;

use crate::core::Cmd;

/// What a change of the pause state is called in the logs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transition {
    Paused,
    Resumed,
}

/// The event for going from `was` to `now`; none when nothing changed, so
/// pausing twice is logged once.
pub fn transition(was: bool, now: bool) -> Option<Transition> {
    match (was, now) {
        (false, true) => Some(Transition::Paused),
        (true, false) => Some(Transition::Resumed),
        _ => None,
    }
}

/// Whether the command talks to a phone (or opens a way for one). While
/// paused these are refused. Exhaustive on purpose: a new command must be
/// decided here.
pub fn needs_network(c: &Cmd) -> bool {
    match c {
        Cmd::Pair
        | Cmd::CheckPeers
        | Cmd::SendClipboard(_)
        | Cmd::SendFiles(_)
        | Cmd::ClipboardChanged { .. }
        | Cmd::ClipboardImageChanged { .. }
        | Cmd::ControlPhone
        | Cmd::ShowScreen
        | Cmd::StartCamera
        | Cmd::StartAudio(_)
        | Cmd::SendText { .. }
        | Cmd::NotifyAction { .. } => true,
        Cmd::CancelPair
        | Cmd::StopCamera
        | Cmd::StopAudio(_)
        | Cmd::SwitchCameraLens
        | Cmd::RepairCamera
        | Cmd::RemoveCamera
        | Cmd::Decide { .. }
        | Cmd::Unpair(_) => false,
    }
}

/// Commands the user does not expect an answer to (background work such as
/// the clipboard listener): refused without a notice.
pub fn is_quiet(c: &Cmd) -> bool {
    matches!(
        c,
        Cmd::ClipboardChanged { .. } | Cmd::ClipboardImageChanged { .. } | Cmd::CheckPeers
    )
}

/// The tray tooltip after the core stopped: what happened and the way out.
pub fn failed_tooltip(reason: &str) -> String {
    // A tooltip holds 127 characters.
    let head = "Relay stopped. Open Relay and choose Restart. ";
    let room = 127usize.saturating_sub(head.chars().count());
    let why: String = reason.chars().take(room).collect();
    format!("{head}{why}")
}

/// The tray tooltip: the short fingerprint, and the pause when it is on.
pub fn tooltip(paused: bool, fingerprint: &str) -> String {
    match (paused, fingerprint.is_empty()) {
        (true, _) => "Relay is paused".to_owned(),
        (false, true) => "Relay".to_owned(),
        (false, false) => format!("Relay ({fingerprint})"),
    }
}

static OPEN: Mutex<Vec<(u64, SessionCloser)>> = Mutex::new(Vec::new());
static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// Keeps a session findable by `close_all` until dropped.
pub struct Registered(u64);

pub fn register(c: SessionCloser) -> Registered {
    let id = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    OPEN.lock().unwrap_or_else(|p| p.into_inner()).push((id, c));
    Registered(id)
}

impl Drop for Registered {
    fn drop(&mut self) {
        OPEN.lock()
            .unwrap_or_else(|p| p.into_inner())
            .retain(|(id, _)| *id != self.0);
    }
}

/// End every open session now.
pub fn close_all() {
    let all: Vec<SessionCloser> = OPEN
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .iter()
        .map(|(_, c)| c.clone())
        .collect();
    for c in all {
        c.close(b"paused");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stopped_core_says_so_in_the_tooltip_within_the_limit() {
        let t = failed_tooltip("address already in use");
        assert!(t.starts_with("Relay stopped"));
        assert!(t.contains("address already in use"));
        assert!(failed_tooltip(&"x".repeat(500)).chars().count() <= 127);
    }

    #[test]
    fn transitions_fire_only_on_a_change() {
        assert_eq!(transition(false, true), Some(Transition::Paused));
        assert_eq!(transition(true, false), Some(Transition::Resumed));
        assert_eq!(transition(true, true), None);
        assert_eq!(transition(false, false), None);
    }

    #[test]
    fn sending_is_refused_but_stopping_and_unpairing_are_not() {
        assert!(needs_network(&Cmd::SendFiles(vec![])));
        assert!(needs_network(&Cmd::ShowScreen));
        assert!(needs_network(&Cmd::StartCamera));
        assert!(needs_network(&Cmd::ControlPhone));
        assert!(needs_network(&Cmd::StartAudio(
            crate::audio_state::Dir::PcToPhone
        )));
        assert!(!needs_network(&Cmd::StopAudio(
            crate::audio_state::Dir::PcToPhone
        )));
        assert!(needs_network(&Cmd::Pair));
        assert!(!needs_network(&Cmd::StopCamera));
        assert!(!needs_network(&Cmd::CancelPair));
        assert!(!needs_network(&Cmd::Decide {
            id: 1,
            accept: true
        }));
    }

    #[test]
    fn background_commands_are_refused_quietly() {
        assert!(is_quiet(&Cmd::ClipboardChanged {
            text: String::new(),
            sensitive: false
        }));
        assert!(!is_quiet(&Cmd::SendFiles(vec![])));
    }

    #[test]
    fn tooltip_says_paused_and_restores_the_fingerprint() {
        assert_eq!(tooltip(true, "ABCDEF123"), "Relay is paused");
        assert_eq!(tooltip(false, "ABCDEF123"), "Relay (ABCDEF123)");
        assert_eq!(tooltip(false, ""), "Relay");
    }
}
