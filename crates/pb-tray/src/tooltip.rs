//! A native tooltip over a whole custom-drawn window: it asks the window
//! (`TTN_GETDISPINFO`) what to say about the point under the pointer, so
//! full names of truncated rows and long labels are one hover away.
#![allow(unsafe_code)]

use windows::{
    core::{w, PWSTR},
    Win32::{
        Foundation::{HWND, LPARAM, RECT, WPARAM},
        UI::{
            Controls::{
                NMTTDISPINFOW, TTF_SUBCLASS, TTM_ADDTOOLW, TTM_NEWTOOLRECT, TTM_SETMAXTIPWIDTH,
                TTS_ALWAYSTIP, TTS_NOPREFIX, TTTOOLINFOW,
            },
            WindowsAndMessaging::{
                CreateWindowExW, DestroyWindow, SendMessageW, CW_USEDEFAULT, WINDOW_STYLE,
                WS_EX_TOPMOST, WS_POPUP,
            },
        },
    },
};

/// `TTN_GETDISPINFOW` (`TTN_FIRST - 10`, with `TTN_FIRST` = -520).
pub const TTN_GETDISPINFOW: u32 = (-530i32) as u32;

pub struct Tip {
    tip: HWND,
    owner: HWND,
}

impl Tip {
    /// A tooltip for `owner`'s whole client area `client`.
    pub fn new(owner: HWND, client: RECT) -> Option<Tip> {
        // SAFETY: a tooltip control created for our window, one tool whose
        // text is asked for on demand (-1 = LPSTR_TEXTCALLBACK).
        unsafe {
            let tip = CreateWindowExW(
                WS_EX_TOPMOST,
                w!("tooltips_class32"),
                None,
                WS_POPUP | WINDOW_STYLE(TTS_ALWAYSTIP | TTS_NOPREFIX),
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                Some(owner),
                None,
                None,
                None,
            )
            .ok()?;
            let mut ti = TTTOOLINFOW {
                cbSize: std::mem::size_of::<TTTOOLINFOW>() as u32,
                uFlags: TTF_SUBCLASS,
                hwnd: owner,
                uId: 1,
                rect: client,
                lpszText: PWSTR(usize::MAX as *mut u16),
                ..Default::default()
            };
            SendMessageW(
                tip,
                TTM_ADDTOOLW,
                Some(WPARAM(0)),
                Some(LPARAM(&mut ti as *mut _ as isize)),
            );
            SendMessageW(tip, TTM_SETMAXTIPWIDTH, Some(WPARAM(0)), Some(LPARAM(420)));
            Some(Tip { tip, owner })
        }
    }

    /// The window was resized.
    pub fn fit(&self, client: RECT) {
        let mut ti = TTTOOLINFOW {
            cbSize: std::mem::size_of::<TTTOOLINFOW>() as u32,
            hwnd: self.owner,
            uId: 1,
            rect: client,
            ..Default::default()
        };
        // SAFETY: TTM_NEWTOOLRECT reads the tool id and the new rect.
        unsafe {
            SendMessageW(
                self.tip,
                TTM_NEWTOOLRECT,
                Some(WPARAM(0)),
                Some(LPARAM(&mut ti as *mut _ as isize)),
            );
        }
    }
}

impl Drop for Tip {
    fn drop(&mut self) {
        // SAFETY: our own control (it may already be gone with the window).
        let _ = unsafe { DestroyWindow(self.tip) };
    }
}

/// Answers a `TTN_GETDISPINFOW` with `text` (at most 79 UTF-16 units; an
/// empty text shows nothing). `lparam` is the `WM_NOTIFY` pointer.
pub fn answer(lparam: LPARAM, text: &str) {
    // SAFETY: for TTN_GETDISPINFOW the pointer is an NMTTDISPINFOW the
    // control owns for the duration of the call.
    unsafe {
        let info = &mut *(lparam.0 as *mut NMTTDISPINFOW);
        let mut n = 0;
        for (i, u) in text.encode_utf16().take(info.szText.len() - 1).enumerate() {
            info.szText[i] = u;
            n = i + 1;
        }
        info.szText[n] = 0;
        info.lpszText = PWSTR(info.szText.as_mut_ptr());
    }
}

/// The tooltip for a described control: its full name, when that says more
/// than a short visible label can (names of rows, long titles, "...").
pub fn text_for(name: &str) -> Option<String> {
    (name.chars().count() > 14).then(|| name.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_long_names_get_a_tooltip() {
        assert_eq!(text_for("Open"), None);
        assert_eq!(text_for("Pair device"), None);
        assert!(text_for("report-final-v2.pdf, 1.2 MB, From Pixel 8").is_some());
        assert!(text_for("More actions for report.pdf").is_some());
    }
}
