//! "Phone camera": one window that answers "does the camera show up?" It
//! lists what has to be true (phone, install, Windows camera access, the
//! stream, Windows' device list), shows a live preview read the way an app
//! reads it, and holds the few buttons that matter (start or stop, switch
//! lens, Windows' camera privacy page, remove).
#![allow(unsafe_code)]

use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};

use windows::{
    core::{w, PCWSTR},
    Win32::{
        Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM},
        Graphics::Gdi::{
            InvalidateRect, StretchDIBits, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS,
            DT_CENTER, DT_END_ELLIPSIS, DT_LEFT, DT_SINGLELINE, DT_VCENTER, SRCCOPY,
        },
        System::{
            Com::{CoInitializeEx, CoUninitialize, COINIT_MULTITHREADED},
            LibraryLoader::GetModuleHandleW,
        },
        UI::{
            Shell::ShellExecuteW,
            WindowsAndMessaging::{
                AdjustWindowRectEx, CreateWindowExW, DefWindowProcW, DestroyWindow,
                GetSystemMetrics, LoadCursorW, PostMessageW, RegisterClassW, SetForegroundWindow,
                SetWindowPos, ShowWindow, IDC_ARROW, SM_CXSCREEN, SM_CYSCREEN, SWP_NOZORDER,
                SW_RESTORE, SW_SHOW, SW_SHOWNORMAL, WM_APP, WM_CLOSE, WM_DESTROY, WM_ERASEBKGND,
                WM_LBUTTONUP, WM_MOUSEMOVE, WM_PAINT, WNDCLASSW, WS_CAPTION, WS_EX_APPWINDOW,
                WS_MINIMIZEBOX, WS_OVERLAPPED, WS_SYSMENU,
            },
        },
    },
};

use crate::{
    camera_preview::Preview,
    camera_status::{rows, Inputs, Mark, Row},
    fluent::{self, glyph, px},
};

const CLASS: PCWSTR = w!("PhoneBridgeCamera");
const W: i32 = 520;
const H: i32 = 676;
const WM_STATUS: u32 = WM_APP + 1;
const ROW_H: i32 = 40;

/// What the buttons ask for; the tray turns them into its own commands.
pub enum Action {
    Toggle,
    Lens,
    Remove,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Button {
    Toggle,
    Lens,
    Privacy,
    Remove,
}

struct State {
    hwnd: isize,
    rows: Vec<Row>,
    streaming: bool,
    preview: Option<Preview>,
    hover: Option<Button>,
    on_action: Box<dyn Fn(Action) + Send>,
    collector_stop: Arc<AtomicBool>,
}

static CURRENT: Mutex<Option<State>> = Mutex::new(None);

pub fn close() {
    let h = CURRENT.lock().ok().and_then(|g| g.as_ref().map(|s| s.hwnd));
    if let Some(h) = h {
        // SAFETY: posting to our own window.
        let _ = unsafe { PostMessageW(Some(HWND(h as *mut _)), WM_CLOSE, WPARAM(0), LPARAM(0)) };
    }
}

fn post_repaint(hwnd: isize) {
    // SAFETY: posting to our own window from any thread.
    let _ = unsafe { PostMessageW(Some(HWND(hwnd as *mut _)), WM_STATUS, WPARAM(0), LPARAM(0)) };
}

struct Layout {
    rows: RECT,
    preview: RECT,
    buttons: [RECT; 4],
}

fn layout(s: f32) -> Layout {
    let p = |v: i32| px(s, v);
    let rows_top = 64;
    let rows_bottom = rows_top + 6 * ROW_H;
    let pw = W - 32;
    let ph = pw * 9 / 16;
    let bw = (W - 32 - 3 * 8) / 4;
    Layout {
        rows: RECT {
            left: p(16),
            top: p(rows_top),
            right: p(W - 16),
            bottom: p(rows_bottom),
        },
        preview: RECT {
            left: p(16),
            top: p(rows_bottom + 8),
            right: p(16 + pw),
            bottom: p(rows_bottom + 8 + ph),
        },
        buttons: std::array::from_fn(|i| {
            let l = 16 + i as i32 * (bw + 8);
            RECT {
                left: p(l),
                top: p(rows_bottom + 8 + ph + 16),
                right: p(l + bw),
                bottom: p(rows_bottom + 8 + ph + 16 + 36),
            }
        }),
    }
}

const BUTTONS: [Button; 4] = [
    Button::Toggle,
    Button::Lens,
    Button::Privacy,
    Button::Remove,
];

fn button_label(b: Button, streaming: bool) -> &'static str {
    match (b, streaming) {
        (Button::Toggle, false) => "Start camera",
        (Button::Toggle, true) => "Stop camera",
        (Button::Lens, _) => "Switch lens",
        (Button::Privacy, _) => "Camera privacy",
        (Button::Remove, _) => "Remove",
    }
}

fn mark_glyph(m: Mark, idle: u32) -> (char, u32) {
    // COLORREF is 0x00BBGGRR.
    match m {
        Mark::Ok => ('\u{E73E}', 0x0050_B000),
        Mark::Bad => (glyph::CLOSE, 0x0023_11E8),
        Mark::Wait => ('\u{E916}', 0x0000_B9FF),
        Mark::Idle => ('\u{E739}', idle),
    }
}

fn paint(hwnd: HWND) {
    let Ok(guard) = CURRENT.lock() else { return };
    let Some(st) = guard.as_ref() else { return };
    let s = fluent::scale(hwnd);
    let p = fluent::palette();
    let f = fluent::Fonts::new(s);
    let l = layout(s);
    let snapshot = st.preview.as_ref().map(Preview::snapshot);
    fluent::paint(hwnd, |hdc, _c| {
        fluent::text(
            hdc,
            f.title,
            "Phone camera",
            RECT {
                left: px(s, 16),
                top: px(s, 16),
                right: px(s, W - 16),
                bottom: px(s, 48),
            },
            p.text,
            DT_LEFT | DT_SINGLELINE | DT_VCENTER,
        );
        let verdict = if st.rows.is_empty() {
            ""
        } else if crate::camera_status::all_ok(&st.rows) {
            "Working: apps can use Phone Bridge Camera"
        } else {
            "Not working yet: see the marks below"
        };
        fluent::text(
            hdc,
            f.small,
            verdict,
            RECT {
                left: px(s, 16),
                top: px(s, 40),
                right: px(s, W - 16),
                bottom: px(s, 62),
            },
            p.text2,
            DT_LEFT | DT_SINGLELINE | DT_VCENTER,
        );
        for (i, r) in st.rows.iter().enumerate() {
            let top = l.rows.top + i as i32 * px(s, ROW_H);
            let (g, color) = mark_glyph(r.mark, p.text2);
            fluent::icon(
                hdc,
                f.icon,
                g,
                RECT {
                    left: l.rows.left,
                    top,
                    right: l.rows.left + px(s, 28),
                    bottom: top + px(s, ROW_H),
                },
                color,
            );
            fluent::text(
                hdc,
                f.body,
                r.label,
                RECT {
                    left: l.rows.left + px(s, 34),
                    top,
                    right: l.rows.left + px(s, 200),
                    bottom: top + px(s, ROW_H),
                },
                p.text,
                DT_LEFT | DT_SINGLELINE | DT_VCENTER,
            );
            fluent::text(
                hdc,
                f.small,
                &r.detail,
                RECT {
                    left: l.rows.left + px(s, 204),
                    top,
                    right: l.rows.right,
                    bottom: top + px(s, ROW_H),
                },
                if r.mark == Mark::Bad {
                    0x0023_11E8
                } else {
                    p.text2
                },
                DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS,
            );
        }
        // Preview: what an app gets (or a hint why not).
        fluent::round_rect(hdc, &l.preview, p.card, Some(p.stroke), px(s, 8), false);
        match snapshot
            .as_ref()
            .filter(|x| x.width > 0 && !x.bgra.is_empty())
        {
            Some(x) => {
                let bmi = BITMAPINFO {
                    bmiHeader: BITMAPINFOHEADER {
                        biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                        biWidth: x.width as i32,
                        biHeight: -(x.height as i32), // top-down
                        biPlanes: 1,
                        biBitCount: 32,
                        biCompression: BI_RGB.0,
                        ..Default::default()
                    },
                    ..Default::default()
                };
                // SAFETY: the buffer holds width x height BGRA pixels.
                unsafe {
                    StretchDIBits(
                        hdc,
                        l.preview.left + px(s, 2),
                        l.preview.top + px(s, 2),
                        l.preview.right - l.preview.left - px(s, 4),
                        l.preview.bottom - l.preview.top - px(s, 4),
                        0,
                        0,
                        x.width as i32,
                        x.height as i32,
                        Some(x.bgra.as_ptr().cast()),
                        &bmi,
                        DIB_RGB_COLORS,
                        SRCCOPY,
                    );
                }
            }
            None => fluent::text(
                hdc,
                f.small,
                if st.streaming {
                    "Opening the camera like an app does\u{2026}"
                } else {
                    "Start the camera to see what apps get"
                },
                l.preview,
                p.text2,
                DT_CENTER | DT_SINGLELINE | DT_VCENTER,
            ),
        }
        for (r, b) in l.buttons.iter().zip(BUTTONS) {
            let accent = b == Button::Toggle;
            if st.hover == Some(b) && !accent {
                fluent::round_rect(hdc, r, p.hover, Some(p.stroke), px(s, 6), false);
                fluent::text(
                    hdc,
                    f.small,
                    button_label(b, st.streaming),
                    *r,
                    p.text,
                    DT_CENTER | DT_SINGLELINE | DT_VCENTER,
                );
            } else {
                fluent::button(hdc, &f, &p, *r, button_label(b, st.streaming), accent, s);
            }
        }
    });
}

fn hit(hwnd: HWND, x: i32, y: i32) -> Option<Button> {
    let l = layout(fluent::scale(hwnd));
    l.buttons
        .iter()
        .zip(BUTTONS)
        .find(|(r, _)| fluent::inside(r, x, y))
        .map(|(_, b)| b)
}

fn open_privacy_settings() {
    // SAFETY: opens Windows' own settings page for the user to change.
    unsafe {
        ShellExecuteW(
            None,
            w!("open"),
            w!("ms-settings:privacy-webcam"),
            None,
            None,
            SW_SHOWNORMAL,
        );
    }
}

extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    // SAFETY: standard window procedure for our own window.
    unsafe {
        match msg {
            WM_ERASEBKGND => LRESULT(1),
            WM_PAINT => {
                paint(hwnd);
                LRESULT(0)
            }
            WM_STATUS => {
                let _ = InvalidateRect(Some(hwnd), None, false);
                LRESULT(0)
            }
            WM_MOUSEMOVE => {
                let (x, y) = (
                    (lparam.0 & 0xffff) as i16 as i32,
                    ((lparam.0 >> 16) & 0xffff) as i16 as i32,
                );
                let h = hit(hwnd, x, y);
                if let Ok(mut g) = CURRENT.lock() {
                    if let Some(st) = g.as_mut() {
                        if st.hover != h {
                            st.hover = h;
                            let _ = InvalidateRect(Some(hwnd), None, false);
                        }
                    }
                }
                fluent::set_cursor(h.is_some());
                LRESULT(0)
            }
            WM_LBUTTONUP => {
                let (x, y) = (
                    (lparam.0 & 0xffff) as i16 as i32,
                    ((lparam.0 >> 16) & 0xffff) as i16 as i32,
                );
                if let Some(b) = hit(hwnd, x, y) {
                    match b {
                        Button::Privacy => open_privacy_settings(),
                        Button::Toggle | Button::Lens | Button::Remove => {
                            if let Ok(g) = CURRENT.lock() {
                                if let Some(st) = g.as_ref() {
                                    (st.on_action)(match b {
                                        Button::Toggle => Action::Toggle,
                                        Button::Lens => Action::Lens,
                                        _ => Action::Remove,
                                    });
                                }
                            }
                        }
                    }
                }
                LRESULT(0)
            }
            WM_CLOSE => {
                let _ = DestroyWindow(hwnd);
                LRESULT(0)
            }
            WM_DESTROY => {
                let st = CURRENT.lock().ok().and_then(|mut g| g.take());
                if let Some(mut st) = st {
                    st.collector_stop.store(true, Ordering::Release);
                    if let Some(mut p) = st.preview.take() {
                        p.stop();
                    }
                }
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

/// Open (or raise) the window. `phone` names the paired phone, if any.
pub fn open(
    phone: Arc<dyn Fn() -> Option<String> + Send + Sync>,
    on_action: Box<dyn Fn(Action) + Send>,
) {
    if let Some(h) = CURRENT.lock().ok().and_then(|g| g.as_ref().map(|s| s.hwnd)) {
        // SAFETY: our own window.
        unsafe {
            let _ = ShowWindow(HWND(h as *mut _), SW_RESTORE);
            let _ = SetForegroundWindow(HWND(h as *mut _));
        }
        return;
    }
    // SAFETY: window creation on the UI thread.
    unsafe {
        let Ok(hinstance) = GetModuleHandleW(None) else {
            return;
        };
        let wc = WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: hinstance.into(),
            lpszClassName: CLASS,
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            ..Default::default()
        };
        RegisterClassW(&wc);
        let style = WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX;
        let Ok(hwnd) = CreateWindowExW(
            WS_EX_APPWINDOW,
            CLASS,
            w!("Phone Bridge camera"),
            style,
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
        let stop = Arc::new(AtomicBool::new(false));
        if let Ok(mut g) = CURRENT.lock() {
            *g = Some(State {
                hwnd: hwnd.0 as isize,
                rows: Vec::new(),
                streaming: false,
                preview: None,
                hover: None,
                on_action,
                collector_stop: stop.clone(),
            });
        }
        let s = fluent::scale(hwnd);
        let mut r = RECT {
            left: 0,
            top: 0,
            right: px(s, W),
            bottom: px(s, H),
        };
        let _ = AdjustWindowRectEx(&mut r, style, false, WS_EX_APPWINDOW);
        let (w, h) = (r.right - r.left, r.bottom - r.top);
        let (sw, sh) = (GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN));
        let _ = SetWindowPos(
            hwnd,
            None,
            (sw - w) / 2,
            ((sh - h) / 2).max(0),
            w,
            h,
            SWP_NOZORDER,
        );
        fluent::frame(hwnd, false);
        let _ = ShowWindow(hwnd, SW_SHOW);
        let raw = hwnd.0 as isize;
        std::thread::spawn(move || collect(raw, phone, stop));
    }
}

/// Every 1.5 s: gather the facts, start or stop the preview, repaint.
fn collect(
    hwnd: isize,
    phone: Arc<dyn Fn() -> Option<String> + Send + Sync>,
    stop: Arc<AtomicBool>,
) {
    // SAFETY: COM and Media Foundation on this worker thread (the device list).
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let _ = windows::Win32::Media::MediaFoundation::MFStartup(
            windows::Win32::Media::MediaFoundation::MF_VERSION,
            windows::Win32::Media::MediaFoundation::MFSTARTUP_FULL,
        );
    }
    while !stop.load(Ordering::Acquire) {
        let streaming = crate::camera::is_running();
        let listed = streaming.then(|| {
            crate::camera_device::list_camera_names()
                .iter()
                .any(|n| crate::camera_device::is_our_camera(n))
        });
        // The preview reads the camera only while it streams and is listed.
        let want_preview = streaming && listed == Some(true);
        let preview_fps = {
            let Ok(mut g) = CURRENT.lock() else { break };
            let Some(st) = g.as_mut() else { break };
            st.streaming = streaming;
            if want_preview && st.preview.is_none() {
                st.preview = Some(Preview::start(Box::new(move || post_repaint(hwnd))));
            } else if !want_preview {
                if let Some(mut p) = st.preview.take() {
                    p.stop();
                }
            }
            st.preview.as_ref().and_then(|p| p.snapshot().fps)
        };
        let installed = crate::camera_register::is_registered();
        let inputs = Inputs {
            phone: phone(),
            installed,
            update_ready: installed && crate::camera_register::needs_install(),
            access: crate::camera_register::camera_access(),
            streaming,
            fps: crate::camera::fps(),
            dark: crate::camera::picture_dark(),
            listed,
            preview_fps,
        };
        if let Ok(mut g) = CURRENT.lock() {
            if let Some(st) = g.as_mut() {
                st.rows = rows(&inputs);
            }
        }
        post_repaint(hwnd);
        for _ in 0..15 {
            if stop.load(Ordering::Acquire) {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
    // SAFETY: undoing the start-up above.
    unsafe {
        let _ = windows::Win32::Media::MediaFoundation::MFShutdown();
        CoUninitialize();
    }
}
