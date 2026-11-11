//! Received clips wait in RAM for clipboard access. Only the newest is retained;
//! a newer local copy supersedes it. No retry timer runs while locked or idle.

use super::{
    arm_timer, balloon, clipboard_seq, kill_timer, notify_ui, State, OWN_SEQ, TIMER_CLIP_RETRY,
};
use crate::clip_retry::PendingClipboard;
use pb_platform_win::clipboard;
use std::io;
use std::sync::{atomic::Ordering, Mutex};
use windows::Win32::Foundation::HWND;

#[derive(Clone, PartialEq, Eq)]
pub(super) enum Source {
    Explicit(String),
    Sync { peer: String, pulled: bool },
    Shared,
    Image(String),
}

enum Payload {
    Text {
        text: String,
        private: bool,
    },
    Image {
        png: Vec<u8>,
        dib: Vec<u8>,
        private: bool,
    },
}

struct Copy {
    from: String,
    source: Source,
    sensitive: bool,
    payload: Payload,
}

static PENDING: Mutex<PendingClipboard<Copy>> = Mutex::new(PendingClipboard::new());

pub(super) fn text(
    hwnd: HWND,
    st: &State,
    from: String,
    text: String,
    sensitive: bool,
    source: Source,
) {
    let keep = st
        .settings
        .lock()
        .map(|s| s.keep_in_history)
        .unwrap_or(false);
    let private = sensitive || (!matches!(source, Source::Explicit(_)) && !keep);
    submit(
        hwnd,
        st,
        Copy {
            from,
            source,
            sensitive,
            payload: Payload::Text { text, private },
        },
    );
}

pub(super) fn image(hwnd: HWND, st: &State, peer: String, from: String, png: Vec<u8>) {
    // Decode once, before trying to open the clipboard; retries reuse this RAM buffer.
    let dib = match pb_platform_win::dib::png_to_dibv5(&png) {
        Ok(dib) => dib,
        Err(e) => {
            st.log.line(&format!("CLIPBOARD picture rejected: {e}"));
            balloon(hwnd, "Relay", "Could not decode the received picture");
            return;
        }
    };
    let private = st
        .settings
        .lock()
        .map(|s| !s.keep_in_history)
        .unwrap_or(true);
    submit(
        hwnd,
        st,
        Copy {
            from,
            source: Source::Image(peer),
            sensitive: false,
            payload: Payload::Image { png, dib, private },
        },
    );
}

fn submit(hwnd: HWND, st: &State, copy: Copy) {
    PENDING
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .replace(clipboard_seq(), copy);
    retry(hwnd, st, true);
}

pub(super) fn retry(hwnd: HWND, st: &State, initial: bool) {
    let locked = notify_ui().locked;
    if locked {
        kill_timer(hwnd, TIMER_CLIP_RETRY);
        if initial {
            st.log.line("CLIPBOARD queued while this PC is locked");
        }
        return;
    }
    let result: io::Result<Option<Copy>> = PENDING
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .try_write(|copy| {
            let permitted = match &copy.source {
                Source::Explicit(peer) => {
                    crate::core::clipboard_allowed(&st.settings, peer, true, false)
                }
                Source::Sync { peer, pulled } => {
                    crate::core::clipboard_allowed(&st.settings, peer, *pulled, false)
                }
                Source::Image(peer) => {
                    crate::core::clipboard_allowed(&st.settings, peer, false, true)
                }
                Source::Shared => st
                    .settings
                    .lock()
                    .map(|s| !s.paused && s.auto_copy_shared_text)
                    .unwrap_or(false),
            };
            if !permitted {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "clipboard permission revoked",
                ));
            }
            // A history switch turned off while queued must not make the pending clip public.
            let hide = !matches!(copy.source, Source::Explicit(_))
                && st
                    .settings
                    .lock()
                    .map(|s| !s.keep_in_history)
                    .unwrap_or(true);
            match &copy.payload {
                Payload::Text { text, private } => {
                    clipboard::try_set_text(text, *private || hide, Some(hwnd))?;
                    OWN_SEQ.store(clipboard_seq(), Ordering::Relaxed);
                    Ok(())
                }
                Payload::Image { png, dib, private } => {
                    clipboard::try_set_image(png, dib, *private || hide, Some(hwnd))?;
                    OWN_SEQ.store(clipboard_seq(), Ordering::Relaxed);
                    Ok(())
                }
            }
        });
    match result {
        Ok(Some(copy)) => {
            st.log.line(match &copy.source {
                Source::Sync { .. } => "CLIPSYNC written to this PC's clipboard",
                Source::Image(_) => "CLIPBOARD picture written to this PC's clipboard",
                Source::Explicit(_) => "CLIPBOARD text written to this PC's clipboard",
                Source::Shared => "SHARED text written to this PC's clipboard",
            });
            match &copy.source {
                Source::Explicit(_) => {
                    let show = st
                        .settings
                        .lock()
                        .map(|s| s.toast_show_text)
                        .unwrap_or(false);
                    let Payload::Text { text, .. } = &copy.payload else {
                        unreachable!()
                    };
                    balloon(
                        hwnd,
                        &format!("Clipboard from {}", copy.from),
                        &crate::shared_text::clipboard_toast_body(copy.sensitive, show, text),
                    );
                }
                Source::Shared => balloon(
                    hwnd,
                    &crate::shared_text::title(&copy.from),
                    crate::shared_text::body(true),
                ),
                Source::Image(_) => balloon(
                    hwnd,
                    &format!("Copied on {}", copy.from),
                    "Picture ready to paste",
                ),
                Source::Sync { .. } => balloon(
                    hwnd,
                    &format!("Copied on {}", copy.from),
                    if copy.sensitive {
                        "Password or code, ready to paste"
                    } else {
                        "Ready to paste"
                    },
                ),
            }
        }
        Ok(None) => {}
        Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
            if initial {
                st.log
                    .line("CLIPBOARD queued: clipboard is busy; will retry");
            }
        }
        Err(e) if e.kind() == io::ErrorKind::PermissionDenied => {
            PENDING.lock().unwrap_or_else(|p| p.into_inner()).clear();
            st.log
                .line("CLIPBOARD queued copy discarded: clipboard access revoked or Relay paused");
        }
        Err(e) => {
            PENDING.lock().unwrap_or_else(|p| p.into_inner()).clear();
            st.log.line(&format!("CLIPBOARD write failed: {e}"));
            balloon(hwnd, "Relay", "Could not set the clipboard");
        }
    }
    if PENDING
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .is_pending()
    {
        arm_timer(hwnd, TIMER_CLIP_RETRY, 250);
    } else {
        kill_timer(hwnd, TIMER_CLIP_RETRY);
    }
}

pub(super) fn local_copy(hwnd: HWND, st: &State, sequence: u32) {
    if !notify_ui().locked
        && PENDING
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .observe_local_copy(sequence)
    {
        kill_timer(hwnd, TIMER_CLIP_RETRY);
        st.log
            .line("CLIPBOARD queued clip superseded by a newer local copy");
    }
}
