//! Fluent modal dialog for the questions the tray asks (pair with this
//! phone? allow control? accept this file?) and for plain notices. It
//! replaces the classic Win32 message box so every window looks alike.
//! Drawn with Direct2D on an opaque surface.
//!
//! Each dialog runs on its own thread with its own message loop, so the tray
//! keeps working while one is open. It is topmost and brought to the front,
//! but never blocks anything. The answer callback runs exactly once.
//!
//! Keys: Esc and the close button answer "no" (dismiss). Enter accepts only
//! on a notice; on a question it does nothing, so a stray Enter can never
//! approve a security prompt. Tab moves between the buttons, Space presses
//! the focused one.
#![allow(unsafe_code)]

use std::{cell::RefCell, time::Instant};

use windows::{
    core::{w, PCWSTR},
    Win32::{
        Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
        Graphics::Gdi::{InvalidateRect, ScreenToClient, ValidateRect},
        System::LibraryLoader::GetModuleHandleW,
        UI::{
            Controls::WM_MOUSELEAVE,
            HiDpi::AdjustWindowRectExForDpi,
            Input::KeyboardAndMouse::{
                GetKeyState, ReleaseCapture, SetCapture, SetFocus, TrackMouseEvent, TME_LEAVE,
                TRACKMOUSEEVENT, VK_ESCAPE, VK_RETURN, VK_SHIFT, VK_SPACE, VK_TAB,
            },
            WindowsAndMessaging::{
                CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetCursorPos,
                GetMessageW, GetSystemMetrics, KillTimer, LoadCursorW, LoadIconW, PostQuitMessage,
                RegisterClassW, SetForegroundWindow, SetTimer, SetWindowPos, ShowWindow,
                TranslateMessage, HWND_TOPMOST, IDC_ARROW, MSG, SM_CXSCREEN, SM_CYSCREEN,
                SWP_NOACTIVATE, SWP_NOZORDER, SW_SHOW, WM_CLOSE, WM_DESTROY, WM_DPICHANGED,
                WM_ERASEBKGND, WM_GETDLGCODE, WM_KEYDOWN, WM_LBUTTONDOWN, WM_LBUTTONUP,
                WM_MOUSEMOVE, WM_PAINT, WM_SETCURSOR, WM_TIMER, WNDCLASSW, WS_CAPTION,
                WS_EX_APPWINDOW, WS_EX_TOPMOST, WS_OVERLAPPED, WS_SYSMENU,
            },
        },
    },
};

use crate::{
    d2d::{Backdrop, Surface},
    ease,
    fluent::{self, from_rect, px, rectf, round_rect, text, text_wrapped, Align, Canvas, Font},
    tokens::RADIUS,
};

const CLASS: PCWSTR = w!("PhoneBridgeDialog");
/// The hover animation's frame timer: runs only while a hover value is moving.
const TIMER_ANIM: usize = 1;
const TICK_MS: u32 = 16;
/// Polls whether the question is still wanted; far slower than a frame loop.
const TIMER_ALIVE: usize = 2;
const ALIVE_MS: u32 = 500;
/// DLGC_WANTALLKEYS: keep Tab, Enter and Esc for us instead of the dialog manager.
const DLGC_WANTALLKEYS: isize = 0x0004;

/// What a dialog says. Pure data, so the wording can be tested.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Dialog {
    /// Window caption (task switcher, taskbar).
    pub title: String,
    pub heading: String,
    pub body: String,
    /// A short code to compare, drawn large in its own card.
    pub code: Option<String>,
    pub primary: String,
    /// `None` makes it a notice with a single button.
    pub secondary: Option<String>,
}

impl Dialog {
    /// A question with two answers. Enter does not answer it.
    pub fn question(heading: String, body: String, primary: &str, secondary: &str) -> Self {
        Self {
            title: "Relay".into(),
            heading,
            body,
            code: None,
            primary: primary.into(),
            secondary: Some(secondary.into()),
        }
    }

    /// A notice with one button.
    pub fn notice(heading: String, body: String) -> Self {
        Self {
            title: "Relay".into(),
            heading,
            body,
            code: None,
            primary: "OK".into(),
            secondary: None,
        }
    }

    /// The pairing check: compare the code with the phone's.
    pub fn pairing(peer_name: &str, sas: &str) -> Self {
        Self {
            code: Some(group_code(sas)),
            ..Self::question(
                format!("Pair with \"{peer_name}\"?"),
                "Check that your phone shows this same code. Only continue if it matches exactly."
                    .into(),
                "Yes, it matches",
                "No",
            )
        }
    }

    fn is_question(&self) -> bool {
        self.secondary.is_some()
    }
}

/// "628183" -> "628 183": two halves (the first takes the odd digit) so the
/// eye can compare it with the phone. Anything that is not a plain run of at
/// least four ASCII digits is returned as it is.
pub fn group_code(code: &str) -> String {
    let code = code.trim();
    if code.len() < 4 || !code.bytes().all(|b| b.is_ascii_digit()) {
        return code.to_owned();
    }
    let (a, b) = code.split_at(code.len().div_ceil(2));
    format!("{a} {b}")
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Button {
    Primary,
    Secondary,
}

/// Calls the answer callback at most once, whatever closes the dialog.
struct Outcome<F: FnOnce(bool)> {
    done: Option<F>,
}

impl<F: FnOnce(bool)> Outcome<F> {
    fn new(done: F) -> Self {
        Self { done: Some(done) }
    }

    /// True if this call delivered the answer.
    fn finish(&mut self, yes: bool) -> bool {
        match self.done.take() {
            Some(f) => {
                f(yes);
                true
            }
            None => false,
        }
    }
}

/// Blend `a` toward `b` (0xRRGGBB), `t` in 0..=1.
fn mix(a: u32, b: u32, t: f32) -> u32 {
    let ch = |shift: u32| {
        let (x, y) = (((a >> shift) & 0xff) as f32, ((b >> shift) & 0xff) as f32);
        ((x + (y - x) * t.clamp(0.0, 1.0)).round() as u32) << shift
    };
    ch(0) | ch(8) | ch(16)
}

/// Which button Tab / Shift+Tab moves to.
fn next_focus(cur: Option<Button>, two: bool, back: bool) -> Option<Button> {
    match (cur, two, back) {
        (_, false, _) => Some(Button::Primary),
        (None, true, false) | (Some(Button::Secondary), true, false) => Some(Button::Primary),
        (Some(Button::Primary), true, false) => Some(Button::Secondary),
        (None, true, true) | (Some(Button::Primary), true, true) => Some(Button::Secondary),
        (Some(Button::Secondary), true, true) => Some(Button::Primary),
    }
}

/// The button focused when the dialog opens: a notice starts on its only
/// button, a question on neither (so Space cannot approve it unseen).
fn start_focus(d: &Dialog) -> Option<Button> {
    (!d.is_question()).then_some(Button::Primary)
}

/// What Enter does: accepts only a notice, never a question.
fn enter_presses(d: &Dialog) -> Option<Button> {
    (!d.is_question()).then_some(Button::Primary)
}

// ---- layout (all in device pixels at scale `s`) ----

const WIDTH: i32 = 400;
const MARGIN: i32 = 24;
const HEADING_MAX: i32 = 44; // two lines
const BODY_MAX: i32 = 108; // six lines
const CODE_H: i32 = 76;
const BUTTON_H: i32 = 32;
const BUTTON_GAP: i32 = 8;
const SOLO_BUTTON_W: i32 = 120;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Layout {
    heading: RECT,
    body: RECT,
    code: Option<RECT>,
    primary: RECT,
    secondary: Option<RECT>,
    /// Client height.
    height: i32,
}

fn rect(l: i32, t: i32, r: i32, b: i32) -> RECT {
    RECT {
        left: l,
        top: t,
        right: r,
        bottom: b,
    }
}

/// Stack heading, body, code card and buttons. `heading_h` and `body_h` are
/// the measured heights of the wrapped text; they are capped so a long text
/// is ellipsized instead of pushing into what is below it.
fn layout(s: f32, heading_h: i32, body_h: i32, has_code: bool, two: bool) -> Layout {
    let (m, left, right) = (px(s, MARGIN), px(s, MARGIN), px(s, WIDTH - MARGIN));
    let heading_h = heading_h.min(px(s, HEADING_MAX));
    let body_h = body_h.min(px(s, BODY_MAX));
    let mut y = m;
    let heading = rect(left, y, right, y + heading_h);
    y += heading_h;
    let body = if body_h > 0 {
        y += px(s, 8);
        let r = rect(left, y, right, y + body_h);
        y += body_h;
        r
    } else {
        rect(left, y, right, y)
    };
    let code = has_code.then(|| {
        y += px(s, 16);
        let r = rect(left, y, right, y + px(s, CODE_H));
        y += px(s, CODE_H);
        r
    });
    y += px(s, 24);
    let bh = px(s, BUTTON_H);
    let (primary, secondary) = if two {
        let gap = px(s, BUTTON_GAP);
        let w = (right - left - gap) / 2;
        (
            rect(left, y, left + w, y + bh),
            Some(rect(right - w, y, right, y + bh)),
        )
    } else {
        (rect(right - px(s, SOLO_BUTTON_W), y, right, y + bh), None)
    };
    Layout {
        heading,
        body,
        code,
        primary,
        secondary,
        height: y + bh + m,
    }
}

fn hit(l: &Layout, x: i32, y: i32) -> Option<Button> {
    if fluent::inside(&l.primary, x, y) {
        Some(Button::Primary)
    } else if l.secondary.is_some_and(|r| fluent::inside(&r, x, y)) {
        Some(Button::Secondary)
    } else {
        None
    }
}

/// Where a dialog of outer size `size` goes after a DPI change: centred on
/// the rect Windows suggests (x, y, width, height).
fn dpi_rect(suggested: RECT, size: (i32, i32)) -> (i32, i32, i32, i32) {
    let cx = (suggested.left + suggested.right) / 2;
    let cy = (suggested.top + suggested.bottom) / 2;
    (cx - size.0 / 2, cy - size.1 / 2, size.0, size.1)
}

// ---- window ----

struct State {
    dialog: Dialog,
    layout: Layout,
    /// Hover amount 0..=1 per button (animated).
    hover: [f32; 2],
    pressed: Option<Button>,
    focus: Option<Button>,
    last_tick: Instant,
    outcome: Outcome<Box<dyn FnOnce(bool)>>,
    /// Asked on every tick and before a button acts: false means the
    /// question is dead (timed out) and the dialog closes without acting.
    alive: Option<Box<dyn Fn() -> bool + Send>>,
}

thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
    /// The dialog's Direct2D surface (one dialog per thread).
    static SURFACE: RefCell<Option<Surface>> = const { RefCell::new(None) };
}

fn idx(b: Button) -> usize {
    match b {
        Button::Primary => 0,
        Button::Secondary => 1,
    }
}

/// Height `s` takes wrapped to `width` in `font`, capped at `max` and cut
/// to whole lines so a long text never shows half a line.
fn text_height(font: Font, s: f32, t: &str, width: i32, max: i32) -> i32 {
    let (h, line) = fluent::wrapped_height(font, s, t, width as f32);
    if h <= 0.0 {
        return 0;
    }
    let max = max as f32;
    let h = if h > max && line > 0.0 {
        (max / line).floor().max(1.0) * line
    } else {
        h
    };
    h.ceil() as i32
}

/// The layout for `d` at scale `s`, with its texts measured. Needs the
/// dialog's surface to exist (DirectWrite).
fn measure(s: f32, d: &Dialog) -> Layout {
    let width = px(s, WIDTH - 2 * MARGIN);
    layout(
        s,
        text_height(Font::Subtitle, s, &d.heading, width, px(s, HEADING_MAX)),
        text_height(Font::Body, s, &d.body, width, px(s, BODY_MAX)),
        d.code.is_some(),
        d.secondary.is_some(),
    )
}

/// What one paint shows, copied out of the state.
struct View<'a> {
    dialog: &'a Dialog,
    layout: Layout,
    hover: [f32; 2],
    focus: Option<Button>,
    pressed: Option<Button>,
}

fn paint_dialog(c: &mut Canvas, v: &View) {
    let (s, p) = (c.s, c.p);
    let (l, d) = (&v.layout, v.dialog);
    text_wrapped(c, Font::Subtitle, &d.heading, from_rect(l.heading), p.text);
    text_wrapped(c, Font::Body, &d.body, from_rect(l.body), p.text2);
    if let (Some(r), Some(code)) = (l.code, &d.code) {
        let r = from_rect(r);
        round_rect(
            c,
            &r,
            Some((p.card, 1.0)),
            Some(p.stroke),
            RADIUS[1] * s,
            false,
        );
        // Spaced like the old GDI card (4 px per character) so the eye can
        // compare it digit by digit with the phone.
        fluent::text_spaced(c, Font::Code, code, r, p.text, Align::Center, 4.0 * s);
    }
    let radius = RADIUS[0] * s;
    let button = |c: &mut Canvas, b: Button, r: RECT, label: &str, accent: bool| {
        let r = from_rect(r);
        let t = v.hover[idx(b)];
        let down = v.pressed == Some(b);
        let (fill, stroke, ink) = button_style(&p, accent, t, down);
        if v.focus == Some(b) {
            // A ring 2 px outside the button.
            let g = 2.0 * s;
            let ring = rectf(r.left - g, r.top - g, r.right + g, r.bottom + g);
            round_rect(c, &ring, None, Some(p.text), radius + g, false);
        }
        round_rect(c, &r, Some((fill, 1.0)), stroke, radius, false);
        text(c, Font::Body, label, r, ink, Align::Center);
    };
    button(c, Button::Primary, l.primary, &d.primary, true);
    if let (Some(r), Some(label)) = (l.secondary, &d.secondary) {
        button(c, Button::Secondary, r, label, false);
    }
}

fn draw(hwnd: HWND) {
    let painted = SURFACE.with(|cell| {
        let mut slot = cell.borrow_mut();
        let Some(surface) = slot.as_mut() else {
            return false;
        };
        fluent::paint_with(surface, hwnd, |c, _| {
            let snap = STATE.with(|st| {
                let mut st = st.borrow_mut();
                let st = st.as_mut()?;
                // Re-measured each paint: the scale may have changed.
                st.layout = measure(c.s, &st.dialog);
                Some((st.dialog.clone(), st.layout, st.hover, st.focus, st.pressed))
            });
            if let Some((dialog, layout, hover, focus, pressed)) = snap {
                let v = View {
                    dialog: &dialog,
                    layout,
                    hover,
                    focus,
                    pressed,
                };
                paint_dialog(c, &v);
            }
        });
        true
    });
    if !painted {
        // SAFETY: our own window; nothing to draw with.
        let _ = unsafe { ValidateRect(Some(hwnd), None) };
    }
}

/// The window size for a client `WIDTH` x `client_h` at scale `s`, with
/// the frame for that scale.
fn frame_size(s: f32, client_h: i32) -> (i32, i32) {
    let mut r = rect(0, 0, px(s, WIDTH), client_h);
    // SAFETY: plain computation on a local rect.
    let _ = unsafe {
        AdjustWindowRectExForDpi(
            &mut r,
            WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU,
            false,
            WS_EX_APPWINDOW | WS_EX_TOPMOST,
            (s * 96.0).round() as u32,
        )
    };
    (r.right - r.left, r.bottom - r.top)
}

/// Deliver the answer (once) and close.
fn answer(hwnd: HWND, yes: bool) {
    // Taken out of the cell first: the callback must not run while it is borrowed.
    let mut outcome = STATE.with(|st| {
        st.borrow_mut()
            .as_mut()
            .map(|st| std::mem::replace(&mut st.outcome, Outcome { done: None }))
    });
    if let Some(o) = outcome.as_mut() {
        o.finish(yes);
    }
    // SAFETY: our own window, on its own thread.
    let _ = unsafe { DestroyWindow(hwnd) };
}

fn press(hwnd: HWND, b: Button) {
    // A button never acts on a dead question.
    if !is_alive() {
        answer(hwnd, false);
        return;
    }
    answer(hwnd, b == Button::Primary);
}

fn is_alive() -> bool {
    STATE.with(|st| {
        st.borrow()
            .as_ref()
            .and_then(|s| s.alive.as_ref().map(|f| f()))
            .unwrap_or(true)
    })
}

fn cursor_client(hwnd: HWND) -> Option<POINT> {
    let mut pt = POINT::default();
    // SAFETY: plain queries on our window.
    unsafe {
        GetCursorPos(&mut pt).ok()?;
        ScreenToClient(hwnd, &mut pt).as_bool().then_some(pt)
    }
}

/// Every button's hover value has reached what the pointer asks for: nothing to animate.
fn hover_settled(hover: &[f32; 2], over: Option<Button>) -> bool {
    [Button::Primary, Button::Secondary]
        .iter()
        .all(|b| ease::settled(hover[idx(*b)], f32::from(over == Some(*b))))
}

/// The slow timer: the question timed out or was withdrawn, so close and answer "no" (a no-op for the
/// asker, which has stopped listening).
fn check_alive(hwnd: HWND) {
    if !is_alive() {
        answer(hwnd, false);
    }
}

/// Starts the frame timer (a repeat just restarts it); the pointer moved, so a hover may change.
fn wake_frames(hwnd: HWND) {
    // SAFETY: our own timer on our own window.
    let _ = unsafe { SetTimer(Some(hwnd), TIMER_ANIM, TICK_MS, None) };
}

/// A button's fill, boundary and label colour. The primary button is the only one filled with the accent;
/// a secondary button has a control boundary (`outline_strong`, 3:1 on the dialog surface) and its label
/// in the text colour. `hover` is 0..=1; `down` while pressed.
fn button_style(
    p: &fluent::Palette,
    accent: bool,
    hover: f32,
    down: bool,
) -> (u32, Option<u32>, u32) {
    if accent {
        let fill = if down {
            mix(p.primary, p.on_primary, 0.25)
        } else {
            mix(p.primary, p.on_primary, hover * 0.12)
        };
        (fill, None, p.on_primary)
    } else {
        let fill = if down {
            mix(p.hover, p.stroke, 0.5)
        } else {
            mix(p.card, p.hover, hover)
        };
        (fill, Some(p.outline_strong), p.text)
    }
}

fn tick(hwnd: HWND) {
    let pt = cursor_client(hwnd);
    let (changed, settled) = STATE.with(|st| {
        let mut st = st.borrow_mut();
        let Some(st) = st.as_mut() else {
            return (false, true);
        };
        let now = Instant::now();
        let dt = now.duration_since(st.last_tick).as_millis().min(100) as u32;
        st.last_tick = now;
        let over = pt.and_then(|p| hit(&st.layout, p.x, p.y));
        let mut changed = false;
        for b in [Button::Primary, Button::Secondary] {
            let target = f32::from(over == Some(b));
            let cur = st.hover[idx(b)];
            let next = ease::approach(cur, target, dt);
            changed |= next != cur;
            st.hover[idx(b)] = next;
        }
        (changed, hover_settled(&st.hover, over))
    });
    if changed {
        // SAFETY: repaint our own window.
        let _ = unsafe { InvalidateRect(Some(hwnd), None, false) };
    }
    if !ease::needs_frame_timer(!settled) {
        // SAFETY: our own timer; the next pointer move starts it again.
        let _ = unsafe { KillTimer(Some(hwnd), TIMER_ANIM) };
    }
}

extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_ERASEBKGND => LRESULT(1),
        WM_PAINT => {
            draw(hwnd);
            LRESULT(0)
        }
        WM_TIMER => {
            match wparam.0 {
                TIMER_ALIVE => check_alive(hwnd),
                _ => tick(hwnd),
            }
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            let mut track = TRACKMOUSEEVENT {
                cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                dwFlags: TME_LEAVE,
                hwndTrack: hwnd,
                dwHoverTime: 0,
            };
            // SAFETY: asks Windows for one WM_MOUSELEAVE for our own window.
            let _ = unsafe { TrackMouseEvent(&mut track) };
            wake_frames(hwnd);
            LRESULT(0)
        }
        WM_MOUSELEAVE => {
            wake_frames(hwnd);
            LRESULT(0)
        }
        WM_GETDLGCODE => LRESULT(DLGC_WANTALLKEYS),
        // Moved to a monitor with another scale: re-measure at the new
        // scale and fit the window to it (centred on Windows' suggestion).
        WM_DPICHANGED => {
            // SAFETY: lParam is the suggested RECT Windows passes for this message.
            let sug = unsafe { *(lparam.0 as *const RECT) };
            let s = ((wparam.0 >> 16) & 0xffff).max(96) as f32 / 96.0;
            let lay = STATE.with(|st| {
                let mut st = st.borrow_mut();
                let st = st.as_mut()?;
                st.layout = measure(s, &st.dialog);
                Some(st.layout)
            });
            if let Some(lay) = lay {
                let (x, y, w, h) = dpi_rect(sug, frame_size(s, lay.height));
                // SAFETY: moves our own window.
                let _ =
                    unsafe { SetWindowPos(hwnd, None, x, y, w, h, SWP_NOZORDER | SWP_NOACTIVATE) };
            }
            // SAFETY: repaint our own window.
            let _ = unsafe { InvalidateRect(Some(hwnd), None, false) };
            LRESULT(0)
        }
        WM_SETCURSOR => {
            let over = cursor_client(hwnd).and_then(|p| {
                STATE.with(|st| st.borrow().as_ref().and_then(|s| hit(&s.layout, p.x, p.y)))
            });
            if over.is_some() {
                fluent::set_cursor(true);
                return LRESULT(1);
            }
            // SAFETY: default handling.
            unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
        }
        WM_LBUTTONDOWN => {
            let (x, y) = (
                (lparam.0 & 0xffff) as i16 as i32,
                ((lparam.0 >> 16) & 0xffff) as i16 as i32,
            );
            let b = STATE.with(|st| {
                let mut st = st.borrow_mut();
                let st = st.as_mut()?;
                st.pressed = hit(&st.layout, x, y);
                st.focus = st.pressed.or(st.focus);
                st.pressed
            });
            if b.is_some() {
                // SAFETY: capture the mouse so the release is ours.
                unsafe { SetCapture(hwnd) };
                // SAFETY: repaint our own window.
                let _ = unsafe { InvalidateRect(Some(hwnd), None, false) };
            }
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            let (x, y) = (
                (lparam.0 & 0xffff) as i16 as i32,
                ((lparam.0 >> 16) & 0xffff) as i16 as i32,
            );
            // SAFETY: release our own capture.
            let _ = unsafe { ReleaseCapture() };
            let hit_b = STATE.with(|st| {
                let mut st = st.borrow_mut();
                let st = st.as_mut()?;
                let down = st.pressed.take();
                let up = hit(&st.layout, x, y);
                down.filter(|d| Some(*d) == up)
            });
            match hit_b {
                Some(b) => press(hwnd, b),
                // SAFETY: repaint our own window.
                None => {
                    let _ = unsafe { InvalidateRect(Some(hwnd), None, false) };
                }
            }
            LRESULT(0)
        }
        WM_KEYDOWN => {
            let key = wparam.0 as u16;
            if key == VK_ESCAPE.0 {
                answer(hwnd, false);
            } else if key == VK_RETURN.0 {
                let b =
                    STATE.with(|st| st.borrow().as_ref().and_then(|s| enter_presses(&s.dialog)));
                if let Some(b) = b {
                    press(hwnd, b);
                }
            } else if key == VK_TAB.0 {
                // SAFETY: plain key-state query.
                let back = unsafe { GetKeyState(VK_SHIFT.0 as i32) } < 0;
                STATE.with(|st| {
                    if let Some(st) = st.borrow_mut().as_mut() {
                        st.focus = next_focus(st.focus, st.dialog.is_question(), back);
                    }
                });
                // SAFETY: repaint our own window.
                let _ = unsafe { InvalidateRect(Some(hwnd), None, false) };
            } else if key == VK_SPACE.0 {
                let b = STATE.with(|st| st.borrow().as_ref().and_then(|s| s.focus));
                if let Some(b) = b {
                    press(hwnd, b);
                }
            }
            LRESULT(0)
        }
        // The close button and Alt+F4 mean "no".
        WM_CLOSE => {
            answer(hwnd, false);
            LRESULT(0)
        }
        WM_DESTROY => {
            // Safety net: a destroy by any other route still answers "no" once.
            let mut outcome = STATE.with(|st| {
                st.borrow_mut()
                    .as_mut()
                    .map(|st| std::mem::replace(&mut st.outcome, Outcome { done: None }))
            });
            if let Some(o) = outcome.as_mut() {
                o.finish(false);
            }
            fluent::unregister_window(hwnd);
            drop(SURFACE.with(|cell| cell.borrow_mut().take()));
            // SAFETY: our own timer and thread.
            unsafe {
                let _ = KillTimer(Some(hwnd), TIMER_ANIM);
                let _ = KillTimer(Some(hwnd), TIMER_ALIVE);
                PostQuitMessage(0);
            }
            LRESULT(0)
        }
        // SAFETY: default handling.
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

/// Ask a yes/no question; `done(true)` only for the primary button. Any other
/// way out (No, Esc, the close button) calls `done(false)`. Runs on its own
/// thread; `done` runs there, exactly once.
pub fn ask(d: Dialog, done: impl FnOnce(bool) + Send + 'static) {
    spawn(d, Box::new(done), None);
}

/// Like [`ask`], for a question that can die while it is open (an offer
/// that times out): `alive` is polled, and once it is false the dialog
/// closes itself and its buttons do nothing.
pub fn ask_while(
    d: Dialog,
    alive: impl Fn() -> bool + Send + 'static,
    done: impl FnOnce(bool) + Send + 'static,
) {
    spawn(d, Box::new(done), Some(Box::new(alive)));
}

/// Show a notice with one button.
pub fn info(d: Dialog) {
    spawn(d, Box::new(|_| {}), None);
}

type Alive = Box<dyn Fn() -> bool + Send>;

fn spawn(d: Dialog, done: Box<dyn FnOnce(bool) + Send>, alive: Option<Alive>) {
    std::thread::spawn(move || run(d, done, alive));
}

/// Dialogs open now: each new one is placed a little lower and to the
/// right, so several at once never hide each other.
static OPEN: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

struct OpenGuard(usize);

impl OpenGuard {
    fn new() -> Self {
        Self(OPEN.fetch_add(1, std::sync::atomic::Ordering::SeqCst))
    }
}

impl Drop for OpenGuard {
    fn drop(&mut self) {
        OPEN.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
    }
}

fn run(d: Dialog, done: Box<dyn FnOnce(bool) + Send>, alive: Option<Alive>) {
    let open = OpenGuard::new();
    // SAFETY: window creation and a message loop on this dedicated thread.
    unsafe {
        let Ok(hinstance) = GetModuleHandleW(None) else {
            // No window: an unanswered question is a "no".
            done(false);
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
            // No background brush: every pixel is ours (WM_ERASEBKGND).
            ..Default::default()
        };
        // Fails harmlessly when an earlier dialog registered it already.
        RegisterClassW(&wc);
        let style = WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU;
        let ex = WS_EX_APPWINDOW | WS_EX_TOPMOST;
        let title: Vec<u16> = d.title.encode_utf16().chain(Some(0)).collect();
        let Ok(hwnd) = CreateWindowExW(
            ex,
            CLASS,
            PCWSTR(title.as_ptr()),
            style,
            0,
            0,
            WIDTH,
            200,
            None,
            None,
            Some(hinstance.into()),
            None,
        ) else {
            done(false);
            return;
        };
        match Surface::new(hwnd, Backdrop::Opaque) {
            Ok(surface) => SURFACE.with(|cell| *cell.borrow_mut() = Some(surface)),
            Err(_) => {
                // Nothing can draw it: an unshown question is a "no".
                let _ = DestroyWindow(hwnd);
                done(false);
                return;
            }
        }
        let s = fluent::scale(hwnd);
        let lay = measure(s, &d);
        STATE.with(|st| {
            *st.borrow_mut() = Some(State {
                dialog: d.clone(),
                layout: lay,
                hover: [0.0; 2],
                pressed: None,
                focus: start_focus(&d),
                last_tick: Instant::now(),
                outcome: Outcome::new(done),
                alive,
            });
        });
        let (w, h) = frame_size(s, lay.height);
        let (sw, sh) = (GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN));
        let _ = SetWindowPos(
            hwnd,
            Some(HWND_TOPMOST),
            ((sw - w) / 2 + (open.0 % 6) as i32 * 32).min((sw - w).max(0)),
            (((sh - h) / 2) + (open.0 % 6) as i32 * 32).clamp(0, (sh - h).max(0)),
            w,
            h,
            SWP_NOACTIVATE,
        );
        fluent::frame(hwnd, false);
        fluent::register_window(hwnd);
        // One frame loop to settle the first hover; it stops itself. The alive check is a slow timer.
        let _ = SetTimer(Some(hwnd), TIMER_ANIM, TICK_MS, None);
        let _ = SetTimer(Some(hwnd), TIMER_ALIVE, ALIVE_MS, None);
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = SetForegroundWindow(hwnd);
        let _ = SetFocus(Some(hwnd));
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    STATE.with(|st| *st.borrow_mut() = None);
    drop(SURFACE.with(|cell| cell.borrow_mut().take()));
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    fn lum(rgb: u32) -> f64 {
        let ch = |sh: u32| {
            let c = ((rgb >> sh) & 0xFF) as f64 / 255.0;
            if c <= 0.03928 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * ch(16) + 0.7152 * ch(8) + 0.0722 * ch(0)
    }

    fn contrast(a: u32, b: u32) -> f64 {
        let (x, y) = (lum(a), lum(b));
        (x.max(y) + 0.05) / (x.min(y) + 0.05)
    }

    #[test]
    fn dialog_buttons_have_3_to_1_boundaries() {
        for dark in [false, true] {
            let p = fluent::palette_for(dark);
            for hover in [0.0, 1.0] {
                // The secondary button: a boundary in the control colour, 3:1 on the dialog surface.
                let (fill, stroke, ink) = button_style(&p, false, hover, false);
                let stroke = stroke.expect("a secondary button has a boundary");
                assert_eq!(stroke, p.outline_strong);
                assert!(contrast(stroke, p.bg) >= 3.0, "dark {dark}");
                assert!(contrast(ink, fill) >= 4.5, "label on its fill, dark {dark}");
                assert_ne!(fill, p.primary, "only the primary button is primary-filled");
            }
            // The primary button is filled with the accent and needs no boundary.
            let (fill, stroke, ink) = button_style(&p, true, 0.0, false);
            assert_eq!((fill, stroke, ink), (p.primary, None, p.on_primary));
            assert!(contrast(ink, fill) >= 4.5, "dark {dark}");
        }
    }

    #[test]
    fn the_dialog_timer_stops_when_settled() {
        // At rest: no hover value is moving, so no frame timer is needed.
        assert!(hover_settled(&[0.0, 0.0], None));
        assert!(!ease::needs_frame_timer(!hover_settled(&[0.0, 0.0], None)));
        assert!(hover_settled(&[1.0, 0.0], Some(Button::Primary)));
        // Moving toward a hovered button, or fading away from one that is not hovered: keep ticking.
        assert!(!hover_settled(&[0.4, 0.0], Some(Button::Primary)));
        assert!(!hover_settled(&[0.0, 0.7], None));
        assert!(ease::needs_frame_timer(!hover_settled(
            &[0.4, 0.0],
            Some(Button::Primary)
        )));
    }

    #[test]
    fn the_alive_check_has_its_own_slow_timer() {
        assert_ne!(TIMER_ANIM, TIMER_ALIVE);
        // Polling is_alive is not a frame loop (checked when the crate is built).
        const _: () = assert!(ALIVE_MS >= 250);
    }

    #[test]
    fn code_is_grouped_in_two_halves() {
        assert_eq!(group_code("628183"), "628 183");
        assert_eq!(group_code("1234"), "12 34");
        assert_eq!(group_code(" 628183 "), "628 183");
    }

    #[test]
    fn an_odd_length_code_gives_the_extra_digit_to_the_first_half() {
        assert_eq!(group_code("12345"), "123 45");
        assert_eq!(group_code("1234567"), "1234 567");
    }

    #[test]
    fn short_or_non_digit_codes_are_left_alone() {
        assert_eq!(group_code(""), "");
        assert_eq!(group_code("123"), "123");
        assert_eq!(group_code("ab12cd"), "ab12cd");
        assert_eq!(group_code("12 34"), "12 34");
        assert_eq!(
            group_code("\u{661}\u{662}\u{663}\u{664}"),
            "\u{661}\u{662}\u{663}\u{664}"
        );
    }

    #[test]
    fn the_pairing_dialog_carries_the_code_apart_from_the_text() {
        let d = Dialog::pairing("RMX3561", "628183");
        assert_eq!(d.heading, "Pair with \"RMX3561\"?");
        assert_eq!(d.code.as_deref(), Some("628 183"));
        assert!(!d.body.contains("628"));
        assert_eq!(d.primary, "Yes, it matches");
        assert_eq!(d.secondary.as_deref(), Some("No"));
        assert!(d.is_question());
    }

    #[test]
    fn done_runs_once_whatever_closes_the_dialog() {
        let calls = Cell::new(0);
        let last = Cell::new(None);
        let mut o = Outcome::new(|yes| {
            calls.set(calls.get() + 1);
            last.set(Some(yes));
        });
        assert!(o.finish(false));
        assert!(!o.finish(true));
        assert!(!o.finish(false));
        assert_eq!(calls.get(), 1);
        assert_eq!(last.get(), Some(false));
    }

    #[test]
    fn enter_never_answers_a_question_but_accepts_a_notice() {
        let q = Dialog::pairing("X", "123456");
        assert_eq!(enter_presses(&q), None);
        let n = Dialog::notice("Done".into(), String::new());
        assert_eq!(enter_presses(&n), Some(Button::Primary));
    }

    #[test]
    fn tab_cycles_between_the_buttons() {
        assert_eq!(next_focus(None, true, false), Some(Button::Primary));
        assert_eq!(
            next_focus(Some(Button::Primary), true, false),
            Some(Button::Secondary)
        );
        assert_eq!(
            next_focus(Some(Button::Secondary), true, false),
            Some(Button::Primary)
        );
        assert_eq!(next_focus(None, true, true), Some(Button::Secondary));
        assert_eq!(
            next_focus(Some(Button::Primary), true, true),
            Some(Button::Secondary)
        );
        assert_eq!(next_focus(None, false, false), Some(Button::Primary));
    }

    #[test]
    fn mix_blends_each_channel() {
        assert_eq!(mix(0x000000, 0xffffff, 0.0), 0x000000);
        assert_eq!(mix(0x000000, 0xffffff, 1.0), 0xffffff);
        assert_eq!(mix(0x102030, 0x102030, 0.7), 0x102030);
        assert_eq!(mix(0x000000, 0x0000fe, 0.5), 0x00007f);
        assert_eq!(mix(0x000000, 0xffffff, 9.0), 0xffffff);
    }

    fn overlaps(a: &RECT, b: &RECT) -> bool {
        a.left < b.right && b.left < a.right && a.top < b.bottom && b.top < a.bottom
    }

    #[test]
    fn nothing_overlaps_and_everything_fits_at_every_scale() {
        for s in [1.0, 1.25, 1.5, 2.0] {
            for (hh, bh) in [(20, 36), (40, 500), (900, 900), (20, 0)] {
                for code in [false, true] {
                    for two in [false, true] {
                        let (hh, bh) = (px(s, hh), px(s, bh));
                        let l = layout(s, hh, bh, code, two);
                        let mut parts = vec![l.heading, l.primary];
                        if l.body.bottom > l.body.top {
                            parts.push(l.body);
                        }
                        parts.extend(l.code);
                        parts.extend(l.secondary);
                        for (i, a) in parts.iter().enumerate() {
                            assert!(a.left >= 0 && a.right <= px(s, WIDTH), "{a:?} width");
                            assert!(a.top >= 0 && a.bottom <= l.height, "{a:?} height");
                            for b in &parts[i + 1..] {
                                assert!(!overlaps(a, b), "{a:?} overlaps {b:?} at {s}");
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn long_text_is_capped_not_pushed() {
        let s = 1.0;
        let l = layout(s, 10_000, 10_000, true, true);
        assert_eq!(l.heading.bottom - l.heading.top, px(s, HEADING_MAX));
        assert_eq!(l.body.bottom - l.body.top, px(s, BODY_MAX));
        assert!(l.height < px(s, 500));
    }

    #[test]
    fn two_buttons_split_the_width_and_one_sits_right() {
        let l = layout(1.0, 20, 36, false, true);
        let sec = l.secondary.unwrap();
        assert_eq!(l.primary.right - l.primary.left, sec.right - sec.left);
        assert!(l.primary.right < sec.left);
        assert_eq!(sec.right, WIDTH - MARGIN);
        let solo = layout(1.0, 20, 36, false, false);
        assert!(solo.secondary.is_none());
        assert_eq!(solo.primary.right, WIDTH - MARGIN);
    }

    #[test]
    fn hit_testing_finds_the_button_under_the_pointer() {
        let l = layout(1.0, 20, 36, true, true);
        let (p, s) = (l.primary, l.secondary.unwrap());
        assert_eq!(hit(&l, p.left + 1, p.top + 1), Some(Button::Primary));
        assert_eq!(hit(&l, s.right - 1, s.bottom - 1), Some(Button::Secondary));
        // The gap, the code card and the margins hit nothing.
        assert_eq!(hit(&l, p.right + 1, p.top + 1), None);
        assert_eq!(hit(&l, p.left, p.top - 1), None);
        assert_eq!(
            hit(&l, l.code.unwrap().left + 5, l.code.unwrap().top + 5),
            None
        );
    }

    #[test]
    fn dialog_buttons_keep_their_order_and_default() {
        // Primary (the answer that acts) on the left, the safe answer on the
        // right; a notice starts focused on its only button, a question on
        // neither.
        let l = layout(1.5, 30, 50, true, true);
        assert!(l.primary.right < l.secondary.unwrap().left);
        assert_eq!(start_focus(&Dialog::pairing("X", "123456")), None);
        assert_eq!(
            start_focus(&Dialog::notice("Done".into(), String::new())),
            Some(Button::Primary)
        );
    }

    #[cfg(windows)]
    #[test]
    fn dialog_measures_whole_lines_and_draws_in_a_hidden_window() {
        let hwnd = crate::d2d::test_window();
        let mut surface =
            crate::d2d::Surface::new(hwnd, crate::d2d::Backdrop::Opaque).expect("surface");
        let mut d = Dialog::pairing("RMX3561", "628183");
        d.body = "word ".repeat(200);
        for s in [1.0, 1.5, 2.0] {
            let l = measure(s, &d);
            let body = l.body.bottom - l.body.top;
            assert!(body > 0 && body <= px(s, BODY_MAX), "capped at {s}: {body}");
            let (_, line) = fluent::wrapped_height(fluent::Font::Body, s, "", 1.0);
            let lines = body as f32 / line;
            assert!((lines - lines.round()).abs() < 0.2, "whole lines: {lines}");
            assert!(l.height > l.primary.bottom);
        }
        let l = measure(1.0, &d);
        surface
            .draw(|dc| {
                let mut c = fluent::Canvas {
                    dc,
                    s: 1.0,
                    p: fluent::palette(),
                };
                let v = View {
                    dialog: &d,
                    layout: l,
                    hover: [0.5, 0.0],
                    focus: Some(Button::Secondary),
                    pressed: None,
                };
                paint_dialog(&mut c, &v);
            })
            .expect("draw");
        drop(surface);
        // SAFETY: our own test window, made above on this thread.
        let _ = unsafe { DestroyWindow(hwnd) };
    }

    #[test]
    fn after_a_dpi_change_the_dialog_stays_centred_on_the_suggestion() {
        let sug = rect(100, 200, 700, 600);
        let (x, y, w, h) = dpi_rect(sug, (800, 500));
        assert_eq!((w, h), (800, 500));
        assert_eq!((x + w / 2, y + h / 2), (400, 400));
    }

    #[test]
    fn the_frame_grows_with_the_scale() {
        let (w1, h1) = frame_size(1.0, 200);
        let (w2, h2) = frame_size(2.0, 400);
        assert!(w1 >= WIDTH && h1 > 200);
        assert!(w2 >= 2 * WIDTH && h2 > 400 && h2 - 400 > h1 - 200);
    }
}
