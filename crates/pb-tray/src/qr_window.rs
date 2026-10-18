//! Small Win32 window that shows the rotating pairing QR code with a
//! countdown, like an authenticator app. One window at a time. Closing it
//! (by the user) revokes the current code immediately.
#![allow(unsafe_code)]

use std::{
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};

use windows::{
    core::{w, PCWSTR},
    Win32::{
        Foundation::{COLORREF, HWND, LPARAM, LRESULT, RECT, WPARAM},
        Graphics::Gdi::{
            BeginPaint, BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, CreateSolidBrush,
            DeleteDC, DeleteObject, DrawTextW, EndPaint, FillRect, InvalidateRect, SelectObject,
            SetBkMode, DT_CENTER, DT_WORDBREAK, HGDIOBJ, PAINTSTRUCT, SRCCOPY, TRANSPARENT,
        },
        System::LibraryLoader::GetModuleHandleW,
        UI::WindowsAndMessaging::{
            CreateWindowExW, DefWindowProcW, DestroyWindow, GetClientRect, KillTimer,
            RegisterClassW, SetForegroundWindow, SetTimer, ShowWindow, CW_USEDEFAULT, SW_SHOW,
            WINDOW_EX_STYLE, WM_CLOSE, WM_DESTROY, WM_ERASEBKGND, WM_PAINT, WM_TIMER, WNDCLASSW,
            WS_CAPTION, WS_OVERLAPPED, WS_SYSMENU,
        },
    },
};

use crate::qr::{self, Matrix};

const CLASS: PCWSTR = w!("PhoneBridgePairQr");
const QUIET: usize = 4;
const TEXT_H: i32 = 120;
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
        // We paint every pixel ourselves (double-buffered): skipping the
        // background erase is what removes the once-a-second flicker.
        WM_ERASEBKGND => LRESULT(1),
        WM_TIMER => {
            // SAFETY: repaint our own window once a second for the countdown.
            let _ = unsafe { InvalidateRect(Some(hwnd), None, false) };
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
    let Ok(guard) = CURRENT.lock() else { return };
    let mut ps = PAINTSTRUCT::default();
    // SAFETY: standard WM_PAINT sequence on the owning thread; brushes are
    // created and deleted here.
    unsafe {
        let screen = BeginPaint(hwnd, &mut ps);
        let mut client = RECT::default();
        let _ = GetClientRect(hwnd, &mut client);
        let (w, h) = (client.right - client.left, client.bottom - client.top);
        let hdc = CreateCompatibleDC(Some(screen));
        let bmp = CreateCompatibleBitmap(screen, w, h);
        let old = SelectObject(hdc, HGDIOBJ(bmp.0));
        SetBkMode(hdc, TRANSPARENT);
        let ui_font = windows::Win32::Graphics::Gdi::CreateFontW(
            -((15.0 * sys_scale()) as i32),
            0,
            0,
            0,
            400,
            0,
            0,
            0,
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
            0,
            w!("Segoe UI Variable Text"),
        );
        let old_font = SelectObject(hdc, HGDIOBJ(ui_font.0));
        let white = CreateSolidBrush(COLORREF(0x00FF_FFFF));
        let black = CreateSolidBrush(COLORREF(0));
        FillRect(hdc, &client, white);
        if let Some(s) = guard.as_ref() {
            let width = client.right - client.left;
            let text_h = (TEXT_H as f32 * sys_scale()) as i32;
            let avail = width.min(client.bottom - client.top - text_h).max(1) as usize;
            let total = s.matrix.size + 2 * QUIET;
            let scale = (avail / total).max(1) as i32;
            let off_x = (width - scale * total as i32) / 2;
            for y in 0..s.matrix.size {
                for x in 0..s.matrix.size {
                    if s.matrix.is_dark(x, y) {
                        let l = off_x + scale * (x + QUIET) as i32;
                        let t = scale * (y + QUIET) as i32;
                        FillRect(
                            hdc,
                            &RECT {
                                left: l,
                                top: t,
                                right: l + scale,
                                bottom: t + scale,
                            },
                            black,
                        );
                    }
                }
            }
            let now = Instant::now();
            let left = s.refresh_in.saturating_sub(now.duration_since(s.shown_at));
            let bar_top = scale * total as i32;
            let window_left = s.closes_at.saturating_duration_since(now).as_secs();
            let mut text: Vec<u16> = format!(
                "Scan with Phone Bridge on your phone (\u{201c}Scan QR\u{201d}).\nNew code in {} s \u{2022} pairing closes in {}:{:02}\nEach code works once. Closing this window cancels pairing.",
                left.as_secs() + 1,
                window_left / 60,
                window_left % 60
            )
            .encode_utf16()
            .collect();
            let mut tr = RECT {
                left: 12,
                top: bar_top + 10,
                right: client.right - 12,
                bottom: client.bottom - 6,
            };
            DrawTextW(hdc, &mut text, &mut tr, DT_CENTER | DT_WORDBREAK);
        }
        for b in [white, black] {
            let _ = DeleteObject(HGDIOBJ(b.0));
        }
        let _ = BitBlt(screen, 0, 0, w, h, Some(hdc), 0, 0, SRCCOPY);
        SelectObject(hdc, old_font);
        let _ = DeleteObject(HGDIOBJ(ui_font.0));
        SelectObject(hdc, old);
        let _ = DeleteObject(HGDIOBJ(bmp.0));
        let _ = DeleteDC(hdc);
        let _ = EndPaint(hwnd, &ps);
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
                w!("Phone Bridge: pair a new device"),
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
    // SAFETY: repaint and raise our own window.
    unsafe {
        let _ = InvalidateRect(Some(hwnd), None, true);
        let _ = SetForegroundWindow(hwnd);
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
