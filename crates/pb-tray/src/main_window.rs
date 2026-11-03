//! The Relay main window: Mica backdrop (opaque token surface when Mica is
//! unavailable), a custom title bar with the standard caption buttons, a
//! nav rail (Home, Timeline, Devices, Settings) and a content pane drawn by
//! `pages`. The Settings page's search box is a native edit (`search_box`)
//! shown only on that page. Size and position are remembered in the
//! settings. Everything
//! here runs on the UI thread (the tray's message loop), so the window's
//! state lives in thread-locals; a borrow is never held across a call that
//! can send messages back into `wndproc`.
#![allow(unsafe_code)]

use std::{
    cell::RefCell,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, OnceLock,
    },
    time::{Duration, Instant},
};

use windows::{
    core::{w, PCWSTR},
    Win32::{
        Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
        Graphics::{
            Direct2D::D2D1_ANTIALIAS_MODE_ALIASED,
            Gdi::{
                EnumDisplayMonitors, GetMonitorInfoW, InvalidateRect, MonitorFromPoint,
                ScreenToClient, UpdateWindow, ValidateRect, HDC, HMONITOR, MONITORINFO,
                MONITOR_DEFAULTTONEAREST,
            },
        },
        System::LibraryLoader::GetModuleHandleW,
        UI::{
            HiDpi::{GetDpiForMonitor, GetDpiForWindow, GetSystemMetricsForDpi, MDT_EFFECTIVE_DPI},
            Input::KeyboardAndMouse::{
                GetFocus, GetKeyState, ReleaseCapture, SetCapture, SetFocus, TrackMouseEvent,
                TME_LEAVE, TME_NONCLIENT, TRACKMOUSEEVENT, VK_DOWN, VK_END, VK_HOME, VK_NEXT,
                VK_PRIOR, VK_RETURN, VK_SHIFT, VK_SPACE, VK_TAB, VK_UP,
            },
            WindowsAndMessaging::{
                CreateWindowExW, DefWindowProcW, GetClientRect, GetCursorPos, GetWindowRect,
                IsIconic, IsZoomed, KillTimer, LoadCursorW, LoadIconW, PostMessageW,
                RegisterClassW, SetCursor, SetForegroundWindow, SetTimer, SetWindowPos, ShowWindow,
                EN_CHANGE, EN_KILLFOCUS, EN_SETFOCUS, HTCLIENT, HTCLOSE, HTMAXBUTTON, HTMINBUTTON,
                IDC_ARROW, IDC_HAND, MINMAXINFO, NCCALCSIZE_PARAMS, SC_CLOSE, SC_MAXIMIZE,
                SC_MINIMIZE, SC_RESTORE, SIZE_RESTORED, SM_CXFRAME, SM_CXPADDEDBORDER,
                SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SW_RESTORE,
                SW_SHOW, WM_CAPTURECHANGED, WM_COMMAND, WM_CTLCOLOREDIT, WM_DESTROY, WM_DPICHANGED,
                WM_ERASEBKGND, WM_EXITSIZEMOVE, WM_GETMINMAXINFO, WM_KEYDOWN, WM_LBUTTONDOWN,
                WM_LBUTTONUP, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_NCACTIVATE, WM_NCCALCSIZE,
                WM_NCHITTEST, WM_NCLBUTTONDBLCLK, WM_NCLBUTTONDOWN, WM_NCLBUTTONUP,
                WM_NCMOUSELEAVE, WM_NCMOUSEMOVE, WM_PAINT, WM_SETCURSOR, WM_SIZE, WM_SYSCOMMAND,
                WM_TIMER, WNDCLASSW, WS_CLIPCHILDREN, WS_EX_APPWINDOW, WS_OVERLAPPEDWINDOW,
            },
        },
    },
};

use crate::{
    app_name::APP_NAME,
    core::{Log, SharedSettings},
    d2d::{self, Backdrop, Surface},
    ease::Anim,
    fluent::{self, rectf, round_rect, text, Align, Canvas, Font, Palette, RectF},
    focus,
    pages::{self, settings, timeline, Hit, Model},
    search_box::{self, SearchBox},
    titlebar::{self, Caption},
    tokens::{motion, RADIUS, SPACE},
    widgets::{
        self,
        scrollbar::{self, Bar, Look, Part},
        Scroll,
    },
};

const CLASS: PCWSTR = w!("PhoneBridgeMain");
const WM_MOUSELEAVE: u32 = 0x02A3;
/// Default and minimum window sizes, nav rail width, in logical px.
const DEFAULT_W: f32 = 1000.0;
const DEFAULT_H: f32 = 700.0;
const MIN_W: f32 = 640.0;
const MIN_H: f32 = 480.0;
const RAIL_W: f32 = 80.0;
/// Frame timer: about 60 fps, only while something animates.
const FRAME_TIMER: usize = 1;
const FRAME_MS: u32 = 16;
/// Focus ids: nav items are 1..=4, the Settings search box `SEARCH_FOCUS`,
/// page hits `HIT_BASE + id`.
const HIT_BASE: u32 = 100;
const SEARCH_FOCUS: u32 = 5;
/// A saved rect must show this much of its caption strip on one monitor.
const MIN_VISIBLE_W: i32 = 64;
const MIN_VISIBLE_H: i32 = 20;
const CAPTION_STRIP: i32 = 40;
/// Control id of the Settings search box.
const ID_SEARCH: u16 = 1;

/// A saved window rectangle: x, y, width, height in screen pixels.
pub type SavedRect = (i32, i32, i32, i32);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    Home,
    Timeline,
    Devices,
    Settings,
}

impl Page {
    /// Nav rail order.
    pub const ALL: [Page; 4] = [Page::Home, Page::Timeline, Page::Devices, Page::Settings];

    fn index(self) -> usize {
        self as usize
    }

    fn label(self) -> &'static str {
        match self {
            Page::Home => "Home",
            Page::Timeline => "Timeline",
            Page::Devices => "Devices",
            Page::Settings => "Settings",
        }
    }

    fn glyph(self) -> char {
        match self {
            Page::Home => '\u{E80F}',
            Page::Timeline => '\u{E81C}',
            Page::Devices => '\u{E8EA}',
            Page::Settings => '\u{E713}',
        }
    }
}

/// What the window needs from the tray: the log, the settings (for the
/// saved rect), a way to build the page model and a handler for page
/// actions other than switching pages (it must only post messages).
pub struct Host {
    pub log: Log,
    pub settings: SharedSettings,
    pub settings_path: PathBuf,
    pub model: Box<dyn Fn() -> Model + Send + Sync>,
    pub act: Box<dyn Fn(pages::Action) + Send + Sync>,
    /// Asks which paired devices still answer as themselves (debounced by
    /// the tray); called when the window enters Settings or Devices.
    pub check_peers: Box<dyn Fn() + Send + Sync>,
}

static HOST: OnceLock<Host> = OnceLock::new();
/// The first paint is logged once per process.
static FIRST_PAINT: AtomicBool = AtomicBool::new(false);

/// Called once by the tray before anything opens the window.
pub fn init(host: Host) {
    let _ = HOST.set(host);
}

/// Something the pointer or keyboard can act on in the client area.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Target {
    Nav(Page),
    Hit(u32),
}

fn focus_id(t: Target) -> u32 {
    match t {
        Target::Nav(p) => p.index() as u32 + 1,
        Target::Hit(id) => HIT_BASE + id,
    }
}

fn target_of(id: u32) -> Option<Target> {
    match id {
        1..=4 => Some(Target::Nav(Page::ALL[id as usize - 1])),
        n if n >= HIT_BASE => Some(Target::Hit(n - HIT_BASE)),
        _ => None,
    }
}

/// Tab order: the nav rail, the search box (Settings), then the page's hits
/// in layout order.
fn focus_order(hits: &[Hit], search: bool) -> Vec<u32> {
    Page::ALL
        .iter()
        .map(|&p| focus_id(Target::Nav(p)))
        .chain(search.then_some(SEARCH_FOCUS))
        .chain(hits.iter().map(|h| focus_id(Target::Hit(h.id))))
        .collect()
}

/// Where Tab (Shift+Tab when `back`) goes from the focus ring `focus`, or
/// from the search box when the keys are in it (`in_search`).
fn tab(order: &[u32], focus: Option<u32>, in_search: bool, back: bool) -> Option<u32> {
    let from = if in_search { Some(SEARCH_FOCUS) } else { focus };
    focus::next(order, from, back)
}

/// The window's regions in client pixels.
struct Frame {
    cap: Caption,
    pane: RectF,
    /// The pane minus its padding: what a page lays out into.
    content: RectF,
    nav: Vec<(RectF, Page)>,
}

fn frame(w: f32, h: f32, s: f32) -> Frame {
    let cap = titlebar::layout(w, s);
    let top = cap.close.bottom;
    let rail = rectf(0.0, top, RAIL_W * s, h);
    let pane = rectf(rail.right, top, w.max(rail.right), h.max(top));
    let pad = SPACE[4] * s;
    let content = rectf(
        pane.left + pad,
        pane.top + pad,
        (pane.right - pad).max(pane.left + pad),
        (pane.bottom - pad).max(pane.top + pad),
    );
    let labels = Page::ALL.map(Page::label);
    let nav = widgets::nav_rail(rail, s, &labels, 0)
        .into_iter()
        .map(|(r, i)| (r, Page::ALL[i]))
        .collect();
    Frame {
        cap,
        pane,
        content,
        nav,
    }
}

fn within(r: &RectF, x: f32, y: f32) -> bool {
    x >= r.left && x < r.right && y >= r.top && y < r.bottom
}

fn target_at(f: &Frame, hits: &[Hit], x: f32, y: f32) -> Option<Target> {
    if let Some((_, p)) = f.nav.iter().find(|(r, _)| within(r, x, y)) {
        return Some(Target::Nav(*p));
    }
    if !within(&f.pane, x, y) {
        return None;
    }
    hits.iter()
        .find(|h| within(&h.r, x, y))
        .map(|h| Target::Hit(h.id))
}

/// The page's full content rect, moved up by the scroll offset (logical px).
fn scrolled(content: RectF, s: f32, sc: &Scroll) -> RectF {
    let off = sc.offset * s;
    let h = (sc.content * s).max(content.bottom - content.top);
    rectf(
        content.left,
        content.top - off,
        content.right,
        content.top - off + h,
    )
}

/// The saved rect if its caption strip is visible enough on some monitor
/// (`monitors` are work areas) to be grabbed; otherwise None (the monitor
/// was unplugged or the layout changed), and the window opens centred.
pub fn restore_rect(saved: Option<SavedRect>, monitors: &[RECT]) -> Option<SavedRect> {
    let (x, y, w, h) = saved?;
    // A crafted or corrupt value whose far edge does not fit an i32: ignored.
    if w <= 0 || h <= 0 || x.checked_add(w).is_none() || y.checked_add(h).is_none() {
        return None;
    }
    // In i64: a monitor edge minus a far-off strip edge can overflow i32.
    let wide = |v: i32| i64::from(v);
    let strip = (
        wide(x),
        wide(y),
        wide(x) + wide(w),
        wide(y) + wide(h.min(CAPTION_STRIP)),
    );
    let visible = monitors.iter().any(|m| {
        let iw = strip.2.min(wide(m.right)) - strip.0.max(wide(m.left));
        let ih = strip.3.min(wide(m.bottom)) - strip.1.max(wide(m.top));
        iw >= wide(MIN_VISIBLE_W) && ih >= wide(MIN_VISIBLE_H)
    });
    visible.then_some((x, y, w, h))
}

/// WM_KEYDOWN's lParam bit 30: the key was already down (auto-repeat).
pub fn is_key_repeat(lparam: isize) -> bool {
    lparam & (1 << 30) != 0
}

/// WM_DPICHANGED: the rect Windows suggests and the new scale.
pub fn on_dpi(suggested: RECT, dpi: u32) -> (RECT, f32) {
    (suggested, dpi.max(96) as f32 / 96.0)
}

/// The smallest window size at scale `s`, so the caption never collapses.
fn min_size(s: f32) -> (i32, i32) {
    ((MIN_W * s).round() as i32, (MIN_H * s).round() as i32)
}

struct State {
    hwnd: isize,
    page: Page,
    model: Model,
    /// Current DPI scale (updated before WM_DPICHANGED resizes the window).
    scale: f32,
    active: bool,
    /// Caption button (`HT*`) under the pointer / held down.
    cap_hover: Option<u32>,
    cap_pressed: Option<u32>,
    nc_tracking: bool,
    tracking: bool,
    nav_hover: Option<Page>,
    /// Hover fade per nav item, indexed like `Page::ALL`.
    nav_fade: [Anim; 4],
    pressed: Option<Target>,
    /// Keyboard focus (a focus id); shown as a ring, cleared by the mouse.
    focus: Option<u32>,
    scroll: [Scroll; 4],
    /// The pointer is over the scrollbar's hit area; its fade (0 = thin).
    bar_hover: bool,
    bar_fade: Anim,
    drag: Option<Drag>,
    /// The clickable thing under the pointer (hover fill, hand cursor).
    hover: Option<Target>,
    /// What the Timeline model was built from (the host's, every day on its
    /// first-view rule); the window applies the user's open / closed days.
    tl_base: Arc<timeline::TimelineModel>,
    timer: bool,
    /// The last restored (not maximized or minimized) window rect.
    normal: Option<SavedRect>,
    opened: Instant,
    timing: Timing,
    /// The Settings page picked in its second column, and the search text.
    sub: settings::Page,
    query: String,
    search_focused: bool,
    /// The page last entered (`note_page`).
    entered: Option<Page>,
}

/// A press on the scrollbar, until the button comes up.
#[derive(Clone, Copy, Debug)]
enum Drag {
    /// The thumb, grabbed at pointer `grab_y` when the offset was `start`.
    Thumb { grab_y: f32, start: f32 },
    /// The track held down: pages toward `y` at `next`, then repeatedly.
    Track { y: f32, next: Instant },
}

impl State {
    fn animating(&self, now: Instant) -> bool {
        self.nav_fade.iter().any(|a| a.running(now))
            || self.bar_fade.running(now)
            || matches!(self.drag, Some(Drag::Track { .. }))
            || self.scroll.iter().any(|s| s.animating(now))
    }

    /// The current page's scrollbar for a frame `f`; its offset is where
    /// the scroll is heading, so a held track click pages from there.
    fn bar(&self, f: &Frame) -> Bar {
        let sc = &self.scroll[self.page.index()];
        Bar {
            content: sc.content,
            view: sc.view,
            offset: sc.target(),
            top: f.content.top,
            len: f.content.bottom - f.content.top,
            s: self.scale,
        }
    }
}

thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
    /// The Direct2D surface, apart from `STATE` so painting never needs both
    /// borrowed (COM objects stay on the UI thread).
    static SURFACE: RefCell<Option<(isize, Surface)>> = const { RefCell::new(None) };
    /// The Settings search box, apart from `STATE` because its calls send
    /// notifications back into `wndproc`: only `borrow()` it around a call.
    static SEARCH: RefCell<Option<SearchBox>> = const { RefCell::new(None) };
    /// The palette of the last paint, to repaint the search box on a theme change.
    static PAINTED: std::cell::Cell<Option<Palette>> = const { std::cell::Cell::new(None) };
}

/// Runs `f` on the search box, if there is one. `f` may send messages to
/// the window (the box is only borrowed shared).
fn with_search<R>(f: impl FnOnce(&SearchBox) -> R) -> Option<R> {
    SEARCH.with(|cell| cell.borrow().as_ref().map(f))
}

/// Runs `f` on the window's state if `hwnd` is the main window. `f` must not
/// call anything that sends messages to the window.
fn with<R>(hwnd: HWND, f: impl FnOnce(&mut State) -> R) -> Option<R> {
    STATE.with(|cell| {
        let mut g = cell.borrow_mut();
        g.as_mut().filter(|st| st.hwnd == hwnd.0 as isize).map(f)
    })
}

fn current() -> Option<HWND> {
    STATE.with(|cell| cell.borrow().as_ref().map(|st| HWND(st.hwnd as *mut _)))
}

pub fn is_open() -> bool {
    current().is_some()
}

/// Whether the Settings page is showing (the tray builds its rows only then).
pub fn shows_settings() -> bool {
    STATE.with(|cell| {
        cell.try_borrow()
            .ok()
            .is_some_and(|g| g.as_ref().is_some_and(|st| st.page == Page::Settings))
    })
}

/// After the page may have changed: entering Settings or Devices asks for
/// a peer check, and Settings rebuilds the model (its rows are built only
/// while it shows). Call with no borrow of the state held.
fn note_page(hwnd: HWND) {
    let Some(Some(page)) = with(hwnd, |st| {
        (st.entered != Some(st.page)).then(|| {
            st.entered = Some(st.page);
            st.page
        })
    }) else {
        return;
    };
    if matches!(page, Page::Settings | Page::Devices) {
        if let Some(host) = HOST.get() {
            (host.check_peers)();
        }
    }
    if page == Page::Settings {
        refresh();
    }
}

/// Whether the window shows something that needs the camera status (the
/// Settings page on Camera, or a search that may list it): measuring it
/// costs a little.
pub fn wants_camera() -> bool {
    STATE.with(|cell| {
        cell.try_borrow().ok().is_some_and(|g| {
            g.as_ref().is_some_and(|st| {
                st.page == Page::Settings
                    && (st.sub == settings::Page::Camera || !st.query.trim().is_empty())
            })
        })
    })
}

/// Shows the search box where the Settings page wants it, or hides it on
/// the other pages. Call with no borrow of the state held.
fn sync_search(hwnd: HWND) {
    let (w, h) = client_size(hwnd);
    let Some(place) = with(hwnd, |st| {
        (st.page == Page::Settings).then(|| {
            let f = frame(w, h, st.scale);
            (settings::search_rect(f.content, f.pane, st.scale), st.scale)
        })
    }) else {
        return;
    };
    with_search(|sb| match place {
        Some((r, s)) => {
            sb.place(r, s);
            sb.show(true);
        }
        None => sb.show(false),
    });
}

/// Starts the Settings rows from the top (a new page or a new search).
fn rewind_settings(st: &mut State) {
    let sc = &mut st.scroll[Page::Settings.index()];
    *sc = Scroll::new(sc.content, sc.view);
}

/// The search box's notifications: the text changed, or it gained or lost
/// the focus.
fn on_search(hwnd: HWND, code: u32) {
    match code {
        EN_CHANGE => {
            let q = with_search(|sb| settings::clamp_query(&sb.text())).unwrap_or_default();
            let changed = with(hwnd, |st| {
                if st.query == q {
                    return false;
                }
                st.query = q.clone();
                st.model.settings.query = q;
                rewind_settings(st);
                true
            });
            if changed == Some(true) {
                // A search may list the camera card: fetch its status now.
                refresh();
                fit_scroll(hwnd);
                repaint(hwnd);
            }
        }
        EN_SETFOCUS | EN_KILLFOCUS => {
            with(hwnd, |st| {
                st.search_focused = code == EN_SETFOCUS;
                // The keys go to the box now: no ring elsewhere.
                if st.search_focused {
                    st.focus = None;
                }
            });
            repaint(hwnd);
        }
        _ => {}
    }
}

fn client_size(hwnd: HWND) -> (f32, f32) {
    let mut r = RECT::default();
    // SAFETY: reads our own window's client rect.
    let _ = unsafe { GetClientRect(hwnd, &mut r) };
    ((r.right - r.left) as f32, (r.bottom - r.top) as f32)
}

/// The frame and the current page's hits, from the live client size.
fn layout_now(hwnd: HWND, st: &State) -> (Frame, Vec<Hit>) {
    let (w, h) = client_size(hwnd);
    let f = frame(w, h, st.scale);
    let sc = &st.scroll[st.page.index()];
    let hits = pages::layout(
        st.page,
        scrolled(f.content, st.scale, sc),
        f.pane,
        st.scale,
        &st.model,
    );
    (f, hits)
}

/// Re-measures every page's scroll range for the current size and model.
fn fit_scroll(hwnd: HWND) {
    let (w, h) = client_size(hwnd);
    with(hwnd, |st| {
        let f = frame(w, h, st.scale);
        let (cw, ch) = (
            f.content.right - f.content.left,
            f.content.bottom - f.content.top,
        );
        for p in Page::ALL {
            let sc = &mut st.scroll[p.index()];
            sc.view = ch / st.scale;
            sc.content = pages::content_height(p, cw, st.scale, &st.model);
            sc.clamp();
        }
    });
}

fn repaint(hwnd: HWND) {
    // SAFETY: repaint request on our own window.
    let _ = unsafe { InvalidateRect(Some(hwnd), None, false) };
}

/// Runs the frame timer exactly while something animates.
fn sync_timer(hwnd: HWND) {
    let now = Instant::now();
    let Some((want, had)) = with(hwnd, |st| {
        let want = st.animating(now);
        let had = std::mem::replace(&mut st.timer, want);
        (want, had)
    }) else {
        return;
    };
    // SAFETY: our own window's timer; killed when idle and on WM_DESTROY.
    unsafe {
        if want && !had {
            SetTimer(Some(hwnd), FRAME_TIMER, FRAME_MS, None);
        } else if !want && had {
            let _ = KillTimer(Some(hwnd), FRAME_TIMER);
        }
    }
}

fn set_nav_hover(st: &mut State, h: Option<Page>, now: Instant) -> bool {
    if st.nav_hover == h {
        return false;
    }
    st.nav_hover = h;
    for p in Page::ALL {
        let target = if h == Some(p) { 1.0 } else { 0.0 };
        let a = st.nav_fade[p.index()];
        if a.to != target {
            st.nav_fade[p.index()] = Anim::new(a.value(now), target, now, motion::FAST_MS);
        }
    }
    true
}

/// Starts the scrollbar's widen / thin fade when its hover (or drag) changes.
fn set_bar_hover(st: &mut State, over: bool, now: Instant) {
    st.bar_hover = over;
    let target = if over || st.drag.is_some() { 1.0 } else { 0.0 };
    if st.bar_fade.to != target {
        let from = st.bar_fade.value(now);
        st.bar_fade = Anim::new(from, target, now, motion::FAST_MS);
    }
}

/// Recomputes what is under the pointer at client (x, y) (scrollbar first,
/// then nav and page hits). True when something visible changed.
fn point_at(hwnd: HWND, st: &mut State, x: f32, y: f32, now: Instant) -> bool {
    let (f, hits) = layout_now(hwnd, st);
    let on_bar = st.drag.is_some() || st.bar(&f).part_at(f.pane.right, x, y).is_some();
    if st.bar_hover != on_bar {
        set_bar_hover(st, on_bar, now);
    }
    let t = if on_bar {
        None
    } else {
        target_at(&f, &hits, x, y)
    };
    let nav = match t {
        Some(Target::Nav(p)) => Some(p),
        _ => None,
    };
    let hover_changed = std::mem::replace(&mut st.hover, t) != t;
    set_nav_hover(st, nav, now) | hover_changed
}

#[cfg(test)]
thread_local! {
    /// The scripted run (`ux`) stands in for the real pointer.
    static FAKE_CURSOR: std::cell::Cell<Option<Option<(f32, f32)>>> =
        const { std::cell::Cell::new(None) };
}

/// The pointer's client position, if it is over our window.
fn cursor_client(hwnd: HWND) -> Option<(f32, f32)> {
    #[cfg(test)]
    if let Some(p) = FAKE_CURSOR.with(|c| c.get()) {
        return p;
    }
    let mut pt = POINT::default();
    // SAFETY: plain queries on our own window.
    unsafe {
        GetCursorPos(&mut pt).ok()?;
        ScreenToClient(hwnd, &mut pt).as_bool().then_some(())?;
    }
    let (w, h) = client_size(hwnd);
    let (x, y) = (pt.x as f32, pt.y as f32);
    (x >= 0.0 && y >= 0.0 && x < w && y < h).then_some((x, y))
}

/// Re-tests hover after the content moved under a still pointer.
fn rehover(hwnd: HWND, st: &mut State, now: Instant) {
    if st.drag.is_none() {
        if let Some((x, y)) = cursor_client(hwnd) {
            point_at(hwnd, st, x, y, now);
        }
    }
}

/// Brings the focused page hit into the pane (Tab to something off screen).
fn reveal_focus(hwnd: HWND, st: &mut State, now: Instant) {
    let Some(id) = st.focus.filter(|&id| id >= HIT_BASE) else {
        return;
    };
    let (f, hits) = layout_now(hwnd, st);
    let Some(h) = hits.iter().find(|h| focus_id(Target::Hit(h.id)) == id) else {
        return;
    };
    let margin = SPACE[1] * st.scale;
    let sc = &mut st.scroll[st.page.index()];
    let shift = if h.r.top < f.content.top + margin {
        h.r.top - f.content.top - margin
    } else if h.r.bottom > f.content.bottom - margin {
        h.r.bottom - f.content.bottom + margin
    } else {
        return;
    };
    let to = sc.offset + shift / st.scale;
    sc.ease_to(to, now);
}

fn switch(st: &mut State, page: Page) {
    if st.page == page {
        return;
    }
    st.page = page;
    st.pressed = None;
    // A focused hit belonged to the old page.
    if st.focus.is_some_and(|id| id >= HIT_BASE) {
        st.focus = None;
    }
}

/// Opens or closes Timeline days through `change`, keeping what the user
/// was looking at where it was (a day closing above must not make the page
/// jump); the scroll range is re-measured and clamped.
fn retimeline(
    hwnd: HWND,
    change: impl FnOnce(&mut pages::timeline_view::Expansion, &timeline::TimelineModel),
) {
    let anchor = with(hwnd, |st| {
        let ix = Page::Timeline.index();
        let anchor = st.model.timeline.anchor(st.scroll[ix].offset);
        let mut exp = st.model.timeline.exp.clone();
        change(&mut exp, &st.model.timeline);
        st.model.timeline = Arc::new(st.tl_base.with_expansion(&exp));
        anchor
    })
    .flatten();
    fit_scroll(hwnd);
    with(hwnd, |st| {
        let held = anchor.and_then(|a| st.model.timeline.restore(&a));
        let sc = &mut st.scroll[Page::Timeline.index()];
        match held {
            Some(o) => sc.jump(o),
            None => sc.clamp(),
        }
    });
}

/// A click, Enter or Space on `t`.
fn activate(hwnd: HWND, t: Target) {
    let action = with(hwnd, |st| match t {
        Target::Nav(p) => {
            switch(st, p);
            None
        }
        Target::Hit(id) => {
            let (_, hits) = layout_now(hwnd, st);
            match hits
                .iter()
                .find(|h| h.id == id)
                .and_then(|h| pages::click(st.page, h, &st.model))
            {
                Some(pages::Action::Go(p)) => {
                    switch(st, p);
                    None
                }
                other => other,
            }
        }
    })
    .flatten();
    // Outside the state borrow: the host posts to the tray window, and the
    // search box sends notifications back here.
    match action {
        Some(pages::Action::TimelineDay(day)) => retimeline(hwnd, |exp, m| {
            exp.toggle(&m.days(), day);
        }),
        Some(pages::Action::TimelineAll(open)) => retimeline(hwnd, |exp, m| {
            exp.set_all(&m.days(), open);
        }),
        Some(pages::Action::SettingsPage(p)) => {
            with(hwnd, |st| {
                st.sub = p;
                st.model.settings.sub = p;
                rewind_settings(st);
            });
            // Picking a page ends a search (EN_CHANGE clears the query).
            with_search(|sb| {
                if !sb.text().is_empty() {
                    sb.set_text("");
                }
            });
            // The Camera page shows the camera status: measure it now.
            refresh();
            fit_scroll(hwnd);
        }
        Some(a) => {
            if let Some(host) = HOST.get() {
                (host.act)(a);
            }
        }
        None => {}
    }
    sync_search(hwnd);
    note_page(hwnd);
    repaint(hwnd);
}

/// What `paint` needs, copied out of the state.
struct Snapshot {
    page: Page,
    model: Model,
    active: bool,
    maximized: bool,
    cap_hover: Option<u32>,
    cap_pressed: Option<u32>,
    nav_t: [f32; 4],
    focus: Option<u32>,
    scroll: Scroll,
    /// Scrollbar fade (0 = thin) and whether the thumb is being dragged.
    bar_t: f32,
    bar_look: Look,
    hover: Option<Target>,
    pressed: Option<Target>,
    search_focused: bool,
}

fn paint(hwnd: HWND) {
    let now = Instant::now();
    // SAFETY: plain query on our own window.
    let maximized = unsafe { IsZoomed(hwnd) }.as_bool();
    let snap = with(hwnd, |st| Snapshot {
        page: st.page,
        model: st.model.clone(),
        active: st.active,
        maximized,
        cap_hover: st.cap_hover,
        cap_pressed: st.cap_pressed,
        nav_t: st.nav_fade.map(|a| a.value(now)),
        focus: st.focus,
        scroll: st.scroll[st.page.index()].clone(),
        bar_t: st.bar_fade.value(now),
        bar_look: if st.drag.is_some() {
            Look::Pressed
        } else if st.bar_hover {
            Look::Hover
        } else {
            Look::Idle
        },
        hover: st.hover,
        pressed: st.pressed,
        search_focused: st.search_focused,
    });
    // The search box takes its colours at its own paint: repaint it when
    // the theme changed.
    let p = fluent::palette();
    if PAINTED.with(|c| c.replace(Some(p))) != Some(p) {
        with_search(|sb| sb.redraw());
    }
    let t_draw = Instant::now();
    let painted = SURFACE.with(|cell| {
        let mut slot = cell.borrow_mut();
        match (snap.as_ref(), slot.as_mut()) {
            (Some(snap), Some((h, surface))) if *h == hwnd.0 as isize => {
                let mica = surface.backdrop() == Backdrop::Mica;
                fluent::paint_with(surface, hwnd, |c, area| draw(c, area, snap, mica));
                true
            }
            _ => false,
        }
    });
    if !painted {
        // Nothing to draw with (yet): validate so WM_PAINT does not repeat.
        // SAFETY: our own window.
        let _ = unsafe { ValidateRect(Some(hwnd), None) };
        return;
    }
    if !FIRST_PAINT.swap(true, Ordering::Relaxed) {
        let draw_ms = t_draw.elapsed().as_millis();
        let (ms, t) =
            with(hwnd, |st| (st.opened.elapsed().as_millis(), st.timing)).unwrap_or_default();
        if let Some(host) = HOST.get() {
            host.log.line(&format!(
                "UI main window first paint {ms} ms (window {} ms, surface {} ms, draw {draw_ms} ms)",
                t.window_ms, t.surface_ms
            ));
        }
    }
}

/// A 2 px accent ring just outside `r`.
fn focus_ring(c: &Canvas, r: RectF) {
    let radius = RADIUS[1] * c.s;
    for grow in [2.0, 1.0] {
        let o = rectf(r.left - grow, r.top - grow, r.right + grow, r.bottom + grow);
        round_rect(c, &o, None, Some(c.p.accent), radius + grow, false);
    }
}

fn draw(c: &mut Canvas, area: RectF, snap: &Snapshot, mica: bool) {
    let (s, p) = (c.s, c.p);
    let f = frame(area.right, area.bottom, s);

    // Content layer: a rounded top-left corner and a hairline edge; its right
    // and bottom edges sit outside the window. Over Mica it is translucent.
    let over = RADIUS[1] * s * 2.0;
    let layer = rectf(
        f.pane.left,
        f.pane.top,
        f.pane.right + over,
        f.pane.bottom + over,
    );
    let alpha = if mica { 0.5 } else { 1.0 };
    round_rect(
        c,
        &layer,
        Some((p.card, alpha)),
        Some(p.stroke),
        RADIUS[1] * s,
        false,
    );

    // Title bar: app name and the caption buttons.
    let title_rgb = if snap.active { p.text } else { p.text2 };
    let tr = rectf(
        16.0 * s,
        f.cap.drag.top,
        f.cap.drag.right,
        f.cap.drag.bottom,
    );
    text(c, Font::Caption, APP_NAME, tr, title_rgb, Align::Left);
    titlebar::draw(
        c,
        &f.cap,
        snap.cap_hover,
        snap.cap_pressed,
        snap.maximized,
        snap.active,
    );

    // Nav rail.
    for (r, page) in &f.nav {
        let selected = *page == snap.page;
        let t = snap.nav_t[page.index()];
        if !selected && t > 0.0 {
            round_rect(c, r, Some((p.hover, t)), None, RADIUS[1] * s, false);
        }
        widgets::draw_nav_item(c, *r, page.glyph(), page.label(), selected);
        if snap.focus == Some(focus_id(Target::Nav(*page))) {
            focus_ring(c, *r);
        }
    }

    // The page, clipped to the pane, with an overlay scrollbar.
    let pr = scrolled(f.content, s, &snap.scroll);
    let hits = pages::layout(snap.page, pr, f.pane, s, &snap.model);
    // SAFETY: clip push/pop pair inside the surface's BeginDraw/EndDraw.
    unsafe {
        c.dc.PushAxisAlignedClip(&f.pane, D2D1_ANTIALIAS_MODE_ALIASED)
    };
    pages::draw(snap.page, c, pr, f.pane, &snap.model, &hits);
    // Hover and pressed fill for any clickable row or button: a tint of the
    // text colour, so it reads on cards, buttons and the page alike.
    if let Some(Target::Hit(id)) = snap.hover {
        if let Some(h) = hits.iter().find(|h| h.id == id) {
            let a = if snap.pressed == snap.hover {
                0.12
            } else {
                0.06
            };
            round_rect(c, &h.r, Some((p.text, a)), None, RADIUS[0] * s, false);
        }
    }
    if snap.page == Page::Settings {
        // The frame around the native edit, which draws itself on top.
        let sb = settings::search_rect(f.content, f.pane, s);
        search_box::draw_frame(c, sb, snap.search_focused);
    }
    if let Some(id) = snap.focus.filter(|&id| id >= HIT_BASE) {
        if let Some(h) = hits.iter().find(|h| focus_id(Target::Hit(h.id)) == id) {
            focus_ring(c, h.r);
        }
    }
    draw_scrollbar(c, &f, snap);
    // SAFETY: pops the clip pushed above.
    unsafe { c.dc.PopAxisAlignedClip() };
}

/// The scrollbar: a thin track and thumb that widen on hover or drag. It is
/// always drawn when the page scrolls (lighter at rest).
fn draw_scrollbar(c: &Canvas, f: &Frame, snap: &Snapshot) {
    let (s, p) = (c.s, c.p);
    let bar = Bar {
        content: snap.scroll.content,
        view: snap.scroll.view,
        offset: snap.scroll.offset,
        top: f.content.top,
        len: f.content.bottom - f.content.top,
        s,
    };
    let Some((top, len)) = bar.thumb() else {
        return;
    };
    let (l, r) = bar.painted_x(f.pane.right, snap.bar_t);
    let round = (r - l) / 2.0;
    let track = rectf(l, bar.top, r, bar.top + bar.len);
    round_rect(
        c,
        &track,
        Some((p.stroke, 0.5 + 0.5 * snap.bar_t)),
        None,
        round,
        false,
    );
    let (alpha, rgb) = scrollbar::thumb_color(snap.bar_look, p.text, p.text2);
    let thumb = rectf(l, bar.top + top, r, bar.top + top + len);
    round_rect(c, &thumb, Some((rgb, alpha)), None, round, false);
}

/// One arrow-key step, in logical px.
const LINE: f32 = 48.0;

/// Keys that scroll the page when the window (not the search box) has focus.
fn scroll_key(vk: u16) -> bool {
    [VK_PRIOR, VK_NEXT, VK_HOME, VK_END, VK_UP, VK_DOWN]
        .iter()
        .any(|k| k.0 == vk)
}

fn caption_button(ht: u32) -> bool {
    matches!(ht, HTMINBUTTON | HTMAXBUTTON | HTCLOSE)
}

fn track(hwnd: HWND, nonclient: bool) {
    let mut t = TRACKMOUSEEVENT {
        cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
        dwFlags: if nonclient {
            TME_LEAVE | TME_NONCLIENT
        } else {
            TME_LEAVE
        },
        hwndTrack: hwnd,
        dwHoverTime: 0,
    };
    // SAFETY: valid struct for our own window.
    let _ = unsafe { TrackMouseEvent(&mut t) };
}

/// The resize frame Windows hangs outside the screen when maximized.
fn maximized_inset(hwnd: HWND) -> i32 {
    // SAFETY: plain metric queries.
    unsafe {
        let dpi = GetDpiForWindow(hwnd);
        GetSystemMetricsForDpi(SM_CXFRAME, dpi) + GetSystemMetricsForDpi(SM_CXPADDEDBORDER, dpi)
    }
}

/// Remembers the window rect while it is neither maximized nor minimized.
fn note_normal(hwnd: HWND) {
    let mut r = RECT::default();
    // SAFETY: queries on our own window.
    let normal = unsafe {
        !IsZoomed(hwnd).as_bool()
            && !IsIconic(hwnd).as_bool()
            && GetWindowRect(hwnd, &mut r).is_ok()
    };
    if normal {
        with(hwnd, |st| {
            st.normal = Some((r.left, r.top, r.right - r.left, r.bottom - r.top))
        });
    }
}

/// Writes the remembered rect to the settings file if it changed.
fn save_rect(hwnd: HWND) {
    let (Some(Some(rect)), Some(host)) = (with(hwnd, |st| st.normal), HOST.get()) else {
        return;
    };
    if let Ok(mut s) = host.settings.lock() {
        if s.main_window_rect != Some(rect) {
            s.main_window_rect = Some(rect);
            let _ = s.save(&host.settings_path);
        }
    }
}

fn lo(v: isize) -> i32 {
    (v & 0xffff) as i16 as i32
}

fn hi(v: isize) -> i32 {
    ((v >> 16) & 0xffff) as i16 as i32
}

extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let now = Instant::now();
    match msg {
        WM_PAINT => {
            paint(hwnd);
            LRESULT(0)
        }
        WM_ERASEBKGND => LRESULT(1),
        // The client area covers the whole window (the title bar is ours).
        // Maximized, Windows hangs the frame off-screen: keep it out.
        WM_NCCALCSIZE => {
            // SAFETY: plain query on our own window.
            if wparam.0 != 0 && unsafe { IsZoomed(hwnd) }.as_bool() {
                let b = maximized_inset(hwnd);
                // SAFETY: with wParam TRUE, lParam is the NCCALCSIZE_PARAMS
                // Windows passes for this message.
                let p = unsafe { &mut *(lparam.0 as *mut NCCALCSIZE_PARAMS) };
                let r = &mut p.rgrc[0];
                r.left += b;
                r.top += b;
                r.right -= b;
                r.bottom -= b;
            }
            LRESULT(0)
        }
        WM_NCHITTEST => {
            let mut pt = POINT {
                x: lo(lparam.0),
                y: hi(lparam.0),
            };
            // SAFETY: maps a screen point into our client area, which is
            // the window (or, maximized, its visible part).
            let _ = unsafe { ScreenToClient(hwnd, &mut pt) };
            let (w, h) = client_size(hwnd);
            let s = with(hwnd, |st| st.scale).unwrap_or_else(|| fluent::scale(hwnd));
            // SAFETY: plain query on our own window.
            let maximized = unsafe { IsZoomed(hwnd) }.as_bool();
            let ht = titlebar::hit_test(pt.x as f32, pt.y as f32, w, h, s, maximized);
            LRESULT(ht as isize)
        }
        // Caption buttons: hover, press and click arrive as non-client
        // messages carrying the hit-test code. Windows must not handle them
        // (it would run its own modal button loop); everything else (drag,
        // edges, the system menu) goes to DefWindowProc.
        WM_NCMOUSEMOVE => {
            let ht = wparam.0 as u32;
            let button = caption_button(ht).then_some(ht);
            let changed = with(hwnd, |st| {
                let before = (st.cap_hover, st.cap_pressed);
                st.cap_hover = button;
                if st.cap_pressed.is_some() && st.cap_pressed != button {
                    st.cap_pressed = None;
                }
                let arm = !st.nc_tracking;
                st.nc_tracking = true;
                (before != (st.cap_hover, st.cap_pressed), arm)
            });
            if let Some((changed, arm)) = changed {
                if arm {
                    track(hwnd, true);
                }
                if changed {
                    repaint(hwnd);
                }
            }
            if button.is_some() {
                LRESULT(0)
            } else {
                // SAFETY: default handling.
                unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
            }
        }
        WM_NCMOUSELEAVE => {
            with(hwnd, |st| {
                st.cap_hover = None;
                st.cap_pressed = None;
                st.nc_tracking = false;
            });
            repaint(hwnd);
            LRESULT(0)
        }
        WM_NCLBUTTONDOWN if caption_button(wparam.0 as u32) => {
            with(hwnd, |st| st.cap_pressed = Some(wparam.0 as u32));
            repaint(hwnd);
            LRESULT(0)
        }
        WM_NCLBUTTONUP if caption_button(wparam.0 as u32) => {
            let ht = wparam.0 as u32;
            let clicked = with(hwnd, |st| st.cap_pressed.take() == Some(ht)).unwrap_or(false);
            repaint(hwnd);
            if clicked {
                // SAFETY: plain query on our own window.
                let maximized = unsafe { IsZoomed(hwnd) }.as_bool();
                let sc = match ht {
                    HTMINBUTTON => SC_MINIMIZE,
                    HTMAXBUTTON if maximized => SC_RESTORE,
                    HTMAXBUTTON => SC_MAXIMIZE,
                    _ => SC_CLOSE,
                };
                // SAFETY: posting to our own window.
                let _ = unsafe {
                    PostMessageW(Some(hwnd), WM_SYSCOMMAND, WPARAM(sc as usize), LPARAM(0))
                };
            }
            LRESULT(0)
        }
        WM_NCLBUTTONDBLCLK if caption_button(wparam.0 as u32) => LRESULT(0),
        WM_NCACTIVATE => {
            with(hwnd, |st| st.active = wparam.0 != 0);
            repaint(hwnd);
            // lParam -1: there is no non-client area for Windows to repaint.
            // SAFETY: default handling.
            unsafe { DefWindowProcW(hwnd, msg, wparam, LPARAM(-1)) }
        }
        WM_GETMINMAXINFO => {
            let (w, h) = min_size(fluent::scale(hwnd));
            // SAFETY: lParam is the MINMAXINFO Windows passes for this message.
            let mmi = unsafe { &mut *(lparam.0 as *mut MINMAXINFO) };
            mmi.ptMinTrackSize = POINT { x: w, y: h };
            LRESULT(0)
        }
        WM_DPICHANGED => {
            // SAFETY: lParam is the suggested RECT Windows passes for this message.
            let suggested = unsafe { *(lparam.0 as *const RECT) };
            let (r, s) = on_dpi(suggested, hi(wparam.0 as isize) as u32);
            // The scale first: SetWindowPos sends WM_SIZE, which repaints.
            with(hwnd, |st| st.scale = s);
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
            fit_scroll(hwnd);
            sync_search(hwnd);
            repaint(hwnd);
            LRESULT(0)
        }
        // Resize and present in the same message, so dragging an edge never
        // shows stale or black borders.
        WM_SIZE => {
            if wparam.0 == SIZE_RESTORED as usize {
                note_normal(hwnd);
            }
            fit_scroll(hwnd);
            sync_search(hwnd);
            // SAFETY: synchronous repaint of our own window.
            unsafe {
                let _ = InvalidateRect(Some(hwnd), None, false);
                let _ = UpdateWindow(hwnd);
            }
            LRESULT(0)
        }
        WM_EXITSIZEMOVE => {
            note_normal(hwnd);
            save_rect(hwnd);
            LRESULT(0)
        }
        WM_TIMER if wparam.0 == FRAME_TIMER => {
            with(hwnd, |st| {
                for sc in &mut st.scroll {
                    sc.tick(now);
                }
                // A track held down keeps paging until the thumb is under
                // the pointer.
                if let Some(Drag::Track { y, next }) = st.drag {
                    if now >= next {
                        let (w, h) = client_size(hwnd);
                        let f = frame(w, h, st.scale);
                        let hit = st.bar(&f).page_at(y);
                        if let Some(t) = hit {
                            st.scroll[st.page.index()].ease_to(t, now);
                        }
                        let next = now + Duration::from_millis(scrollbar::REPEAT_MS);
                        st.drag = Some(Drag::Track { y, next });
                    }
                }
                rehover(hwnd, st, now);
            });
            sync_timer(hwnd);
            repaint(hwnd);
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            let (x, y) = (lo(lparam.0) as f32, hi(lparam.0) as f32);
            let r = with(hwnd, |st| {
                // Dragging the thumb: the pointer may be anywhere (captured).
                if let Some(Drag::Thumb { grab_y, start }) = st.drag {
                    let (w, h) = client_size(hwnd);
                    let f = frame(w, h, st.scale);
                    let to = st.bar(&f).drag_to(start, grab_y, y);
                    st.scroll[st.page.index()].jump(to);
                    return (true, false);
                }
                if let Some(Drag::Track { next, .. }) = st.drag {
                    st.drag = Some(Drag::Track { y, next });
                    return (false, false);
                }
                // Back from the caption: its leave message may still be queued.
                let cap = st.cap_hover.take().is_some() | st.cap_pressed.take().is_some();
                let changed = point_at(hwnd, st, x, y, now) | cap;
                let arm = !std::mem::replace(&mut st.tracking, true);
                (changed, arm)
            });
            if let Some((changed, arm)) = r {
                if arm {
                    track(hwnd, false);
                }
                if changed {
                    sync_timer(hwnd);
                    repaint(hwnd);
                }
            }
            LRESULT(0)
        }
        WM_MOUSELEAVE => {
            with(hwnd, |st| {
                set_nav_hover(st, None, now);
                if st.drag.is_none() {
                    set_bar_hover(st, false, now);
                }
                st.hover = None;
                st.tracking = false;
                st.pressed = None;
            });
            sync_timer(hwnd);
            repaint(hwnd);
            LRESULT(0)
        }
        WM_LBUTTONDOWN => {
            let (x, y) = (lo(lparam.0) as f32, hi(lparam.0) as f32);
            // The scrollbar first: its hit area is wider than what is drawn.
            let grabbed = with(hwnd, |st| {
                let (w, h) = client_size(hwnd);
                let f = frame(w, h, st.scale);
                let bar = st.bar(&f);
                let sc = &mut st.scroll[st.page.index()];
                let part = bar.part_at(f.pane.right, x, y)?;
                st.drag = Some(match part {
                    Part::Thumb => Drag::Thumb {
                        grab_y: y,
                        start: sc.offset,
                    },
                    Part::Above | Part::Below => {
                        if let Some(t) = bar.page_at(y) {
                            sc.ease_to(t, now);
                        }
                        let first = Duration::from_millis(scrollbar::REPEAT_DELAY_MS);
                        Drag::Track {
                            y,
                            next: now + first,
                        }
                    }
                });
                st.pressed = None;
                st.focus = None;
                set_bar_hover(st, true, now);
                Some(())
            })
            .flatten()
            .is_some();
            if grabbed {
                // SAFETY: capture on our own window: the drag goes on
                // outside it, and WM_CAPTURECHANGED ends it.
                unsafe {
                    SetCapture(hwnd);
                    if GetFocus() != hwnd {
                        let _ = SetFocus(Some(hwnd));
                    }
                }
                sync_timer(hwnd);
                repaint(hwnd);
                return LRESULT(0);
            }
            let in_search = with(hwnd, |st| {
                let (f, hits) = layout_now(hwnd, st);
                st.pressed = target_at(&f, &hits, x, y);
                // The pointer took over: hide the keyboard focus ring.
                st.focus = None;
                st.page == Page::Settings
                    && within(&settings::search_rect(f.content, f.pane, st.scale), x, y)
            })
            .unwrap_or(false);
            // The search frame (glyph, border) focuses its edit; any other
            // click takes the keyboard back from it. After the borrow: the
            // edit notifies us of the focus change.
            let into_box = in_search && with_search(|sb| sb.focus()).is_some();
            // SAFETY: focus query and change on our own window.
            unsafe {
                if !into_box && GetFocus() != hwnd {
                    let _ = SetFocus(Some(hwnd));
                }
            }
            repaint(hwnd);
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            let (x, y) = (lo(lparam.0) as f32, hi(lparam.0) as f32);
            if with(hwnd, |st| st.drag.is_some()).unwrap_or(false) {
                // After the borrow: this sends WM_CAPTURECHANGED.
                // SAFETY: releases our own capture.
                let _ = unsafe { ReleaseCapture() };
                with(hwnd, |st| {
                    st.drag = None;
                    point_at(hwnd, st, x, y, now);
                });
                sync_timer(hwnd);
                repaint(hwnd);
                return LRESULT(0);
            }
            let t = with(hwnd, |st| {
                let (f, hits) = layout_now(hwnd, st);
                let up = target_at(&f, &hits, x, y);
                let down = st.pressed.take();
                down.filter(|_| down == up)
            })
            .flatten();
            match t {
                Some(t) => activate(hwnd, t),
                None => repaint(hwnd),
            }
            LRESULT(0)
        }
        // Capture taken away (alt-tab, a dialog): the drag is over.
        WM_CAPTURECHANGED => {
            with(hwnd, |st| {
                st.drag = None;
                set_bar_hover(st, false, now);
            });
            sync_timer(hwnd);
            repaint(hwnd);
            LRESULT(0)
        }
        // The pointer is a hand over anything clickable, an arrow elsewhere
        // (including the scrollbar).
        WM_SETCURSOR if lo(lparam.0) as u32 == HTCLIENT => {
            let hand = with(hwnd, |st| st.hover.is_some()).unwrap_or(false);
            // SAFETY: loads a system cursor and sets it for this message.
            unsafe {
                if let Ok(c) = LoadCursorW(None, if hand { IDC_HAND } else { IDC_ARROW }) {
                    SetCursor(Some(c));
                }
            }
            LRESULT(1)
        }
        WM_KEYDOWN if scroll_key(wparam.0 as u16) => {
            let key = wparam.0 as u16;
            with(hwnd, |st| {
                let sc = &mut st.scroll[st.page.index()];
                let to = match key {
                    k if k == VK_PRIOR.0 => {
                        scrollbar::page_by(sc.content, sc.view, sc.target(), -1.0)
                    }
                    k if k == VK_NEXT.0 => {
                        scrollbar::page_by(sc.content, sc.view, sc.target(), 1.0)
                    }
                    k if k == VK_HOME.0 => 0.0,
                    k if k == VK_END.0 => sc.content,
                    k if k == VK_UP.0 => sc.target() - LINE,
                    _ => sc.target() + LINE,
                };
                sc.ease_to(to, now);
            });
            sync_timer(hwnd);
            repaint(hwnd);
            LRESULT(0)
        }
        WM_MOUSEWHEEL => {
            let delta = hi(wparam.0 as isize);
            with(hwnd, |st| st.scroll[st.page.index()].wheel(delta, now));
            sync_timer(hwnd);
            repaint(hwnd);
            LRESULT(0)
        }
        WM_KEYDOWN if wparam.0 == VK_TAB.0 as usize => {
            // SAFETY: plain keyboard state query.
            let back = unsafe { GetKeyState(VK_SHIFT.0 as i32) } < 0;
            // The search box forwards Tab here too.
            let has_search = with_search(|_| ()).is_some();
            let moved = with(hwnd, |st| {
                let (_, hits) = layout_now(hwnd, st);
                let search = has_search && st.page == Page::Settings;
                let order = focus_order(&hits, search);
                let next = tab(&order, st.focus, st.search_focused, back);
                let into = next == Some(SEARCH_FOCUS);
                st.focus = if into { None } else { next };
                reveal_focus(hwnd, st, now);
                (into, st.search_focused)
            });
            sync_timer(hwnd);
            // After the borrow: focus changes notify us.
            match moved {
                Some((true, _)) => {
                    with_search(|sb| sb.focus());
                }
                Some((false, true)) => {
                    // SAFETY: focus change to our own window.
                    let _ = unsafe { SetFocus(Some(hwnd)) };
                }
                _ => {}
            }
            repaint(hwnd);
            LRESULT(0)
        }
        WM_KEYDOWN if wparam.0 == VK_RETURN.0 as usize || wparam.0 == VK_SPACE.0 as usize => {
            // Held down: act once, not at the keyboard repeat rate.
            if is_key_repeat(lparam.0) {
                return LRESULT(0);
            }
            let t = with(hwnd, |st| st.focus.and_then(target_of)).flatten();
            if let Some(t) = t {
                activate(hwnd, t);
            }
            LRESULT(0)
        }
        WM_COMMAND if lo(wparam.0 as isize) as u16 == ID_SEARCH && lparam.0 != 0 => {
            on_search(hwnd, hi(wparam.0 as isize) as u16 as u32);
            LRESULT(0)
        }
        WM_CTLCOLOREDIT => {
            let p = fluent::palette();
            let hdc = windows::Win32::Graphics::Gdi::HDC(wparam.0 as *mut _);
            with_search(|sb| sb.on_ctl_color(hdc, &p))
                // SAFETY: default handling.
                .unwrap_or_else(|| unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) })
        }
        WM_DESTROY => {
            // The edit goes first, outside every borrow (it may notify us).
            let search = SEARCH.with(|cell| cell.borrow_mut().take());
            drop(search);
            save_rect(hwnd);
            fluent::unregister_window(hwnd);
            // SAFETY: our own timer; harmless if it is not running.
            let _ = unsafe { KillTimer(Some(hwnd), FRAME_TIMER) };
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
            let was_main = STATE.with(|cell| {
                let mut g = cell.borrow_mut();
                let main = g.as_ref().is_some_and(|st| st.hwnd == hwnd.0 as isize);
                if main {
                    *g = None;
                }
                main
            });
            if was_main {
                // Idle again: back under the process's EcoQoS.
                let _ = pb_platform_win::power::set_thread_eco_exempt(false);
            }
            LRESULT(0)
        }
        // SAFETY: default handling.
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

/// Work areas of all monitors.
fn monitors() -> Vec<RECT> {
    unsafe extern "system" fn add(
        mon: HMONITOR,
        _: HDC,
        _: *mut RECT,
        data: LPARAM,
    ) -> windows::core::BOOL {
        let mut mi = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        // SAFETY: `data` is the Vec passed to EnumDisplayMonitors below,
        // alive for the whole (synchronous) enumeration.
        unsafe {
            if GetMonitorInfoW(mon, &mut mi).as_bool() {
                (*(data.0 as *mut Vec<RECT>)).push(mi.rcWork);
            }
        }
        true.into()
    }
    let mut out: Vec<RECT> = Vec::new();
    // SAFETY: synchronous enumeration writing into `out` through the callback.
    unsafe {
        let _ = EnumDisplayMonitors(None, None, Some(add), LPARAM(&mut out as *mut _ as isize));
    }
    out
}

/// The default size, centred on the work area of the monitor under the
/// pointer, at that monitor's scale.
fn default_rect() -> SavedRect {
    let mut pt = POINT::default();
    let mut mi = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    let (mut dx, mut dy) = (96u32, 96u32);
    // SAFETY: plain queries; failures leave the defaults above.
    let wa = unsafe {
        let _ = GetCursorPos(&mut pt);
        let mon = MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST);
        let _ = GetDpiForMonitor(mon, MDT_EFFECTIVE_DPI, &mut dx, &mut dy);
        if GetMonitorInfoW(mon, &mut mi).as_bool() {
            mi.rcWork
        } else {
            RECT {
                left: 0,
                top: 0,
                right: 1280,
                bottom: 800,
            }
        }
    };
    let s = dx.max(96) as f32 / 96.0;
    let w = ((DEFAULT_W * s) as i32).min(wa.right - wa.left);
    let h = ((DEFAULT_H * s) as i32).min(wa.bottom - wa.top);
    (
        wa.left + (wa.right - wa.left - w) / 2,
        wa.top + (wa.bottom - wa.top - h) / 2,
        w,
        h,
    )
}

/// How long making the window and its surface took (first-paint log).
#[derive(Clone, Copy, Debug, Default)]
struct Timing {
    window_ms: u128,
    surface_ms: u128,
}

/// Creates the window (hidden, title bar already dropped) and then its
/// surface at the final client size, Mica when `for_mica` and available
/// (`d2d::make_window` remakes it plain, with an opaque surface, otherwise).
fn make_window(rect: SavedRect, for_mica: bool) -> Result<(HWND, Surface, Timing), String> {
    let t0 = Instant::now();
    let want = if for_mica {
        Backdrop::Mica
    } else {
        Backdrop::Opaque
    };
    let made = d2d::make_window(want, |extra| {
        // SAFETY: class registration (a repeat fails harmlessly), window
        // creation on the UI thread, then a frame recalculation (our
        // WM_NCCALCSIZE drops the standard title bar) before the surface
        // measures the client area.
        unsafe {
            let hinstance = GetModuleHandleW(None).map_err(|e| e.to_string())?;
            let wc = WNDCLASSW {
                lpfnWndProc: Some(wndproc),
                hInstance: hinstance.into(),
                lpszClassName: CLASS,
                hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
                // The app icon (resource 1) for the taskbar and Alt+Tab.
                hIcon: LoadIconW(
                    Some(hinstance.into()),
                    PCWSTR(std::ptr::without_provenance::<u16>(1)),
                )
                .unwrap_or_default(),
                // No background brush: every pixel is ours (WM_ERASEBKGND).
                ..Default::default()
            };
            RegisterClassW(&wc);
            let title: Vec<u16> = APP_NAME.encode_utf16().chain([0]).collect();
            let h = CreateWindowExW(
                WS_EX_APPWINDOW | extra,
                CLASS,
                PCWSTR(title.as_ptr()),
                WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN,
                rect.0,
                rect.1,
                rect.2,
                rect.3,
                None,
                None,
                Some(hinstance.into()),
                None,
            )
            .map_err(|e| e.to_string())?;
            let _ = SetWindowPos(
                h,
                None,
                0,
                0,
                0,
                0,
                SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
            );
            Ok(h)
        }
    })?;
    // A discarded Mica attempt counts as window time.
    let window_ms = t0.elapsed().as_millis().saturating_sub(made.surface_ms);
    Ok((
        made.hwnd,
        made.surface,
        Timing {
            window_ms,
            surface_ms: made.surface_ms,
        },
    ))
}

fn create(page: Page, opened: Instant) {
    let Some(host) = HOST.get() else {
        return;
    };
    // The tray idles under EcoQoS (efficient cores, low clock); while this
    // window is open the UI thread runs at full speed (undone on WM_DESTROY).
    let _ = pb_platform_win::power::set_thread_eco_exempt(true);
    let model = (host.model)();
    let tl_base = model.timeline.clone();
    let saved = host.settings.lock().ok().and_then(|s| s.main_window_rect);
    let rect = restore_rect(saved, &monitors()).unwrap_or_else(default_rect);
    let (hwnd, surface, timing) = match make_window(rect, fluent::transparency_on()) {
        Ok(v) => v,
        Err(e) => {
            host.log.line(&format!("UI main window failed: {e}"));
            let _ = pb_platform_win::power::set_thread_eco_exempt(false);
            return;
        }
    };
    fluent::frame(hwnd, false);
    let now = Instant::now();
    SURFACE.with(|cell| *cell.borrow_mut() = Some((hwnd.0 as isize, surface)));
    STATE.with(|cell| {
        *cell.borrow_mut() = Some(State {
            hwnd: hwnd.0 as isize,
            page,
            model,
            scale: fluent::scale(hwnd),
            active: true,
            cap_hover: None,
            cap_pressed: None,
            nc_tracking: false,
            tracking: false,
            nav_hover: None,
            nav_fade: [Anim::new(0.0, 0.0, now, 0); 4],
            pressed: None,
            focus: None,
            scroll: std::array::from_fn(|_| Scroll::new(0.0, 0.0)),
            bar_hover: false,
            bar_fade: Anim::new(0.0, 0.0, now, 0),
            drag: None,
            hover: None,
            tl_base,
            timer: false,
            normal: Some(rect),
            opened,
            timing,
            sub: settings::Page::Devices,
            query: String::new(),
            search_focused: false,
            entered: None,
        })
    });
    match SearchBox::new(hwnd, ID_SEARCH) {
        Ok(sb) => {
            // UTF-16 units: never more characters than the query keeps.
            sb.limit(settings::QUERY_MAX);
            SEARCH.with(|cell| *cell.borrow_mut() = Some(sb));
        }
        Err(e) => host.log.line(&format!("UI search box failed: {e}")),
    }
    sync_search(hwnd);
    // Before the first paint: Settings needs its rows built.
    note_page(hwnd);
    fluent::register_window(hwnd);
    // SAFETY: show and raise our own window.
    unsafe {
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = SetForegroundWindow(hwnd);
    }
}

/// Opens the main window on `page`, or brings the open one forward (restored
/// if minimized) and switches it to `page`. UI thread only.
pub fn open(page: Page) {
    let opened = Instant::now();
    let Some(hwnd) = current() else {
        create(page, opened);
        return;
    };
    with(hwnd, |st| switch(st, page));
    sync_search(hwnd);
    note_page(hwnd);
    // SAFETY: restore and raise our own window.
    unsafe {
        if IsIconic(hwnd).as_bool() {
            let _ = ShowWindow(hwnd, SW_RESTORE);
        }
        let _ = SetForegroundWindow(hwnd);
    }
    repaint(hwnd);
}

/// Repaints the open window without rebuilding the model (a Timeline file
/// check came back). UI thread only.
pub fn repaint_page() {
    if let Some(hwnd) = current() {
        repaint(hwnd);
    }
}

/// The model changed: rebuild it and repaint. UI thread only.
pub fn refresh() {
    let Some(hwnd) = current() else {
        return;
    };
    let mut model = HOST.get().map(|h| (h.model)()).unwrap_or_default();
    // Called every second by the tray: repaint only on a real change.
    let changed = with(hwnd, |st| {
        // The Settings sub-page and query are the window's own.
        model.settings.sub = st.sub;
        model.settings.query = st.query.clone();
        // The Timeline's open days are the window's own too: the host's
        // model is rebuilt only when its entries changed, and then the
        // user's choices are applied to it and the view is held.
        let ix = Page::Timeline.index();
        let mut anchor = None;
        let base = model.timeline.clone();
        if st.tl_base.groups != base.groups || st.tl_base.history_on != base.history_on {
            anchor = st.model.timeline.anchor(st.scroll[ix].offset);
            model.timeline = Arc::new(base.with_expansion(&st.model.timeline.exp));
            st.tl_base = base;
        } else {
            model.timeline = st.model.timeline.clone();
        }
        let changed = st.model != model;
        st.model = model;
        (changed, anchor)
    });
    if let Some((true, anchor)) = changed {
        fit_scroll(hwnd);
        // A new entry above the view must not push what is being read away.
        with(hwnd, |st| {
            if let Some(o) = anchor.and_then(|a| st.model.timeline.restore(&a)) {
                st.scroll[Page::Timeline.index()].jump(o);
            }
        });
        repaint(hwnd);
    }
}

#[cfg(test)]
#[path = "main_window_ux.rs"]
mod ux;

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::UI::WindowsAndMessaging::{DestroyWindow, HTCAPTION, HTCLIENT};

    fn rect(left: i32, top: i32, right: i32, bottom: i32) -> RECT {
        RECT {
            left,
            top,
            right,
            bottom,
        }
    }

    #[test]
    fn saved_rect_off_every_monitor_is_ignored() {
        let monitors = [rect(0, 0, 1920, 1040), rect(1920, 0, 3840, 1040)];
        // On the second monitor: kept.
        assert_eq!(
            restore_rect(Some((2000, 100, 1000, 700)), &monitors),
            Some((2000, 100, 1000, 700))
        );
        // A monitor that was unplugged: dropped.
        assert_eq!(restore_rect(Some((4000, 100, 1000, 700)), &monitors), None);
        // Only a sliver on screen (the caption cannot be grabbed): dropped.
        assert_eq!(restore_rect(Some((-995, 100, 1000, 700)), &monitors), None);
        assert_eq!(restore_rect(Some((100, 1035, 1000, 700)), &monitors), None);
        // Degenerate sizes and nothing saved.
        assert_eq!(restore_rect(Some((100, 100, 0, 700)), &monitors), None);
        assert_eq!(restore_rect(None, &monitors), None);
    }

    #[test]
    fn a_crafted_saved_rect_is_ignored_without_overflow() {
        let monitors = [rect(0, 0, 1920, 1040), rect(1920, 0, 3840, 1040)];
        for r in [
            (i32::MAX, 0, 10, 10),
            (0, i32::MAX, 10, 10),
            (0, 0, i32::MAX, 10),
            (0, 0, 10, i32::MAX),
            (i32::MAX, i32::MAX, i32::MAX, i32::MAX),
            (i32::MIN, 0, 10, 10),
            (0, i32::MIN, 10, 10),
            (i32::MIN, i32::MIN, i32::MAX, i32::MAX),
        ] {
            assert_eq!(restore_rect(Some(r), &monitors), None, "{r:?}");
        }
        let far = [rect(i32::MIN, i32::MIN, i32::MAX, i32::MAX)];
        assert_eq!(restore_rect(Some((i32::MIN, 0, 10, 10)), &far), None);
    }

    #[test]
    fn enter_held_down_activates_once() {
        // First press: bit 30 clear (repeat count 1).
        assert!(!is_key_repeat(0x0000_0001));
        assert!(!is_key_repeat(0x001C_0001));
        // Auto-repeat: bit 30 set.
        assert!(is_key_repeat(0x4000_0001));
        assert!(is_key_repeat(0x401C_0001));
    }

    #[test]
    fn dpi_change_uses_the_suggested_rect_and_new_scale() {
        let suggested = rect(10, 20, 1510, 1070);
        assert_eq!(on_dpi(suggested, 144), (suggested, 1.5));
        assert_eq!(on_dpi(suggested, 96).1, 1.0);
        // A bogus 0 never yields a zero scale.
        assert_eq!(on_dpi(suggested, 0).1, 1.0);
    }

    #[test]
    fn minimum_size_scales() {
        assert_eq!(min_size(1.0), (640, 480));
        assert_eq!(min_size(1.5), (960, 720));
    }

    #[test]
    fn frame_has_rail_left_of_pane_under_caption() {
        let f = frame(1000.0, 700.0, 1.0);
        assert_eq!(f.pane.top, f.cap.close.bottom);
        assert_eq!(f.pane.left, RAIL_W);
        assert_eq!(f.nav.len(), 4);
        assert!(f
            .nav
            .iter()
            .all(|(r, _)| r.right <= f.pane.left && r.top >= f.pane.top));
        assert!(f.content.left > f.pane.left && f.content.bottom < f.pane.bottom);
        // The nav area is client area, not caption.
        let (r, _) = f.nav[0];
        let ht = titlebar::hit_test(r.left + 5.0, r.top + 5.0, 1000.0, 700.0, 1.0, false);
        assert_eq!(ht, HTCLIENT);
        assert_eq!(
            titlebar::hit_test(300.0, 10.0, 1000.0, 700.0, 1.0, false),
            HTCAPTION
        );
    }

    #[test]
    fn clicks_map_to_nav_items_and_page_hits() {
        let f = frame(1000.0, 700.0, 1.5);
        let (r, _) = f.nav[1];
        assert_eq!(
            target_at(&f, &[], r.left + 2.0, r.top + 2.0),
            Some(Target::Nav(Page::Timeline))
        );
        let hit = Hit {
            id: 7,
            r: rectf(400.0, 300.0, 500.0, 340.0),
        };
        assert_eq!(target_at(&f, &[hit], 450.0, 320.0), Some(Target::Hit(7)));
        assert_eq!(target_at(&f, &[hit], 600.0, 600.0), None);
    }

    #[test]
    fn the_search_box_sits_between_the_rail_and_the_page() {
        let hit = |id| Hit {
            id,
            r: rectf(0.0, 0.0, 1.0, 1.0),
        };
        let order = focus_order(&[hit(1), hit(2)], true);
        assert_eq!(
            order,
            [1, 2, 3, 4, SEARCH_FOCUS, HIT_BASE + 1, HIT_BASE + 2]
        );
        assert_eq!(target_of(SEARCH_FOCUS), None, "not a click target");
        // Tab from the last nav item enters the box; from the box, the
        // first page link; Shift+Tab from that link goes back to the box.
        assert_eq!(tab(&order, Some(4), false, false), Some(SEARCH_FOCUS));
        assert_eq!(tab(&order, None, true, false), Some(HIT_BASE + 1));
        assert_eq!(tab(&order, None, true, true), Some(4));
        assert_eq!(
            tab(&order, Some(HIT_BASE + 1), false, true),
            Some(SEARCH_FOCUS)
        );
        // Keys in the box win over a stale ring.
        assert_eq!(tab(&order, Some(2), true, false), Some(HIT_BASE + 1));
    }

    #[test]
    fn tab_order_is_the_rail_then_the_page() {
        let hit = |id| Hit {
            id,
            r: rectf(0.0, 0.0, 1.0, 1.0),
        };
        let order = focus_order(&[hit(0), hit(3)], false);
        assert_eq!(order, [1, 2, 3, 4, HIT_BASE, HIT_BASE + 3]);
        assert_eq!(target_of(2), Some(Target::Nav(Page::Timeline)));
        assert_eq!(target_of(HIT_BASE + 3), Some(Target::Hit(3)));
        assert_eq!(target_of(0), None);
        // Shift+Tab from nothing lands on the page's last hit.
        assert_eq!(focus::next(&order, None, true), Some(HIT_BASE + 3));
    }

    #[test]
    fn scrolled_content_moves_up_by_the_offset() {
        let mut sc = Scroll::new(1000.0, 400.0);
        sc.offset = 100.0;
        let r = scrolled(rectf(0.0, 50.0, 300.0, 450.0), 2.0, &sc);
        assert_eq!((r.top, r.bottom), (50.0 - 200.0, 50.0 - 200.0 + 2000.0));
    }

    #[test]
    fn window_and_surface_are_made_hidden_either_way() {
        for for_mica in [true, false] {
            let (hwnd, mut surface, _) = make_window((0, 0, 800, 600), for_mica).expect("window");
            println!("for_mica {for_mica}: got {:?}", surface.backdrop());
            if !for_mica {
                assert_eq!(surface.backdrop(), Backdrop::Opaque);
            }
            surface.resize(900, 650);
            surface.draw(|_| {}).expect("draw");
            drop(surface);
            // SAFETY: our own hidden test window.
            let _ = unsafe { DestroyWindow(hwnd) };
        }
    }

    /// Cost of opening the window, split like the first-paint log line; the
    /// second round shows what the shared device and caches save. Run with
    /// `cargo test -p pb-tray -- --ignored first_paint_timing --nocapture`.
    #[test]
    #[ignore]
    fn first_paint_timing() {
        for round in 1..=2 {
            let t0 = Instant::now();
            let (hwnd, mut surface, t) = make_window((100, 100, 1000, 700), false).expect("window");
            let (window_ms, surface_ms) = (t.window_ms, t.surface_ms);
            let t1 = Instant::now();
            let (w, h) = client_size(hwnd);
            let snap = Snapshot {
                page: Page::Home,
                model: Model::default(),
                active: true,
                maximized: false,
                cap_hover: None,
                cap_pressed: None,
                nav_t: [0.0; 4],
                focus: None,
                scroll: Scroll::new(0.0, 0.0),
                bar_t: 0.0,
                bar_look: Look::Idle,
                hover: None,
                pressed: None,
                search_focused: false,
            };
            surface.resize(w as u32, h as u32);
            surface
                .draw(|dc| {
                    let mut c = Canvas {
                        dc,
                        s: 1.0,
                        p: fluent::palette(),
                    };
                    draw(&mut c, rectf(0.0, 0.0, w, h), &snap, false);
                })
                .expect("draw");
            let draw_ms = t1.elapsed().as_millis();
            println!(
                "round {round}: total {} ms (window {window_ms} ms, surface {surface_ms} ms, draw {draw_ms} ms)",
                t0.elapsed().as_millis()
            );
            drop(surface);
            // SAFETY: our own hidden test window.
            let _ = unsafe { DestroyWindow(hwnd) };
        }
    }
    #[test]
    fn scrollbar_pages_and_hover_draw_in_a_hidden_window() {
        let (hwnd, mut surface, _) = make_window((100, 100, 1000, 700), false).expect("window");
        let (w, h) = client_size(hwnd);
        surface.resize(w as u32, h as u32);
        let mut sc = Scroll::new(3000.0, 500.0);
        sc.offset = 700.0;
        for (t, look) in [(0.0, Look::Idle), (1.0, Look::Hover), (1.0, Look::Pressed)] {
            let snap = Snapshot {
                page: Page::Timeline,
                model: Model::default(),
                active: true,
                maximized: false,
                cap_hover: None,
                cap_pressed: None,
                nav_t: [0.0; 4],
                focus: None,
                scroll: sc.clone(),
                bar_t: t,
                bar_look: look,
                hover: Some(Target::Hit(1)),
                pressed: None,
                search_focused: false,
            };
            surface
                .draw(|dc| {
                    let mut c = Canvas {
                        dc,
                        s: 1.0,
                        p: fluent::palette(),
                    };
                    draw(&mut c, rectf(0.0, 0.0, w, h), &snap, false);
                })
                .expect("draw");
        }
        drop(surface);
        // SAFETY: our own hidden test window.
        let _ = unsafe { DestroyWindow(hwnd) };
    }

    #[test]
    fn scroll_keys_are_recognised() {
        for k in [VK_PRIOR, VK_NEXT, VK_HOME, VK_END, VK_UP, VK_DOWN] {
            assert!(scroll_key(k.0));
        }
        assert!(!scroll_key(VK_TAB.0) && !scroll_key(VK_RETURN.0));
    }
}
