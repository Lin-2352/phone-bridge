//! Settings window, laid out like Windows 11 Settings: a sidebar of pages and
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
            InvalidateRect, ScreenToClient, DT_CENTER, DT_END_ELLIPSIS, DT_LEFT, DT_NOPREFIX,
            DT_PATH_ELLIPSIS, DT_SINGLELINE, DT_VCENTER, DT_WORDBREAK,
        },
        System::LibraryLoader::GetModuleHandleW,
        UI::{
            Input::KeyboardAndMouse::VK_ESCAPE,
            WindowsAndMessaging::{
                AdjustWindowRectEx, CreateWindowExW, DefWindowProcW, DestroyWindow, GetClientRect,
                GetCursorPos, GetSystemMetrics, KillTimer, LoadCursorW, LoadIconW, RegisterClassW,
                SetForegroundWindow, SetTimer, SetWindowPos, ShowWindow, HTCLIENT, IDC_ARROW,
                SM_CXSCREEN, SM_CYSCREEN, SWP_NOZORDER, SW_SHOW, WM_CHAR, WM_DESTROY,
                WM_ERASEBKGND, WM_KEYDOWN, WM_LBUTTONUP, WM_MOUSEWHEEL, WM_PAINT, WM_SETCURSOR,
                WM_TIMER, WNDCLASSW, WS_CAPTION, WS_EX_APPWINDOW, WS_MINIMIZEBOX, WS_OVERLAPPED,
                WS_SYSMENU,
            },
        },
    },
};

use crate::app_name::APP_NAME;
use crate::camera_status::{CameraButton, Mark};
use crate::fluent::{self, px};
use crate::settings_groups::{label_of, page_of, page_title_of, pages, search, Page};
use crate::settings_rows::{rows, Row};
pub use crate::settings_rows::{Action, CameraAct, ChoiceKey, Key, Model};

const CLASS: PCWSTR = w!("PhoneBridgeSettings");
const W: i32 = 800;
const H: i32 = 730;
const MARGIN: i32 = 24;
const SIDEBAR: i32 = 188;
/// Height of the search strip above the scrolling content.
const SEARCH_H: i32 = 60;
/// Longest query kept (characters).
const QUERY_MAX: usize = 64;
const TIMER_REFRESH: usize = 1;

/// What the window paints: the page title and the option rows on the page.
enum Item {
    /// Page title.
    Caption(&'static str),
    /// Small page name above the search results that sit on that page.
    PageName(&'static str),
    /// Nothing to show.
    Message(&'static str),
    Row(Row),
}

/// The rows of one page, under its title. A section header is dropped: the
/// sidebar page already names the section. While `query` is not empty the
/// matches from every page replace the page, each under its page name.
fn items(m: &Model, page: Page, query: &str) -> Vec<Item> {
    if !query.trim().is_empty() {
        let all = rows(m);
        let hits = search(&all, m, query);
        if hits.is_empty() {
            return vec![Item::Message("No settings match")];
        }
        let mut out = Vec::new();
        let mut last = "";
        for i in hits {
            let r = &all[i];
            let t = page_title_of(r);
            if t != last {
                out.push(Item::PageName(t));
                last = t;
            }
            out.push(Item::Row(r.clone()));
        }
        return out;
    }
    let title = pages()
        .iter()
        .find(|(p, _)| *p == page)
        .map_or("", |(_, t)| *t);
    let mut out = vec![Item::Caption(title)];
    out.extend(
        rows(m)
            .into_iter()
            .filter(|r| label_of(r).map(page_of) == Some(page))
            .map(Item::Row),
    );
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
        Row::CameraStatus => 120,
        Row::CameraWindow => 68,
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
    Page(Page),
    Camera(CameraAct),
}

struct State {
    hwnd: isize,
    model: Model,
    scroll: i32,
    page: Page,
    /// The search box text; typing goes here (the window has no controls).
    query: String,
    /// First half of a typed character outside the BMP, waiting for its pair.
    high: Option<u16>,
    targets: Vec<(RECT, Target)>,
    on_action: Box<dyn Fn(Action) + Send>,
}

static CURRENT: Mutex<Option<State>> = Mutex::new(None);

fn item_height(i: &Item) -> i32 {
    match i {
        Item::Caption(..) => 44,
        Item::PageName(..) => 32,
        Item::Message(..) => 56,
        Item::Row(r) => row_height(r),
    }
}

fn content_height(s: f32, m: &Model, page: Page, query: &str) -> i32 {
    px(s, SEARCH_H)
        + items(m, page, query)
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
    let page = st.page;
    let query = st.query.clone();
    let mut targets = Vec::new();
    fluent::paint(hwnd, |hdc, c| {
        let (l, r) = (px(s, SIDEBAR + MARGIN), c.right - px(s, MARGIN));
        let mut y = px(s, SEARCH_H) - scroll;
        for item in items(&m, page, &query) {
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
                Item::Caption(t) => {
                    if visible {
                        let tr = RECT {
                            left: l + px(s, 2),
                            ..card
                        };
                        fluent::text(
                            hdc,
                            f.title,
                            t,
                            tr,
                            p.text,
                            DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS,
                        );
                    }
                    y += h + px(s, 4);
                    continue;
                }
                Item::PageName(t) => {
                    if visible {
                        let tr = RECT {
                            left: l + px(s, 2),
                            top: y + px(s, 8),
                            ..card
                        };
                        fluent::text(
                            hdc,
                            f.small,
                            t,
                            tr,
                            p.text2,
                            DT_LEFT | DT_SINGLELINE | DT_VCENTER,
                        );
                    }
                    y += h + px(s, 4);
                    continue;
                }
                Item::Message(t) => {
                    if visible {
                        let tr = RECT {
                            left: l + px(s, 2),
                            ..card
                        };
                        fluent::text(
                            hdc,
                            f.body,
                            t,
                            tr,
                            p.text2,
                            DT_LEFT | DT_SINGLELINE | DT_VCENTER,
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
                        Key::Passwords | Key::ShareImages => m.share,
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
                        &format!("{} {}", APP_NAME, env!("CARGO_PKG_VERSION")),
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
                Row::CameraStatus => {
                    fluent::round_rect(hdc, &card, p.card, Some(p.stroke), px(s, 8), false);
                    let (mark, title, detail, button, repair) = match &m.camera {
                        Some(c) => (
                            c.mark,
                            c.title.as_str(),
                            c.detail.as_str(),
                            c.button,
                            c.repair,
                        ),
                        None => (
                            Mark::Idle,
                            "Camera is off",
                            "Use your phone as a webcam in any app",
                            CameraButton::Start,
                            false,
                        ),
                    };
                    let dot = RECT {
                        left: inner_l,
                        top: y + px(s, 19),
                        right: inner_l + px(s, 10),
                        bottom: y + px(s, 29),
                    };
                    let colour = match mark {
                        Mark::Ok => p.accent,
                        Mark::Bad => p.danger,
                        Mark::Wait | Mark::Idle => p.text2,
                    };
                    fluent::round_rect(hdc, &dot, colour, None, px(s, 10), false);
                    let tr = RECT {
                        left: inner_l + px(s, 22),
                        top: y + px(s, 12),
                        right: inner_r - px(s, 150),
                        bottom: y + px(s, 36),
                    };
                    fluent::text(
                        hdc,
                        f.body,
                        title,
                        tr,
                        p.text,
                        DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS,
                    );
                    let dr = RECT {
                        top: y + px(s, 40),
                        bottom: y + px(s, 100),
                        ..tr
                    };
                    fluent::text(
                        hdc,
                        f.small,
                        detail,
                        dr,
                        p.text2,
                        DT_LEFT | DT_WORDBREAK | DT_END_ELLIPSIS,
                    );
                    let label = match button {
                        CameraButton::Start => "Start camera",
                        CameraButton::Cancel => "Cancel",
                        CameraButton::Stop => "Stop camera",
                    };
                    let bw = px(s, 132);
                    let top = if repair { 18 } else { 44 };
                    let b = RECT {
                        left: inner_r - bw,
                        top: y + px(s, top),
                        right: inner_r,
                        bottom: y + px(s, top + 32),
                    };
                    fluent::button(hdc, &f, &p, b, label, true, s);
                    targets.push((b, Target::Camera(CameraAct::Toggle)));
                    if repair {
                        let rb = RECT {
                            top: y + px(s, 66),
                            bottom: y + px(s, 98),
                            ..b
                        };
                        fluent::button(hdc, &f, &p, rb, "Repair camera", false, s);
                        targets.push((rb, Target::Camera(CameraAct::Repair)));
                    }
                }
                Row::CameraWindow => {
                    fluent::round_rect(hdc, &card, p.card, Some(p.stroke), px(s, 8), false);
                    let tr = RECT {
                        left: inner_l,
                        top: y + px(s, 13),
                        right: inner_r - px(s, 112),
                        bottom: y + px(s, 35),
                    };
                    fluent::text(
                        hdc,
                        f.body,
                        "Camera window",
                        tr,
                        p.text,
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
                        "Preview and the full checklist",
                        sr,
                        p.text2,
                        DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS,
                    );
                    let b = RECT {
                        left: inner_r - px(s, 100),
                        top: y + px(s, 18),
                        right: inner_r,
                        bottom: y + px(s, 50),
                    };
                    fluent::button(hdc, &f, &p, b, "Open", false, s);
                    targets.push((b, Target::Camera(CameraAct::OpenWindow)));
                }
            }
            y += h + px(s, 4);
        }
        // Search strip, drawn over the scrolled content so it stays put.
        let strip = RECT {
            left: px(s, SIDEBAR),
            top: 0,
            right: c.right,
            bottom: px(s, SEARCH_H),
        };
        fluent::round_rect(hdc, &strip, p.bg, None, 0, false);
        let sb = RECT {
            left: l,
            top: px(s, 12),
            right: r,
            bottom: px(s, 48),
        };
        fluent::round_rect(hdc, &sb, p.card, Some(p.stroke), px(s, 8), false);
        let ic = RECT {
            left: sb.left + px(s, 8),
            right: sb.left + px(s, 36),
            ..sb
        };
        fluent::icon(hdc, f.icon_big, fluent::glyph::SEARCH, ic, p.text2);
        let tx = sb.left + px(s, 42);
        let tr = RECT {
            left: tx,
            right: sb.right - px(s, 10),
            ..sb
        };
        if query.is_empty() {
            fluent::text(
                hdc,
                f.body,
                "Search settings",
                tr,
                p.text2,
                DT_LEFT | DT_SINGLELINE | DT_VCENTER,
            );
        } else {
            fluent::text(
                hdc,
                f.body,
                &query,
                tr,
                p.text,
                DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX,
            );
        }
        let cx = (tx + fluent::text_width(hdc, f.body, &query)).min(sb.right - px(s, 10));
        let caret = RECT {
            left: cx,
            top: sb.top + px(s, 9),
            right: cx + px(s, 1).max(1),
            bottom: sb.bottom - px(s, 9),
        };
        fluent::round_rect(hdc, &caret, p.text, None, 0, false);
        // Sidebar, drawn last so it stays put while the page scrolls.
        let bar = RECT {
            left: 0,
            top: 0,
            right: px(s, SIDEBAR),
            bottom: c.bottom,
        };
        fluent::round_rect(hdc, &bar, p.bg, None, 0, false);
        let edge = RECT {
            left: bar.right - px(s, 1).max(1),
            ..bar
        };
        fluent::round_rect(hdc, &edge, p.stroke, None, 0, false);
        let mut ny = px(s, 16);
        for (pg, title) in pages() {
            let item = RECT {
                left: px(s, 10),
                top: ny,
                right: bar.right - px(s, 10),
                bottom: ny + px(s, 40),
            };
            if pg == page {
                fluent::round_rect(hdc, &item, p.card, Some(p.stroke), px(s, 8), false);
                let pill = RECT {
                    left: item.left + px(s, 2),
                    top: item.top + px(s, 10),
                    right: item.left + px(s, 5),
                    bottom: item.bottom - px(s, 10),
                };
                fluent::round_rect(hdc, &pill, p.accent, None, px(s, 2), false);
            }
            let tr = RECT {
                left: item.left + px(s, 16),
                ..item
            };
            fluent::text(
                hdc,
                f.body,
                title,
                tr,
                if pg == page { p.text } else { p.text2 },
                DT_LEFT | DT_SINGLELINE | DT_VCENTER,
            );
            targets.push((item, Target::Page(pg)));
            ny += px(s, 44);
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
                    let max = (content_height(s, &st.model, st.page, &st.query) - c.bottom).max(0);
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
                Some(Target::Camera(a)) => act(Action::Camera(a)),
                Some(Target::Page(pg)) => {
                    if let Ok(mut g) = CURRENT.lock() {
                        if let Some(st) = g.as_mut() {
                            st.page = pg;
                            st.scroll = 0;
                            st.query.clear();
                            st.high = None;
                        }
                    }
                    // SAFETY: repaint our own window.
                    let _ = unsafe { InvalidateRect(Some(hwnd), None, false) };
                    act(Action::Refresh);
                }
                None => {}
            }
            LRESULT(0)
        }
        WM_KEYDOWN if wparam.0 == VK_ESCAPE.0 as usize => {
            // The first Esc clears the search; the next closes the window.
            let cleared = CURRENT.lock().ok().is_some_and(|mut g| {
                g.as_mut().is_some_and(|st| {
                    let had = !st.query.is_empty();
                    st.query.clear();
                    st.high = None;
                    st.scroll = 0;
                    had
                })
            });
            if cleared {
                // SAFETY: repaint our own window.
                let _ = unsafe { InvalidateRect(Some(hwnd), None, false) };
            } else {
                // SAFETY: our own window.
                let _ = unsafe { DestroyWindow(hwnd) };
            }
            LRESULT(0)
        }
        WM_CHAR => {
            let changed = CURRENT.lock().ok().is_some_and(|mut g| {
                g.as_mut()
                    .is_some_and(|st| type_unit(&mut st.query, &mut st.high, wparam.0 as u16))
            });
            if changed {
                if let Ok(mut g) = CURRENT.lock() {
                    if let Some(st) = g.as_mut() {
                        st.scroll = 0;
                    }
                }
                // SAFETY: repaint our own window.
                let _ = unsafe { InvalidateRect(Some(hwnd), None, false) };
                act(Action::Refresh);
            }
            LRESULT(0)
        }
        WM_TIMER if wparam.0 == TIMER_REFRESH => {
            // The camera changes on its own; ask for a fresh snapshot while
            // its page (or a search that may show it) is up.
            let watching = CURRENT
                .lock()
                .ok()
                .and_then(|g| {
                    g.as_ref()
                        .map(|s| s.page == Page::Camera || !s.query.is_empty())
                })
                .unwrap_or(false);
            if watching {
                act(Action::Refresh);
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            // SAFETY: our own timer.
            let _ = unsafe { KillTimer(Some(hwnd), TIMER_REFRESH) };
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

/// Applies one UTF-16 unit from WM_CHAR to the search text: printable
/// characters append, Backspace removes the last one, Ctrl+F and other
/// control characters are ignored (the box is always focused). Returns
/// whether the text changed.
fn type_unit(query: &mut String, high: &mut Option<u16>, unit: u16) -> bool {
    match unit {
        0x08 => {
            *high = None;
            query.pop().is_some()
        }
        0..=0x1f | 0x7f => false,
        0xd800..=0xdbff => {
            *high = Some(unit);
            false
        }
        _ => {
            let ch = match (high.take(), unit) {
                (Some(h), 0xdc00..=0xdfff) => {
                    char::decode_utf16([h, unit]).next().and_then(Result::ok)
                }
                (_, 0xdc00..=0xdfff) => None,
                _ => char::from_u32(unit as u32),
            };
            match ch {
                Some(c) if query.chars().count() < QUERY_MAX => {
                    query.push(c);
                    true
                }
                _ => false,
            }
        }
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
            w!("Relay settings"),
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
                page: Page::Devices,
                query: String::new(),
                high: None,
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
        let _ = SetTimer(Some(hwnd), TIMER_REFRESH, 1000, None);
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

/// Whether the window is showing something that needs the camera status
/// (its page, or a search that may list it): measuring it costs a little.
pub fn wants_camera() -> bool {
    CURRENT
        .lock()
        .ok()
        .and_then(|g| {
            g.as_ref()
                .map(|s| s.page == Page::Camera || !s.query.is_empty())
        })
        .unwrap_or(false)
}

pub fn is_open() -> bool {
    CURRENT.lock().map(|g| g.is_some()).unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn typed(units: &[u16]) -> String {
        let (mut q, mut h) = (String::new(), None);
        for u in units {
            type_unit(&mut q, &mut h, *u);
        }
        q
    }

    #[test]
    fn typing_appends_and_backspace_edits() {
        let t: Vec<u16> = "cam".encode_utf16().collect();
        assert_eq!(typed(&t), "cam");
        let mut u = t.clone();
        u.push(0x08);
        assert_eq!(typed(&u), "ca");
        assert_eq!(typed(&[0x08]), "");
    }

    #[test]
    fn control_keys_are_not_text() {
        // Ctrl+F (0x06), Enter, Tab, DEL.
        assert_eq!(typed(&[0x06, 0x0d, 0x09, 0x7f]), "");
    }

    #[test]
    fn a_character_outside_the_bmp_arrives_in_two_units() {
        let u: Vec<u16> = "a\u{1f4f7}".encode_utf16().collect();
        assert_eq!(u.len(), 3);
        assert_eq!(typed(&u), "a\u{1f4f7}");
        // A lone low surrogate is dropped.
        assert_eq!(typed(&[0xdc00]), "");
    }

    #[test]
    fn the_query_is_capped() {
        let u: Vec<u16> = "x".repeat(QUERY_MAX + 10).encode_utf16().collect();
        assert_eq!(typed(&u).chars().count(), QUERY_MAX);
    }
}
