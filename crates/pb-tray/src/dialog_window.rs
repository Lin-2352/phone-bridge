//! Fluent modal dialog for the questions the tray asks (pair with this
//! phone? allow control? accept this file?) and for plain notices. It
//! replaces the classic Win32 message box so every window looks alike.
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
        Graphics::Gdi::{
            CreateFontW, DeleteObject, GetDC, InvalidateRect, ReleaseDC, ScreenToClient,
            SelectObject, SetTextCharacterExtra, DT_CALCRECT, DT_CENTER, DT_END_ELLIPSIS, DT_LEFT,
            DT_NOPREFIX, DT_SINGLELINE, DT_VCENTER, DT_WORDBREAK, HDC, HGDIOBJ,
        },
        System::LibraryLoader::GetModuleHandleW,
        UI::{
            Input::KeyboardAndMouse::{
                GetKeyState, ReleaseCapture, SetCapture, SetFocus, VK_ESCAPE, VK_RETURN, VK_SHIFT,
                VK_SPACE, VK_TAB,
            },
            WindowsAndMessaging::{
                AdjustWindowRectEx, CreateWindowExW, DefWindowProcW, DestroyWindow,
                DispatchMessageW, GetCursorPos, GetMessageW, GetSystemMetrics, KillTimer,
                LoadCursorW, LoadIconW, PostQuitMessage, RegisterClassW, SetForegroundWindow,
                SetTimer, SetWindowPos, ShowWindow, TranslateMessage, HWND_TOPMOST, IDC_ARROW, MSG,
                SM_CXSCREEN, SM_CYSCREEN, SWP_NOACTIVATE, SW_SHOW, WM_CLOSE, WM_DESTROY,
                WM_ERASEBKGND, WM_GETDLGCODE, WM_KEYDOWN, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_PAINT,
                WM_SETCURSOR, WM_TIMER, WNDCLASSW, WS_CAPTION, WS_EX_APPWINDOW, WS_EX_TOPMOST,
                WS_OVERLAPPED, WS_SYSMENU,
            },
        },
    },
};

use crate::{
    ease,
    fluent::{self, px},
};

const CLASS: PCWSTR = w!("PhoneBridgeDialog");
const TIMER_ANIM: usize = 1;
const TICK_MS: u32 = 16;
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

/// Blend `a` toward `b` (COLORREF 0x00BBGGRR), `t` in 0..=1.
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
}

thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
}

fn idx(b: Button) -> usize {
    match b {
        Button::Primary => 0,
        Button::Secondary => 1,
    }
}

/// Height of `s` wrapped to `width` in `font`.
fn text_height(hdc: HDC, font: HGDIOBJ, s: &str, width: i32) -> i32 {
    if s.is_empty() {
        return 0;
    }
    let mut w: Vec<u16> = s.encode_utf16().collect();
    let mut r = rect(0, 0, width, 0);
    // SAFETY: valid DC; DT_CALCRECT only measures; the font is restored.
    unsafe {
        let old = SelectObject(hdc, font);
        windows::Win32::Graphics::Gdi::DrawTextW(
            hdc,
            &mut w,
            &mut r,
            DT_CALCRECT | DT_WORDBREAK | DT_NOPREFIX,
        );
        SelectObject(hdc, old);
    }
    r.bottom - r.top
}

fn measure(hdc: HDC, f: &fluent::Fonts, s: f32, d: &Dialog) -> Layout {
    let width = px(s, WIDTH - 2 * MARGIN);
    layout(
        s,
        text_height(hdc, f.title, &d.heading, width),
        text_height(hdc, f.body, &d.body, width),
        d.code.is_some(),
        d.secondary.is_some(),
    )
}

fn big_font(s: f32) -> HGDIOBJ {
    // SAFETY: plain font creation; the caller deletes it.
    let f = unsafe {
        CreateFontW(
            -px(s, 32),
            0,
            0,
            0,
            600,
            0,
            0,
            0,
            Default::default(),
            Default::default(),
            Default::default(),
            windows::Win32::Graphics::Gdi::FONT_QUALITY(5),
            0,
            w!("Consolas"),
        )
    };
    HGDIOBJ(f.0)
}

fn draw(hwnd: HWND) {
    let s = fluent::scale(hwnd);
    let p = fluent::palette();
    fluent::paint(hwnd, |hdc, _c| {
        let f = fluent::Fonts::new(s);
        let (l, d, hover, focus, pressed) = STATE.with(|st| {
            let mut st = st.borrow_mut();
            let Some(st) = st.as_mut() else {
                return (Layout::default(), None, [0.0; 2], None, None);
            };
            st.layout = measure(hdc, &f, s, &st.dialog);
            (
                st.layout,
                Some(st.dialog.clone()),
                st.hover,
                st.focus,
                st.pressed,
            )
        });
        let Some(d) = d else { return };
        let wrap = DT_LEFT | DT_WORDBREAK | DT_END_ELLIPSIS | DT_NOPREFIX;
        fluent::text(hdc, f.title, &d.heading, l.heading, p.text, wrap);
        fluent::text(hdc, f.body, &d.body, l.body, p.text2, wrap);
        if let (Some(r), Some(code)) = (l.code, &d.code) {
            fluent::round_rect(
                hdc,
                &r,
                p.card,
                Some(p.stroke),
                fluent::corner_radius(s, 8),
                false,
            );
            let font = big_font(s);
            // SAFETY: valid DC; spacing is reset right after drawing.
            unsafe { SetTextCharacterExtra(hdc, px(s, 4)) };
            fluent::text(
                hdc,
                font,
                code,
                r,
                p.text,
                DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX,
            );
            // SAFETY: as above.
            unsafe {
                SetTextCharacterExtra(hdc, 0);
                let _ = DeleteObject(font);
            }
        }
        let button = |b: Button, r: RECT, label: &str, accent: bool| {
            let t = hover[idx(b)];
            let down = pressed == Some(b);
            let (fill, stroke, ink) = if accent {
                let base = mix(p.accent, p.on_accent, t * 0.12);
                let fill = if down {
                    mix(p.accent, p.on_accent, 0.25)
                } else {
                    base
                };
                (fill, None, p.on_accent)
            } else {
                let fill = if down {
                    mix(p.hover, p.stroke, 0.5)
                } else {
                    mix(p.card, p.hover, t)
                };
                (fill, Some(p.stroke), p.text)
            };
            fluent::round_rect(hdc, &r, fill, stroke, fluent::corner_radius(s, 8), false);
            fluent::text(
                hdc,
                f.body,
                label,
                r,
                ink,
                DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS | DT_NOPREFIX,
            );
            if focus == Some(b) {
                let ring = RECT {
                    left: r.left - px(s, 2),
                    top: r.top - px(s, 2),
                    right: r.right + px(s, 2),
                    bottom: r.bottom + px(s, 2),
                };
                fluent::round_rect(
                    hdc,
                    &ring,
                    p.bg,
                    Some(p.text),
                    fluent::corner_radius(s, 10),
                    false,
                );
                // Redraw the button face inside the ring.
                fluent::round_rect(hdc, &r, fill, stroke, fluent::corner_radius(s, 8), false);
                fluent::text(
                    hdc,
                    f.body,
                    label,
                    r,
                    ink,
                    DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS | DT_NOPREFIX,
                );
            }
        };
        button(Button::Primary, l.primary, &d.primary, true);
        if let (Some(r), Some(label)) = (l.secondary, &d.secondary) {
            button(Button::Secondary, r, label, false);
        }
    });
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
    answer(hwnd, b == Button::Primary);
}

fn cursor_client(hwnd: HWND) -> Option<POINT> {
    let mut pt = POINT::default();
    // SAFETY: plain queries on our window.
    unsafe {
        GetCursorPos(&mut pt).ok()?;
        ScreenToClient(hwnd, &mut pt).as_bool().then_some(pt)
    }
}

fn tick(hwnd: HWND) {
    let pt = cursor_client(hwnd);
    let changed = STATE.with(|st| {
        let mut st = st.borrow_mut();
        let Some(st) = st.as_mut() else { return false };
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
        changed
    });
    if changed {
        // SAFETY: repaint our own window.
        let _ = unsafe { InvalidateRect(Some(hwnd), None, false) };
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
            tick(hwnd);
            LRESULT(0)
        }
        WM_GETDLGCODE => LRESULT(DLGC_WANTALLKEYS),
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
            // SAFETY: our own timer and thread.
            unsafe {
                let _ = KillTimer(Some(hwnd), TIMER_ANIM);
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
    spawn(d, Box::new(done));
}

/// Show a notice with one button.
pub fn info(d: Dialog) {
    spawn(d, Box::new(|_| {}));
}

fn spawn(d: Dialog, done: Box<dyn FnOnce(bool) + Send>) {
    std::thread::spawn(move || run(d, done));
}

fn run(d: Dialog, done: Box<dyn FnOnce(bool) + Send>) {
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
        let s = fluent::scale(hwnd);
        let dc = GetDC(Some(hwnd));
        let fonts = fluent::Fonts::new(s);
        let lay = measure(dc, &fonts, s, &d);
        drop(fonts);
        ReleaseDC(Some(hwnd), dc);
        STATE.with(|st| {
            *st.borrow_mut() = Some(State {
                dialog: d.clone(),
                layout: lay,
                hover: [0.0; 2],
                pressed: None,
                // A notice starts on its only button; a question starts on neither.
                focus: (!d.is_question()).then_some(Button::Primary),
                last_tick: Instant::now(),
                outcome: Outcome::new(done),
            });
        });
        let mut r = rect(0, 0, px(s, WIDTH), lay.height);
        let _ = AdjustWindowRectEx(&mut r, style, false, ex);
        let (w, h) = (r.right - r.left, r.bottom - r.top);
        let (sw, sh) = (GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN));
        let _ = SetWindowPos(
            hwnd,
            Some(HWND_TOPMOST),
            (sw - w) / 2,
            ((sh - h) / 2).max(0),
            w,
            h,
            SWP_NOACTIVATE,
        );
        fluent::frame(hwnd, false);
        let _ = SetTimer(Some(hwnd), TIMER_ANIM, TICK_MS, None);
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

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
}
