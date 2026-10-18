//! Fluent-style flyout opened by left-clicking the tray icon (design v2):
//! link status, a drop zone for files, and three tiles (Universal Clipboard,
//! Send files, Pair device). Closes when it loses focus. Per-monitor DPI aware,
//! follows the system light/dark theme, rounded corners via DWM.
#![allow(unsafe_code)]

use std::sync::Mutex;

use windows::{
    core::{w, PCWSTR},
    Win32::{
        Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
        Graphics::{
            Dwm::{
                DwmSetWindowAttribute, DWMWA_USE_IMMERSIVE_DARK_MODE,
                DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND,
            },
            Gdi::{
                BeginPaint, BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, CreateFontW,
                CreatePen, CreateSolidBrush, DeleteDC, DeleteObject, DrawTextW, EndPaint, FillRect,
                InvalidateRect, RoundRect, SelectObject, SetBkMode, SetTextColor, DT_CENTER,
                DT_LEFT, DT_SINGLELINE, DT_VCENTER, DT_WORDBREAK, HDC, HGDIOBJ, PAINTSTRUCT,
                PS_DASH, PS_SOLID, SRCCOPY, TRANSPARENT,
            },
        },
        System::{
            LibraryLoader::GetModuleHandleW,
            Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD},
        },
        UI::{
            HiDpi::GetDpiForWindow,
            Shell::{DragAcceptFiles, DragFinish, DragQueryFileW, HDROP},
            WindowsAndMessaging::{
                CreateWindowExW, DefWindowProcW, DestroyWindow, GetClientRect, GetCursorPos,
                RegisterClassW, SetForegroundWindow, SetWindowPos, ShowWindow, HWND_TOPMOST,
                SWP_NOACTIVATE, SW_SHOW, WA_INACTIVE, WM_ACTIVATE, WM_DESTROY, WM_DROPFILES,
                WM_ERASEBKGND, WM_LBUTTONUP, WM_PAINT, WNDCLASSW, WS_EX_TOOLWINDOW, WS_EX_TOPMOST,
                WS_POPUP,
            },
        },
    },
};

const CLASS: PCWSTR = w!("PhoneBridgeFlyout");
/// Logical (96-dpi) size of the flyout.
const W: i32 = 340;
const H: i32 = 300;

/// What the flyout shows; filled in by the UI before opening.
#[derive(Clone, Default)]
pub struct Model {
    pub peer: Option<String>,
    pub clipboard_on: bool,
    pub last: String,
}

/// User actions the flyout reports back to the UI thread.
pub enum Action {
    ToggleClipboard,
    SendFiles,
    Pair,
    Dropped(Vec<std::path::PathBuf>),
}

struct State {
    hwnd: isize,
    model: Model,
    on_action: Box<dyn Fn(Action) + Send>,
}

static CURRENT: Mutex<Option<State>> = Mutex::new(None);

fn dark_mode() -> bool {
    let mut v: u32 = 1;
    let mut size = 4u32;
    // SAFETY: reads one DWORD into `v`; failure leaves the light default.
    let r = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize"),
            w!("AppsUseLightTheme"),
            RRF_RT_REG_DWORD,
            None,
            Some(&mut v as *mut u32 as *mut _),
            Some(&mut size),
        )
    };
    r.is_ok() && v == 0
}

struct Palette {
    bg: u32,
    tile: u32,
    stroke: u32,
    text: u32,
    text2: u32,
    accent: u32,
    on_accent: u32,
}

fn palette() -> Palette {
    // COLORREF is 0x00BBGGRR.
    if dark_mode() {
        Palette {
            bg: 0x2c2c2c,
            tile: 0x3a3a3a,
            stroke: 0x4a4a4a,
            text: 0xffffff,
            text2: 0xc5c5c5,
            accent: 0xffa793,
            on_accent: 0x40140a,
        }
    } else {
        Palette {
            bg: 0xf5f3f3,
            tile: 0xffffff,
            stroke: 0xdddddd,
            text: 0x1b1b1b,
            text2: 0x5d5d5d,
            accent: 0xc94b2a,
            on_accent: 0xffffff,
        }
    }
}

/// Layout rectangles in physical pixels for a given DPI scale.
struct Layout {
    drop: RECT,
    tiles: [RECT; 3],
}

fn layout(s: f32) -> Layout {
    let px = |v: i32| (v as f32 * s).round() as i32;
    let pad = px(16);
    let drop = RECT {
        left: pad,
        top: px(64),
        right: px(W) - pad,
        bottom: px(150),
    };
    let tw = (px(W) - 2 * pad - 2 * px(8)) / 3;
    let ty = px(164);
    let tiles = std::array::from_fn(|i| {
        let l = pad + i as i32 * (tw + px(8));
        RECT {
            left: l,
            top: ty,
            right: l + tw,
            bottom: ty + px(74),
        }
    });
    Layout { drop, tiles }
}

fn scale(hwnd: HWND) -> f32 {
    // SAFETY: plain query on our window.
    (unsafe { GetDpiForWindow(hwnd) }.max(96)) as f32 / 96.0
}

unsafe fn font(s: f32, size: i32, weight: i32) -> HGDIOBJ {
    let f = CreateFontW(
        -((size as f32 * s).round() as i32),
        0,
        0,
        0,
        weight,
        0,
        0,
        0,
        Default::default(),
        Default::default(),
        Default::default(),
        Default::default(),
        0,
        w!("Segoe UI Variable Text"),
    );
    HGDIOBJ(f.0)
}

unsafe fn text(
    hdc: HDC,
    s: &str,
    r: RECT,
    color: u32,
    flags: windows::Win32::Graphics::Gdi::DRAW_TEXT_FORMAT,
) {
    let mut w: Vec<u16> = s.encode_utf16().collect();
    let mut r = r;
    SetTextColor(hdc, COLORREF(color));
    DrawTextW(hdc, &mut w, &mut r, flags);
}

unsafe fn round_rect(hdc: HDC, r: &RECT, fill: u32, stroke: u32, radius: i32, dashed: bool) {
    let brush = CreateSolidBrush(COLORREF(fill));
    let pen = CreatePen(if dashed { PS_DASH } else { PS_SOLID }, 1, COLORREF(stroke));
    let ob = SelectObject(hdc, HGDIOBJ(brush.0));
    let op = SelectObject(hdc, HGDIOBJ(pen.0));
    let _ = RoundRect(hdc, r.left, r.top, r.right, r.bottom, radius, radius);
    SelectObject(hdc, ob);
    SelectObject(hdc, op);
    let _ = DeleteObject(HGDIOBJ(brush.0));
    let _ = DeleteObject(HGDIOBJ(pen.0));
}

fn paint(hwnd: HWND) {
    let Ok(guard) = CURRENT.lock() else { return };
    let Some(st) = guard.as_ref() else { return };
    let s = scale(hwnd);
    let p = palette();
    let l = layout(s);
    let px = |v: i32| (v as f32 * s).round() as i32;
    let mut ps = PAINTSTRUCT::default();
    // SAFETY: standard double-buffered WM_PAINT; every GDI object created
    // here is selected out and deleted before returning.
    unsafe {
        let screen = BeginPaint(hwnd, &mut ps);
        let mut c = RECT::default();
        let _ = GetClientRect(hwnd, &mut c);
        let hdc = CreateCompatibleDC(Some(screen));
        let bmp = CreateCompatibleBitmap(screen, c.right, c.bottom);
        let old_bmp = SelectObject(hdc, HGDIOBJ(bmp.0));
        SetBkMode(hdc, TRANSPARENT);
        let bg = CreateSolidBrush(COLORREF(p.bg));
        FillRect(hdc, &c, bg);
        let _ = DeleteObject(HGDIOBJ(bg.0));

        let f_title = font(s, 15, 600);
        let f_body = font(s, 12, 400);
        let old_font = SelectObject(hdc, f_title);
        let (title, sub) = match &st.model.peer {
            Some(n) => (n.clone(), "Linked".to_owned()),
            None => (
                "No device paired".to_owned(),
                "Choose Pair device to start".to_owned(),
            ),
        };
        text(
            hdc,
            &title,
            RECT {
                left: px(16),
                top: px(14),
                right: c.right - px(16),
                bottom: px(36),
            },
            p.text,
            DT_LEFT | DT_SINGLELINE | DT_VCENTER,
        );
        SelectObject(hdc, f_body);
        text(
            hdc,
            &sub,
            RECT {
                left: px(16),
                top: px(36),
                right: c.right - px(16),
                bottom: px(54),
            },
            p.text2,
            DT_LEFT | DT_SINGLELINE | DT_VCENTER,
        );

        round_rect(hdc, &l.drop, p.tile, p.stroke, px(10), true);
        let drop_label = match &st.model.peer {
            Some(n) => format!("Drop files here to send to {n}"),
            None => "Pair a device to send files".to_owned(),
        };
        text(
            hdc,
            &drop_label,
            l.drop,
            p.text2,
            DT_CENTER | DT_SINGLELINE | DT_VCENTER,
        );

        let labels = ["Universal Clipboard", "Send files", "Pair device"];
        for (i, r) in l.tiles.iter().enumerate() {
            let on = i == 0 && st.model.clipboard_on;
            round_rect(
                hdc,
                r,
                if on { p.accent } else { p.tile },
                if on { p.accent } else { p.stroke },
                px(8),
                false,
            );
            let mut tr = *r;
            tr.left += px(10);
            tr.right -= px(6);
            tr.top += px(34);
            text(
                hdc,
                labels[i],
                tr,
                if on { p.on_accent } else { p.text },
                DT_LEFT | DT_WORDBREAK,
            );
            // Small state dot for the toggle tile.
            if i == 0 {
                let dot = RECT {
                    left: r.left + px(10),
                    top: r.top + px(12),
                    right: r.left + px(20),
                    bottom: r.top + px(22),
                };
                round_rect(
                    hdc,
                    &dot,
                    if on { p.on_accent } else { p.text2 },
                    if on { p.on_accent } else { p.text2 },
                    px(10),
                    false,
                );
            }
        }
        text(
            hdc,
            &st.model.last,
            RECT {
                left: px(16),
                top: px(252),
                right: c.right - px(16),
                bottom: px(284),
            },
            p.text2,
            DT_LEFT | DT_SINGLELINE | DT_VCENTER,
        );

        let _ = BitBlt(screen, 0, 0, c.right, c.bottom, Some(hdc), 0, 0, SRCCOPY);
        SelectObject(hdc, old_font);
        let _ = DeleteObject(f_title);
        let _ = DeleteObject(f_body);
        SelectObject(hdc, old_bmp);
        let _ = DeleteObject(HGDIOBJ(bmp.0));
        let _ = DeleteDC(hdc);
        let _ = EndPaint(hwnd, &ps);
    }
}

fn hit(hwnd: HWND, x: i32, y: i32) -> Option<Action> {
    let l = layout(scale(hwnd));
    let inside = |r: &RECT| x >= r.left && x < r.right && y >= r.top && y < r.bottom;
    if inside(&l.tiles[0]) {
        Some(Action::ToggleClipboard)
    } else if inside(&l.tiles[1]) || inside(&l.drop) {
        Some(Action::SendFiles)
    } else if inside(&l.tiles[2]) {
        Some(Action::Pair)
    } else {
        None
    }
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
        WM_LBUTTONUP => {
            let x = (lparam.0 & 0xffff) as i16 as i32;
            let y = ((lparam.0 >> 16) & 0xffff) as i16 as i32;
            if let Some(a) = hit(hwnd, x, y) {
                let keep_open = matches!(a, Action::ToggleClipboard);
                act(a);
                if keep_open {
                    if let Ok(mut g) = CURRENT.lock() {
                        if let Some(st) = g.as_mut() {
                            st.model.clipboard_on = !st.model.clipboard_on;
                        }
                    }
                    // SAFETY: repaint our own window.
                    let _ = unsafe { InvalidateRect(Some(hwnd), None, false) };
                } else {
                    close();
                }
            }
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
        WM_ACTIVATE if (wparam.0 & 0xffff) as u32 == WA_INACTIVE => {
            close();
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
            ..Default::default()
        };
        RegisterClassW(&wc);
        let Ok(hwnd) = CreateWindowExW(
            WS_EX_TOOLWINDOW | WS_EX_TOPMOST,
            CLASS,
            w!("Phone Bridge"),
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
                on_action,
            });
        }
        let s = scale(hwnd);
        let (w, h) = ((W as f32 * s) as i32, (H as f32 * s) as i32);
        let mut pt = POINT::default();
        let _ = GetCursorPos(&mut pt);
        // Above-left of the tray icon, like Windows' own flyouts.
        let _ = SetWindowPos(
            hwnd,
            Some(HWND_TOPMOST),
            (pt.x - w + (24.0 * s) as i32).max(0),
            (pt.y - h - (16.0 * s) as i32).max(0),
            w,
            h,
            SWP_NOACTIVATE,
        );
        let pref = DWMWCP_ROUND;
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            &pref as *const _ as *const _,
            4,
        );
        let dark: i32 = dark_mode() as i32;
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_USE_IMMERSIVE_DARK_MODE,
            &dark as *const _ as *const _,
            4,
        );
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
