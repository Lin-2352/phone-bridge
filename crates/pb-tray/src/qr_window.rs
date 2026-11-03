//! Small Win32 window that shows the rotating pairing QR code with a
//! countdown, like an authenticator app. One window at a time. Closing it
//! (by the user) revokes the current code immediately.
#![allow(unsafe_code)]

use std::{
    cell::RefCell,
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};

use windows::{
    core::{w, PCWSTR},
    Win32::{
        Foundation::{HWND, LPARAM, LRESULT, WPARAM},
        Graphics::{
            Direct2D::{D2D1_ANTIALIAS_MODE_ALIASED, D2D1_ANTIALIAS_MODE_PER_PRIMITIVE},
            Gdi::{InvalidateRect, ValidateRect},
        },
        System::LibraryLoader::GetModuleHandleW,
        UI::WindowsAndMessaging::{
            CreateWindowExW, DefWindowProcW, DestroyWindow, KillTimer, RegisterClassW,
            SetForegroundWindow, SetTimer, ShowWindow, CW_USEDEFAULT, SW_SHOW, WINDOW_EX_STYLE,
            WM_CLOSE, WM_DESTROY, WM_ERASEBKGND, WM_PAINT, WM_TIMER, WNDCLASSW, WS_CAPTION,
            WS_OVERLAPPED, WS_SYSMENU,
        },
    },
};

use crate::{
    d2d::{color, Backdrop, Surface},
    fluent::{self, rectf, Align, Canvas, Font, RectF},
    qr::{self, Matrix},
};

const CLASS: PCWSTR = w!("PhoneBridgePairQr");
const QUIET: usize = 4;
const TEXT_H: i32 = 120;
/// Height of one text line under the code, logical px.
const LINE_H: f32 = 24.0;
const TIMER_ID: usize = 1;

struct Shown {
    hwnd: isize,
    matrix: Matrix,
    shown_at: Instant,
    refresh_in: Duration,
    closes_at: Instant,
}

static CURRENT: Mutex<Option<Shown>> = Mutex::new(None);
static ON_USER_CLOSE: OnceLock<Box<dyn Fn() + Send + Sync>> = OnceLock::new();

thread_local! {
    /// The window's Direct2D surface (COM objects stay on the UI thread),
    /// with the window it belongs to.
    static SURFACE: RefCell<Option<(isize, Surface)>> = const { RefCell::new(None) };
}

fn sys_scale() -> f32 {
    // SAFETY: plain query.
    (unsafe { windows::Win32::UI::HiDpi::GetDpiForSystem() }.max(96)) as f32 / 96.0
}

/// Called when the user closes the window (not when we close it).
pub fn set_on_user_close(f: Box<dyn Fn() + Send + Sync>) {
    let _ = ON_USER_CLOSE.set(f);
}

extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_PAINT => {
            paint(hwnd);
            LRESULT(0)
        }
        // We paint every pixel ourselves (Direct2D): skipping the
        // background erase is what removes the once-a-second flicker.
        WM_ERASEBKGND => LRESULT(1),
        WM_TIMER => {
            // SAFETY: repaint our own window once a second for the countdown.
            let _ = unsafe { InvalidateRect(Some(hwnd), None, false) };
            LRESULT(0)
        }
        // Esc (or Enter) closes it, like the X: the code is revoked.
        windows::Win32::UI::WindowsAndMessaging::WM_KEYDOWN
            if wparam.0 == 0x1B || wparam.0 == 0x0D =>
        {
            // SAFETY: a message to our own window.
            let _ = unsafe {
                windows::Win32::UI::WindowsAndMessaging::PostMessageW(
                    Some(hwnd),
                    WM_CLOSE,
                    WPARAM(0),
                    LPARAM(0),
                )
            };
            LRESULT(0)
        }
        WM_CLOSE => {
            if let Some(f) = ON_USER_CLOSE.get() {
                f();
            }
            // SAFETY: our own window.
            let _ = unsafe { DestroyWindow(hwnd) };
            LRESULT(0)
        }
        WM_DESTROY => {
            // SAFETY: timer created in `show` for this window.
            let _ = unsafe { KillTimer(Some(hwnd), TIMER_ID) };
            fluent::unregister_window(hwnd);
            // Taken out first, dropped outside the borrow (dropping the last
            // surface runs the device idle hook).
            let surface = SURFACE.with(|cell| {
                let mut slot = cell.borrow_mut();
                if slot.as_ref().is_some_and(|(h, _)| *h == hwnd.0 as isize) {
                    slot.take()
                } else {
                    None
                }
            });
            drop(surface);
            if let Ok(mut g) = CURRENT.lock() {
                if g.as_ref().is_some_and(|s| s.hwnd == hwnd.0 as isize) {
                    *g = None;
                }
            }
            LRESULT(0)
        }
        // SAFETY: default handling.
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

fn paint(hwnd: HWND) {
    SURFACE.with(|cell| {
        let mut slot = cell.borrow_mut();
        if slot.as_ref().map(|(h, _)| *h) != Some(hwnd.0 as isize) {
            *slot = Surface::new(hwnd, Backdrop::Opaque)
                .ok()
                .map(|s| (hwnd.0 as isize, s));
        }
        match slot.as_mut() {
            Some((_, surface)) => fluent::paint_with(surface, hwnd, draw),
            // Nothing can draw (no Direct2D at all): validate so WM_PAINT
            // does not repeat forever.
            // SAFETY: our own window.
            None => unsafe {
                let _ = ValidateRect(Some(hwnd), None);
            },
        }
    });
}

/// The three lines under the code: what to do, the countdowns, the rules.
fn lines(new_code_in: Duration, closes_in: Duration) -> [String; 3] {
    let secs = closes_in.as_secs();
    [
        "Scan with Relay on your phone (\u{201c}Scan QR\u{201d}).".to_owned(),
        format!(
            "New code in {} s \u{2022} pairing closes in {}:{:02}",
            new_code_in.as_secs() + 1,
            secs / 60,
            secs % 60
        ),
        "Each code works once. Closing this window cancels pairing.".to_owned(),
    ]
}

fn draw(c: &mut Canvas, client: RectF) {
    let Ok(guard) = CURRENT.lock() else { return };
    let Some(s) = guard.as_ref() else { return };
    // The window was sized with the system scale; lay out with the same.
    c.s = sys_scale();
    let width = client.right as i32;
    let text_h = (TEXT_H as f32 * c.s) as i32;
    let avail = width.min(client.bottom as i32 - text_h).max(1) as usize;
    let total = s.matrix.size + 2 * QUIET;
    let scale = (avail / total).max(1) as i32;
    let off_x = (width - scale * total as i32) / 2;
    let side = scale * total as i32;
    let px = |v: i32| v as f32;
    // SAFETY: drawing calls inside the surface's BeginDraw/EndDraw.
    unsafe {
        // Always dark on white with its quiet zone, whatever the theme:
        // phone scanners expect that. Aliased on whole pixels, so the
        // modules meet without seams.
        c.dc.SetAntialiasMode(D2D1_ANTIALIAS_MODE_ALIASED);
        if let Ok(white) = c.dc.CreateSolidColorBrush(&color(0xFFFFFF, 1.0), None) {
            c.dc.FillRectangle(&rectf(px(off_x), 0.0, px(off_x + side), px(side)), &white);
        }
        if let Ok(black) = c.dc.CreateSolidColorBrush(&color(0x000000, 1.0), None) {
            for y in 0..s.matrix.size {
                for x in 0..s.matrix.size {
                    if s.matrix.is_dark(x, y) {
                        let l = off_x + scale * (x + QUIET) as i32;
                        let t = scale * (y + QUIET) as i32;
                        let r = rectf(px(l), px(t), px(l + scale), px(t + scale));
                        c.dc.FillRectangle(&r, &black);
                    }
                }
            }
        }
        c.dc.SetAntialiasMode(D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
    }
    let now = Instant::now();
    let new_code_in = s.refresh_in.saturating_sub(now.duration_since(s.shown_at));
    let closes_in = s.closes_at.saturating_duration_since(now);
    let line_h = LINE_H * c.s;
    let mut top = side as f32 + 10.0 * c.s;
    for line in lines(new_code_in, closes_in) {
        let r = rectf(12.0 * c.s, top, client.right - 12.0 * c.s, top + line_h);
        fluent::text(c, Font::Body, &line, r, c.p.text, Align::Center);
        top += line_h;
    }
}

/// Show (or update with a fresh code) the pairing QR window.
pub fn show(code: &str, refresh_in: Duration, closes_in: Duration) -> Result<(), String> {
    let matrix = qr::encode(code).map_err(|e| format!("QR encode failed: {e:?}"))?;
    let now = Instant::now();
    let existing = CURRENT.lock().ok().and_then(|g| g.as_ref().map(|s| s.hwnd));
    let hwnd = match existing {
        Some(h) => HWND(h as *mut _),
        // SAFETY: class registration (idempotent) and window creation on the UI thread.
        None => unsafe {
            let hinstance = GetModuleHandleW(None).map_err(|e| e.to_string())?;
            let wc = WNDCLASSW {
                lpfnWndProc: Some(wndproc),
                hInstance: hinstance.into(),
                lpszClassName: CLASS,
                ..Default::default()
            };
            RegisterClassW(&wc);
            let h = CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                CLASS,
                w!("Relay: pair a new device"),
                WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                (460.0 * sys_scale()) as i32,
                (620.0 * sys_scale()) as i32,
                None,
                None,
                Some(hinstance.into()),
                None,
            )
            .map_err(|e| e.to_string())?;
            SetTimer(Some(h), TIMER_ID, 1000, None);
            fluent::frame(h, false);
            fluent::register_window(h);
            let _ = ShowWindow(h, SW_SHOW);
            h
        },
    };
    if let Ok(mut g) = CURRENT.lock() {
        *g = Some(Shown {
            hwnd: hwnd.0 as isize,
            matrix,
            shown_at: now,
            refresh_in,
            closes_at: now + closes_in,
        });
    }
    // SAFETY: repaint our own window, and raise it only the first time: a
    // new code every 30 s must not pull focus from what is being typed.
    unsafe {
        let _ = InvalidateRect(Some(hwnd), None, true);
        if existing.is_none() {
            let _ = SetForegroundWindow(hwnd);
        }
    }
    Ok(())
}

/// Close the window without revoking (pairing moved on, or expired).
pub fn close() {
    let h = CURRENT.lock().ok().and_then(|g| g.as_ref().map(|s| s.hwnd));
    if let Some(h) = h {
        // SAFETY: our own window; DestroyWindow does not send WM_CLOSE, so the
        // user-close callback is not triggered.
        let _ = unsafe { DestroyWindow(HWND(h as *mut _)) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::d2d::test_window;

    /// Each line is drawn on one row and cut with "…" if too wide, so all
    /// three (with the widest countdown) must fit the client area.
    #[cfg(windows)]
    #[test]
    fn every_line_fits_the_window_at_100_percent() {
        let hwnd = test_window();
        let mut surface = Surface::new(hwnd, Backdrop::Opaque).expect("surface");
        // 460 px window minus the fixed frame (about 8 px a side) and the
        // 12 px margins.
        let room = 460.0 - 2.0 * 8.0 - 2.0 * 12.0;
        let mut widths = Vec::new();
        surface
            .draw(|dc| {
                let c = Canvas {
                    dc,
                    s: 1.0,
                    p: fluent::palette(),
                };
                for line in lines(Duration::from_secs(59), Duration::from_secs(599)) {
                    widths.push((fluent::text_width(&c, Font::Body, &line), line));
                }
            })
            .expect("draw");
        for (w, line) in widths {
            assert!(w > 0.0 && w <= room, "{line:?} is {w} px, room {room}");
        }
        drop(surface);
        // SAFETY: our own test window, made above on this thread.
        unsafe {
            let _ = DestroyWindow(hwnd);
        }
    }

    #[test]
    fn countdown_line_counts_up_to_the_next_second() {
        let [_, l, _] = lines(Duration::from_millis(29_400), Duration::from_secs(125));
        assert_eq!(l, "New code in 30 s \u{2022} pairing closes in 2:05");
    }
}
