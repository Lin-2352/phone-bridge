//! Fluent-style flyout opened by left-clicking the tray icon: link status,
//! a drop zone for files, and three tiles (Share clipboard, Send files, Pair
//! device), plus settings and close. It stays open when another window is
//! clicked, so files can be dragged in from Explorer; it closes with Esc, the
//! close button, the tray icon, or after a drop.
#![allow(unsafe_code)]

use std::sync::Mutex;

use windows::{
    core::{w, PCWSTR},
    Win32::{
        Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
        Graphics::Gdi::{
            InvalidateRect, ScreenToClient, DT_CENTER, DT_END_ELLIPSIS, DT_LEFT, DT_SINGLELINE,
            DT_VCENTER, DT_WORDBREAK,
        },
        System::LibraryLoader::GetModuleHandleW,
        UI::{
            Input::KeyboardAndMouse::{TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT, VK_ESCAPE},
            Shell::{DragAcceptFiles, DragFinish, DragQueryFileW, HDROP},
            WindowsAndMessaging::{
                CreateWindowExW, DefWindowProcW, DestroyWindow, GetCursorPos, LoadCursorW,
                RegisterClassW, SetForegroundWindow, SetWindowPos, ShowWindow, HTCLIENT,
                HWND_TOPMOST, IDC_ARROW, SWP_NOACTIVATE, SW_SHOW, WM_DESTROY, WM_DROPFILES,
                WM_ERASEBKGND, WM_KEYDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_PAINT, WM_SETCURSOR,
                WNDCLASSW, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
            },
        },
    },
};

use crate::fluent::{self, glyph, px};

const WM_MOUSELEAVE: u32 = 0x02A3;
const CLASS: PCWSTR = w!("PhoneBridgeFlyout");
/// Logical (96-dpi) size of the flyout.
const W: i32 = 360;
const H: i32 = 312;

/// What the flyout shows; filled in by the UI before opening.
#[derive(Clone, Default)]
pub struct Model {
    pub peer: Option<String>,
    pub clipboard_on: bool,
    pub last: String,
}

/// User actions the flyout reports back to the UI thread.
pub enum Action {
    ToggleClipboard,
    SendFiles,
    Pair,
    Settings,
    Dropped(Vec<std::path::PathBuf>),
}

struct State {
    hwnd: isize,
    model: Model,
    hover: Option<Hit>,
    tracking: bool,
    on_action: Box<dyn Fn(Action) + Send>,
}

#[derive(Clone, Copy, PartialEq)]
enum Hit {
    Drop,
    Clip,
    Send,
    Pair,
    Settings,
    Close,
}

static CURRENT: Mutex<Option<State>> = Mutex::new(None);

/// Layout rectangles in physical pixels for a given DPI scale.
struct Layout {
    badge: RECT,
    settings: RECT,
    close: RECT,
    drop: RECT,
    tiles: [RECT; 3],
    footer: RECT,
}

fn layout(s: f32) -> Layout {
    let p = |v: i32| px(s, v);
    let sq = |l: i32, t: i32, n: i32| RECT {
        left: p(l),
        top: p(t),
        right: p(l + n),
        bottom: p(t + n),
    };
    let tw = (p(W) - 2 * p(16) - 2 * p(8)) / 3;
    Layout {
        badge: sq(16, 16, 40),
        settings: sq(W - 16 - 32 - 36, 20, 32),
        close: sq(W - 16 - 32, 20, 32),
        drop: RECT {
            left: p(16),
            top: p(72),
            right: p(W - 16),
            bottom: p(164),
        },
        tiles: std::array::from_fn(|i| {
            let l = p(16) + i as i32 * (tw + p(8));
            RECT {
                left: l,
                top: p(176),
                right: l + tw,
                bottom: p(256),
            }
        }),
        footer: RECT {
            left: p(16),
            top: p(268),
            right: p(W - 16),
            bottom: p(296),
        },
    }
}

fn paint(hwnd: HWND) {
    let Ok(guard) = CURRENT.lock() else { return };
    let Some(st) = guard.as_ref() else { return };
    let s = fluent::scale(hwnd);
    let p = fluent::palette();
    let f = fluent::Fonts::new(s);
    let l = layout(s);
    let hv = st.hover;
    fluent::paint(hwnd, |hdc, c| {
        // Header: device badge, name, status, settings and close.
        fluent::round_rect(hdc, &l.badge, p.card, Some(p.stroke), px(s, 10), false);
        fluent::icon(hdc, f.icon_big, glyph::PHONE, l.badge, p.accent);
        let (title, sub) = match &st.model.peer {
            Some(n) => (n.clone(), "Linked".to_owned()),
            None => (
                "No device paired".to_owned(),
                "Pair a phone to start".to_owned(),
            ),
        };
        let tx = l.badge.right + px(s, 12);
        fluent::text(
            hdc,
            f.title,
            &title,
            RECT {
                left: tx,
                top: px(s, 16),
                right: l.settings.left - px(s, 4),
                bottom: px(s, 36),
            },
            p.text,
            DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS,
        );
        fluent::text(
            hdc,
            f.small,
            &sub,
            RECT {
                left: tx,
                top: px(s, 36),
                right: l.settings.left,
                bottom: px(s, 56),
            },
            p.text2,
            DT_LEFT | DT_SINGLELINE | DT_VCENTER,
        );
        for (r, g, h) in [
            (l.settings, glyph::SETTINGS, Hit::Settings),
            (l.close, glyph::CLOSE, Hit::Close),
        ] {
            if hv == Some(h) {
                fluent::round_rect(hdc, &r, p.hover, None, px(s, 6), false);
            }
            fluent::icon(hdc, f.icon, g, r, p.text2);
        }

        // Drop zone.
        let drop_fill = if hv == Some(Hit::Drop) {
            p.hover
        } else {
            p.card
        };
        fluent::round_rect(hdc, &l.drop, drop_fill, Some(p.text2), px(s, 10), true);
        let mut ic = l.drop;
        ic.bottom = ic.top + (ic.bottom - ic.top) / 2 + px(s, 4);
        ic.top += px(s, 10);
        fluent::icon(hdc, f.icon_big, glyph::UPLOAD, ic, p.text2);
        let label = match &st.model.peer {
            Some(n) => format!("Drop files here to send to {n}"),
            None => "Pair a device to send files".to_owned(),
        };
        let mut lr = l.drop;
        lr.top = ic.bottom;
        lr.bottom -= px(s, 10);
        fluent::text(
            hdc,
            f.small,
            &label,
            lr,
            p.text2,
            DT_CENTER | DT_SINGLELINE | DT_VCENTER,
        );

        // Tiles.
        let tiles = [
            (glyph::CLIPBOARD, "Share clipboard", Hit::Clip),
            (glyph::SEND, "Send files", Hit::Send),
            (glyph::ADD, "Pair device", Hit::Pair),
        ];
        for (r, (g, label, h)) in l.tiles.iter().zip(tiles) {
            let on = h == Hit::Clip && st.model.clipboard_on;
            let (fill, fg) = if on {
                (p.accent, p.on_accent)
            } else if hv == Some(h) {
                (p.hover, p.text)
            } else {
                (p.card, p.text)
            };
            fluent::round_rect(
                hdc,
                r,
                fill,
                if on { None } else { Some(p.stroke) },
                px(s, 8),
                false,
            );
            let ir = RECT {
                left: r.left + px(s, 12),
                top: r.top + px(s, 12),
                right: r.left + px(s, 36),
                bottom: r.top + px(s, 36),
            };
            fluent::icon(hdc, f.icon, g, ir, fg);
            let tr = RECT {
                left: r.left + px(s, 12),
                top: r.top + px(s, 42),
                right: r.right - px(s, 6),
                bottom: r.bottom - px(s, 4),
            };
            fluent::text(hdc, f.small, label, tr, fg, DT_LEFT | DT_WORDBREAK);
        }

        // Footer: last activity.
        if !st.model.last.is_empty() {
            fluent::text(
                hdc,
                f.small,
                &st.model.last,
                l.footer,
                p.text2,
                DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS,
            );
        }
        let _ = c;
    });
}

fn hit(hwnd: HWND, x: i32, y: i32) -> Option<Hit> {
    let l = layout(fluent::scale(hwnd));
    [
        (l.settings, Hit::Settings),
        (l.close, Hit::Close),
        (l.drop, Hit::Drop),
        (l.tiles[0], Hit::Clip),
        (l.tiles[1], Hit::Send),
        (l.tiles[2], Hit::Pair),
    ]
    .into_iter()
    .find(|(r, _)| fluent::inside(r, x, y))
    .map(|(_, h)| h)
}

fn act(a: Action) {
    if let Ok(g) = CURRENT.lock() {
        if let Some(st) = g.as_ref() {
            (st.on_action)(a);
        }
    }
}

fn repaint(hwnd: HWND) {
    // SAFETY: repaint our own window.
    let _ = unsafe { InvalidateRect(Some(hwnd), None, false) };
}

extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let xy = || {
        (
            (lparam.0 & 0xffff) as i16 as i32,
            ((lparam.0 >> 16) & 0xffff) as i16 as i32,
        )
    };
    match msg {
        WM_PAINT => {
            paint(hwnd);
            LRESULT(0)
        }
        WM_ERASEBKGND => LRESULT(1),
        WM_SETCURSOR if (lparam.0 & 0xffff) as u32 == HTCLIENT => {
            let mut pt = POINT::default();
            // SAFETY: cursor position mapped into our client area.
            unsafe {
                let _ = GetCursorPos(&mut pt);
                let _ = ScreenToClient(hwnd, &mut pt);
            }
            fluent::set_cursor(hit(hwnd, pt.x, pt.y).is_some());
            LRESULT(1)
        }
        WM_MOUSEMOVE => {
            let (x, y) = xy();
            let h = hit(hwnd, x, y);
            let mut changed = false;
            if let Ok(mut g) = CURRENT.lock() {
                if let Some(st) = g.as_mut() {
                    changed = st.hover != h;
                    st.hover = h;
                    if !st.tracking {
                        st.tracking = true;
                        let mut t = TRACKMOUSEEVENT {
                            cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                            dwFlags: TME_LEAVE,
                            hwndTrack: hwnd,
                            dwHoverTime: 0,
                        };
                        // SAFETY: valid struct for our window.
                        let _ = unsafe { TrackMouseEvent(&mut t) };
                    }
                }
            }
            if changed {
                repaint(hwnd);
            }
            LRESULT(0)
        }
        WM_MOUSELEAVE => {
            if let Ok(mut g) = CURRENT.lock() {
                if let Some(st) = g.as_mut() {
                    st.hover = None;
                    st.tracking = false;
                }
            }
            repaint(hwnd);
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            let (x, y) = xy();
            match hit(hwnd, x, y) {
                Some(Hit::Clip) => {
                    act(Action::ToggleClipboard);
                    if let Ok(mut g) = CURRENT.lock() {
                        if let Some(st) = g.as_mut() {
                            st.model.clipboard_on = !st.model.clipboard_on;
                        }
                    }
                    repaint(hwnd);
                }
                Some(Hit::Send | Hit::Drop) => {
                    act(Action::SendFiles);
                    close();
                }
                Some(Hit::Pair) => {
                    act(Action::Pair);
                    close();
                }
                Some(Hit::Settings) => {
                    act(Action::Settings);
                    close();
                }
                Some(Hit::Close) => close(),
                None => {}
            }
            LRESULT(0)
        }
        WM_KEYDOWN if wparam.0 == VK_ESCAPE.0 as usize => {
            close();
            LRESULT(0)
        }
        WM_DROPFILES => {
            let drop = HDROP(wparam.0 as *mut _);
            let mut files = Vec::new();
            // SAFETY: the HDROP comes from the shell for this message; we query
            // each name into a sized buffer and release it with DragFinish.
            unsafe {
                let n = DragQueryFileW(drop, u32::MAX, None);
                for i in 0..n {
                    let len = DragQueryFileW(drop, i, None) as usize;
                    let mut buf = vec![0u16; len + 1];
                    DragQueryFileW(drop, i, Some(&mut buf));
                    files.push(std::path::PathBuf::from(String::from_utf16_lossy(
                        &buf[..len],
                    )));
                }
                DragFinish(drop);
            }
            if !files.is_empty() {
                act(Action::Dropped(files));
            }
            close();
            LRESULT(0)
        }
        WM_DESTROY => {
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

/// Open the flyout near the cursor (the tray icon), or close it if open.
pub fn toggle(model: Model, on_action: Box<dyn Fn(Action) + Send>) {
    if CURRENT.lock().map(|g| g.is_some()).unwrap_or(false) {
        close();
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
            WS_EX_TOOLWINDOW | WS_EX_TOPMOST,
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
            *g = Some(State {
                hwnd: hwnd.0 as isize,
                model,
                hover: None,
                tracking: false,
                on_action,
            });
        }
        let s = fluent::scale(hwnd);
        let (w, h) = (px(s, W), px(s, H));
        let mut pt = POINT::default();
        let _ = GetCursorPos(&mut pt);
        // Above-left of the tray icon, like Windows' own flyouts.
        let _ = SetWindowPos(
            hwnd,
            Some(HWND_TOPMOST),
            (pt.x - w + px(s, 24)).max(0),
            (pt.y - h - px(s, 16)).max(0),
            w,
            h,
            SWP_NOACTIVATE,
        );
        fluent::frame(hwnd, false);
        DragAcceptFiles(hwnd, true);
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = SetForegroundWindow(hwnd);
    }
}

pub fn close() {
    let h = CURRENT.lock().ok().and_then(|g| g.as_ref().map(|s| s.hwnd));
    if let Some(h) = h {
        // SAFETY: our own window.
        let _ = unsafe { DestroyWindow(HWND(h as *mut _)) };
    }
}
