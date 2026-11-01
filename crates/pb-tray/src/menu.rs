//! Right-click menu for the tray icon, drawn in the Fluent style with
//! Direct2D on an opaque surface (icons, rounded corners, hover,
//! light/dark). A few verbs only; everything else lives in the flyout and
//! the main window.
#![allow(unsafe_code)]

use std::{cell::RefCell, sync::Mutex};

use windows::{
    core::{w, PCWSTR},
    Win32::{
        Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
        Graphics::Gdi::{InvalidateRect, ValidateRect},
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

use crate::{
    d2d::{Backdrop, Surface},
    fluent::{self, from_rect, px, rectf, round_rect, text, Align, Canvas, Font},
    tokens::RADIUS,
};

const WM_MOUSELEAVE: u32 = 0x02A3;
const CLASS: PCWSTR = w!("PhoneBridgeMenu");
const W: i32 = 248;
const ITEM: i32 = 36;
const SEP: i32 = 9;
const PAD: i32 = 4;

/// One entry; `id: 0` is a separator.
#[derive(Clone)]
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

thread_local! {
    /// The Direct2D surface (UI thread), keyed by the window it draws.
    static SURFACE: RefCell<Option<(isize, Surface)>> = const { RefCell::new(None) };
}

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
    // Copied out so the lock is not held while drawing.
    let snap = CURRENT.lock().ok().and_then(|g| {
        g.as_ref()
            .filter(|st| st.hwnd == hwnd.0 as isize)
            .map(|st| (st.items.clone(), st.hover))
    });
    let painted = SURFACE.with(|cell| {
        let mut slot = cell.borrow_mut();
        match (snap, slot.as_mut()) {
            (Some((items, hover)), Some((h, surface))) if *h == hwnd.0 as isize => {
                fluent::paint_with(surface, hwnd, |c, _| draw(c, &items, hover));
                true
            }
            _ => false,
        }
    });
    if !painted {
        // Nothing to draw with: validate so WM_PAINT does not repeat.
        // SAFETY: our own window.
        let _ = unsafe { ValidateRect(Some(hwnd), None) };
    }
}

fn draw(c: &mut Canvas, items: &[Item], hover: Option<usize>) {
    let (s, p) = (c.s, c.p);
    for (i, (item, r)) in items.iter().zip(rows(s, items)).enumerate() {
        let r = from_rect(r);
        if item.id == 0 {
            let line = rectf(
                r.left + 8.0 * s,
                r.top + (4.0 * s).round(),
                r.right - 8.0 * s,
                r.top + (4.0 * s).round() + 1.0,
            );
            round_rect(c, &line, Some((p.stroke, 1.0)), None, 0.0, false);
            continue;
        }
        if hover == Some(i) {
            round_rect(c, &r, Some((p.hover, 1.0)), None, RADIUS[0] * s, false);
        }
        let ic = rectf(r.left + 8.0 * s, r.top, r.left + 36.0 * s, r.bottom);
        fluent::icon(c, item.glyph, ic, p.text);
        let tr = rectf(r.left + 44.0 * s, r.top, r.right - 8.0 * s, r.bottom);
        text(c, Font::Body, &item.label, tr, p.text, Align::Left);
    }
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
            fluent::unregister_window(hwnd);
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
            // No background brush: every pixel is ours (WM_ERASEBKGND).
            ..Default::default()
        };
        RegisterClassW(&wc);
        let Ok(hwnd) = CreateWindowExW(
            WS_EX_TOOLWINDOW | WS_EX_TOPMOST,
            CLASS,
            w!("Relay"),
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
        let Ok(surface) = Surface::new(hwnd, Backdrop::Opaque) else {
            // Nothing can draw (no Direct2D at all): no menu.
            let _ = DestroyWindow(hwnd);
            return;
        };
        SURFACE.with(|cell| *cell.borrow_mut() = Some((hwnd.0 as isize, surface)));
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
        fluent::register_window(hwnd);
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

#[cfg(test)]
mod tests {
    use super::*;

    fn items() -> Vec<Item> {
        vec![
            Item::new(fluent::glyph::PHONE, "Open Relay", 24),
            Item::separator(),
            Item::new(fluent::glyph::POWER, "Quit Relay", 2),
        ]
    }

    #[test]
    fn menu_rows_stack_and_separators_are_thin() {
        for s in [1.0, 1.5, 2.0] {
            let r = rows(s, &items());
            assert_eq!(r.len(), 3);
            assert!(r.windows(2).all(|w| w[0].bottom == w[1].top));
            assert!(r[1].bottom - r[1].top < r[0].bottom - r[0].top);
            assert_eq!(height(s, &items()), r[2].bottom + px(s, PAD));
        }
    }

    #[cfg(windows)]
    #[test]
    fn menu_draws_into_a_hidden_window() {
        let hwnd = crate::d2d::test_window();
        let mut surface = Surface::new(hwnd, Backdrop::Opaque).expect("surface");
        surface
            .draw(|dc| {
                let mut c = Canvas {
                    dc,
                    s: 1.5,
                    p: fluent::palette(),
                };
                draw(&mut c, &items(), Some(0));
            })
            .expect("draw");
        drop(surface);
        // SAFETY: our own test window, made above on this thread.
        let _ = unsafe { DestroyWindow(hwnd) };
    }
}
