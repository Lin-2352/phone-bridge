//! A small status pill near the tray for answers to the user's own actions
//! ("Asking RMX3561…", "RMX3561 said no", "Controlling RMX3561"). Shell
//! balloons are hidden by Do Not Disturb, and these are not notifications:
//! they are the app answering what the user just did. One pill at a time;
//! a new message replaces the old one. Never takes focus.
#![allow(unsafe_code)]

use std::sync::Mutex;

use windows::{
    core::{w, PCWSTR},
    Win32::{
        Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM},
        Graphics::Gdi::{
            GetMonitorInfoW, InvalidateRect, MonitorFromPoint, DT_END_ELLIPSIS, DT_LEFT,
            DT_SINGLELINE, DT_VCENTER, MONITORINFO, MONITOR_DEFAULTTOPRIMARY,
        },
        System::LibraryLoader::GetModuleHandleW,
        UI::WindowsAndMessaging::{
            CreateWindowExW, DefWindowProcW, DestroyWindow, KillTimer, LoadCursorW, RegisterClassW,
            SetTimer, SetWindowPos, ShowWindow, HWND_TOPMOST, IDC_ARROW, SWP_NOACTIVATE,
            SW_SHOWNOACTIVATE, WM_DESTROY, WM_ERASEBKGND, WM_LBUTTONUP, WM_PAINT, WM_TIMER,
            WNDCLASSW, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
        },
    },
};

use crate::fluent::{self, px};

const CLASS: PCWSTR = w!("PhoneBridgeStatus");
/// Logical (96-dpi) size.
const W: i32 = 440;
const H: i32 = 52;
const TIMER_HIDE: usize = 1;

struct Pill {
    hwnd: isize,
    text: String,
}

static CURRENT: Mutex<Option<Pill>> = Mutex::new(None);

extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_ERASEBKGND => LRESULT(1),
        WM_PAINT => {
            let text = CURRENT
                .lock()
                .ok()
                .and_then(|g| g.as_ref().map(|p| p.text.clone()))
                .unwrap_or_default();
            let s = fluent::scale(hwnd);
            let p = fluent::palette();
            fluent::paint(hwnd, |hdc, c| {
                let f = fluent::Fonts::new(s);
                let mut r = c;
                r.left += px(s, 18);
                r.right -= px(s, fluent::ROW_PAD);
                fluent::text(
                    hdc,
                    f.body,
                    &text,
                    r,
                    p.text,
                    DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS,
                );
            });
            LRESULT(0)
        }
        // Click to dismiss; it also goes away by itself.
        WM_TIMER | WM_LBUTTONUP => {
            close();
            LRESULT(0)
        }
        WM_DESTROY => {
            if let Ok(mut g) = CURRENT.lock() {
                if g.as_ref().is_some_and(|p| p.hwnd == hwnd.0 as isize) {
                    *g = None;
                }
            }
            LRESULT(0)
        }
        // SAFETY: default handling.
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

/// Show `text` for `secs` seconds (replacing any current message). UI thread.
pub fn show(text: &str, secs: u32) {
    // Reuse the open pill: new text, timer restarted.
    let open = CURRENT.lock().ok().and_then(|mut g| {
        g.as_mut().map(|p| {
            p.text = text.to_owned();
            p.hwnd
        })
    });
    if let Some(h) = open {
        let hwnd = HWND(h as *mut _);
        // SAFETY: our own window, on the UI thread.
        unsafe {
            let _ = KillTimer(Some(hwnd), TIMER_HIDE);
            SetTimer(Some(hwnd), TIMER_HIDE, secs * 1000, None);
            let _ = InvalidateRect(Some(hwnd), None, true);
        }
        return;
    }
    // SAFETY: window creation and positioning on the UI thread.
    unsafe {
        let Ok(hinstance) = GetModuleHandleW(None) else {
            return;
        };
        let wc = WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: hinstance.into(),
            lpszClassName: CLASS,
            // Without a class cursor Windows keeps showing the busy cursor.
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            ..Default::default()
        };
        RegisterClassW(&wc);
        let Ok(hwnd) = CreateWindowExW(
            WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_NOACTIVATE,
            CLASS,
            w!("Phone Bridge"),
            WS_POPUP,
            0,
            0,
            W,
            H,
            None,
            None,
            Some(hinstance.into()),
            None,
        ) else {
            return;
        };
        if let Ok(mut g) = CURRENT.lock() {
            *g = Some(Pill {
                hwnd: hwnd.0 as isize,
                text: text.to_owned(),
            });
        }
        let s = fluent::scale(hwnd);
        let (w, h) = (px(s, W), px(s, H));
        // Bottom-right of the primary monitor's work area, above the taskbar.
        let mon = MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY);
        let mut mi = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        let (x, y) = if GetMonitorInfoW(mon, &mut mi).as_bool() {
            let wa = mi.rcWork;
            (
                wa.right - w - px(s, fluent::GROUP_GAP),
                wa.bottom - h - px(s, fluent::GROUP_GAP),
            )
        } else {
            (0, 0)
        };
        let _ = SetWindowPos(hwnd, Some(HWND_TOPMOST), x, y, w, h, SWP_NOACTIVATE);
        fluent::frame(hwnd, false);
        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        SetTimer(Some(hwnd), TIMER_HIDE, secs * 1000, None);
    }
}

pub fn close() {
    let h = CURRENT.lock().ok().and_then(|g| g.as_ref().map(|p| p.hwnd));
    if let Some(h) = h {
        // SAFETY: our own window.
        let _ = unsafe { DestroyWindow(HWND(h as *mut _)) };
    }
}
