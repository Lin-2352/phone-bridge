//! Small Fluent (Windows 11) drawing kit shared by the flyout, the menu and
//! the settings window: theme, palette, fonts, Segoe Fluent Icons glyphs,
//! toggles and buttons, double-buffered painting, DWM window frame.
#![allow(unsafe_code)]

use std::sync::{
    atomic::{AtomicU8, Ordering},
    OnceLock,
};

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
                RoundRect, SelectObject, SetBkMode, SetTextColor, DRAW_TEXT_FORMAT, DT_CALCRECT,
                DT_CENTER, DT_SINGLELINE, DT_VCENTER, HDC, HGDIOBJ, PAINTSTRUCT, PS_DASH, PS_NULL,
                PS_SOLID, SRCCOPY, TRANSPARENT,
            },
            GdiPlus::{
                DashStyleDash, FillModeAlternate, GdipAddPathArc, GdipAddPathRectangle,
                GdipClosePathFigure, GdipCreateFromHDC, GdipCreatePath, GdipCreatePen1,
                GdipCreateSolidFill, GdipDeleteBrush, GdipDeleteGraphics, GdipDeletePath,
                GdipDeletePen, GdipDrawPath, GdipFillPath, GdipSetPenDashArray,
                GdipSetPenDashStyle, GdipSetPixelOffsetMode, GdipSetSmoothingMode, GdiplusStartup,
                GdiplusStartupInput, GpGraphics, GpPath, PixelOffsetModeHalf,
                SmoothingModeAntiAlias, UnitPixel,
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
    pub const SETTINGS: char = '\u{E713}';
    pub const POWER: char = '\u{E7E8}';
    pub const PHONE: char = '\u{E8EA}';
    pub const CLIPBOARD: char = '\u{E77F}';
    pub const ADD: char = '\u{E710}';
    pub const CLOSE: char = '\u{E711}';
    pub const UPLOAD: char = '\u{E898}';
    pub const KEYBOARD: char = '\u{E765}';
    pub const SEARCH: char = '\u{E721}';
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

/// Gap between groups and the window margin, in logical px.
pub const GROUP_GAP: i32 = 16;
/// Padding inside a row, in logical px.
pub const ROW_PAD: i32 = 12;

pub fn scale(hwnd: HWND) -> f32 {
    // SAFETY: plain query on our window.
    (unsafe { GetDpiForWindow(hwnd) }.max(96)) as f32 / 96.0
}

pub fn px(s: f32, v: i32) -> i32 {
    (v as f32 * s).round() as i32
}

/// Corner radius (logical px at scale `s`), never 0 so corners stay round.
pub fn corner_radius(s: f32, logical: i32) -> i32 {
    px(s, logical).max(1)
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
    // DrawTextW on an empty buffer faults (with an ellipsis flag it writes to it).
    if s.is_empty() {
        return;
    }
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

/// Width in pixels `s` takes in `font` on one line.
pub fn text_width(hdc: HDC, font: HGDIOBJ, s: &str) -> i32 {
    if s.is_empty() {
        return 0;
    }
    let mut w: Vec<u16> = s.encode_utf16().collect();
    let mut r = RECT::default();
    // SAFETY: valid DC; DT_CALCRECT only measures; the font is restored.
    unsafe {
        let old = SelectObject(hdc, font);
        DrawTextW(hdc, &mut w, &mut r, DT_CALCRECT | DT_SINGLELINE);
        SelectObject(hdc, old);
    }
    r.right - r.left
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

/// COLORREF (0x00BBGGRR) to opaque GDI+ ARGB (0xFFRRGGBB).
fn argb(c: u32) -> u32 {
    0xff00_0000 | (c & 0xff) << 16 | (c & 0xff00) | (c >> 16) & 0xff
}

/// GDI+ is started once and never shut down while the process runs;
/// `false` if it could not start (shapes then fall back to plain GDI).
fn gdiplus_ready() -> bool {
    static READY: OnceLock<bool> = OnceLock::new();
    *READY.get_or_init(|| {
        let input = GdiplusStartupInput {
            GdiplusVersion: 1,
            ..Default::default()
        };
        let mut token = 0usize;
        // SAFETY: valid input; the output may be null because GDI+ keeps
        // its own background thread (SuppressBackgroundThread is false).
        unsafe { GdiplusStartup(&mut token, &input, std::ptr::null_mut()) }.0 == 0
    })
}

/// Adds a rounded rectangle (`d` = corner ellipse diameter) to `path`.
///
/// SAFETY: `path` must be a live GDI+ path.
unsafe fn add_round_path(path: *mut GpPath, x: f32, y: f32, w: f32, h: f32, d: f32) {
    let d = d.min(w).min(h);
    // SAFETY: per this function's contract.
    unsafe {
        if d <= 0.0 {
            GdipAddPathRectangle(path, x, y, w, h);
            return;
        }
        GdipAddPathArc(path, x, y, d, d, 180.0, 90.0);
        GdipAddPathArc(path, x + w - d, y, d, d, 270.0, 90.0);
        GdipAddPathArc(path, x + w - d, y + h - d, d, d, 0.0, 90.0);
        GdipAddPathArc(path, x, y + h - d, d, d, 90.0, 90.0);
        GdipClosePathFigure(path);
    }
}

/// Anti-aliased `round_rect`; `false` (nothing drawn) if GDI+ is unusable.
fn smooth_round_rect(
    hdc: HDC,
    r: &RECT,
    fill: u32,
    stroke: Option<u32>,
    radius: i32,
    dashed: bool,
) -> bool {
    if !gdiplus_ready() {
        return false;
    }
    let mut g: *mut GpGraphics = std::ptr::null_mut();
    // SAFETY: every GDI+ object made here is deleted before returning; the
    // graphics is deleted last so its drawing is flushed to `hdc`.
    unsafe {
        if GdipCreateFromHDC(hdc, &mut g).0 != 0 || g.is_null() {
            return false;
        }
        GdipSetSmoothingMode(g, SmoothingModeAntiAlias);
        // Integer coordinates are pixel edges, so a fill covers exactly the
        // pixels GDI would and a 1 px stroke on a .5 line is crisp.
        GdipSetPixelOffsetMode(g, PixelOffsetModeHalf);
        // The outline sits on the outermost pixel row, like GDI's pen.
        let inset = if stroke.is_some() { 0.5 } else { 0.0 };
        let x = r.left as f32 + inset;
        let y = r.top as f32 + inset;
        let w = (r.right - r.left) as f32 - 2.0 * inset;
        let h = (r.bottom - r.top) as f32 - 2.0 * inset;
        let mut path: *mut GpPath = std::ptr::null_mut();
        if w > 0.0 && h > 0.0 && GdipCreatePath(FillModeAlternate, &mut path).0 == 0 {
            add_round_path(path, x, y, w, h, radius as f32);
            let mut brush = std::ptr::null_mut();
            if GdipCreateSolidFill(argb(fill), &mut brush).0 == 0 {
                GdipFillPath(g, brush as _, path);
                GdipDeleteBrush(brush as _);
            }
            if let Some(c) = stroke {
                let mut pen = std::ptr::null_mut();
                if GdipCreatePen1(argb(c), 1.0, UnitPixel, &mut pen).0 == 0 {
                    if dashed {
                        GdipSetPenDashStyle(pen, DashStyleDash);
                        // GDI's PS_DASH pattern: 18 px on, 6 px off.
                        let dash = [18.0f32, 6.0];
                        GdipSetPenDashArray(pen, dash.as_ptr(), dash.len() as i32);
                    }
                    GdipDrawPath(g, pen, path);
                    GdipDeletePen(pen);
                }
            }
            GdipDeletePath(path);
        }
        GdipDeleteGraphics(g);
    }
    true
}

/// Rounded rectangle; `stroke: None` draws fill only. `radius` is the
/// corner ellipse size (as GDI's `RoundRect`); drawn anti-aliased with GDI+.
pub fn round_rect(hdc: HDC, r: &RECT, fill: u32, stroke: Option<u32>, radius: i32, dashed: bool) {
    if !smooth_round_rect(hdc, r, fill, stroke, radius, dashed) {
        gdi_round_rect(hdc, r, fill, stroke, radius, dashed);
    }
}

/// Plain GDI fallback for `round_rect` (jagged edges).
fn gdi_round_rect(hdc: HDC, r: &RECT, fill: u32, stroke: Option<u32>, radius: i32, dashed: bool) {
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
        round_rect(hdc, &r, p.accent, None, corner_radius(s, 8), false);
        text(
            hdc,
            f.body,
            label,
            r,
            p.on_accent,
            DT_CENTER | DT_VCENTER | DT_SINGLELINE,
        );
    } else {
        round_rect(hdc, &r, p.card, Some(p.stroke), corner_radius(s, 8), false);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spacing_constants_are_on_the_4px_grid() {
        assert_eq!(GROUP_GAP % 4, 0);
        assert_eq!(ROW_PAD % 4, 0);
    }

    #[test]
    fn radius_scales_with_dpi() {
        assert_eq!(corner_radius(96.0 / 96.0, 8), 8);
        assert_eq!(corner_radius(192.0 / 96.0, 8), 16);
        assert_eq!(corner_radius(1.25, 6), 8);
    }

    #[test]
    fn radius_is_never_zero() {
        assert_eq!(corner_radius(1.0, 0), 1);
        assert_eq!(corner_radius(0.1, 1), 1);
        assert_eq!(corner_radius(1.0, -4), 1);
    }

    /// COLORREF is 0x00BBGGRR; GDI+ wants opaque 0xAARRGGBB.
    #[test]
    fn colorref_becomes_opaque_argb() {
        assert_eq!(argb(0xb86700), 0xff0067b8);
        assert_eq!(argb(0x000000), 0xff000000);
        assert_eq!(argb(0xffffff), 0xffffffff);
        assert_eq!(argb(0x1c2bc4), 0xffc42b1c);
    }

    /// The GDI+ path draws the same colours GDI would, edge to edge.
    #[test]
    fn round_rect_draws_antialiased_with_the_palette_colour() {
        use windows::Win32::Graphics::Gdi::{GetDC, GetPixel, ReleaseDC};
        assert!(gdiplus_ready());
        // SAFETY: a memory DC and bitmap made and freed here.
        unsafe {
            let screen = GetDC(None);
            let hdc = CreateCompatibleDC(Some(screen));
            let bmp = CreateCompatibleBitmap(screen, 40, 40);
            let old = SelectObject(hdc, HGDIOBJ(bmp.0));
            let all = RECT {
                left: 0,
                top: 0,
                right: 40,
                bottom: 40,
            };
            round_rect(hdc, &all, 0x000000, None, 0, false);
            let r = RECT {
                left: 4,
                top: 4,
                right: 36,
                bottom: 36,
            };
            assert!(smooth_round_rect(
                hdc,
                &r,
                0xb86700,
                Some(0x1c2bc4),
                16,
                false
            ));
            assert_eq!(GetPixel(hdc, 20, 20).0, 0xb86700, "fill");
            assert_eq!(GetPixel(hdc, 20, 4).0, 0x1c2bc4, "top edge");
            assert_eq!(GetPixel(hdc, 20, 3).0, 0x000000, "outside");
            assert_eq!(GetPixel(hdc, 4, 4).0, 0x000000, "rounded corner");
            SelectObject(hdc, old);
            let _ = DeleteObject(HGDIOBJ(bmp.0));
            let _ = DeleteDC(hdc);
            ReleaseDC(None, screen);
        }
    }

    /// An empty string once crashed the whole tray inside `DrawTextW`.
    #[test]
    fn drawing_empty_text_is_harmless() {
        use windows::Win32::Graphics::Gdi::{
            CreateCompatibleDC, DeleteDC, DT_END_ELLIPSIS, DT_LEFT,
        };
        // SAFETY: a memory DC made and freed here.
        unsafe {
            let hdc = CreateCompatibleDC(None);
            let f = Fonts::new(1.0);
            let r = RECT {
                left: 0,
                top: 0,
                right: 100,
                bottom: 20,
            };
            text(hdc, f.small, "", r, 0, DT_LEFT | DT_END_ELLIPSIS);
            text(hdc, f.small, "", r, 0, DT_LEFT);
            let _ = DeleteDC(hdc);
        }
    }
}
