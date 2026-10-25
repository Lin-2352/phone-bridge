//! Fluent-style flyout opened by left-clicking the tray icon: link status,
//! a drop zone for files, and Quick Settings style toggle tiles (Camera,
//! Phone screen, Control phone or Pair device, Clipboard, Send files) under
//! small group captions, a status line, plus settings and close. Hover and
//! press fade in and out (`ease`). It stays open when another window is
//! clicked, so files can be dragged in from Explorer; it closes with Esc, the
//! close button, the tray icon, or after a drop.
#![allow(unsafe_code)]

use std::sync::Mutex;
use std::time::Instant;

use windows::{
    core::{w, PCWSTR},
    Win32::{
        Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
        Graphics::Gdi::{
            GetMonitorInfoW, InvalidateRect, MonitorFromPoint, ScreenToClient, DT_END_ELLIPSIS,
            DT_LEFT, DT_SINGLELINE, DT_VCENTER, DT_WORDBREAK, MONITORINFO,
            MONITOR_DEFAULTTONEAREST,
        },
        System::LibraryLoader::GetModuleHandleW,
        UI::{
            Input::KeyboardAndMouse::{TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT, VK_ESCAPE},
            Shell::{DragAcceptFiles, DragFinish, DragQueryFileW, HDROP},
            WindowsAndMessaging::{
                CreateWindowExW, DefWindowProcW, DestroyWindow, GetCursorPos, GetWindowRect,
                KillTimer, LoadCursorW, RegisterClassW, SetForegroundWindow, SetTimer,
                SetWindowPos, ShowWindow, HTCLIENT, HWND_TOPMOST, IDC_ARROW, SWP_NOACTIVATE,
                SW_SHOW, WM_DESTROY, WM_DROPFILES, WM_ERASEBKGND, WM_KEYDOWN, WM_LBUTTONDOWN,
                WM_LBUTTONUP, WM_MOUSEMOVE, WM_PAINT, WM_SETCURSOR, WM_TIMER, WNDCLASSW,
                WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
            },
        },
    },
};

use crate::ease;
use crate::fluent::{self, glyph, px};
use crate::flyout_tiles::{release_target, status_line, tiles, TileKind};

const WM_MOUSELEAVE: u32 = 0x02A3;
const CLASS: PCWSTR = w!("PhoneBridgeFlyout");
/// Logical (96-dpi) size of the flyout.
const W: i32 = 360;
const H: i32 = 404;
/// Hover and press animation tick (about 60 fps); killed once settled.
const ANIM_TIMER: usize = 1;
const ANIM_MS: u32 = 16;
/// Characters the status line keeps before it is clipped with an ellipsis.
const STATUS_CHARS: usize = 56;
const CAMERA: char = '\u{E722}';
const CHEVRON: char = '\u{E76C}';

/// What the flyout shows; filled in by the UI before opening.
#[derive(Clone, Default)]
pub struct Model {
    pub peer: Option<String>,
    pub clipboard_on: bool,
    /// The phone-screen window is open.
    pub screen_on: bool,
    /// The phone camera is streaming, and its frames per second.
    pub camera_on: bool,
    pub camera_fps: u32,
    pub last: String,
}

/// User actions the flyout reports back to the UI thread.
pub enum Action {
    ToggleClipboard,
    SendFiles,
    Pair,
    ControlPhone,
    ShowScreen,
    ToggleCamera,
    CameraWindow,
    Settings,
    Dropped(Vec<std::path::PathBuf>),
}

struct State {
    hwnd: isize,
    model: Model,
    hover: Option<Hit>,
    /// Where the left button went down, until it is released or leaves.
    pressed: Option<Hit>,
    tracking: bool,
    /// Eased hover and press amounts (0..=1), one per `Hit::slot`.
    hover_t: [f32; SLOTS],
    press_t: [f32; SLOTS],
    /// The animation timer runs; `last_tick` is its previous step.
    animating: bool,
    last_tick: Instant,
    on_action: Box<dyn Fn(Action) + Send>,
}

#[derive(Clone, Copy, PartialEq)]
enum Hit {
    Drop,
    Tile(TileKind),
    /// The chevron on the camera tile: the camera status window.
    CameraInfo,
    Settings,
    Close,
}

const SLOTS: usize = 9;

impl Hit {
    fn slot(self) -> usize {
        match self {
            Hit::Drop => 0,
            Hit::Tile(TileKind::Camera) => 1,
            Hit::Tile(TileKind::Screen) => 2,
            Hit::Tile(TileKind::Clipboard) => 3,
            Hit::Tile(TileKind::SendFile) => 4,
            Hit::Tile(TileKind::Pair) => 5,
            Hit::CameraInfo => 6,
            Hit::Settings => 7,
            Hit::Close => 8,
        }
    }
}

/// False for a dimmed tile, which neither highlights nor reacts to clicks.
fn clickable(m: &Model, h: Hit) -> bool {
    match h {
        Hit::Tile(k) => tiles(m).iter().any(|t| t.kind == k && t.enabled),
        // Like the dimmed Send files tile: nothing to send to without a phone.
        Hit::Drop => m.peer.is_some(),
        _ => true,
    }
}

static CURRENT: Mutex<Option<State>> = Mutex::new(None);
/// Where the user last dragged the flyout (this session); it reopens there.
static LAST_POS: Mutex<Option<(i32, i32)>> = Mutex::new(None);
const WM_NCHITTEST: u32 = 0x0084;
const WM_NCLBUTTONDBLCLK: u32 = 0x00A3;
const WM_EXITSIZEMOVE: u32 = 0x0232;
const HTCAPTION: isize = 2;

/// Layout rectangles in physical pixels for a given DPI scale.
struct Layout {
    badge: RECT,
    settings: RECT,
    close: RECT,
    drop: RECT,
    /// Group captions: Camera & Screen, Connection, Files.
    captions: [RECT; 3],
    /// Horizontal rules between the tile rows and above the status line.
    dividers: [RECT; 2],
    /// Indexed like `Hit::slot` minus one: Camera, Screen, Clipboard,
    /// SendFile, Pair.
    tiles: [RECT; 5],
    camera_info: RECT,
    status: RECT,
}

fn tile_index(k: TileKind) -> usize {
    Hit::Tile(k).slot() - 1
}

fn layout(s: f32) -> Layout {
    let p = |v: i32| px(s, v);
    let sq = |l: i32, t: i32, n: i32| RECT {
        left: p(l),
        top: p(t),
        right: p(l + n),
        bottom: p(t + n),
    };
    let row = |t: i32, b: i32| RECT {
        left: p(16),
        top: p(t),
        right: p(W - 16),
        bottom: p(b),
    };
    // Three columns: Camera, Screen | Pair on the first row, Clipboard and
    // Send files on the second.
    let tw = (p(W) - 2 * p(16) - 2 * p(8)) / 3;
    let col = |c: i32| p(16) + c * (tw + p(8));
    let cell = |c: i32, top: i32| RECT {
        left: col(c),
        top: p(top),
        right: col(c) + tw,
        bottom: p(top + 76),
    };
    let caption = |c: i32, span: i32, top: i32| RECT {
        left: col(c),
        top: p(top),
        right: col(c + span - 1) + tw,
        bottom: p(top + 18),
    };
    let camera = cell(0, 158);
    Layout {
        badge: sq(16, 16, 40),
        settings: sq(W - 16 - 32 - 36, 20, 32),
        close: sq(W - 16 - 32, 20, 32),
        drop: row(72, 124),
        captions: [caption(0, 2, 136), caption(2, 1, 136), caption(0, 3, 256)],
        dividers: [row(246, 247), row(354, 355)],
        tiles: [
            camera,
            cell(1, 158),
            cell(0, 276),
            cell(1, 276),
            cell(2, 158),
        ],
        camera_info: RECT {
            left: camera.right - p(32),
            top: camera.top + p(4),
            right: camera.right - p(4),
            bottom: camera.top + p(32),
        },
        status: row(364, 388),
    }
}

/// Blend two COLORREFs: `t` = 0 gives `a`, 1 gives `b`.
fn mix(a: u32, b: u32, t: f32) -> u32 {
    let t = t.clamp(0.0, 1.0);
    let ch = |sh: u32| {
        let (x, y) = (((a >> sh) & 0xff) as f32, ((b >> sh) & 0xff) as f32);
        ((x + (y - x) * t).round() as u32) << sh
    };
    ch(0) | ch(8) | ch(16)
}

fn paint(hwnd: HWND) {
    let Ok(guard) = CURRENT.lock() else { return };
    let Some(st) = guard.as_ref() else { return };
    let s = fluent::scale(hwnd);
    let p = fluent::palette();
    let f = fluent::Fonts::new(s);
    let l = layout(s);
    let hov = |h: Hit| st.hover_t[h.slot()];
    let prs = |h: Hit| st.press_t[h.slot()];
    let radius = fluent::corner_radius(s, 8);
    fluent::paint(hwnd, |hdc, _| {
        // Header: device badge, name, status, settings and close.
        fluent::round_rect(hdc, &l.badge, p.card, Some(p.stroke), px(s, 10), false);
        fluent::icon(hdc, f.icon_big, glyph::PHONE, l.badge, p.accent);
        let (title, sub) = match &st.model.peer {
            Some(n) => (n.clone(), "Paired".to_owned()),
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
            if hov(h) > 0.0 || prs(h) > 0.0 {
                let fill = mix(mix(p.bg, p.hover, hov(h)), p.stroke, prs(h));
                fluent::round_rect(hdc, &r, fill, None, px(s, 6), false);
            }
            fluent::icon(hdc, f.icon, g, r, p.text2);
        }

        // Drop zone: icon and hint on one line.
        let drop_fill = mix(
            mix(p.card, p.hover, hov(Hit::Drop)),
            p.stroke,
            prs(Hit::Drop),
        );
        fluent::round_rect(hdc, &l.drop, drop_fill, Some(p.text2), px(s, 10), true);
        let ic = RECT {
            left: l.drop.left + px(s, 16),
            top: l.drop.top,
            right: l.drop.left + px(s, 44),
            bottom: l.drop.bottom,
        };
        fluent::icon(hdc, f.icon_big, glyph::UPLOAD, ic, p.text2);
        let label = match &st.model.peer {
            Some(n) => format!("Drop files here to send to {n}"),
            None => "Pair a device to send files".to_owned(),
        };
        let lr = RECT {
            left: ic.right + px(s, 8),
            right: l.drop.right - px(s, 12),
            ..l.drop
        };
        fluent::text(
            hdc,
            f.small,
            &label,
            lr,
            p.text2,
            DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS,
        );

        // Group captions and dividers.
        for (r, c) in l
            .captions
            .iter()
            .zip(["Camera & Screen", "Connection", "Files"])
        {
            fluent::text(
                hdc,
                f.small,
                c,
                *r,
                p.text2,
                DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS,
            );
        }
        for r in &l.dividers {
            fluent::round_rect(hdc, r, p.stroke, None, 0, false);
        }

        // Tiles.
        for t in tiles(&st.model) {
            let h = Hit::Tile(t.kind);
            let r = l.tiles[tile_index(t.kind)];
            let (fill, stroke, fg) = if !t.enabled {
                (p.card, Some(p.stroke), mix(p.text, p.card, 0.6))
            } else if t.on {
                let base = mix(p.accent, p.bg, 0.12 * hov(h));
                (mix(base, p.bg, 0.25 * prs(h)), None, p.on_accent)
            } else {
                let base = mix(p.card, p.hover, hov(h));
                (mix(base, p.stroke, prs(h)), Some(p.stroke), p.text)
            };
            fluent::round_rect(hdc, &r, fill, stroke, radius, false);
            let g = match t.kind {
                TileKind::Camera => CAMERA,
                TileKind::Screen => glyph::PHONE,
                TileKind::Clipboard => glyph::CLIPBOARD,
                TileKind::SendFile => glyph::SEND,
                TileKind::Pair if st.model.peer.is_some() => glyph::KEYBOARD,
                TileKind::Pair => glyph::ADD,
            };
            let ir = RECT {
                left: r.left + px(s, 12),
                top: r.top + px(s, 10),
                right: r.left + px(s, 36),
                bottom: r.top + px(s, 34),
            };
            fluent::icon(hdc, f.icon, g, ir, fg);
            if t.kind == TileKind::Camera {
                // The camera status window stays reachable even while the
                // tile itself is dimmed.
                let c = Hit::CameraInfo;
                let over = if t.on { p.on_accent } else { p.stroke };
                if hov(c) > 0.0 || prs(c) > 0.0 {
                    let f2 = mix(fill, over, 0.2 * hov(c) + 0.2 * prs(c));
                    fluent::round_rect(hdc, &l.camera_info, f2, None, px(s, 6), false);
                }
                let cf = if t.enabled { fg } else { p.text2 };
                fluent::icon(hdc, f.small, CHEVRON, l.camera_info, cf);
            }
            let tr = RECT {
                left: r.left + px(s, 12),
                top: r.top + px(s, 38),
                right: r.right - px(s, 6),
                bottom: r.bottom - px(s, 4),
            };
            fluent::text(hdc, f.small, &t.label, tr, fg, DT_LEFT | DT_WORDBREAK);
        }

        // Status line: the phone and the last activity.
        fluent::text(
            hdc,
            f.small,
            &status_line(&st.model, STATUS_CHARS),
            l.status,
            p.text2,
            DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS,
        );
    });
}

fn hit(hwnd: HWND, x: i32, y: i32) -> Option<Hit> {
    let l = layout(fluent::scale(hwnd));
    let t = |k: TileKind| (l.tiles[tile_index(k)], Hit::Tile(k));
    [
        (l.settings, Hit::Settings),
        (l.close, Hit::Close),
        (l.drop, Hit::Drop),
        // Before the camera tile, which it sits on.
        (l.camera_info, Hit::CameraInfo),
        t(TileKind::Camera),
        t(TileKind::Screen),
        t(TileKind::Clipboard),
        t(TileKind::SendFile),
        t(TileKind::Pair),
    ]
    .into_iter()
    .find(|(r, _)| fluent::inside(r, x, y))
    .map(|(_, h)| h)
}

/// Where each eased value is heading: hovered and pressed targets are 1.
fn targets(st: &State) -> ([f32; SLOTS], [f32; SLOTS]) {
    let mut hover = [0.0; SLOTS];
    let mut press = [0.0; SLOTS];
    for (h, out) in [(st.hover, &mut hover), (st.pressed, &mut press)] {
        if let Some(h) = h.filter(|&h| clickable(&st.model, h)) {
            out[h.slot()] = 1.0;
        }
    }
    (hover, press)
}

/// Start the animation timer (if it is not running) after a hover or press
/// change, and repaint.
fn animate(hwnd: HWND) {
    if let Ok(mut g) = CURRENT.lock() {
        if let Some(st) = g.as_mut() {
            if !st.animating {
                st.animating = true;
                st.last_tick = Instant::now();
                // SAFETY: a timer on our own window; killed when settled or
                // on WM_DESTROY.
                unsafe { SetTimer(Some(hwnd), ANIM_TIMER, ANIM_MS, None) };
            }
        }
    }
    repaint(hwnd);
}

/// One animation step; stops the timer once every value is settled.
fn tick(hwnd: HWND) {
    if let Ok(mut g) = CURRENT.lock() {
        if let Some(st) = g.as_mut() {
            let now = Instant::now();
            let dt = now.duration_since(st.last_tick).as_millis().clamp(1, 100) as u32;
            st.last_tick = now;
            let (hover, press) = targets(st);
            let mut settled = true;
            for i in 0..SLOTS {
                st.hover_t[i] = ease::approach(st.hover_t[i], hover[i], dt);
                st.press_t[i] = ease::approach(st.press_t[i], press[i], dt);
                settled &= ease::settled(st.hover_t[i], hover[i])
                    && ease::settled(st.press_t[i], press[i]);
            }
            if settled {
                st.animating = false;
                // SAFETY: our own timer.
                let _ = unsafe { KillTimer(Some(hwnd), ANIM_TIMER) };
            }
        }
    }
    repaint(hwnd);
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
        // Empty areas act as a title bar: drag the flyout anywhere.
        WM_NCHITTEST => {
            let mut pt = POINT {
                x: (lparam.0 & 0xffff) as i16 as i32,
                y: ((lparam.0 >> 16) & 0xffff) as i16 as i32,
            };
            // SAFETY: maps a screen point into our client area.
            let _ = unsafe { ScreenToClient(hwnd, &mut pt) };
            if hit(hwnd, pt.x, pt.y).is_some() {
                LRESULT(HTCLIENT as isize)
            } else {
                LRESULT(HTCAPTION)
            }
        }
        // A popup must not maximise on a double-click of its drag area.
        WM_NCLBUTTONDBLCLK => LRESULT(0),
        WM_EXITSIZEMOVE => {
            let mut r = RECT::default();
            // SAFETY: our own window.
            if unsafe { GetWindowRect(hwnd, &mut r) }.is_ok() {
                if let Ok(mut p) = LAST_POS.lock() {
                    *p = Some((r.left, r.top));
                }
            }
            LRESULT(0)
        }
        WM_SETCURSOR if (lparam.0 & 0xffff) as u32 == HTCLIENT => {
            let mut pt = POINT::default();
            // SAFETY: cursor position mapped into our client area.
            unsafe {
                let _ = GetCursorPos(&mut pt);
                let _ = ScreenToClient(hwnd, &mut pt);
            }
            let h = hit(hwnd, pt.x, pt.y);
            let hand = h.is_some_and(|h| {
                CURRENT
                    .lock()
                    .ok()
                    .and_then(|g| g.as_ref().map(|s| clickable(&s.model, h)))
                    .unwrap_or(false)
            });
            fluent::set_cursor(hand);
            LRESULT(1)
        }
        WM_TIMER if wparam.0 == ANIM_TIMER => {
            tick(hwnd);
            LRESULT(0)
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
                animate(hwnd);
            }
            LRESULT(0)
        }
        WM_MOUSELEAVE => {
            if let Ok(mut g) = CURRENT.lock() {
                if let Some(st) = g.as_mut() {
                    st.hover = None;
                    st.pressed = None;
                    st.tracking = false;
                }
            }
            animate(hwnd);
            LRESULT(0)
        }
        WM_LBUTTONDOWN => {
            let (x, y) = xy();
            let h = hit(hwnd, x, y);
            if let Ok(mut g) = CURRENT.lock() {
                if let Some(st) = g.as_mut() {
                    st.pressed = h;
                }
            }
            animate(hwnd);
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            let (x, y) = xy();
            let mut paired = false;
            let mut target = None;
            if let Ok(mut g) = CURRENT.lock() {
                if let Some(st) = g.as_mut() {
                    let pressed = st.pressed.take();
                    paired = st.model.peer.is_some();
                    // A dimmed tile ignores the click, and so does a release
                    // away from where the press started.
                    target = release_target(pressed, hit(hwnd, x, y))
                        .filter(|&h| clickable(&st.model, h));
                }
            }
            animate(hwnd);
            match target {
                Some(Hit::Tile(TileKind::Clipboard)) => {
                    act(Action::ToggleClipboard);
                    if let Ok(mut g) = CURRENT.lock() {
                        if let Some(st) = g.as_mut() {
                            st.model.clipboard_on = !st.model.clipboard_on;
                        }
                    }
                    animate(hwnd);
                }
                Some(Hit::Tile(TileKind::SendFile) | Hit::Drop) => {
                    act(Action::SendFiles);
                    close();
                }
                Some(Hit::Tile(TileKind::Pair)) => {
                    act(if paired {
                        Action::ControlPhone
                    } else {
                        Action::Pair
                    });
                    close();
                }
                Some(Hit::Tile(TileKind::Screen)) => {
                    act(Action::ShowScreen);
                    close();
                }
                Some(Hit::Tile(TileKind::Camera)) => {
                    act(Action::ToggleCamera);
                    close();
                }
                Some(Hit::CameraInfo) => {
                    act(Action::CameraWindow);
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
            // SAFETY: our own timer; harmless if it is not running.
            let _ = unsafe { KillTimer(Some(hwnd), ANIM_TIMER) };
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
            w!("Relay"),
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
                pressed: None,
                tracking: false,
                hover_t: [0.0; SLOTS],
                press_t: [0.0; SLOTS],
                animating: false,
                last_tick: Instant::now(),
                on_action,
            });
        }
        let s = fluent::scale(hwnd);
        let (w, h) = (px(s, W), px(s, H));
        let mut pt = POINT::default();
        let _ = GetCursorPos(&mut pt);
        // Above-left of the tray icon, like Windows' own flyouts, unless the
        // user dragged it somewhere this session.
        let (mut x, mut y) = LAST_POS
            .lock()
            .ok()
            .and_then(|p| *p)
            .unwrap_or((pt.x - w + px(s, 24), pt.y - h - px(s, 16)));
        // Keep it fully on the monitor's work area (taskbar excluded).
        let mon = MonitorFromPoint(
            POINT {
                x: x + w / 2,
                y: y + h / 2,
            },
            MONITOR_DEFAULTTONEAREST,
        );
        let mut mi = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if GetMonitorInfoW(mon, &mut mi).as_bool() {
            let wa = mi.rcWork;
            x = x.clamp(wa.left, (wa.right - w).max(wa.left));
            y = y.clamp(wa.top, (wa.bottom - h).max(wa.top));
        }
        let _ = SetWindowPos(hwnd, Some(HWND_TOPMOST), x, y, w, h, SWP_NOACTIVATE);
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
