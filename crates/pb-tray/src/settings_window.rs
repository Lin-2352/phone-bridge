//! Settings window, laid out like Windows 11 Settings: section headers and
//! rounded cards with toggles and buttons, light/dark, per-monitor DPI,
//! mouse-wheel scrolling. Reads a [`Model`] snapshot and reports [`Action`]s;
//! the UI thread applies them and calls [`update`].
#![allow(unsafe_code)]

use std::sync::Mutex;

use windows::{
    core::{w, PCWSTR},
    Win32::{
        Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
        Graphics::Gdi::{
            InvalidateRect, ScreenToClient, DT_CENTER, DT_END_ELLIPSIS, DT_LEFT, DT_PATH_ELLIPSIS,
            DT_SINGLELINE, DT_VCENTER,
        },
        System::LibraryLoader::GetModuleHandleW,
        UI::{
            Input::KeyboardAndMouse::VK_ESCAPE,
            WindowsAndMessaging::{
                AdjustWindowRectEx, CreateWindowExW, DefWindowProcW, DestroyWindow, GetClientRect,
                GetCursorPos, GetSystemMetrics, LoadCursorW, LoadIconW, RegisterClassW,
                SetForegroundWindow, SetWindowPos, ShowWindow, HTCLIENT, IDC_ARROW, SM_CXSCREEN,
                SM_CYSCREEN, SWP_NOZORDER, SW_SHOW, WM_DESTROY, WM_ERASEBKGND, WM_KEYDOWN,
                WM_LBUTTONUP, WM_MOUSEWHEEL, WM_PAINT, WM_SETCURSOR, WNDCLASSW, WS_CAPTION,
                WS_EX_APPWINDOW, WS_MINIMIZEBOX, WS_OVERLAPPED, WS_SYSMENU,
            },
        },
    },
};

use crate::fluent::{self, px};
use crate::settings_groups::{group_of, groups, label_of};
use crate::settings_rows::{rows, Row};
pub use crate::settings_rows::{Action, ChoiceKey, Key, Model};

const CLASS: PCWSTR = w!("PhoneBridgeSettings");
const W: i32 = 560;
const H: i32 = 640;
const MARGIN: i32 = 24;

/// What the window paints: group captions and the option rows under them.
enum Item {
    /// Group caption; the flag marks the first one (no divider above it).
    Caption(&'static str, bool),
    Row(Row),
}

/// The settings rows laid out under the group captions. Rows keep their
/// order within a group; a section header is kept as a sub-heading unless it
/// repeats the caption. Groups without rows get no caption.
fn items(m: &Model) -> Vec<Item> {
    let all = rows(m);
    let mut out = Vec::new();
    for (group, title) in groups() {
        let mut section: Option<&'static str> = None;
        let mut shown: Option<&'static str> = None;
        let mut started = false;
        for row in &all {
            if let Row::Header(t) = row {
                section = Some(t);
                continue;
            }
            if label_of(row).map(group_of) != Some(group) {
                continue;
            }
            if !started {
                out.push(Item::Caption(title, out.is_empty()));
                started = true;
            }
            if section != shown {
                shown = section;
                if let Some(t) = section.filter(|t| *t != title) {
                    out.push(Item::Row(Row::Header(t)));
                }
            }
            out.push(Item::Row(row.clone()));
        }
    }
    out
}

fn row_height(r: &Row) -> i32 {
    match r {
        Row::Header(_) => 40,
        Row::Theme => 64,
        Row::Toggle(..) => 68,
        Row::Choice(..) => 100,
        Row::App(_) => 56,
        Row::Folder => 76,
        Row::Device(_) => 64,
        Row::Pair => 60,
        Row::About => 68,
    }
}

/// Clickable areas computed while painting, used for hit-testing.
#[derive(Clone, Copy)]
enum Target {
    Theme(u8),
    Toggle(Key),
    Change,
    Open,
    Remove(usize),
    Pair,
    Choice(ChoiceKey, u8),
    Mute(usize),
}

struct State {
    hwnd: isize,
    model: Model,
    scroll: i32,
    targets: Vec<(RECT, Target)>,
    on_action: Box<dyn Fn(Action) + Send>,
}

static CURRENT: Mutex<Option<State>> = Mutex::new(None);

fn item_height(i: &Item) -> i32 {
    match i {
        Item::Caption(..) => 36,
        Item::Row(r) => row_height(r),
    }
}

fn content_height(s: f32, m: &Model) -> i32 {
    px(s, 16)
        + items(m)
            .iter()
            .map(|i| px(s, item_height(i) + 4))
            .sum::<i32>()
        + px(s, 16)
}

fn paint(hwnd: HWND) {
    let Ok(mut guard) = CURRENT.lock() else {
        return;
    };
    let Some(st) = guard.as_mut() else { return };
    let s = fluent::scale(hwnd);
    let p = fluent::palette();
    let f = fluent::Fonts::new(s);
    let m = st.model.clone();
    let scroll = st.scroll;
    let mut targets = Vec::new();
    fluent::paint(hwnd, |hdc, c| {
        let (l, r) = (px(s, MARGIN), c.right - px(s, MARGIN));
        let mut y = px(s, 16) - scroll;
        for item in items(&m) {
            let h = px(s, item_height(&item));
            let card = RECT {
                left: l,
                top: y,
                right: r,
                bottom: y + h,
            };
            let inner_l = l + px(s, 16);
            let inner_r = r - px(s, 16);
            let visible = card.bottom > 0 && card.top < c.bottom;
            let row = match item {
                Item::Caption(t, first) => {
                    if visible {
                        if !first {
                            let line = RECT {
                                top: y + px(s, 4),
                                bottom: y + px(s, 4) + px(s, 1).max(1),
                                ..card
                            };
                            fluent::round_rect(hdc, &line, p.stroke, None, 0, false);
                        }
                        let tr = RECT {
                            left: l + px(s, 2),
                            top: y + px(s, 10),
                            ..card
                        };
                        fluent::text(
                            hdc,
                            f.small,
                            t,
                            tr,
                            p.text2,
                            DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS,
                        );
                    }
                    y += h + px(s, 4);
                    continue;
                }
                Item::Row(row) => row,
            };
            match row {
                Row::Header(t) => {
                    if visible {
                        let tr = RECT {
                            left: l + px(s, 2),
                            top: y + px(s, 14),
                            ..card
                        };
                        fluent::text(
                            hdc,
                            f.body,
                            t,
                            tr,
                            p.text,
                            DT_LEFT | DT_SINGLELINE | DT_VCENTER,
                        );
                    }
                }
                _ if !visible => {}
                Row::Theme => {
                    fluent::round_rect(hdc, &card, p.card, Some(p.stroke), px(s, 8), false);
                    let tr = RECT {
                        left: inner_l,
                        right: inner_l + px(s, 160),
                        ..card
                    };
                    fluent::text(
                        hdc,
                        f.body,
                        "Theme",
                        tr,
                        p.text,
                        DT_LEFT | DT_SINGLELINE | DT_VCENTER,
                    );
                    let seg_w = px(s, 88);
                    let total = seg_w * 3;
                    let mut x = inner_r - total;
                    let sy = y + (h - px(s, 32)) / 2;
                    for (i, label) in ["System", "Light", "Dark"].iter().enumerate() {
                        let br = RECT {
                            left: x,
                            top: sy,
                            right: x + seg_w - px(s, 4),
                            bottom: sy + px(s, 32),
                        };
                        fluent::button(hdc, &f, &p, br, label, m.theme == i as u8, s);
                        targets.push((br, Target::Theme(i as u8)));
                        x += seg_w;
                    }
                }
                Row::Toggle(k, title, sub) => {
                    fluent::round_rect(hdc, &card, p.card, Some(p.stroke), px(s, 8), false);
                    let enabled = match k {
                        Key::Passwords => m.share,
                        Key::NotifyLock
                        | Key::NotifyClear
                        | Key::NotifyOpen
                        | Key::NotifyOtp
                        | Key::NotifyQuietMirror => m.notify,
                        _ => true,
                    };
                    let tr = RECT {
                        left: inner_l,
                        top: y + px(s, 13),
                        right: inner_r - px(s, 56),
                        bottom: y + px(s, 35),
                    };
                    fluent::text(
                        hdc,
                        f.body,
                        title,
                        tr,
                        if enabled { p.text } else { p.text2 },
                        DT_LEFT | DT_SINGLELINE | DT_VCENTER,
                    );
                    let sr = RECT {
                        top: y + px(s, 35),
                        bottom: y + px(s, 55),
                        ..tr
                    };
                    fluent::text(
                        hdc,
                        f.small,
                        sub,
                        sr,
                        p.text2,
                        DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS,
                    );
                    fluent::toggle(hdc, s, &p, inner_r, y + h / 2, m.get(k), enabled);
                    if enabled {
                        targets.push((card, Target::Toggle(k)));
                    }
                }
                Row::Choice(k, title, labels) => {
                    fluent::round_rect(hdc, &card, p.card, Some(p.stroke), px(s, 8), false);
                    let tr = RECT {
                        left: inner_l,
                        top: y + px(s, 12),
                        right: inner_r,
                        bottom: y + px(s, 38),
                    };
                    fluent::text(
                        hdc,
                        f.body,
                        title,
                        tr,
                        if m.notify { p.text } else { p.text2 },
                        DT_LEFT | DT_SINGLELINE | DT_VCENTER,
                    );
                    let seg_w = (inner_r - inner_l) / 3;
                    let mut x = inner_l;
                    let sy = y + px(s, 50);
                    for (i, label) in labels.iter().enumerate() {
                        let br = RECT {
                            left: x,
                            top: sy,
                            right: x + seg_w - px(s, 6),
                            bottom: sy + px(s, 34),
                        };
                        fluent::button(hdc, &f, &p, br, label, m.choice(k) == i as u8, s);
                        if m.notify {
                            targets.push((br, Target::Choice(k, i as u8)));
                        }
                        x += seg_w;
                    }
                }
                Row::App(i) => {
                    fluent::round_rect(hdc, &card, p.card, Some(p.stroke), px(s, 8), false);
                    let (name, shown) = &m.apps[i];
                    let tr = RECT {
                        left: inner_l,
                        right: inner_r - px(s, 56),
                        ..card
                    };
                    fluent::text(
                        hdc,
                        f.body,
                        name,
                        tr,
                        if m.notify { p.text } else { p.text2 },
                        DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS,
                    );
                    fluent::toggle(hdc, s, &p, inner_r, y + h / 2, *shown, m.notify);
                    if m.notify {
                        targets.push((card, Target::Mute(i)));
                    }
                }
                Row::Folder => {
                    fluent::round_rect(hdc, &card, p.card, Some(p.stroke), px(s, 8), false);
                    let btn_w = px(s, 76);
                    let change = RECT {
                        left: inner_r - btn_w,
                        top: y + px(s, 22),
                        right: inner_r,
                        bottom: y + px(s, 54),
                    };
                    let open = RECT {
                        left: change.left - btn_w - px(s, 8),
                        right: change.left - px(s, 8),
                        ..change
                    };
                    let tr = RECT {
                        left: inner_l,
                        top: y + px(s, 14),
                        right: open.left - px(s, 12),
                        bottom: y + px(s, 36),
                    };
                    fluent::text(
                        hdc,
                        f.body,
                        "Save received files to",
                        tr,
                        p.text,
                        DT_LEFT | DT_SINGLELINE | DT_VCENTER,
                    );
                    let pr = RECT {
                        top: y + px(s, 36),
                        bottom: y + px(s, 58),
                        ..tr
                    };
                    fluent::text(
                        hdc,
                        f.small,
                        &m.save_dir,
                        pr,
                        p.text2,
                        DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_PATH_ELLIPSIS,
                    );
                    fluent::button(hdc, &f, &p, open, "Open", false, s);
                    fluent::button(hdc, &f, &p, change, "Change", true, s);
                    targets.push((open, Target::Open));
                    targets.push((change, Target::Change));
                }
                Row::Device(i) => {
                    fluent::round_rect(hdc, &card, p.card, Some(p.stroke), px(s, 8), false);
                    let (name, fp) = &m.devices[i];
                    let ic = RECT {
                        left: inner_l,
                        top: y,
                        right: inner_l + px(s, 28),
                        bottom: y + h,
                    };
                    fluent::icon(hdc, f.icon_big, fluent::glyph::PHONE, ic, p.accent);
                    let tr = RECT {
                        left: inner_l + px(s, 40),
                        top: y + px(s, 11),
                        right: inner_r - px(s, 100),
                        bottom: y + px(s, 33),
                    };
                    fluent::text(
                        hdc,
                        f.body,
                        name,
                        tr,
                        p.text,
                        DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS,
                    );
                    let fr = RECT {
                        top: y + px(s, 33),
                        bottom: y + px(s, 53),
                        ..tr
                    };
                    fluent::text(
                        hdc,
                        f.small,
                        &format!("Fingerprint {fp}"),
                        fr,
                        p.text2,
                        DT_LEFT | DT_SINGLELINE | DT_VCENTER,
                    );
                    let rb = RECT {
                        left: inner_r - px(s, 88),
                        top: y + px(s, 16),
                        right: inner_r,
                        bottom: y + px(s, 48),
                    };
                    fluent::round_rect(hdc, &rb, p.card, Some(p.stroke), px(s, 8), false);
                    fluent::text(
                        hdc,
                        f.body,
                        "Remove",
                        rb,
                        p.danger,
                        DT_CENTER | DT_SINGLELINE | DT_VCENTER,
                    );
                    targets.push((rb, Target::Remove(i)));
                }
                Row::Pair => {
                    fluent::round_rect(hdc, &card, p.card, Some(p.stroke), px(s, 8), false);
                    let tr = RECT {
                        left: inner_l,
                        right: inner_r - px(s, 140),
                        ..card
                    };
                    let label = if m.devices.is_empty() {
                        "No device paired yet"
                    } else {
                        "Add another device"
                    };
                    fluent::text(
                        hdc,
                        f.body,
                        label,
                        tr,
                        p.text,
                        DT_LEFT | DT_SINGLELINE | DT_VCENTER,
                    );
                    let b = RECT {
                        left: inner_r - px(s, 128),
                        top: y + px(s, 14),
                        right: inner_r,
                        bottom: y + px(s, 46),
                    };
                    fluent::button(hdc, &f, &p, b, "Pair device", true, s);
                    targets.push((b, Target::Pair));
                }
                Row::About => {
                    fluent::round_rect(hdc, &card, p.card, Some(p.stroke), px(s, 8), false);
                    let tr = RECT {
                        left: inner_l,
                        top: y + px(s, 13),
                        right: inner_r,
                        bottom: y + px(s, 35),
                    };
                    fluent::text(
                        hdc,
                        f.body,
                        &format!("Phone Bridge {}", env!("CARGO_PKG_VERSION")),
                        tr,
                        p.text,
                        DT_LEFT | DT_SINGLELINE | DT_VCENTER,
                    );
                    let fr = RECT {
                        top: y + px(s, 35),
                        bottom: y + px(s, 55),
                        ..tr
                    };
                    fluent::text(
                        hdc,
                        f.small,
                        &format!("This PC's fingerprint {}", m.my_fingerprint),
                        fr,
                        p.text2,
                        DT_LEFT | DT_SINGLELINE | DT_VCENTER,
                    );
                }
            }
            y += h + px(s, 4);
        }
    });
    st.targets = targets;
}

fn target_at(x: i32, y: i32) -> Option<Target> {
    let g = CURRENT.lock().ok()?;
    let st = g.as_ref()?;
    st.targets
        .iter()
        .rev() // buttons are pushed after their card
        .find(|(r, _)| fluent::inside(r, x, y))
        .map(|(_, t)| *t)
}

fn act(a: Action) {
    if let Ok(g) = CURRENT.lock() {
        if let Some(st) = g.as_ref() {
            (st.on_action)(a);
        }
    }
}

extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
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
            fluent::set_cursor(target_at(pt.x, pt.y).is_some());
            LRESULT(1)
        }
        WM_MOUSEWHEEL => {
            let delta = ((wparam.0 >> 16) & 0xffff) as i16 as i32;
            let s = fluent::scale(hwnd);
            let mut c = RECT::default();
            // SAFETY: our own window.
            let _ = unsafe { GetClientRect(hwnd, &mut c) };
            if let Ok(mut g) = CURRENT.lock() {
                if let Some(st) = g.as_mut() {
                    let max = (content_height(s, &st.model) - c.bottom).max(0);
                    st.scroll = (st.scroll - delta * px(s, 48) / 120).clamp(0, max);
                }
            }
            // SAFETY: repaint our own window.
            let _ = unsafe { InvalidateRect(Some(hwnd), None, false) };
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            let x = (lparam.0 & 0xffff) as i16 as i32;
            let y = ((lparam.0 >> 16) & 0xffff) as i16 as i32;
            match target_at(x, y) {
                Some(Target::Theme(t)) => act(Action::Theme(t)),
                Some(Target::Toggle(k)) => act(Action::Toggle(k)),
                Some(Target::Change) => act(Action::ChangeFolder),
                Some(Target::Open) => act(Action::OpenFolder),
                Some(Target::Remove(i)) => act(Action::Remove(i)),
                Some(Target::Pair) => act(Action::Pair),
                Some(Target::Choice(k, i)) => act(Action::Choice(k, i)),
                Some(Target::Mute(i)) => act(Action::Mute(i)),
                None => {}
            }
            LRESULT(0)
        }
        WM_KEYDOWN if wparam.0 == VK_ESCAPE.0 as usize => {
            // SAFETY: our own window.
            let _ = unsafe { DestroyWindow(hwnd) };
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

/// Open (or bring to front) the settings window.
pub fn open(model: Model, on_action: Box<dyn Fn(Action) + Send>) {
    if let Some(h) = CURRENT.lock().ok().and_then(|g| g.as_ref().map(|s| s.hwnd)) {
        update(model);
        // SAFETY: our own window; restore it if minimised, then raise it.
        unsafe {
            let _ = ShowWindow(
                HWND(h as *mut _),
                windows::Win32::UI::WindowsAndMessaging::SW_RESTORE,
            );
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
            hIcon: LoadIconW(
                Some(hinstance.into()),
                PCWSTR(std::ptr::without_provenance::<u16>(1)),
            )
            .unwrap_or_default(),
            ..Default::default()
        };
        RegisterClassW(&wc);
        let style = WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX;
        let Ok(hwnd) = CreateWindowExW(
            WS_EX_APPWINDOW,
            CLASS,
            w!("Phone Bridge settings"),
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
        if let Ok(mut g) = CURRENT.lock() {
            *g = Some(State {
                hwnd: hwnd.0 as isize,
                model,
                scroll: 0,
                targets: Vec::new(),
                on_action,
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
        let _ = SetForegroundWindow(hwnd);
    }
}

/// New snapshot after a change; repaints if the window is open.
pub fn update(model: Model) {
    let h = CURRENT.lock().ok().and_then(|mut g| {
        g.as_mut().map(|st| {
            st.model = model;
            st.hwnd
        })
    });
    if let Some(h) = h {
        let hwnd = HWND(h as *mut _);
        fluent::frame(hwnd, false);
        // SAFETY: repaint our own window.
        let _ = unsafe { InvalidateRect(Some(hwnd), None, false) };
    }
}

pub fn is_open() -> bool {
    CURRENT.lock().map(|g| g.is_some()).unwrap_or(false)
}
