//! Right-click menu for the tray icon, drawn in the Fluent style (icons,
//! rounded corners, hover, light/dark). A few verbs only; everything else
//! lives in the flyout and the settings window.
#![allow(unsafe_code)]

use std::sync::Mutex;

use windows::{
    core::{w, PCWSTR},
    Win32::{
        Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
        Graphics::Gdi::{InvalidateRect, DT_LEFT, DT_SINGLELINE, DT_VCENTER},
        System::LibraryLoader::GetModuleHandleW,
        UI::{
            Input::KeyboardAndMouse::{TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT, VK_ESCAPE},
            WindowsAndMessaging::{
                CreateWindowExW, DefWindowProcW, DestroyWindow, GetSystemMetrics, LoadCursorW,
                PostMessageW, RegisterClassW, SetForegroundWindow, SetWindowPos, ShowWindow,
                HWND_TOPMOST, IDC_ARROW, SM_CXSCREEN, SWP_NOACTIVATE, SW_SHOW, WA_INACTIVE,
                WM_ACTIVATE, WM_COMMAND, WM_DESTROY, WM_ERASEBKGND, WM_KEYDOWN, WM_LBUTTONUP,
                WM_MOUSEMOVE, WM_PAINT, WNDCLASSW, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
            },
        },
    },
};

use crate::fluent::{self, px};

const WM_MOUSELEAVE: u32 = 0x02A3;
const CLASS: PCWSTR = w!("PhoneBridgeMenu");
const W: i32 = 248;
const ITEM: i32 = 36;
const SEP: i32 = 9;
const PAD: i32 = 4;

/// One entry; `id: 0` is a separator.
pub struct Item {
    pub glyph: char,
    pub label: String,
    pub id: usize,
}

impl Item {
    pub fn new(glyph: char, label: &str, id: usize) -> Self {
        Self {
            glyph,
            label: label.to_owned(),
            id,
        }
    }
    pub fn separator() -> Self {
        Self {
            glyph: ' ',
            label: String::new(),
            id: 0,
        }
    }
}

struct State {
    hwnd: isize,
    owner: isize,
    items: Vec<Item>,
    hover: Option<usize>,
    tracking: bool,
}

static CURRENT: Mutex<Option<State>> = Mutex::new(None);

fn rows(s: f32, items: &[Item]) -> Vec<RECT> {
    let mut y = px(s, PAD);
    items
        .iter()
        .map(|i| {
            let h = px(s, if i.id == 0 { SEP } else { ITEM });
            let r = RECT {
                left: px(s, PAD),
                top: y,
                right: px(s, W - PAD),
                bottom: y + h,
            };
            y += h;
            r
        })
        .collect()
}

fn height(s: f32, items: &[Item]) -> i32 {
    rows(s, items).last().map_or(0, |r| r.bottom) + px(s, PAD)
}

fn paint(hwnd: HWND) {
    let Ok(g) = CURRENT.lock() else { return };
    let Some(st) = g.as_ref() else { return };
    let s = fluent::scale(hwnd);
    let p = fluent::palette();
    let f = fluent::Fonts::new(s);
    fluent::paint(hwnd, |hdc, _| {
        for (i, (item, r)) in st.items.iter().zip(rows(s, &st.items)).enumerate() {
            if item.id == 0 {
                let line = RECT {
                    left: r.left + px(s, 8),
                    top: r.top + px(s, 4),
                    right: r.right - px(s, 8),
                    bottom: r.top + px(s, 5),
                };
                fluent::round_rect(hdc, &line, p.stroke, None, 0, false);
                continue;
            }
            if st.hover == Some(i) {
                fluent::round_rect(hdc, &r, p.hover, None, px(s, 6), false);
            }
            let ic = RECT {
                left: r.left + px(s, 8),
                right: r.left + px(s, 36),
                ..r
            };
            fluent::icon(hdc, f.icon, item.glyph, ic, p.text);
            let tr = RECT {
                left: r.left + px(s, 44),
                ..r
            };
            fluent::text(
                hdc,
                f.body,
                &item.label,
                tr,
                p.text,
                DT_LEFT | DT_VCENTER | DT_SINGLELINE,
            );
        }
    });
}

fn hit(hwnd: HWND, x: i32, y: i32) -> Option<usize> {
    let g = CURRENT.lock().ok()?;
    let st = g.as_ref()?;
    let s = fluent::scale(hwnd);
    rows(s, &st.items)
        .iter()
        .position(|r| fluent::inside(r, x, y))
        .filter(|&i| st.items[i].id != 0)
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
            fluent::set_cursor(false);
            if changed {
                // SAFETY: repaint our own window.
                let _ = unsafe { InvalidateRect(Some(hwnd), None, false) };
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
            // SAFETY: repaint our own window.
            let _ = unsafe { InvalidateRect(Some(hwnd), None, false) };
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            let (x, y) = xy();
            if let Some(i) = hit(hwnd, x, y) {
                let target = CURRENT
                    .lock()
                    .ok()
                    .and_then(|g| g.as_ref().map(|st| (st.owner, st.items[i].id)));
                if let Some((owner, id)) = target {
                    // SAFETY: posting to the tray's own message window.
                    let _ = unsafe {
                        PostMessageW(
                            Some(HWND(owner as *mut _)),
                            WM_COMMAND,
                            WPARAM(id),
                            LPARAM(0),
                        )
                    };
                }
                close();
            }
            LRESULT(0)
        }
        WM_KEYDOWN if wparam.0 == VK_ESCAPE.0 as usize => {
            close();
            LRESULT(0)
        }
        // A menu closes as soon as you click anywhere else.
        WM_ACTIVATE if (wparam.0 & 0xffff) as u32 == WA_INACTIVE => {
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

/// Show the menu above the cursor; choices arrive at `owner` as WM_COMMAND.
pub fn show(owner: HWND, items: Vec<Item>) {
    close();
    // SAFETY: window creation and positioning on the UI thread.
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
        let Ok(hwnd) = CreateWindowExW(
            WS_EX_TOOLWINDOW | WS_EX_TOPMOST,
            CLASS,
            w!("Phone Bridge"),
            WS_POPUP,
            0,
            0,
            W,
            100,
            None,
            None,
            Some(hinstance.into()),
            None,
        ) else {
            return;
        };
        let s = fluent::scale(hwnd);
        let (w, h) = (px(s, W), height(s, &items));
        if let Ok(mut g) = CURRENT.lock() {
            *g = Some(State {
                hwnd: hwnd.0 as isize,
                owner: owner.0 as isize,
                items,
                hover: None,
                tracking: false,
            });
        }
        let mut pt = POINT::default();
        let _ = windows::Win32::UI::WindowsAndMessaging::GetCursorPos(&mut pt);
        let screen_w = GetSystemMetrics(SM_CXSCREEN);
        let x = (pt.x - w / 2).clamp(0, (screen_w - w).max(0));
        let _ = SetWindowPos(
            hwnd,
            Some(HWND_TOPMOST),
            x,
            (pt.y - h - px(s, 8)).max(0),
            w,
            h,
            SWP_NOACTIVATE,
        );
        fluent::frame(hwnd, false);
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
