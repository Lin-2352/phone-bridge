//! Small Fluent (Windows 11) drawing kit shared by the flyout, the menu and
//! the settings window: theme, palette, fonts, Segoe Fluent Icons glyphs,
//! toggles and buttons, double-buffered painting, DWM window frame.
#![allow(unsafe_code)]

use std::sync::atomic::{AtomicU8, Ordering};

use windows::{
    core::{w, PCWSTR},
    Win32::{
        Foundation::{COLORREF, HWND, RECT},
        Graphics::{
            Dwm::{
                DwmSetWindowAttribute, DWMWA_SYSTEMBACKDROP_TYPE, DWMWA_USE_IMMERSIVE_DARK_MODE,
                DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND,
            },
            Gdi::{
                BeginPaint, BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, CreateFontW,
                CreatePen, CreateSolidBrush, DeleteDC, DeleteObject, DrawTextW, EndPaint, FillRect,
                RoundRect, SelectObject, SetBkMode, SetTextColor, DRAW_TEXT_FORMAT, DT_CENTER,
                DT_SINGLELINE, DT_VCENTER, HDC, HGDIOBJ, PAINTSTRUCT, PS_DASH, PS_NULL, PS_SOLID,
                SRCCOPY, TRANSPARENT,
            },
        },
        System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD},
        UI::{
            HiDpi::GetDpiForWindow,
            WindowsAndMessaging::{GetClientRect, LoadCursorW, SetCursor, IDC_ARROW, IDC_HAND},
        },
    },
};

/// Segoe Fluent Icons code points used across the tray UI.
pub mod glyph {
    pub const SEND: char = '\u{E724}';
    pub const FOLDER: char = '\u{E838}';
    pub const SETTINGS: char = '\u{E713}';
    pub const POWER: char = '\u{E7E8}';
    pub const PHONE: char = '\u{E8EA}';
    pub const CLIPBOARD: char = '\u{E77F}';
    pub const ADD: char = '\u{E710}';
    pub const CLOSE: char = '\u{E711}';
    pub const UPLOAD: char = '\u{E898}';
    pub const KEYBOARD: char = '\u{E765}';
}

/// 0 = follow Windows, 1 = light, 2 = dark (from settings).
static THEME: AtomicU8 = AtomicU8::new(0);

pub fn set_theme(mode: u8) {
    THEME.store(mode.min(2), Ordering::Relaxed);
}

fn windows_dark() -> bool {
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

pub fn dark() -> bool {
    match THEME.load(Ordering::Relaxed) {
        1 => false,
        2 => true,
        _ => windows_dark(),
    }
}

/// COLORREF values (0x00BBGGRR).
pub struct Palette {
    pub bg: u32,
    pub card: u32,
    pub hover: u32,
    pub stroke: u32,
    pub text: u32,
    pub text2: u32,
    pub accent: u32,
    pub on_accent: u32,
    pub danger: u32,
}

pub fn palette() -> Palette {
    if dark() {
        Palette {
            bg: 0x202020,
            card: 0x2d2d2d,
            hover: 0x383838,
            stroke: 0x3f3f3f,
            text: 0xffffff,
            text2: 0xc5c5c5,
            accent: 0xffc34c, // #4CC2FF, Windows 11 dark accent
            on_accent: 0x000000,
            danger: 0x8b99ff,
        }
    } else {
        Palette {
            bg: 0xf3f3f3,
            card: 0xfbfbfb,
            hover: 0xf0f0f0,
            stroke: 0xe5e5e5,
            text: 0x1a1a1a,
            text2: 0x5f5f5f,
            accent: 0xb86700, // #0067B8
            on_accent: 0xffffff,
            danger: 0x1c2bc4,
        }
    }
}

pub fn scale(hwnd: HWND) -> f32 {
    // SAFETY: plain query on our window.
    (unsafe { GetDpiForWindow(hwnd) }.max(96)) as f32 / 96.0
}

pub fn px(s: f32, v: i32) -> i32 {
    (v as f32 * s).round() as i32
}

pub fn inside(r: &RECT, x: i32, y: i32) -> bool {
    x >= r.left && x < r.right && y >= r.top && y < r.bottom
}

/// Fonts created for one paint pass; deleted on drop.
pub struct Fonts {
    pub title: HGDIOBJ,
    pub body: HGDIOBJ,
    pub small: HGDIOBJ,
    pub icon: HGDIOBJ,
    pub icon_big: HGDIOBJ,
}

fn make_font(s: f32, size: i32, weight: i32, face: PCWSTR) -> HGDIOBJ {
    // SAFETY: plain font creation; the handle is deleted by `Fonts::drop`.
    let f = unsafe {
        CreateFontW(
            -px(s, size),
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
            // CLEARTYPE_QUALITY
            windows::Win32::Graphics::Gdi::FONT_QUALITY(5),
            0,
            face,
        )
    };
    HGDIOBJ(f.0)
}

impl Fonts {
    pub fn new(s: f32) -> Self {
        let ui = w!("Segoe UI Variable Text");
        let icons = w!("Segoe Fluent Icons");
        Self {
            title: make_font(s, 15, 600, ui),
            body: make_font(s, 13, 400, ui),
            small: make_font(s, 12, 400, ui),
            icon: make_font(s, 16, 400, icons),
            icon_big: make_font(s, 20, 400, icons),
        }
    }
}

impl Drop for Fonts {
    fn drop(&mut self) {
        for f in [self.title, self.body, self.small, self.icon, self.icon_big] {
            // SAFETY: fonts we created; not selected into a DC any more.
            let _ = unsafe { DeleteObject(f) };
        }
    }
}

/// Draw `s` with `font` into `r`.
pub fn text(hdc: HDC, font: HGDIOBJ, s: &str, r: RECT, color: u32, flags: DRAW_TEXT_FORMAT) {
    let mut w: Vec<u16> = s.encode_utf16().collect();
    let mut r = r;
    // SAFETY: valid DC; the previous font is restored.
    unsafe {
        let old = SelectObject(hdc, font);
        SetTextColor(hdc, COLORREF(color));
        DrawTextW(hdc, &mut w, &mut r, flags);
        SelectObject(hdc, old);
    }
}

pub fn icon(hdc: HDC, font: HGDIOBJ, g: char, r: RECT, color: u32) {
    text(
        hdc,
        font,
        &g.to_string(),
        r,
        color,
        DT_CENTER | DT_VCENTER | DT_SINGLELINE,
    );
}

/// Rounded rectangle; `stroke: None` draws fill only.
pub fn round_rect(hdc: HDC, r: &RECT, fill: u32, stroke: Option<u32>, radius: i32, dashed: bool) {
    // SAFETY: GDI objects are selected out and deleted before returning.
    unsafe {
        let brush = CreateSolidBrush(COLORREF(fill));
        let pen = match stroke {
            Some(c) => CreatePen(if dashed { PS_DASH } else { PS_SOLID }, 1, COLORREF(c)),
            None => CreatePen(PS_NULL, 0, COLORREF(0)),
        };
        let ob = SelectObject(hdc, HGDIOBJ(brush.0));
        let op = SelectObject(hdc, HGDIOBJ(pen.0));
        let _ = RoundRect(hdc, r.left, r.top, r.right, r.bottom, radius, radius);
        SelectObject(hdc, ob);
        SelectObject(hdc, op);
        let _ = DeleteObject(HGDIOBJ(brush.0));
        let _ = DeleteObject(HGDIOBJ(pen.0));
    }
}

/// Windows 11 toggle switch (40x20 logical) whose right edge is at `right`,
/// vertically centred on `cy`. Returns its rectangle for hit-testing.
pub fn toggle(hdc: HDC, s: f32, p: &Palette, right: i32, cy: i32, on: bool, enabled: bool) -> RECT {
    let (w, h) = (px(s, 40), px(s, 20));
    let r = RECT {
        left: right - w,
        top: cy - h / 2,
        right,
        bottom: cy + h / 2,
    };
    let knob = px(s, 12);
    let inset = (h - knob) / 2;
    let (track, edge, dot) = match (on, enabled) {
        (true, true) => (p.accent, None, p.on_accent),
        (false, true) => (p.card, Some(p.text2), p.text2),
        (_, false) => (p.card, Some(p.stroke), p.stroke),
    };
    round_rect(hdc, &r, track, edge.or(Some(track)), h, false);
    let kx = if on {
        r.right - inset - knob
    } else {
        r.left + inset
    };
    let k = RECT {
        left: kx,
        top: r.top + inset,
        right: kx + knob,
        bottom: r.top + inset + knob,
    };
    round_rect(hdc, &k, dot, None, knob, false);
    r
}

/// Standard button (accent or neutral) with a centred label.
pub fn button(hdc: HDC, f: &Fonts, p: &Palette, r: RECT, label: &str, accent: bool, s: f32) {
    if accent {
        round_rect(hdc, &r, p.accent, None, px(s, 8), false);
        text(
            hdc,
            f.body,
            label,
            r,
            p.on_accent,
            DT_CENTER | DT_VCENTER | DT_SINGLELINE,
        );
    } else {
        round_rect(hdc, &r, p.card, Some(p.stroke), px(s, 8), false);
        text(
            hdc,
            f.body,
            label,
            r,
            p.text,
            DT_CENTER | DT_VCENTER | DT_SINGLELINE,
        );
    }
}

/// Double-buffered WM_PAINT: `draw` gets a memory DC and the client size.
pub fn paint(hwnd: HWND, draw: impl FnOnce(HDC, RECT)) {
    let mut ps = PAINTSTRUCT::default();
    // SAFETY: standard double-buffered paint; all objects are released.
    unsafe {
        let screen = BeginPaint(hwnd, &mut ps);
        let mut c = RECT::default();
        let _ = GetClientRect(hwnd, &mut c);
        let hdc = CreateCompatibleDC(Some(screen));
        let bmp = CreateCompatibleBitmap(screen, c.right, c.bottom);
        let old = SelectObject(hdc, HGDIOBJ(bmp.0));
        SetBkMode(hdc, TRANSPARENT);
        let bg = CreateSolidBrush(COLORREF(palette().bg));
        FillRect(hdc, &c, bg);
        let _ = DeleteObject(HGDIOBJ(bg.0));
        draw(hdc, c);
        let _ = BitBlt(screen, 0, 0, c.right, c.bottom, Some(hdc), 0, 0, SRCCOPY);
        SelectObject(hdc, old);
        let _ = DeleteObject(HGDIOBJ(bmp.0));
        let _ = DeleteDC(hdc);
        let _ = EndPaint(hwnd, &ps);
    }
}

/// Rounded corners, dark title/frame to match the theme, optional Mica.
pub fn frame(hwnd: HWND, mica: bool) {
    // SAFETY: attribute writes on our own window; failures are cosmetic.
    unsafe {
        let pref = DWMWCP_ROUND;
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            &pref as *const _ as *const _,
            4,
        );
        let d: i32 = dark() as i32;
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_USE_IMMERSIVE_DARK_MODE,
            &d as *const _ as *const _,
            4,
        );
        if mica {
            let backdrop: i32 = 2; // DWMSBT_MAINWINDOW
            let _ = DwmSetWindowAttribute(
                hwnd,
                DWMWA_SYSTEMBACKDROP_TYPE,
                &backdrop as *const _ as *const _,
                4,
            );
        }
    }
}

/// Arrow normally, hand over something clickable (WM_SETCURSOR helper).
pub fn set_cursor(hand: bool) {
    // SAFETY: system cursors are shared and never freed.
    unsafe {
        if let Ok(c) = LoadCursorW(None, if hand { IDC_HAND } else { IDC_ARROW }) {
            SetCursor(Some(c));
        }
    }
}
