//! Fluent-style flyout opened by left-clicking the tray icon: a header with
//! the phone's name and status (the Home hero card's builder), a drop zone
//! for files, Quick Settings style toggle tiles (Camera, Phone screen,
//! Control phone or Pair device, Clipboard, Send files) under small group
//! captions, a status line with "Open Relay", plus settings and close.
//! Drawn with Direct2D over Mica (opaque token surface when Mica is
//! unavailable). Hover and press fade in and out (`ease`). It stays open
//! when another window is clicked, so files can be dragged in from
//! Explorer; it closes with Esc, the close button, the tray icon, or after
//! a drop.
#![allow(unsafe_code)]

use std::{cell::RefCell, sync::Mutex, time::Instant};

use windows::{
    core::{w, PCWSTR},
    Win32::{
        Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
        Graphics::Gdi::{
            GetMonitorInfoW, InvalidateRect, MonitorFromPoint, ScreenToClient, UpdateWindow,
            ValidateRect, MONITORINFO, MONITOR_DEFAULTTONEAREST,
        },
        System::LibraryLoader::GetModuleHandleW,
        UI::{
            Input::KeyboardAndMouse::{TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT, VK_ESCAPE},
            Shell::{DragAcceptFiles, DragFinish, DragQueryFileW, HDROP},
            WindowsAndMessaging::{
                CreateWindowExW, DefWindowProcW, DestroyWindow, GetCursorPos, GetWindowRect,
                KillTimer, LoadCursorW, RegisterClassW, SetForegroundWindow, SetTimer,
                SetWindowPos, ShowWindow, HTCLIENT, HWND_TOPMOST, IDC_ARROW, SWP_NOACTIVATE,
                SWP_NOZORDER, SW_SHOW, WM_DESTROY, WM_DPICHANGED, WM_DROPFILES, WM_ERASEBKGND,
                WM_KEYDOWN, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_PAINT, WM_SETCURSOR,
                WM_SIZE, WM_TIMER, WNDCLASSW, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
            },
        },
    },
};

use crate::{
    d2d::{self, Backdrop, Surface},
    ease,
    fluent::{self, glyph, px, rectf, round_rect, text, text_wrapped, Align, Canvas, Font, RectF},
    flyout_tiles::{release_target, status_line, tiles, TileKind},
    pages::home::{self, PhoneCard},
    tokens::RADIUS,
};

const WM_MOUSELEAVE: u32 = 0x02A3;
const CLASS: PCWSTR = w!("PhoneBridgeFlyout");
/// Logical (96-dpi) size of the flyout.
const W: i32 = 360;
const H: i32 = 412;
/// Logical width of the "Open Relay" button.
const OPEN_W: i32 = 112;
/// Hover and press animation tick (about 60 fps); killed once settled.
const ANIM_TIMER: usize = 1;
const ANIM_MS: u32 = 16;
/// Characters the status line keeps before it is clipped with an ellipsis.
const STATUS_CHARS: usize = 56;
/// Opacity of cards and tiles over Mica, so the backdrop shows through.
const MICA_ALPHA: f32 = 0.6;
const CAMERA: char = '\u{E722}';
const CHEVRON: char = '\u{E76C}';

/// What the flyout shows; filled in by the UI before opening.
#[derive(Clone, Default)]
pub struct Model {
    pub peer: Option<String>,
    /// The paired phone as Home's hero card shows it (`home::phone_card`).
    pub phone: Option<PhoneCard>,
    /// Unix seconds (whole minutes) for "Last seen 5 min ago".
    pub now: u64,
    pub clipboard_on: bool,
    /// The phone-screen window is open.
    pub screen_on: bool,
    /// The phone camera is streaming, and its frames per second.
    pub camera_on: bool,
    pub camera_fps: u32,
    pub last: String,
}

/// User actions the flyout reports back to the UI thread.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    ToggleClipboard,
    SendFiles,
    Pair,
    ControlPhone,
    ShowScreen,
    ToggleCamera,
    CameraWindow,
    Settings,
    /// The main window on Home.
    OpenRelay,
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

#[derive(Clone, Copy, Debug, PartialEq)]
enum Hit {
    Drop,
    Tile(TileKind),
    /// The chevron on the camera tile: the camera status window.
    CameraInfo,
    Settings,
    Close,
    OpenRelay,
}

const SLOTS: usize = 10;

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
            Hit::OpenRelay => 9,
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

thread_local! {
    /// The Direct2D surface (UI thread), keyed by the window it draws.
    static SURFACE: RefCell<Option<(isize, Surface)>> = const { RefCell::new(None) };
}

/// Layout rectangles in physical pixels for a given DPI scale.
struct Layout {
    /// Phone badge, name and status; not a hit, so it drags the flyout.
    header: RECT,
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
    open_relay: RECT,
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
    let settings = sq(W - 16 - 32 - 36, 20, 32);
    let open_relay = RECT {
        left: p(W - 16 - OPEN_W),
        ..row(364, 396)
    };
    Layout {
        header: RECT {
            left: p(16),
            top: p(16),
            right: settings.left - p(4),
            bottom: p(56),
        },
        badge: sq(16, 16, 40),
        settings,
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
        status: RECT {
            right: open_relay.left - p(8),
            ..row(364, 396)
        },
        open_relay,
    }
}

fn rf(r: RECT) -> RectF {
    rectf(r.left as f32, r.top as f32, r.right as f32, r.bottom as f32)
}

/// What `paint` needs, copied out of the state.
struct Snapshot {
    model: Model,
    hover_t: [f32; SLOTS],
    press_t: [f32; SLOTS],
}

fn paint(hwnd: HWND) {
    let snap = CURRENT.lock().ok().and_then(|g| {
        g.as_ref()
            .filter(|st| st.hwnd == hwnd.0 as isize)
            .map(|st| Snapshot {
                model: st.model.clone(),
                hover_t: st.hover_t,
                press_t: st.press_t,
            })
    });
    let painted = SURFACE.with(|cell| {
        let mut slot = cell.borrow_mut();
        match (snap.as_ref(), slot.as_mut()) {
            (Some(snap), Some((h, surface))) if *h == hwnd.0 as isize => {
                let mica = surface.backdrop() == Backdrop::Mica;
                fluent::paint_with(surface, hwnd, |c, _| draw(c, snap, mica));
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

/// Hover and press as translucent layers over whatever is below, so the
/// same drawing works opaque and over Mica.
fn states(c: &Canvas, r: &RectF, hov: f32, prs: f32, radius: f32, over: (u32, u32)) {
    if hov > 0.0 {
        round_rect(c, r, Some((over.0, hov)), None, radius, false);
    }
    if prs > 0.0 {
        round_rect(c, r, Some((over.1, prs)), None, radius, false);
    }
}

fn draw(c: &mut Canvas, snap: &Snapshot, mica: bool) {
    let (s, p) = (c.s, c.p);
    let l = layout(s);
    let m = &snap.model;
    let hov = |h: Hit| snap.hover_t[h.slot()];
    let prs = |h: Hit| snap.press_t[h.slot()];
    let a = if mica { MICA_ALPHA } else { 1.0 };
    let (small, big) = (RADIUS[0] * s, RADIUS[1] * s);

    // Header: the phone as Home's hero card shows it, settings and close.
    let badge = rf(l.badge);
    let (title, sub, sub_rgb, tint) = match &m.phone {
        Some(card) => {
            let tint = if card.connected { p.accent } else { p.text2 };
            match home::warning(card) {
                Some(w) => (card.name.clone(), w, p.warning, tint),
                None => (
                    card.name.clone(),
                    home::status_line(card, m.now),
                    p.text2,
                    tint,
                ),
            }
        }
        None => (
            "No device paired".to_owned(),
            "Pair a phone to start".to_owned(),
            p.text2,
            p.text2,
        ),
    };
    let half = (badge.right - badge.left) / 2.0;
    round_rect(c, &badge, Some((tint, 0.14)), None, half, false);
    fluent::icon_sized(c, Font::IconLarge, glyph::PHONE, badge, tint);
    let h = rf(l.header);
    let tx = badge.right + 12.0 * s;
    let mid = (h.top + h.bottom) / 2.0;
    text(
        c,
        Font::Subtitle,
        &title,
        rectf(tx, h.top, h.right, mid),
        p.text,
        Align::Left,
    );
    text(
        c,
        Font::Caption,
        &sub,
        rectf(tx, mid, h.right, h.bottom),
        sub_rgb,
        Align::Left,
    );
    for (r, g, hit) in [
        (l.settings, glyph::SETTINGS, Hit::Settings),
        (l.close, glyph::CLOSE, Hit::Close),
    ] {
        let r = rf(r);
        states(c, &r, hov(hit), prs(hit), small, (p.hover, p.stroke));
        fluent::icon(c, g, r, p.text2);
    }

    // Drop zone: icon and hint on one line.
    let drop = rf(l.drop);
    round_rect(c, &drop, Some((p.card, a)), None, big, false);
    states(
        c,
        &drop,
        hov(Hit::Drop),
        prs(Hit::Drop),
        big,
        (p.hover, p.stroke),
    );
    round_rect(c, &drop, None, Some(p.text2), big, true);
    let ic = rectf(
        drop.left + 16.0 * s,
        drop.top,
        drop.left + 44.0 * s,
        drop.bottom,
    );
    fluent::icon_sized(c, Font::IconLarge, glyph::UPLOAD, ic, p.text2);
    let label = match &m.peer {
        Some(n) => format!("Drop files here to send to {n}"),
        None => "Pair a device to send files".to_owned(),
    };
    let lr = rectf(
        ic.right + 8.0 * s,
        drop.top,
        drop.right - 12.0 * s,
        drop.bottom,
    );
    text(c, Font::Caption, &label, lr, p.text2, Align::Left);

    // Group captions and dividers.
    for (r, cap) in l
        .captions
        .iter()
        .zip(["Camera & Screen", "Connection", "Files"])
    {
        text(c, Font::Caption, cap, rf(*r), p.text2, Align::Left);
    }
    for r in &l.dividers {
        round_rect(c, &rf(*r), Some((p.stroke, 1.0)), None, 0.0, false);
    }

    // Tiles.
    for t in tiles(m) {
        let hit = Hit::Tile(t.kind);
        let r = rf(l.tiles[tile_index(t.kind)]);
        let fg = if !t.enabled {
            round_rect(c, &r, Some((p.card, a)), Some(p.stroke), small, false);
            mix(p.text, p.card, 0.6)
        } else if t.on {
            round_rect(c, &r, Some((p.accent, 1.0)), None, small, false);
            states(c, &r, 0.12 * hov(hit), 0.25 * prs(hit), small, (p.bg, p.bg));
            p.on_accent
        } else {
            round_rect(c, &r, Some((p.card, a)), Some(p.stroke), small, false);
            states(c, &r, hov(hit), prs(hit), small, (p.hover, p.stroke));
            p.text
        };
        let g = match t.kind {
            TileKind::Camera => CAMERA,
            TileKind::Screen => glyph::PHONE,
            TileKind::Clipboard => glyph::CLIPBOARD,
            TileKind::SendFile => glyph::SEND,
            TileKind::Pair if m.peer.is_some() => glyph::KEYBOARD,
            TileKind::Pair => glyph::ADD,
        };
        let ir = rectf(
            r.left + 12.0 * s,
            r.top + 10.0 * s,
            r.left + 36.0 * s,
            r.top + 34.0 * s,
        );
        fluent::icon(c, g, ir, fg);
        if t.kind == TileKind::Camera {
            // The camera status window stays reachable even while the
            // tile itself is dimmed.
            let ci = rf(l.camera_info);
            let over = if t.on { p.on_accent } else { p.stroke };
            let k = 0.2 * hov(Hit::CameraInfo) + 0.2 * prs(Hit::CameraInfo);
            if k > 0.0 {
                round_rect(c, &ci, Some((over, k)), None, small, false);
            }
            let cf = if t.enabled { fg } else { p.text2 };
            fluent::icon_sized(c, Font::IconSmall, CHEVRON, ci, cf);
        }
        let tr = rectf(
            r.left + 12.0 * s,
            r.top + 38.0 * s,
            r.right - 6.0 * s,
            r.bottom - 4.0 * s,
        );
        text_wrapped(c, Font::Caption, &t.label, tr, fg);
    }

    // Footer: the phone and the last activity, and "Open Relay".
    text(
        c,
        Font::Caption,
        &status_line(m, STATUS_CHARS),
        rf(l.status),
        p.text2,
        Align::Left,
    );
    let o = rf(l.open_relay);
    round_rect(c, &o, Some((p.card, 1.0)), Some(p.stroke), small, false);
    states(
        c,
        &o,
        hov(Hit::OpenRelay),
        prs(Hit::OpenRelay),
        small,
        (p.hover, p.stroke),
    );
    text(c, Font::Body, "Open Relay", o, p.text, Align::Center);
}

/// Blend two 0xRRGGBB colours: `t` = 0 gives `a`, 1 gives `b`.
fn mix(a: u32, b: u32, t: f32) -> u32 {
    let t = t.clamp(0.0, 1.0);
    let ch = |sh: u32| {
        let (x, y) = (((a >> sh) & 0xff) as f32, ((b >> sh) & 0xff) as f32);
        ((x + (y - x) * t).round() as u32) << sh
    };
    ch(0) | ch(8) | ch(16)
}

/// What is under client point (`x`, `y`); `None` is drag area.
fn hit_at(l: &Layout, x: i32, y: i32) -> Option<Hit> {
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
        (l.open_relay, Hit::OpenRelay),
    ]
    .into_iter()
    .find(|(r, _)| fluent::inside(r, x, y))
    .map(|(_, h)| h)
}

fn hit(hwnd: HWND, x: i32, y: i32) -> Option<Hit> {
    hit_at(&layout(fluent::scale(hwnd)), x, y)
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
        // Resize and present in the same message: no stale or black edges.
        WM_SIZE => {
            // SAFETY: synchronous repaint of our own window.
            unsafe {
                let _ = InvalidateRect(Some(hwnd), None, false);
                let _ = UpdateWindow(hwnd);
            }
            LRESULT(0)
        }
        // Dragged onto a monitor with another scale: take the size Windows
        // suggests, so the scaled layout fits the window.
        WM_DPICHANGED => {
            // SAFETY: lParam is the suggested RECT Windows passes for this message.
            let r = unsafe { *(lparam.0 as *const RECT) };
            // SAFETY: moves our own window to the rect Windows suggested.
            let _ = unsafe {
                SetWindowPos(
                    hwnd,
                    None,
                    r.left,
                    r.top,
                    r.right - r.left,
                    r.bottom - r.top,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                )
            };
            repaint(hwnd);
            LRESULT(0)
        }
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
                Some(Hit::OpenRelay) => {
                    act(Action::OpenRelay);
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
            fluent::unregister_window(hwnd);
            // SAFETY: our own timer; harmless if it is not running.
            let _ = unsafe { KillTimer(Some(hwnd), ANIM_TIMER) };
            // The surface's composition target refers to this window.
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

/// Open the flyout near the cursor (the tray icon), or close it if open.
pub fn toggle(model: Model, on_action: Box<dyn Fn(Action) + Send>) {
    if is_open() {
        close();
        return;
    }
    let made = d2d::make_window(Backdrop::Mica, |extra| {
        // SAFETY: class registration (a repeat fails harmlessly) and window
        // creation on the UI thread; the window stays hidden.
        unsafe {
            let hinstance = GetModuleHandleW(None).map_err(|e| e.to_string())?;
            let wc = WNDCLASSW {
                lpfnWndProc: Some(wndproc),
                hInstance: hinstance.into(),
                lpszClassName: CLASS,
                // Without a class cursor Windows keeps showing the busy cursor.
                hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
                // No background brush: every pixel is ours (WM_ERASEBKGND).
                ..Default::default()
            };
            RegisterClassW(&wc);
            CreateWindowExW(
                WS_EX_TOOLWINDOW | WS_EX_TOPMOST | extra,
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
            )
            .map_err(|e| e.to_string())
        }
    });
    // Nothing can draw (no Direct2D at all): no flyout.
    let Ok(d2d::Made { hwnd, surface, .. }) = made else {
        return;
    };
    // Only the final window: a discarded Mica attempt never gets here.
    SURFACE.with(|cell| *cell.borrow_mut() = Some((hwnd.0 as isize, surface)));
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
    // SAFETY: positioning and showing our own window on the UI thread.
    unsafe {
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
        fluent::register_window(hwnd);
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = SetForegroundWindow(hwnd);
    }
}

pub fn is_open() -> bool {
    CURRENT.lock().map(|g| g.is_some()).unwrap_or(false)
}

/// The phone's status changed while the flyout is open: redraw the header,
/// only when it really changed. UI thread only.
pub fn update_phone(phone: Option<PhoneCard>, now: u64) {
    let changed = CURRENT.lock().ok().and_then(|mut g| {
        let st = g.as_mut()?;
        let changed = st.model.phone != phone || st.model.now != now;
        st.model.phone = phone;
        st.model.now = now;
        changed.then_some(st.hwnd)
    });
    if let Some(h) = changed {
        repaint(HWND(h as *mut _));
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

    fn overlaps(a: &RECT, b: &RECT) -> bool {
        a.left < b.right && b.left < a.right && a.top < b.bottom && b.top < a.bottom
    }

    #[test]
    fn flyout_has_open_relay_and_header() {
        for s in [1.0, 1.5, 2.0] {
            let l = layout(s);
            let (h, o) = (l.header, l.open_relay);
            assert!(h.right > h.left && h.bottom > h.top, "header at {s}");
            assert!(o.right > o.left && o.bottom > o.top, "Open Relay at {s}");
            let (w, ht) = (px(s, W), px(s, H));
            for r in [h, o] {
                assert!(r.left >= 0 && r.right <= w, "{r:?} fits the width at {s}");
                assert!(r.top >= 0 && r.bottom <= ht, "{r:?} fits the height at {s}");
                for t in l.tiles.iter().chain([&l.drop, &l.settings, &l.close]) {
                    assert!(!overlaps(&r, t), "{r:?} overlaps {t:?} at {s}");
                }
            }
            assert!(
                !overlaps(&o, &l.status),
                "Open Relay beside the status line"
            );
            assert!(!overlaps(&h, &o));
            assert!(l.tiles.iter().all(|t| t.bottom <= ht));
            // The button is a hit; the header stays a drag area.
            let (x, y) = ((o.left + o.right) / 2, (o.top + o.bottom) / 2);
            assert!(matches!(hit_at(&l, x, y), Some(Hit::OpenRelay)));
            let (x, y) = (l.badge.right + px(s, 20), (h.top + h.bottom) / 2);
            assert!(fluent::inside(&h, x, y));
            assert!(hit_at(&l, x, y).is_none(), "header text drags the flyout");
        }
    }

    #[test]
    fn every_hit_has_its_own_slot() {
        let hits = [
            Hit::Drop,
            Hit::Tile(TileKind::Camera),
            Hit::Tile(TileKind::Screen),
            Hit::Tile(TileKind::Clipboard),
            Hit::Tile(TileKind::SendFile),
            Hit::Tile(TileKind::Pair),
            Hit::CameraInfo,
            Hit::Settings,
            Hit::Close,
            Hit::OpenRelay,
        ];
        let mut slots: Vec<usize> = hits.iter().map(|h| h.slot()).collect();
        slots.sort();
        slots.dedup();
        assert_eq!(slots.len(), SLOTS);
        assert!(slots.iter().all(|&i| i < SLOTS));
    }

    #[test]
    fn hit_testing_finds_tiles_and_the_camera_chevron() {
        let l = layout(1.5);
        let cam = l.tiles[tile_index(TileKind::Camera)];
        assert!(matches!(
            hit_at(&l, cam.left + 2, cam.bottom - 2),
            Some(Hit::Tile(TileKind::Camera))
        ));
        let ci = l.camera_info;
        assert!(matches!(
            hit_at(&l, ci.left + 2, ci.top + 2),
            Some(Hit::CameraInfo)
        ));
        assert!(hit_at(&l, 1, 1).is_none());
    }

    #[cfg(windows)]
    #[test]
    fn flyout_draws_into_a_hidden_window_with_and_without_a_phone() {
        use crate::pages::home::Path;
        let hwnd = crate::d2d::test_window();
        let mut surface = Surface::new(hwnd, Backdrop::Opaque).expect("surface");
        let card = PhoneCard {
            name: "RMX3561".into(),
            fingerprint: "abcd 1234".into(),
            connected: true,
            last_seen: None,
            path: Path::Usb,
            changed: false,
        };
        let phones = [
            None,
            Some(card.clone()),
            Some(PhoneCard {
                changed: true,
                connected: false,
                ..card
            }),
        ];
        for phone in phones {
            let snap = Snapshot {
                model: Model {
                    peer: phone.as_ref().map(|c| c.name.clone()),
                    phone,
                    camera_on: true,
                    camera_fps: 30,
                    ..Default::default()
                },
                hover_t: [0.5; SLOTS],
                press_t: [0.25; SLOTS],
            };
            for mica in [false, true] {
                surface
                    .draw(|dc| {
                        let mut c = Canvas {
                            dc,
                            s: 1.5,
                            p: fluent::palette(),
                        };
                        draw(&mut c, &snap, mica);
                    })
                    .expect("draw");
            }
        }
        drop(surface);
        // SAFETY: our own test window, made above on this thread.
        let _ = unsafe { DestroyWindow(hwnd) };
    }
}
