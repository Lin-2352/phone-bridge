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
            Input::KeyboardAndMouse::{
                TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT, VK_DOWN, VK_END, VK_ESCAPE, VK_HOME,
                VK_RETURN, VK_SPACE, VK_UP,
            },
            WindowsAndMessaging::{
                CreateWindowExW, DefWindowProcW, DestroyWindow, GetSystemMetrics, LoadCursorW,
                PostMessageW, RegisterClassW, SetForegroundWindow, SetWindowPos, ShowWindow,
                HWND_TOPMOST, IDC_ARROW, SM_CXSCREEN, SWP_NOACTIVATE, SWP_NOZORDER, SW_SHOW,
                WA_INACTIVE, WM_ACTIVATE, WM_COMMAND, WM_DESTROY, WM_ERASEBKGND, WM_KEYDOWN,
                WM_LBUTTONUP, WM_MOUSEMOVE, WM_PAINT, WNDCLASSW, WS_EX_TOOLWINDOW, WS_EX_TOPMOST,
                WS_POPUP,
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
const W: i32 = 296;
const ITEM: i32 = 36;
/// A row that also says why it is off.
const ITEM_REASON: i32 = 52;
const HEADING: i32 = 28;
const SEP: i32 = 9;
const PAD: i32 = 4;

/// One entry; `id: 0` is a separator.
#[derive(Clone)]
pub struct Item {
    pub glyph: char,
    pub label: String,
    pub id: usize,
    /// A checkable row: `Some(on)`.
    pub checked: Option<bool>,
    /// Why the row cannot be used now; it is shown dimmed under the label.
    pub disabled: Option<String>,
    /// A group title, not a choice.
    pub heading: bool,
}

impl Item {
    pub fn new(glyph: char, label: &str, id: usize) -> Self {
        Self {
            glyph,
            label: label.to_owned(),
            id,
            checked: None,
            disabled: None,
            heading: false,
        }
    }
    pub fn separator() -> Self {
        Self::new(' ', "", 0)
    }
    #[cfg(test)]
    pub fn heading(label: &str) -> Self {
        Self {
            heading: true,
            ..Self::new(' ', label, 0)
        }
    }
    pub fn checked(mut self, on: bool) -> Self {
        self.checked = Some(on);
        self
    }
    pub fn disabled(mut self, reason: &str) -> Self {
        self.disabled = Some(reason.to_owned());
        self
    }
    /// Whether a click on the row does anything.
    pub fn clickable(&self) -> bool {
        self.id != 0 && !self.heading && self.disabled.is_none()
    }
    fn height(&self) -> i32 {
        if self.heading {
            HEADING
        } else if self.id == 0 {
            SEP
        } else if self.disabled.is_some() {
            ITEM_REASON
        } else {
            ITEM
        }
    }
}

/// Where a menu of `w` x `h` goes for an anchor point and the work area
/// of the anchor's monitor: centred on the anchor, above it when that fits
/// (a bottom taskbar), else below it (a top taskbar), and always inside the
/// work area.
pub fn place(anchor: POINT, w: i32, h: i32, work: RECT, gap: i32) -> (i32, i32) {
    let x = (anchor.x - w / 2).clamp(work.left, (work.right - w).max(work.left));
    let above = anchor.y - h - gap;
    let y = if above >= work.top {
        above
    } else {
        anchor.y + gap
    };
    (x, y.clamp(work.top, (work.bottom - h).max(work.top)))
}

/// What a key does in the menu: move the highlight to the next clickable
/// row (wrapping), the first or last one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Nav {
    Next,
    Prev,
    First,
    Last,
}

/// The row after `nav` from `current`, skipping separators, headings and
/// disabled rows. None when no row can be chosen.
pub fn navigate(items: &[Item], current: Option<usize>, nav: Nav) -> Option<usize> {
    let ok: Vec<usize> = (0..items.len()).filter(|&i| items[i].clickable()).collect();
    if ok.is_empty() {
        return None;
    }
    let at = current.and_then(|c| ok.iter().position(|&i| i == c));
    let n = ok.len();
    let k = match (nav, at) {
        (Nav::First, _) | (Nav::Next, None) => 0,
        (Nav::Last, _) | (Nav::Prev, None) => n - 1,
        (Nav::Next, Some(k)) => (k + 1) % n,
        (Nav::Prev, Some(k)) => (k + n - 1) % n,
    };
    Some(ok[k])
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
            let h = px(s, i.height());
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
        if item.heading {
            let tr = rectf(r.left + 12.0 * s, r.top, r.right - 8.0 * s, r.bottom);
            text(c, Font::Caption, &item.label, tr, p.text2, Align::Left);
            continue;
        }
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
        if hover == Some(i) && item.clickable() {
            round_rect(c, &r, Some((p.hover, 1.0)), None, RADIUS[0] * s, false);
        }
        let ink = if item.disabled.is_some() {
            p.text2
        } else {
            p.text
        };
        let ic = rectf(r.left + 8.0 * s, r.top, r.left + 36.0 * s, r.bottom);
        match item.checked {
            Some(true) => fluent::icon(c, fluent::glyph::CHECK, ic, p.accent),
            Some(false) => {}
            None => fluent::icon(c, item.glyph, ic, ink),
        }
        let tr = rectf(r.left + 44.0 * s, r.top, r.right - 8.0 * s, r.bottom);
        match &item.disabled {
            Some(why) => {
                let mid = r.top + 28.0 * s;
                let label = rectf(tr.left, r.top + 4.0 * s, tr.right, mid);
                let reason = rectf(tr.left, mid - 6.0 * s, tr.right, r.bottom - 2.0 * s);
                text(c, Font::Body, &item.label, label, ink, Align::Left);
                text(c, Font::Caption, why, reason, p.text2, Align::Left);
            }
            None => text(c, Font::Body, &item.label, tr, ink, Align::Left),
        }
    }
}

fn hit(hwnd: HWND, x: i32, y: i32) -> Option<usize> {
    let g = CURRENT.lock().ok()?;
    let st = g.as_ref()?;
    let s = fluent::scale(hwnd);
    rows(s, &st.items)
        .iter()
        .position(|r| fluent::inside(r, x, y))
        .filter(|&i| st.items[i].clickable())
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
        // Up / Down / Home / End move the highlight; Enter and Space choose.
        WM_KEYDOWN => {
            let vk = wparam.0 as u16;
            let nav = match vk {
                k if k == VK_DOWN.0 => Some(Nav::Next),
                k if k == VK_UP.0 => Some(Nav::Prev),
                k if k == VK_HOME.0 => Some(Nav::First),
                k if k == VK_END.0 => Some(Nav::Last),
                _ => None,
            };
            if let Some(nav) = nav {
                if let Ok(mut g) = CURRENT.lock() {
                    if let Some(st) = g.as_mut() {
                        st.hover = navigate(&st.items, st.hover, nav);
                    }
                }
                // SAFETY: repaint our own window.
                let _ = unsafe { InvalidateRect(Some(hwnd), None, false) };
                return LRESULT(0);
            }
            if vk == VK_RETURN.0 || vk == VK_SPACE.0 {
                let target = CURRENT.lock().ok().and_then(|g| {
                    g.as_ref()
                        .and_then(|st| st.hover.map(|i| (st.owner, st.items[i].id)))
                });
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
                    close();
                }
                return LRESULT(0);
            }
            // SAFETY: default handling.
            unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
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
        // The monitor that owns the tray icon (the shell's rect, else the
        // cursor): move there first so the window takes that monitor's DPI.
        let mut pt = POINT::default();
        let _ = windows::Win32::UI::WindowsAndMessaging::GetCursorPos(&mut pt);
        if let Some(r) = crate::ui::tray_icon_rect(owner) {
            pt = POINT {
                x: (r.left + r.right) / 2,
                y: (r.top + r.bottom) / 2,
            };
        }
        let _ = SetWindowPos(hwnd, None, pt.x, pt.y, 1, 1, SWP_NOACTIVATE | SWP_NOZORDER);
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
        let work = crate::flyout::work_area(pt.x, pt.y).unwrap_or(RECT {
            left: 0,
            top: 0,
            right: GetSystemMetrics(SM_CXSCREEN),
            bottom: GetSystemMetrics(windows::Win32::UI::WindowsAndMessaging::SM_CYSCREEN),
        });
        let (x, y) = place(pt, w, h, work, px(s, 8));
        let _ = SetWindowPos(hwnd, Some(HWND_TOPMOST), x, y, w, h, SWP_NOACTIVATE);
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
            Item::heading("Quick settings"),
            Item::new(' ', "Pause Relay", 26).checked(true),
            Item::new(' ', "Share clipboard", 5).checked(false),
            Item::new(fluent::glyph::SEND, "Send files", 4).disabled("Pair a phone first"),
            Item::separator(),
            Item::new(fluent::glyph::POWER, "Quit Relay", 2),
        ]
    }

    #[test]
    fn menu_rows_stack_and_separators_are_thin() {
        for s in [1.0, 1.5, 2.0] {
            let r = rows(s, &items());
            assert_eq!(r.len(), 8);
            assert!(r.windows(2).all(|w| w[0].bottom == w[1].top));
            assert!(r[1].bottom - r[1].top < r[0].bottom - r[0].top);
            // A heading is shorter, a row with a reason is taller.
            assert!(r[2].bottom - r[2].top < r[0].bottom - r[0].top);
            assert!(r[5].bottom - r[5].top > r[0].bottom - r[0].top);
            assert_eq!(height(s, &items()), r[7].bottom + px(s, PAD));
        }
    }

    fn work(l: i32, t: i32, r: i32, b: i32) -> RECT {
        RECT {
            left: l,
            top: t,
            right: r,
            bottom: b,
        }
    }

    #[test]
    fn menu_opens_above_a_bottom_taskbar_icon_on_the_monitor_that_owns_it() {
        // Second monitor to the right of the primary: x stays on it.
        let w2 = work(1920, 0, 3840, 1040);
        let (x, y) = place(POINT { x: 3700, y: 1060 }, 296, 300, w2, 8);
        assert_eq!((x, y), (3840 - 296, 1040 - 300));
        assert!(x >= w2.left);
        // A monitor left of the primary (negative x).
        let w0 = work(-1920, 0, 0, 1040);
        let (x, _) = place(POINT { x: -50, y: 1060 }, 296, 300, w0, 8);
        assert!(x >= w0.left && x + 296 <= w0.right);
    }

    #[test]
    fn menu_opens_below_a_top_taskbar_icon_and_stays_in_the_work_area() {
        let w = work(0, 48, 1920, 1080);
        let (_, y) = place(POINT { x: 1900, y: 24 }, 296, 300, w, 8);
        assert_eq!(y, 48, "below the anchor, kept under the bar");
        let (_, y) = place(POINT { x: 10, y: 5000 }, 296, 300, w, 8);
        assert!(y + 300 <= w.bottom);
    }

    #[test]
    fn arrows_skip_headings_separators_and_disabled_rows_and_wrap() {
        let it = items();
        // Clickable: 0 (Open), 3 (Pause), 4 (Share), 7 (Quit).
        assert_eq!(navigate(&it, None, Nav::Next), Some(0));
        assert_eq!(navigate(&it, None, Nav::Prev), Some(7));
        assert_eq!(navigate(&it, Some(0), Nav::Next), Some(3));
        assert_eq!(navigate(&it, Some(4), Nav::Next), Some(7));
        assert_eq!(navigate(&it, Some(7), Nav::Next), Some(0), "wraps");
        assert_eq!(navigate(&it, Some(0), Nav::Prev), Some(7), "wraps back");
        assert_eq!(navigate(&it, Some(3), Nav::First), Some(0));
        assert_eq!(navigate(&it, Some(3), Nav::Last), Some(7));
        assert_eq!(navigate(&[Item::separator()], None, Nav::Next), None);
    }

    #[test]
    fn only_plain_enabled_rows_take_clicks() {
        let it = items();
        let live: Vec<bool> = it.iter().map(Item::clickable).collect();
        assert_eq!(live, [true, false, false, true, true, false, false, true]);
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
