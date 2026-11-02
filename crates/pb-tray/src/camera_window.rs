//! "Phone camera": one window that answers "does the camera show up?" It
//! lists what has to be true (phone, install, Windows camera access, the
//! stream, Windows' device list), shows a live preview read the way an app
//! reads it, and holds the few buttons that matter (start or stop, switch
//! lens, Windows' camera privacy page, remove).
#![allow(unsafe_code)]

use std::{
    cell::RefCell,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};

use windows::{
    core::{w, PCWSTR},
    Win32::{
        Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM},
        Graphics::{
            Direct2D::{
                Common::{D2D1_ALPHA_MODE_IGNORE, D2D1_PIXEL_FORMAT, D2D_SIZE_U},
                D2D1_BITMAP_PROPERTIES1, D2D1_INTERPOLATION_MODE_LINEAR,
            },
            Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM,
            Gdi::{InvalidateRect, ValidateRect},
        },
        System::{
            Com::{CoInitializeEx, CoUninitialize, COINIT_MULTITHREADED},
            LibraryLoader::GetModuleHandleW,
        },
        UI::{
            Shell::ShellExecuteW,
            WindowsAndMessaging::{
                AdjustWindowRectEx, CreateWindowExW, DefWindowProcW, DestroyWindow,
                GetSystemMetrics, LoadCursorW, LoadIconW, PostMessageW, RegisterClassW,
                SetForegroundWindow, SetWindowPos, ShowWindow, IDC_ARROW, SM_CXSCREEN, SM_CYSCREEN,
                SWP_NOZORDER, SW_RESTORE, SW_SHOW, SW_SHOWNORMAL, WM_APP, WM_CLOSE, WM_DESTROY,
                WM_ERASEBKGND, WM_LBUTTONUP, WM_MOUSEMOVE, WM_PAINT, WNDCLASSW, WS_CAPTION,
                WS_EX_APPWINDOW, WS_MINIMIZEBOX, WS_OVERLAPPED, WS_SYSMENU,
            },
        },
    },
};

use crate::{
    camera_preview::{Preview, Snapshot},
    camera_status::{button, offer_repair, rows, CameraButton, Inputs, Mark, Row},
    d2d::{Backdrop, Surface},
    fluent::{self, from_rect, glyph, px, rectf, Align, Canvas, Font, RectF, GROUP_GAP},
    tokens,
};

const CLASS: PCWSTR = w!("PhoneBridgeCamera");
const W: i32 = 520;
const H: i32 = 688;
const WM_STATUS: u32 = WM_APP + 1;
const ROW_H: i32 = 36;

/// What the buttons ask for; the tray turns them into its own commands.
pub enum Action {
    Toggle,
    Lens,
    Remove,
    Repair,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Button {
    Toggle,
    Lens,
    Privacy,
    Remove,
    Repair,
}

/// While the camera is off and needs repair, "Repair camera" takes the place
/// of "Switch lens" (the lens only matters while it streams).
fn shown(b: Button, repair: bool) -> Button {
    if b == Button::Lens && repair {
        Button::Repair
    } else {
        b
    }
}

struct State {
    hwnd: isize,
    rows: Vec<Row>,
    streaming: bool,
    pending: bool,
    repair: bool,
    preview: Option<Preview>,
    hover: Option<Button>,
    on_action: Box<dyn Fn(Action) + Send>,
    collector_stop: Arc<AtomicBool>,
}

static CURRENT: Mutex<Option<State>> = Mutex::new(None);

thread_local! {
    /// The window's Direct2D surface (COM objects stay on the UI thread),
    /// with the window it belongs to.
    static SURFACE: RefCell<Option<(isize, Surface)>> = const { RefCell::new(None) };
}

pub fn close() {
    let h = CURRENT.lock().ok().and_then(|g| g.as_ref().map(|s| s.hwnd));
    if let Some(h) = h {
        // SAFETY: posting to our own window.
        let _ = unsafe { PostMessageW(Some(HWND(h as *mut _)), WM_CLOSE, WPARAM(0), LPARAM(0)) };
    }
}

fn post_repaint(hwnd: isize) {
    // SAFETY: posting to our own window from any thread.
    let _ = unsafe { PostMessageW(Some(HWND(hwnd as *mut _)), WM_STATUS, WPARAM(0), LPARAM(0)) };
}

struct Layout {
    rows: RECT,
    preview: RECT,
    buttons: [RECT; 4],
}

fn layout(s: f32) -> Layout {
    let p = |v: i32| px(s, v);
    let rows_top = 64;
    // Six rows, a seventh when the camera could not start.
    let rows_bottom = rows_top + 7 * ROW_H;
    let pw = W - 2 * GROUP_GAP;
    let ph = pw * 9 / 16;
    let bw = (W - 2 * GROUP_GAP - 3 * 8) / 4;
    Layout {
        rows: RECT {
            left: p(GROUP_GAP),
            top: p(rows_top),
            right: p(W - GROUP_GAP),
            bottom: p(rows_bottom),
        },
        preview: RECT {
            left: p(GROUP_GAP),
            top: p(rows_bottom + 8),
            right: p(GROUP_GAP + pw),
            bottom: p(rows_bottom + 8 + ph),
        },
        buttons: std::array::from_fn(|i| {
            let l = GROUP_GAP + i as i32 * (bw + 8);
            RECT {
                left: p(l),
                top: p(rows_bottom + 8 + ph + GROUP_GAP),
                right: p(l + bw),
                bottom: p(rows_bottom + 8 + ph + GROUP_GAP + 36),
            }
        }),
    }
}

const BUTTONS: [Button; 4] = [
    Button::Toggle,
    Button::Lens,
    Button::Privacy,
    Button::Remove,
];

fn button_label(b: Button, streaming: bool, pending: bool) -> &'static str {
    match b {
        Button::Toggle => match button(streaming, pending) {
            CameraButton::Start => "Start camera",
            CameraButton::Cancel => "Cancel",
            CameraButton::Stop => "Stop camera",
        },
        Button::Lens => "Switch lens",
        Button::Privacy => "Camera privacy",
        Button::Remove => "Remove",
        Button::Repair => "Repair camera",
    }
}

fn mark_glyph(m: Mark, idle: u32) -> (char, u32) {
    // 0xRRGGBB.
    match m {
        Mark::Ok => ('\u{E73E}', 0x00B050),
        Mark::Bad => (glyph::CLOSE, 0xE81123),
        Mark::Wait => ('\u{E916}', 0xFFB900),
        Mark::Idle => ('\u{E739}', idle),
    }
}

/// What one paint shows, copied out of the state so drawing holds no lock.
struct View {
    rows: Vec<Row>,
    streaming: bool,
    pending: bool,
    repair: bool,
    hover: Option<Button>,
    snapshot: Option<Snapshot>,
}

fn paint(hwnd: HWND) {
    let view = {
        let Ok(guard) = CURRENT.lock() else { return };
        let Some(st) = guard.as_ref() else { return };
        View {
            rows: st.rows.clone(),
            streaming: st.streaming,
            pending: st.pending,
            repair: st.repair,
            hover: st.hover,
            snapshot: st.preview.as_ref().map(Preview::snapshot),
        }
    };
    SURFACE.with(|cell| {
        let mut slot = cell.borrow_mut();
        if slot.as_ref().map(|(h, _)| *h) != Some(hwnd.0 as isize) {
            *slot = Surface::new(hwnd, Backdrop::Opaque)
                .ok()
                .map(|s| (hwnd.0 as isize, s));
        }
        match slot.as_mut() {
            Some((_, surface)) => fluent::paint_with(surface, hwnd, |c, _| draw(c, &view)),
            // Nothing can draw (no Direct2D at all): validate so WM_PAINT
            // does not repeat forever.
            // SAFETY: our own window.
            None => unsafe {
                let _ = ValidateRect(Some(hwnd), None);
            },
        }
    });
}

/// The camera's picture (top-down BGRA) stretched into `r`. Its alpha
/// byte is ignored, as the GDI blit ignored it.
fn draw_picture(c: &Canvas, x: &Snapshot, r: RectF) {
    if x.width == 0 || x.bgra.len() < x.width as usize * x.height as usize * 4 {
        return;
    }
    let props = D2D1_BITMAP_PROPERTIES1 {
        pixelFormat: D2D1_PIXEL_FORMAT {
            format: DXGI_FORMAT_B8G8R8A8_UNORM,
            alphaMode: D2D1_ALPHA_MODE_IGNORE,
        },
        dpiX: 96.0,
        dpiY: 96.0,
        ..Default::default()
    };
    let size = D2D_SIZE_U {
        width: x.width,
        height: x.height,
    };
    // SAFETY: the buffer holds width x height BGRA pixels (checked above);
    // drawing calls inside the surface's BeginDraw/EndDraw.
    unsafe {
        if let Ok(bmp) =
            c.dc.CreateBitmap(size, Some(x.bgra.as_ptr().cast()), x.width * 4, &props)
        {
            c.dc.DrawBitmap(
                &bmp,
                Some(&r),
                1.0,
                D2D1_INTERPOLATION_MODE_LINEAR,
                None,
                None,
            );
        }
    }
}

fn draw(c: &mut Canvas, v: &View) {
    let (s, p) = (c.s, c.p);
    let l = layout(s);
    let f = |v: i32| px(s, v) as f32;
    fluent::text(
        c,
        Font::Subtitle,
        "Phone camera",
        rectf(f(16), f(16), f(W - 16), f(48)),
        p.text,
        Align::Left,
    );
    let verdict = if v.rows.is_empty() {
        ""
    } else if crate::camera_status::all_ok(&v.rows) {
        "Working: apps can use Relay Camera"
    } else {
        "Not working yet: see the marks below"
    };
    fluent::text(
        c,
        Font::Caption,
        verdict,
        rectf(f(16), f(40), f(W - 16), f(62)),
        p.text2,
        Align::Left,
    );
    let (left, right) = (l.rows.left as f32, l.rows.right as f32);
    for (i, r) in v.rows.iter().enumerate() {
        let top = (l.rows.top + i as i32 * px(s, ROW_H)) as f32;
        let bottom = top + f(ROW_H);
        let (g, color) = mark_glyph(r.mark, p.text2);
        fluent::icon(c, g, rectf(left, top, left + f(28), bottom), color);
        fluent::text(
            c,
            Font::Body,
            r.label,
            rectf(left + f(34), top, left + f(200), bottom),
            p.text,
            Align::Left,
        );
        fluent::text(
            c,
            Font::Caption,
            &r.detail,
            rectf(left + f(204), top, right, bottom),
            if r.mark == Mark::Bad {
                0xE81123
            } else {
                p.text2
            },
            Align::Left,
        );
    }
    // Preview: what an app gets (or a hint why not).
    let preview = from_rect(l.preview);
    fluent::round_rect(
        c,
        &preview,
        Some((p.card, 1.0)),
        Some(p.stroke),
        tokens::RADIUS[0] * s,
        false,
    );
    match v
        .snapshot
        .as_ref()
        .filter(|x| x.width > 0 && !x.bgra.is_empty())
    {
        Some(x) => {
            let inset = f(2);
            let r = rectf(
                preview.left + inset,
                preview.top + inset,
                preview.right - inset,
                preview.bottom - inset,
            );
            draw_picture(c, x, r);
        }
        None => fluent::text(
            c,
            Font::Caption,
            if v.streaming {
                "Opening the camera like an app does\u{2026}"
            } else {
                "Start the camera to see what apps get"
            },
            preview,
            p.text2,
            Align::Center,
        ),
    }
    for (r, b) in l.buttons.iter().zip(BUTTONS) {
        let b = shown(b, v.repair);
        let accent = b == Button::Toggle;
        let r = from_rect(*r);
        let label = button_label(b, v.streaming, v.pending);
        if v.hover == Some(b) && !accent {
            // GDI took the corner's diameter (6 px); Direct2D the radius.
            fluent::round_rect(c, &r, Some((p.hover, 1.0)), Some(p.stroke), 3.0 * s, false);
            // The label in the same font as the plain button, so it does not jump.
            fluent::text(c, Font::Body, label, r, p.text, Align::Center);
        } else {
            fluent::button(c, r, label, accent);
        }
    }
}

fn hit(hwnd: HWND, x: i32, y: i32) -> Option<Button> {
    let l = layout(fluent::scale(hwnd));
    let repair = CURRENT
        .lock()
        .ok()
        .and_then(|g| g.as_ref().map(|s| s.repair))
        .unwrap_or(false);
    l.buttons
        .iter()
        .zip(BUTTONS)
        .find(|(r, _)| fluent::inside(r, x, y))
        .map(|(_, b)| shown(b, repair))
}

fn open_privacy_settings() {
    // SAFETY: opens Windows' own settings page for the user to change.
    unsafe {
        ShellExecuteW(
            None,
            w!("open"),
            w!("ms-settings:privacy-webcam"),
            None,
            None,
            SW_SHOWNORMAL,
        );
    }
}

extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    // SAFETY: standard window procedure for our own window.
    unsafe {
        match msg {
            WM_ERASEBKGND => LRESULT(1),
            WM_PAINT => {
                paint(hwnd);
                LRESULT(0)
            }
            WM_STATUS => {
                let _ = InvalidateRect(Some(hwnd), None, false);
                LRESULT(0)
            }
            WM_MOUSEMOVE => {
                let (x, y) = (
                    (lparam.0 & 0xffff) as i16 as i32,
                    ((lparam.0 >> 16) & 0xffff) as i16 as i32,
                );
                let h = hit(hwnd, x, y);
                if let Ok(mut g) = CURRENT.lock() {
                    if let Some(st) = g.as_mut() {
                        if st.hover != h {
                            st.hover = h;
                            let _ = InvalidateRect(Some(hwnd), None, false);
                        }
                    }
                }
                fluent::set_cursor(h.is_some());
                LRESULT(0)
            }
            WM_LBUTTONUP => {
                let (x, y) = (
                    (lparam.0 & 0xffff) as i16 as i32,
                    ((lparam.0 >> 16) & 0xffff) as i16 as i32,
                );
                if let Some(b) = hit(hwnd, x, y) {
                    match b {
                        Button::Privacy => open_privacy_settings(),
                        Button::Toggle | Button::Lens | Button::Remove | Button::Repair => {
                            if let Ok(g) = CURRENT.lock() {
                                if let Some(st) = g.as_ref() {
                                    (st.on_action)(match b {
                                        Button::Toggle => Action::Toggle,
                                        Button::Lens => Action::Lens,
                                        Button::Repair => Action::Repair,
                                        _ => Action::Remove,
                                    });
                                }
                            }
                        }
                    }
                }
                LRESULT(0)
            }
            WM_CLOSE => {
                let _ = DestroyWindow(hwnd);
                LRESULT(0)
            }
            WM_DESTROY => {
                fluent::unregister_window(hwnd);
                // Taken out first, dropped outside the borrow (dropping the last
                // surface runs the device idle hook).
                let surface = SURFACE.with(|cell| {
                    let mut slot = cell.borrow_mut();
                    if slot.as_ref().is_some_and(|(h, _)| *h == hwnd.0 as isize) {
                        slot.take()
                    } else {
                        None
                    }
                });
                drop(surface);
                let st = CURRENT.lock().ok().and_then(|mut g| g.take());
                if let Some(mut st) = st {
                    st.collector_stop.store(true, Ordering::Release);
                    if let Some(mut p) = st.preview.take() {
                        p.stop();
                    }
                }
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

/// Open (or raise) the window. `phone` names the paired phone, if any.
pub fn open(
    phone: Arc<dyn Fn() -> Option<String> + Send + Sync>,
    on_action: Box<dyn Fn(Action) + Send>,
) {
    if let Some(h) = CURRENT.lock().ok().and_then(|g| g.as_ref().map(|s| s.hwnd)) {
        // SAFETY: our own window.
        unsafe {
            let _ = ShowWindow(HWND(h as *mut _), SW_RESTORE);
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
            // The app icon (resource 1), as the settings window has it.
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
            w!("Relay camera"),
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
        let stop = Arc::new(AtomicBool::new(false));
        if let Ok(mut g) = CURRENT.lock() {
            *g = Some(State {
                hwnd: hwnd.0 as isize,
                rows: Vec::new(),
                streaming: false,
                pending: false,
                repair: false,
                preview: None,
                hover: None,
                on_action,
                collector_stop: stop.clone(),
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
        fluent::register_window(hwnd);
        let _ = ShowWindow(hwnd, SW_SHOW);
        let raw = hwnd.0 as isize;
        std::thread::spawn(move || collect(raw, phone, stop));
    }
}

/// Every 1.5 s: gather the facts, start or stop the preview, repaint.
fn collect(
    hwnd: isize,
    phone: Arc<dyn Fn() -> Option<String> + Send + Sync>,
    stop: Arc<AtomicBool>,
) {
    // SAFETY: COM and Media Foundation on this worker thread (the device list).
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let _ = windows::Win32::Media::MediaFoundation::MFStartup(
            windows::Win32::Media::MediaFoundation::MF_VERSION,
            windows::Win32::Media::MediaFoundation::MFSTARTUP_FULL,
        );
    }
    while !stop.load(Ordering::Acquire) {
        let streaming = crate::camera::is_running();
        let pending = crate::camera::pending();
        let listed = streaming.then(|| {
            crate::camera_device::list_camera_names()
                .iter()
                .any(|n| crate::camera_device::is_our_camera(n))
        });
        // The preview reads the camera only while it streams and is listed.
        let want_preview = streaming && listed == Some(true);
        let preview_fps = {
            let Ok(mut g) = CURRENT.lock() else { break };
            let Some(st) = g.as_mut() else { break };
            st.streaming = streaming;
            st.pending = pending;
            if want_preview && st.preview.is_none() {
                st.preview = Some(Preview::start(Box::new(move || post_repaint(hwnd))));
            } else if !want_preview {
                if let Some(mut p) = st.preview.take() {
                    p.stop();
                }
            }
            st.preview.as_ref().and_then(|p| p.snapshot().fps)
        };
        let installed = crate::camera_register::is_registered();
        let update_ready = installed && crate::camera_register::needs_install();
        let start_error = crate::camera::last_error();
        let repair = !streaming
            && offer_repair(
                installed,
                update_ready,
                &start_error,
                crate::camera::playback_error(),
            );
        let inputs = Inputs {
            phone: phone(),
            installed,
            update_ready,
            access: crate::camera_register::camera_access(),
            streaming,
            fps: crate::camera::fps(),
            dark: crate::camera::picture_dark(),
            listed,
            preview_fps,
            pending,
            start_error,
            lens: streaming.then(|| crate::camera::lens() as u8),
        };
        if let Ok(mut g) = CURRENT.lock() {
            if let Some(st) = g.as_mut() {
                st.rows = rows(&inputs);
                st.repair = repair;
            }
        }
        post_repaint(hwnd);
        for _ in 0..15 {
            if stop.load(Ordering::Acquire) {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
    // SAFETY: undoing the start-up above.
    unsafe {
        let _ = windows::Win32::Media::MediaFoundation::MFShutdown();
        CoUninitialize();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every part of the window draws on a Direct2D canvas: rows, a
    /// picture (alpha byte 0, as the camera gives it), hover and the hint.
    #[cfg(windows)]
    #[test]
    fn the_window_draws_with_and_without_a_picture() {
        let hwnd = crate::d2d::test_window();
        let mut surface = Surface::new(hwnd, Backdrop::Opaque).expect("surface");
        let rows = rows(&Inputs {
            phone: Some("RMX3561".into()),
            installed: true,
            update_ready: false,
            access: crate::camera_status::Access::Allow,
            streaming: true,
            fps: 30,
            dark: false,
            listed: Some(true),
            preview_fps: Some(30),
            pending: false,
            start_error: None,
            lens: Some(0),
        });
        let picture = Snapshot {
            width: 4,
            height: 2,
            bgra: [0x40, 0x80, 0xC0, 0x00].repeat(8),
            fps: Some(30),
        };
        for (snapshot, hover) in [
            (Some(picture), Some(Button::Lens)),
            (None, None),
            (Some(Snapshot::default()), Some(Button::Toggle)),
        ] {
            let view = View {
                rows: rows.clone(),
                streaming: true,
                pending: false,
                repair: false,
                hover,
                snapshot,
            };
            surface
                .draw(|dc| {
                    let mut c = Canvas {
                        dc,
                        s: 1.5,
                        p: fluent::palette(),
                    };
                    draw(&mut c, &view);
                })
                .expect("draw");
        }
        drop(surface);
        // SAFETY: our own test window, made above on this thread.
        unsafe {
            let _ = DestroyWindow(hwnd);
        }
    }
}
