//! The transfer window: a small topmost window (never takes focus) that
//! appears when a file transfer starts and lists every transfer with its
//! progress, speed, ETA, "n of m", Cancel, Retry, Open and Show in folder.
//!
//! It draws from `transfers::rows()` on a 250 ms timer; the transport's
//! progress callback only stores numbers, so a transfer never floods the
//! message queue. Closing it dismisses finished rows; running transfers go
//! on and the window comes back when the next one starts or ends.
#![allow(unsafe_code)]

use std::{
    cell::RefCell,
    sync::atomic::{AtomicBool, AtomicIsize, Ordering},
};

use windows::{
    core::{w, PCWSTR},
    Win32::{
        Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
        Graphics::Gdi::{InvalidateRect, ScreenToClient, ValidateRect},
        System::LibraryLoader::GetModuleHandleW,
        UI::{
            Input::KeyboardAndMouse::{ReleaseCapture, SetCapture, VK_ESCAPE},
            WindowsAndMessaging::{
                CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetCursorPos,
                GetMessageW, KillTimer, LoadCursorW, PostMessageW, PostQuitMessage, RegisterClassW,
                SetTimer, SetWindowPos, ShowWindow, SystemParametersInfoW, TranslateMessage,
                HWND_TOPMOST, IDC_ARROW, MSG, SPI_GETWORKAREA, SWP_NOACTIVATE, SW_SHOWNOACTIVATE,
                WM_APP, WM_CLOSE, WM_DESTROY, WM_ERASEBKGND, WM_KEYDOWN, WM_LBUTTONDOWN,
                WM_LBUTTONUP, WM_MOUSEMOVE, WM_PAINT, WM_SETCURSOR, WM_TIMER, WNDCLASSW,
                WS_CAPTION, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_OVERLAPPED, WS_SYSMENU,
            },
        },
    },
};

use crate::{
    d2d::{Backdrop, Surface},
    fluent::{self, from_rect, px, rectf, round_rect, text, text_wrapped, Align, Canvas, Font},
    tokens::RADIUS,
    transfers::{self, CancelBy, Dir, Row, State},
};

const CLASS: PCWSTR = w!("RelayTransfers");
const TIMER: usize = 1;
const WM_POKE: u32 = WM_APP + 7;
const WIDTH: i32 = 440;
const MARGIN: i32 = 16;
const ROW_H: i32 = 112;
const MAX_ROWS: usize = 4;
const BUTTON_H: i32 = 28;

pub(crate) static HWND_RAW: AtomicIsize = AtomicIsize::new(0);
static OPENING: AtomicBool = AtomicBool::new(false);
/// The person closed the window while transfers were running: it stays away (the transfers carry on) until they
/// choose "Show file transfers" in the tray menu, or every transfer has ended.
static HIDDEN: AtomicBool = AtomicBool::new(false);

/// Whether the window stays hidden: only while something still runs.
pub fn next_hidden(hidden: bool, running: usize) -> bool {
    hidden && running > 0
}

fn running_count() -> usize {
    transfers::rows()
        .iter()
        .filter(|r| matches!(r.state, State::Waiting | State::Active))
        .count()
}

/// How many transfers run behind a hidden window (0 when the window is up or nothing runs).
pub fn hidden_running() -> usize {
    let running = running_count();
    let hidden = next_hidden(HIDDEN.load(Ordering::SeqCst), running);
    HIDDEN.store(hidden, Ordering::SeqCst);
    if hidden {
        running
    } else {
        0
    }
}

/// The tray's "Show file transfers": bring the window back.
pub fn show_again() {
    HIDDEN.store(false, Ordering::SeqCst);
    poke();
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Cancel,
    Retry,
    Open,
    Show,
    Dismiss,
}

impl Action {
    fn label(self) -> &'static str {
        match self {
            Action::Cancel => "Cancel",
            Action::Retry => "Retry",
            Action::Open => "Open",
            Action::Show => "Show in folder",
            Action::Dismiss => "Dismiss",
        }
    }
}

/// The buttons a row offers, left to right.
pub fn buttons_for(row: &Row) -> Vec<Action> {
    match row.state {
        State::Waiting | State::Active => vec![Action::Cancel],
        State::Done => {
            let mut v = Vec::new();
            if row.can_open {
                v.push(Action::Open);
                v.push(Action::Show);
            }
            v.push(Action::Dismiss);
            v
        }
        State::Failed(_) | State::Cancelled(_) => {
            let mut v = Vec::new();
            if row.can_retry {
                v.push(Action::Retry);
            }
            v.push(Action::Dismiss);
            v
        }
    }
}

fn rect(l: i32, t: i32, r: i32, b: i32) -> RECT {
    RECT {
        left: l,
        top: t,
        right: r,
        bottom: b,
    }
}

#[derive(Clone, Debug)]
pub struct RowLayout {
    pub name: RECT,
    pub label: RECT,
    pub status: RECT,
    pub bar: RECT,
    pub buttons: Vec<(Action, RECT)>,
}

/// Row layouts at scale `s` (device px) and the client height. Only the
/// newest `MAX_ROWS` rows are shown.
pub fn layout(s: f32, rows: &[Row]) -> (Vec<RowLayout>, i32) {
    let (m, w) = (px(s, MARGIN), px(s, WIDTH));
    let shown = rows.len().min(MAX_ROWS);
    let first = rows.len() - shown;
    let mut out = Vec::new();
    for (i, row) in rows[first..].iter().enumerate() {
        let top = m + i as i32 * px(s, ROW_H);
        let name = rect(m, top, w - m - px(s, 70), top + px(s, 22));
        let label = rect(w - m - px(s, 64), top, w - m, top + px(s, 22));
        let status = rect(m, top + px(s, 26), w - m, top + px(s, 62));
        let bar = rect(m, top + px(s, 66), w - m, top + px(s, 70));
        let by = top + px(s, 76);
        let mut x = m;
        let mut buttons = Vec::new();
        for a in buttons_for(row) {
            let bw = px(
                s,
                match a {
                    Action::Show => 118,
                    _ => 84,
                },
            );
            buttons.push((a, rect(x, by, x + bw, by + px(s, BUTTON_H))));
            x += bw + px(s, 8);
        }
        out.push(RowLayout {
            name,
            label,
            status,
            bar,
            buttons,
        });
    }
    (out, m * 2 + shown as i32 * px(s, ROW_H) - px(s, 8))
}

/// The (row index within the shown rows, action) under a point.
pub fn hit(layouts: &[RowLayout], x: i32, y: i32) -> Option<(usize, Action)> {
    layouts.iter().enumerate().find_map(|(i, l)| {
        l.buttons
            .iter()
            .find(|(_, r)| fluent::inside(r, x, y))
            .map(|(a, _)| (i, *a))
    })
}

fn shown_rows() -> Vec<Row> {
    let mut rows = transfers::rows();
    let n = rows.len().saturating_sub(MAX_ROWS);
    rows.drain(..n);
    rows
}

// ---- window ----

#[derive(Default)]
struct Ui {
    hover: Option<(usize, Action)>,
    pressed: Option<(usize, Action)>,
}

thread_local! {
    static UI: RefCell<Ui> = RefCell::new(Ui::default());
    static SURFACE: RefCell<Option<Surface>> = const { RefCell::new(None) };
}

/// A transfer started, changed state or ended: make sure the window is up
/// and repaints. Safe from any thread.
pub fn poke() {
    // Hidden by the person: do not pop up again on every progress update.
    let hidden = next_hidden(HIDDEN.load(Ordering::SeqCst), running_count());
    HIDDEN.store(hidden, Ordering::SeqCst);
    if hidden {
        return;
    }
    let raw = HWND_RAW.load(Ordering::SeqCst);
    if raw != 0 {
        // SAFETY: posting to our own window from any thread.
        let _ = unsafe { PostMessageW(Some(HWND(raw as *mut _)), WM_POKE, WPARAM(0), LPARAM(0)) };
        return;
    }
    if transfers::rows().is_empty() || OPENING.swap(true, Ordering::SeqCst) {
        return;
    }
    std::thread::spawn(|| {
        run();
        HWND_RAW.store(0, Ordering::SeqCst);
        OPENING.store(false, Ordering::SeqCst);
        // Something may have started while we were closing.
        if !transfers::rows().is_empty() {
            poke();
        }
    });
}

fn run() {
    // SAFETY: window creation and a message loop on this dedicated thread.
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
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
            CLASS,
            w!("Relay: file transfers"),
            WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU,
            0,
            0,
            px(1.0, WIDTH),
            200,
            None,
            None,
            Some(hinstance.into()),
            None,
        ) else {
            return;
        };
        match Surface::new(hwnd, Backdrop::Opaque) {
            Ok(s) => SURFACE.with(|c| *c.borrow_mut() = Some(s)),
            Err(_) => {
                let _ = DestroyWindow(hwnd);
                return;
            }
        }
        fluent::frame(hwnd, false);
        fluent::register_window(hwnd);
        HWND_RAW.store(hwnd.0 as isize, Ordering::SeqCst);
        place(hwnd);
        let _ = SetTimer(Some(hwnd), TIMER, 250, None);
        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    drop(SURFACE.with(|c| c.borrow_mut().take()));
}

/// Size to the rows and sit at the bottom right of the work area, without
/// taking focus.
fn place(hwnd: HWND) {
    let s = fluent::scale(hwnd);
    let (_, ch) = layout(s, &shown_rows());
    let mut r = rect(0, 0, px(s, WIDTH), ch.max(px(s, 60)));
    // SAFETY: plain computations and a move of our own window.
    unsafe {
        let _ = windows::Win32::UI::HiDpi::AdjustWindowRectExForDpi(
            &mut r,
            WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU,
            false,
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
            (s * 96.0).round() as u32,
        );
        let (w, h) = (r.right - r.left, r.bottom - r.top);
        let mut work = RECT::default();
        let _ = SystemParametersInfoW(
            SPI_GETWORKAREA,
            0,
            Some(&mut work as *mut RECT as *mut _),
            Default::default(),
        );
        let (x, y) = (work.right - w - px(s, 16), work.bottom - h - px(s, 16));
        let _ = SetWindowPos(hwnd, Some(HWND_TOPMOST), x, y, w, h, SWP_NOACTIVATE);
    }
}

fn client_pt(hwnd: HWND) -> Option<POINT> {
    let mut pt = POINT::default();
    // SAFETY: plain queries on our window.
    unsafe {
        GetCursorPos(&mut pt).ok()?;
        ScreenToClient(hwnd, &mut pt).as_bool().then_some(pt)
    }
}

fn act(row: &Row, a: Action) {
    match a {
        Action::Cancel => {
            transfers::cancel(row.id, CancelBy::User);
        }
        Action::Retry => transfers::retry(row.id),
        Action::Dismiss => transfers::dismiss(row.id),
        Action::Open | Action::Show => {
            let Some(p) = row.path.clone() else { return };
            std::thread::spawn(move || {
                if !transfers::file_actions(Some(&p)).open {
                    crate::ui::pill("File moved or deleted", 5);
                    return;
                }
                let s = p.to_string_lossy().into_owned();
                let ok = if a == Action::Open {
                    crate::ui::shell_open(&s, None)
                } else {
                    crate::ui::show_in_folder(&s)
                };
                if !ok {
                    crate::ui::pill("Windows couldn't open that file", 5);
                }
            });
        }
    }
}

extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let repaint = || {
        // SAFETY: repaint our own window.
        let _ = unsafe { InvalidateRect(Some(hwnd), None, false) };
    };
    match msg {
        WM_ERASEBKGND => LRESULT(1),
        WM_PAINT => {
            draw(hwnd);
            LRESULT(0)
        }
        WM_POKE | WM_TIMER => {
            if msg == WM_TIMER {
                transfers::tick();
            }
            if transfers::rows().is_empty() {
                // SAFETY: our own window.
                let _ = unsafe { DestroyWindow(hwnd) };
                return LRESULT(0);
            }
            place(hwnd);
            if let Some(p) = client_pt(hwnd) {
                let (l, _) = layout(fluent::scale(hwnd), &shown_rows());
                UI.with(|u| u.borrow_mut().hover = hit(&l, p.x, p.y));
            }
            repaint();
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            let (x, y) = (
                (lparam.0 & 0xffff) as i16 as i32,
                ((lparam.0 >> 16) & 0xffff) as i16 as i32,
            );
            let (l, _) = layout(fluent::scale(hwnd), &shown_rows());
            let h = hit(&l, x, y);
            let changed = UI.with(|u| std::mem::replace(&mut u.borrow_mut().hover, h) != h);
            if changed {
                repaint();
            }
            LRESULT(0)
        }
        WM_SETCURSOR => {
            let over = UI.with(|u| u.borrow().hover.is_some());
            if over {
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
            let (l, _) = layout(fluent::scale(hwnd), &shown_rows());
            let h = hit(&l, x, y);
            UI.with(|u| u.borrow_mut().pressed = h);
            if h.is_some() {
                // SAFETY: capture so the release is ours.
                unsafe { SetCapture(hwnd) };
                repaint();
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
            let rows = shown_rows();
            let (l, _) = layout(fluent::scale(hwnd), &rows);
            let up = hit(&l, x, y);
            let down = UI.with(|u| u.borrow_mut().pressed.take());
            if let (Some(d), Some(u)) = (down, up) {
                if d == u {
                    if let Some(row) = rows.get(d.0) {
                        act(row, d.1);
                    }
                }
            }
            repaint();
            LRESULT(0)
        }
        WM_KEYDOWN => {
            if wparam.0 as u16 == VK_ESCAPE.0 {
                // SAFETY: closing our own window through its own handler.
                let _ = unsafe { PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0)) };
            }
            LRESULT(0)
        }
        WM_CLOSE => {
            // Closing while something still runs hides the window; the tray menu brings it back.
            if running_count() > 0 {
                HIDDEN.store(true, Ordering::SeqCst);
            }
            // Finished rows go; running transfers carry on.
            for r in transfers::rows() {
                if !matches!(r.state, State::Waiting | State::Active) {
                    transfers::dismiss(r.id);
                }
            }
            // SAFETY: our own window.
            let _ = unsafe { DestroyWindow(hwnd) };
            LRESULT(0)
        }
        WM_DESTROY => {
            HWND_RAW.store(0, Ordering::SeqCst);
            fluent::unregister_window(hwnd);
            // SAFETY: our own timer and thread.
            unsafe {
                let _ = KillTimer(Some(hwnd), TIMER);
                PostQuitMessage(0);
            }
            LRESULT(0)
        }
        // SAFETY: default handling.
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

fn mix(a: u32, b: u32, t: f32) -> u32 {
    let ch = |sh: u32| {
        let (x, y) = (((a >> sh) & 0xff) as f32, ((b >> sh) & 0xff) as f32);
        ((x + (y - x) * t.clamp(0.0, 1.0)).round() as u32) << sh
    };
    ch(0) | ch(8) | ch(16)
}

fn draw(hwnd: HWND) {
    let painted = SURFACE.with(|cell| {
        let mut slot = cell.borrow_mut();
        let Some(surface) = slot.as_mut() else {
            return false;
        };
        fluent::paint_with(surface, hwnd, |c, _| {
            let rows = shown_rows();
            let (l, _) = layout(c.s, &rows);
            let (hover, pressed) = UI.with(|u| {
                let u = u.borrow();
                (u.hover, u.pressed)
            });
            paint_rows(c, &rows, &l, hover, pressed);
        });
        true
    });
    if !painted {
        // SAFETY: our own window; nothing to draw with.
        let _ = unsafe { ValidateRect(Some(hwnd), None) };
    }
}

fn paint_rows(
    c: &mut Canvas,
    rows: &[Row],
    layouts: &[RowLayout],
    hover: Option<(usize, Action)>,
    pressed: Option<(usize, Action)>,
) {
    let (s, p) = (c.s, c.p);
    for (i, (row, l)) in rows.iter().zip(layouts).enumerate() {
        let arrow = match row.dir {
            Dir::Send => "\u{2191} ",
            Dir::Receive => "\u{2193} ",
        };
        text(
            c,
            Font::BodyStrong,
            &format!("{arrow}{}", row.name),
            from_rect(l.name),
            p.text,
            Align::Left,
        );
        if let Some(label) = row.batch_label() {
            text(
                c,
                Font::Caption,
                &label,
                from_rect(l.label),
                p.text2,
                Align::Right,
            );
        }
        let status_ink = match row.state {
            State::Failed(_) => p.danger,
            _ => p.text2,
        };
        text_wrapped(
            c,
            Font::Caption,
            &row.status_line(),
            from_rect(l.status),
            status_ink,
        );
        // The bar: track, then the done part (indeterminate-looking at 0 %).
        let bar = from_rect(l.bar);
        round_rect(c, &bar, Some((p.stroke, 1.0)), None, 2.0 * s, false);
        let frac = f32::from(row.percent() as u16) / 100.0;
        if frac > 0.0 {
            let fill = rectf(
                bar.left,
                bar.top,
                bar.left + (bar.right - bar.left) * frac,
                bar.bottom,
            );
            let ink = match row.state {
                State::Failed(_) | State::Cancelled(_) => p.text2,
                State::Done => p.success,
                _ => p.primary,
            };
            round_rect(c, &fill, Some((ink, 1.0)), None, 2.0 * s, false);
        }
        for (a, r) in &l.buttons {
            let r = from_rect(*r);
            let key = Some((i, *a));
            let t = if hover == key { 1.0 } else { 0.0 };
            let down = pressed == key;
            let accent = matches!(a, Action::Retry | Action::Open);
            let (fill, stroke, ink) = if accent {
                let f = if down {
                    mix(p.primary, p.on_primary, 0.25)
                } else {
                    mix(p.primary, p.on_primary, t * 0.12)
                };
                (f, None, p.on_primary)
            } else {
                let f = if down {
                    mix(p.hover, p.stroke, 0.5)
                } else {
                    mix(p.card, p.hover, t)
                };
                (f, Some(p.stroke), p.text)
            };
            round_rect(c, &r, Some((fill, 1.0)), stroke, RADIUS[0] * s, false);
            text(c, Font::Body, a.label(), r, ink, Align::Center);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transfers::{Failure, Registry};
    use std::time::Instant;

    #[test]
    fn a_hidden_window_stays_hidden_while_transfers_run_and_resets_when_they_end() {
        assert!(
            next_hidden(true, 2),
            "still running: stay hidden, no pop-up on every update"
        );
        assert!(
            !next_hidden(true, 0),
            "all done: the next transfer shows the window again"
        );
        assert!(!next_hidden(false, 3), "never hidden by itself");
    }

    fn row_in(state: &str) -> Row {
        let mut r = Registry::new();
        let now = Instant::now();
        let (id, _) = r.add(
            transfers::NewTransfer {
                dir: Dir::Send,
                peer: "Pixel".into(),
                name: "a.zip".into(),
                size: 100,
                path: Some("C:\\a.zip".into()),
                batch: None,
            },
            now,
        );
        match state {
            "active" => r.set_progress(id, 10, 100),
            "done" => r.finish(id, None, now),
            "failed" => r.fail(
                id,
                Failure {
                    title: "x".into(),
                    hint: String::new(),
                },
                now,
            ),
            "cancelled" => {
                r.cancel(id, CancelBy::User, now);
            }
            _ => {}
        }
        r.row(id, now).unwrap()
    }

    #[test]
    fn a_running_row_offers_only_cancel() {
        assert_eq!(buttons_for(&row_in("waiting")), vec![Action::Cancel]);
        assert_eq!(buttons_for(&row_in("active")), vec![Action::Cancel]);
    }

    #[test]
    fn a_done_row_offers_open_show_dismiss() {
        assert_eq!(
            buttons_for(&row_in("done")),
            vec![Action::Open, Action::Show, Action::Dismiss]
        );
    }

    #[test]
    fn a_failed_or_cancelled_send_offers_retry() {
        for st in ["failed", "cancelled"] {
            assert_eq!(
                buttons_for(&row_in(st)),
                vec![Action::Retry, Action::Dismiss],
                "{st}"
            );
        }
    }

    #[test]
    fn buttons_are_hit_where_they_are_drawn_and_nowhere_else() {
        let rows = [row_in("done"), row_in("active")];
        let (l, h) = layout(1.0, &rows);
        assert!(h > 0);
        let (a, r) = l[0].buttons[1];
        assert_eq!(a, Action::Show);
        assert_eq!(hit(&l, r.left + 2, r.top + 2), Some((0, Action::Show)));
        let (a, r) = l[1].buttons[0];
        assert_eq!(a, Action::Cancel);
        assert_eq!(hit(&l, r.left + 2, r.top + 2), Some((1, Action::Cancel)));
        assert_eq!(hit(&l, 1, 1), None);
        // Buttons are at least 16 px tall and wide.
        for row in &l {
            for (_, r) in &row.buttons {
                assert!(r.bottom - r.top >= 16 && r.right - r.left >= 16);
            }
        }
    }

    #[test]
    fn rows_do_not_overlap_and_more_than_four_show_the_newest() {
        let rows: Vec<Row> = (0..6).map(|_| row_in("active")).collect();
        let (l, _) = layout(1.0, &rows);
        assert_eq!(l.len(), MAX_ROWS);
        for pair in l.windows(2) {
            assert!(pair[0].buttons[0].1.bottom <= pair[1].name.top);
        }
    }
}

#[cfg(test)]
#[path = "transfer_window_ux.rs"]
mod transfer_window_ux;
