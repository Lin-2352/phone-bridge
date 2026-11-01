//! Small Fluent (Windows 11) drawing kit shared by the tray windows: theme,
//! token palette, fonts, Segoe Fluent Icons glyphs, toggles and buttons, DWM
//! window frame. Windows draw through a Direct2D `Canvas` (`paint_with`);
//! the `gdi_*` functions keep the windows not yet moved to Direct2D working.
#![allow(unsafe_code)]

use std::{
    cell::RefCell,
    collections::HashMap,
    sync::{
        atomic::{AtomicU8, Ordering},
        Mutex, OnceLock,
    },
};

use windows::{
    core::{w, PCWSTR},
    Win32::{
        Foundation::{COLORREF, HWND, RECT},
        Graphics::{
            Direct2D::{
                Common::D2D_RECT_F, ID2D1DeviceContext, ID2D1Factory, ID2D1StrokeStyle,
                D2D1_ANTIALIAS_MODE_PER_PRIMITIVE, D2D1_CAP_STYLE_FLAT, D2D1_DASH_STYLE_CUSTOM,
                D2D1_DRAW_TEXT_OPTIONS_CLIP, D2D1_DRAW_TEXT_OPTIONS_ENABLE_COLOR_FONT,
                D2D1_LINE_JOIN_MITER, D2D1_ROUNDED_RECT, D2D1_STROKE_STYLE_PROPERTIES,
                D2D1_TEXT_ANTIALIAS_MODE_DEFAULT, D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE,
            },
            DirectWrite::{
                IDWriteFontCollection, IDWriteTextFormat, DWRITE_FONT_STRETCH_NORMAL,
                DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_WEIGHT, DWRITE_MEASURING_MODE_NATURAL,
                DWRITE_PARAGRAPH_ALIGNMENT_CENTER, DWRITE_TEXT_ALIGNMENT,
                DWRITE_TEXT_ALIGNMENT_CENTER, DWRITE_TEXT_ALIGNMENT_LEADING,
                DWRITE_TEXT_ALIGNMENT_TRAILING, DWRITE_TEXT_METRICS, DWRITE_TRIMMING,
                DWRITE_TRIMMING_GRANULARITY_CHARACTER, DWRITE_WORD_WRAPPING_NO_WRAP,
            },
            Dwm::{
                DwmSetWindowAttribute, DWMSBT_MAINWINDOW, DWMWA_SYSTEMBACKDROP_TYPE,
                DWMWA_USE_IMMERSIVE_DARK_MODE, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND,
                DWM_SYSTEMBACKDROP_TYPE,
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

use crate::{
    d2d::{self, color, Backdrop, Surface},
    tokens::{self, ty},
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

/// One DWORD from the user's Themes\Personalize key, if it is set.
fn personalize(name: PCWSTR) -> Option<u32> {
    let mut v: u32 = 0;
    let mut size = 4u32;
    // SAFETY: reads one DWORD into `v`; failure leaves it unused.
    let r = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize"),
            name,
            RRF_RT_REG_DWORD,
            None,
            Some(&mut v as *mut u32 as *mut _),
            Some(&mut size),
        )
    };
    r.is_ok().then_some(v)
}

fn windows_dark() -> bool {
    personalize(w!("AppsUseLightTheme")) == Some(0)
}

/// Windows "Transparency effects" (on unless explicitly turned off).
pub fn transparency_on() -> bool {
    personalize(w!("EnableTransparency")) != Some(0)
}

pub fn dark() -> bool {
    match THEME.load(Ordering::Relaxed) {
        1 => false,
        2 => true,
        _ => windows_dark(),
    }
}

/// Theme colours as 0xRRGGBB, from the design tokens.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Palette {
    pub bg: u32,
    pub card: u32,
    /// Hover fill; not a token, a grey step between `card` and `stroke`.
    pub hover: u32,
    pub stroke: u32,
    pub text: u32,
    pub text2: u32,
    pub accent: u32,
    pub on_accent: u32,
    pub danger: u32,
    // Used from the widgets and the main window (Tasks 4 on).
    #[allow(dead_code)]
    pub success: u32,
    #[allow(dead_code)]
    pub warning: u32,
    #[allow(dead_code)]
    pub amber: u32,
}

pub fn palette() -> Palette {
    let (t, hover) = if dark() {
        (&tokens::DARK, 0x383838)
    } else {
        (&tokens::LIGHT, 0xF0F0F0)
    };
    Palette {
        bg: t.surface,
        card: t.surface_variant,
        hover,
        stroke: t.outline,
        text: t.text,
        text2: t.text_muted,
        accent: t.primary,
        on_accent: t.on_primary,
        danger: t.danger,
        success: t.success,
        warning: t.warning,
        amber: t.accent,
    }
}

/// 0xRRGGBB to a GDI COLORREF value (0x00BBGGRR).
pub fn colorref(rgb: u32) -> u32 {
    (rgb & 0xff) << 16 | (rgb & 0xff00) | (rgb >> 16) & 0xff
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

// ---------------------------------------------------------------------------
// Direct2D canvas
// ---------------------------------------------------------------------------

/// A rectangle in pixels (Direct2D draws at 96 DPI, so DIPs are pixels).
pub type RectF = D2D_RECT_F;

pub fn rectf(left: f32, top: f32, right: f32, bottom: f32) -> RectF {
    RectF {
        left,
        top,
        right,
        bottom,
    }
}

/// One paint pass: the window's device context, its DPI scale and the
/// palette of the current theme.
pub struct Canvas<'a> {
    pub dc: &'a ID2D1DeviceContext,
    pub s: f32,
    pub p: Palette,
}

impl Canvas<'_> {
    /// Logical px to pixels at this canvas's scale.
    #[allow(dead_code)] // used by the widgets (pages use them from Task 8 on)
    pub fn px(&self, v: f32) -> f32 {
        v * self.s
    }
}

/// Type ramp of the tray UI, sizes from the tokens.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[allow(dead_code)] // the main window and widgets use the rest (Tasks 4 on)
pub enum Font {
    Caption,
    Body,
    BodyStrong,
    Subtitle,
    Title,
    Display,
    Icon,
    /// 20 px glyphs: empty states and the nav rail.
    IconLarge,
    /// 10 px glyphs: the caption buttons (the size Windows draws them at).
    IconSmall,
}

impl Font {
    /// Face, size in logical px and weight.
    fn spec(self) -> (PCWSTR, f32, i32) {
        let text = w!("Segoe UI Variable Text");
        let display = w!("Segoe UI Variable Display");
        match self {
            Font::Caption => (text, ty::CAPTION, 400),
            Font::Body => (text, ty::BODY, 400),
            Font::BodyStrong => (text, ty::BODY, 600),
            Font::Subtitle => (text, ty::SUBTITLE, 600),
            Font::Title => (display, ty::TITLE, 600),
            Font::Display => (display, ty::DISPLAY, 600),
            Font::Icon => (w!("Segoe Fluent Icons"), 16.0, 400),
            Font::IconLarge => (w!("Segoe Fluent Icons"), 20.0, 400),
            Font::IconSmall => (w!("Segoe Fluent Icons"), 10.0, 400),
        }
    }
}

/// Horizontal placement of a line of text; it is always centred vertically.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[allow(dead_code)] // Right is used by the widgets (pages use them from Task 8 on)
pub enum Align {
    Left,
    Center,
    Right,
}

impl Align {
    fn dwrite(self) -> DWRITE_TEXT_ALIGNMENT {
        match self {
            Align::Left => DWRITE_TEXT_ALIGNMENT_LEADING,
            Align::Center => DWRITE_TEXT_ALIGNMENT_CENTER,
            Align::Right => DWRITE_TEXT_ALIGNMENT_TRAILING,
        }
    }
}

type FormatKey = (Font, u32, Align);

thread_local! {
    /// Text formats per (font, scale, alignment). DirectWrite formats are
    /// device independent, so they survive device loss.
    static FORMATS: RefCell<HashMap<FormatKey, IDWriteTextFormat>> =
        RefCell::new(HashMap::new());
    /// The dashed outline style per scale.
    static DASHES: RefCell<HashMap<u32, ID2D1StrokeStyle>> = RefCell::new(HashMap::new());
}

/// A single-line format: no wrapping, ellipsis trimming, centred vertically.
fn make_format(font: Font, s: f32, align: Align) -> windows::core::Result<IDWriteTextFormat> {
    let (face, size, weight) = font.spec();
    let (_, dwrite) = d2d::factories();
    // SAFETY: DirectWrite calls on objects owned here; the trimming sign is
    // kept alive by the format it is set on.
    unsafe {
        let f = dwrite.CreateTextFormat(
            face,
            None::<&IDWriteFontCollection>,
            DWRITE_FONT_WEIGHT(weight),
            DWRITE_FONT_STYLE_NORMAL,
            DWRITE_FONT_STRETCH_NORMAL,
            size * s,
            w!(""),
        )?;
        f.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
        f.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
        f.SetTextAlignment(align.dwrite())?;
        let sign = dwrite.CreateEllipsisTrimmingSign(&f)?;
        let trim = DWRITE_TRIMMING {
            granularity: DWRITE_TRIMMING_GRANULARITY_CHARACTER,
            delimiter: 0,
            delimiterCount: 0,
        };
        f.SetTrimming(&trim, &sign)?;
        Ok(f)
    }
}

fn format(font: Font, s: f32, align: Align) -> Option<IDWriteTextFormat> {
    let key = (font, s.to_bits(), align);
    FORMATS.with(|m| {
        let mut m = m.borrow_mut();
        if let Some(f) = m.get(&key) {
            return Some(f.clone());
        }
        let f = make_format(font, s, align).ok()?;
        m.insert(key, f.clone());
        Some(f)
    })
}

fn utf16(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

/// One line of `s` in `r`, centred vertically, cut with "…" if it does not fit.
pub fn text(c: &Canvas, font: Font, s: &str, r: RectF, rgb: u32, align: Align) {
    if s.is_empty() || r.right <= r.left || r.bottom <= r.top {
        return;
    }
    let Some(f) = format(font, c.s, align) else {
        return;
    };
    // SAFETY: drawing calls inside the surface's BeginDraw/EndDraw.
    unsafe {
        let Ok(brush) = c.dc.CreateSolidColorBrush(&color(rgb, 1.0), None) else {
            return;
        };
        // Colour fonts so emoji draw in colour; clipped to the box.
        c.dc.DrawText(
            &utf16(s),
            &f,
            &r,
            &brush,
            D2D1_DRAW_TEXT_OPTIONS_ENABLE_COLOR_FONT | D2D1_DRAW_TEXT_OPTIONS_CLIP,
            DWRITE_MEASURING_MODE_NATURAL,
        );
    }
}

/// Laid-out metrics of `s` in a `w` x `h` box (`None` if DirectWrite fails).
#[allow(dead_code)] // used by the widgets, and by the tests
fn metrics(
    c: &Canvas,
    font: Font,
    s: &str,
    w: f32,
    h: f32,
    align: Align,
) -> Option<DWRITE_TEXT_METRICS> {
    let f = format(font, c.s, align)?;
    let (_, dwrite) = d2d::factories();
    let mut m = DWRITE_TEXT_METRICS::default();
    // SAFETY: a text layout made and measured here.
    unsafe {
        let layout = dwrite.CreateTextLayout(&utf16(s), &f, w, h).ok()?;
        layout.GetMetrics(&mut m).ok()?;
    }
    Some(m)
}

/// Width in pixels `s` takes in `font` on one line.
#[allow(dead_code)] // used by the widgets, and by the tests
pub fn text_width(c: &Canvas, font: Font, s: &str) -> f32 {
    if s.is_empty() {
        return 0.0;
    }
    metrics(c, font, s, f32::MAX / 4.0, f32::MAX / 4.0, Align::Left)
        .map_or(0.0, |m| m.widthIncludingTrailingWhitespace)
}

/// Width `s` actually takes when laid out (and trimmed) in `r`.
#[cfg(test)]
pub(crate) fn laid_out_width(c: &Canvas, font: Font, s: &str, r: RectF, align: Align) -> f32 {
    metrics(c, font, s, r.right - r.left, r.bottom - r.top, align).map_or(0.0, |m| m.width)
}

/// A Segoe Fluent Icons glyph centred in `r`.
#[allow(dead_code)] // used by the widgets (pages use them from Task 8 on)
pub fn icon(c: &Canvas, g: char, r: RectF, rgb: u32) {
    icon_sized(c, Font::Icon, g, r, rgb);
}

/// A glyph centred in `r` in `font` (`Font::Icon` or `Font::IconLarge`).
pub fn icon_sized(c: &Canvas, font: Font, g: char, r: RectF, rgb: u32) {
    text(c, font, &g.to_string(), r, rgb, Align::Center);
}

fn dash_style(s: f32) -> Option<ID2D1StrokeStyle> {
    DASHES.with(|m| {
        let mut m = m.borrow_mut();
        if let Some(d) = m.get(&s.to_bits()) {
            return Some(d.clone());
        }
        let props = D2D1_STROKE_STYLE_PROPERTIES {
            startCap: D2D1_CAP_STYLE_FLAT,
            endCap: D2D1_CAP_STYLE_FLAT,
            dashCap: D2D1_CAP_STYLE_FLAT,
            lineJoin: D2D1_LINE_JOIN_MITER,
            miterLimit: 10.0,
            dashStyle: D2D1_DASH_STYLE_CUSTOM,
            dashOffset: 0.0,
        };
        // In stroke widths (1 px): 4 logical px on, 3 off.
        let dashes = [4.0 * s, 3.0 * s];
        let (d2d, _) = d2d::factories();
        // The Direct2D 1.0 call: a plain `ID2D1StrokeStyle` is all we need.
        let base: &ID2D1Factory = d2d;
        // SAFETY: a device-independent stroke style made here.
        let d = unsafe { base.CreateStrokeStyle(&props, Some(&dashes)) }.ok()?;
        m.insert(s.to_bits(), d.clone());
        Some(d)
    })
}

/// Rounded rectangle, anti-aliased. `fill` is a colour with its alpha
/// (0..=1), `stroke` a 1 px outline drawn inside the edge. Unlike the GDI
/// version, `radius` is the corner radius in pixels (not the diameter).
pub fn round_rect(
    c: &Canvas,
    r: &RectF,
    fill: Option<(u32, f32)>,
    stroke: Option<u32>,
    radius: f32,
    dashed: bool,
) {
    let (w, h) = (r.right - r.left, r.bottom - r.top);
    if w <= 0.0 || h <= 0.0 {
        return;
    }
    let rounded = |inset: f32| {
        let rad = radius
            .min((w - 2.0 * inset).min(h - 2.0 * inset) / 2.0)
            .max(0.0);
        D2D1_ROUNDED_RECT {
            rect: rectf(
                r.left + inset,
                r.top + inset,
                r.right - inset,
                r.bottom - inset,
            ),
            radiusX: rad,
            radiusY: rad,
        }
    };
    // SAFETY: drawing calls inside the surface's BeginDraw/EndDraw.
    unsafe {
        if let Some((rgb, alpha)) = fill {
            if let Ok(brush) = c.dc.CreateSolidColorBrush(&color(rgb, alpha), None) {
                c.dc.FillRoundedRectangle(&rounded(0.0), &brush);
            }
        }
        if let Some(rgb) = stroke {
            if let Ok(brush) = c.dc.CreateSolidColorBrush(&color(rgb, 1.0), None) {
                // A 1 px line centred half a pixel in stays crisp.
                let style = if dashed { dash_style(c.s) } else { None };
                c.dc.DrawRoundedRectangle(&rounded(0.5), &brush, 1.0, style.as_ref());
            }
        }
    }
}

/// Windows 11 toggle switch (40x20 logical) whose right edge is at `right`,
/// vertically centred on `cy`. Returns its rectangle for hit-testing.
#[allow(dead_code)] // used by the widgets (pages use them from Task 8 on)
pub fn toggle(c: &Canvas, right: f32, cy: f32, on: bool, enabled: bool) -> RectF {
    let (w, h) = (40.0 * c.s, 20.0 * c.s);
    let r = rectf(right - w, cy - h / 2.0, right, cy + h / 2.0);
    let knob = 12.0 * c.s;
    let inset = (h - knob) / 2.0;
    let p = &c.p;
    let (track, edge, dot) = match (on, enabled) {
        (true, true) => (p.accent, None, p.on_accent),
        (false, true) => (p.card, Some(p.text2), p.text2),
        (_, false) => (p.card, Some(p.stroke), p.stroke),
    };
    round_rect(c, &r, Some((track, 1.0)), edge, h / 2.0, false);
    let kx = if on {
        r.right - inset - knob
    } else {
        r.left + inset
    };
    let k = rectf(kx, r.top + inset, kx + knob, r.top + inset + knob);
    round_rect(c, &k, Some((dot, 1.0)), None, knob / 2.0, false);
    r
}

/// Standard button (accent or neutral) with a centred label.
#[allow(dead_code)] // used by the widgets (pages use them from Task 8 on)
pub fn button(c: &Canvas, r: RectF, label: &str, accent: bool) {
    let radius = tokens::RADIUS[0] * c.s;
    let p = c.p;
    if accent {
        round_rect(c, &r, Some((p.accent, 1.0)), None, radius, false);
        text(c, Font::Body, label, r, p.on_accent, Align::Center);
    } else {
        round_rect(c, &r, Some((p.card, 1.0)), Some(p.stroke), radius, false);
        text(c, Font::Body, label, r, p.text, Align::Center);
    }
}

/// WM_PAINT through Direct2D: validates the window, sizes the surface to the
/// client area, clears it (opaque: the token background; Mica: transparent)
/// and hands `draw` a canvas and the client rectangle.
pub fn paint_with(surface: &mut Surface, hwnd: HWND, draw: impl FnOnce(&mut Canvas, RectF)) {
    let mut ps = PAINTSTRUCT::default();
    let mut client = RECT::default();
    // SAFETY: BeginPaint/EndPaint pair on our own window, on its thread.
    unsafe {
        BeginPaint(hwnd, &mut ps);
        let _ = GetClientRect(hwnd, &mut client);
    }
    let (w, h) = (client.right - client.left, client.bottom - client.top);
    surface.resize(w.max(1) as u32, h.max(1) as u32);
    let s = scale(hwnd);
    let p = palette();
    let mica = surface.backdrop() == Backdrop::Mica;
    let area = rectf(0.0, 0.0, w as f32, h as f32);
    // A failed frame leaves the old one on screen; device loss is handled
    // (and the window invalidated) by the surface itself.
    let _ = surface.draw(|dc| {
        // SAFETY: state calls inside the surface's BeginDraw/EndDraw.
        unsafe {
            dc.Clear(Some(&color(p.bg, if mica { 0.0 } else { 1.0 })));
            dc.SetAntialiasMode(D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
            // ClearType needs an opaque target; on Mica it fringes.
            dc.SetTextAntialiasMode(if mica {
                D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE
            } else {
                D2D1_TEXT_ANTIALIAS_MODE_DEFAULT
            });
        }
        let mut canvas = Canvas { dc, s, p };
        draw(&mut canvas, area);
    });
    // SAFETY: ends the paint begun above.
    unsafe {
        let _ = EndPaint(hwnd, &ps);
    }
}

/// Windows that repaint (and re-theme their frame) when the theme changes.
static THEMED: Mutex<Vec<isize>> = Mutex::new(Vec::new());

/// Repaint `hwnd` on every theme change, until `unregister_window`.
pub fn register_window(hwnd: HWND) {
    if let Ok(mut v) = THEMED.lock() {
        if !v.contains(&(hwnd.0 as isize)) {
            v.push(hwnd.0 as isize);
        }
    }
}

pub fn unregister_window(hwnd: HWND) {
    if let Ok(mut v) = THEMED.lock() {
        v.retain(|&h| h != hwnd.0 as isize);
    }
}

#[cfg(test)]
static INVALIDATED: Mutex<Vec<isize>> = Mutex::new(Vec::new());

/// Test hook: the windows `on_theme_change` repainted since the last call.
#[cfg(test)]
fn invalidations() -> Vec<isize> {
    INVALIDATED
        .lock()
        .map(|mut v| std::mem::take(&mut *v))
        .unwrap_or_default()
}

/// The Windows or Relay theme changed: re-read `dark()`, match each
/// registered window's frame to it and repaint the window.
pub fn on_theme_change() {
    let windows = THEMED.lock().map(|v| v.clone()).unwrap_or_default();
    let d = dark() as i32;
    for h in windows {
        #[cfg(test)]
        {
            // Test handles are fake; record instead of touching windows.
            let _ = d;
            if let Ok(mut v) = INVALIDATED.lock() {
                v.push(h);
            }
        }
        #[cfg(not(test))]
        // SAFETY: attribute write and repaint request on a registered
        // window (unregistered in its WM_DESTROY); failures are harmless.
        unsafe {
            let hwnd = HWND(h as *mut _);
            let _ = DwmSetWindowAttribute(
                hwnd,
                DWMWA_USE_IMMERSIVE_DARK_MODE,
                &d as *const _ as *const _,
                4,
            );
            let _ = windows::Win32::Graphics::Gdi::InvalidateRect(Some(hwnd), None, false);
        }
    }
}

// ---------------------------------------------------------------------------
// GDI (until every window has moved to the canvas, Task 13)
// ---------------------------------------------------------------------------

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

/// Draw `s` with `font` into `r` (GDI; `color` is 0xRRGGBB).
pub fn gdi_text(hdc: HDC, font: HGDIOBJ, s: &str, r: RECT, color: u32, flags: DRAW_TEXT_FORMAT) {
    // DrawTextW on an empty buffer faults (with an ellipsis flag it writes to it).
    if s.is_empty() {
        return;
    }
    let mut w: Vec<u16> = s.encode_utf16().collect();
    let mut r = r;
    // SAFETY: valid DC; the previous font is restored.
    unsafe {
        let old = SelectObject(hdc, font);
        SetTextColor(hdc, COLORREF(colorref(color)));
        DrawTextW(hdc, &mut w, &mut r, flags);
        SelectObject(hdc, old);
    }
}

/// Width in pixels `s` takes in `font` on one line (GDI).
pub fn gdi_text_width(hdc: HDC, font: HGDIOBJ, s: &str) -> i32 {
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

pub fn gdi_icon(hdc: HDC, font: HGDIOBJ, g: char, r: RECT, color: u32) {
    gdi_text(
        hdc,
        font,
        &g.to_string(),
        r,
        color,
        DT_CENTER | DT_VCENTER | DT_SINGLELINE,
    );
}

/// 0xRRGGBB to opaque GDI+ ARGB (0xFFRRGGBB).
fn argb(rgb: u32) -> u32 {
    0xff00_0000 | (rgb & 0x00ff_ffff)
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

/// Anti-aliased `gdi_round_rect`; `false` (nothing drawn) if GDI+ is unusable.
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

/// Rounded rectangle (GDI); `stroke: None` draws fill only. Colours are
/// 0xRRGGBB. `radius` is the corner ellipse size (as GDI's `RoundRect`);
/// drawn anti-aliased with GDI+.
pub fn gdi_round_rect(
    hdc: HDC,
    r: &RECT,
    fill: u32,
    stroke: Option<u32>,
    radius: i32,
    dashed: bool,
) {
    if !smooth_round_rect(hdc, r, fill, stroke, radius, dashed) {
        jagged_round_rect(hdc, r, fill, stroke, radius, dashed);
    }
}

/// Plain GDI fallback for `gdi_round_rect` (jagged edges).
fn jagged_round_rect(
    hdc: HDC,
    r: &RECT,
    fill: u32,
    stroke: Option<u32>,
    radius: i32,
    dashed: bool,
) {
    // SAFETY: GDI objects are selected out and deleted before returning.
    unsafe {
        let brush = CreateSolidBrush(COLORREF(colorref(fill)));
        let pen = match stroke {
            Some(c) => CreatePen(
                if dashed { PS_DASH } else { PS_SOLID },
                1,
                COLORREF(colorref(c)),
            ),
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

/// Windows 11 toggle switch (GDI), as `toggle`.
pub fn gdi_toggle(
    hdc: HDC,
    s: f32,
    p: &Palette,
    right: i32,
    cy: i32,
    on: bool,
    enabled: bool,
) -> RECT {
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
    gdi_round_rect(hdc, &r, track, edge.or(Some(track)), h, false);
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
    gdi_round_rect(hdc, &k, dot, None, knob, false);
    r
}

/// Standard button (GDI), as `button`.
pub fn gdi_button(hdc: HDC, f: &Fonts, p: &Palette, r: RECT, label: &str, accent: bool, s: f32) {
    if accent {
        gdi_round_rect(hdc, &r, p.accent, None, corner_radius(s, 8), false);
        gdi_text(
            hdc,
            f.body,
            label,
            r,
            p.on_accent,
            DT_CENTER | DT_VCENTER | DT_SINGLELINE,
        );
    } else {
        gdi_round_rect(hdc, &r, p.card, Some(p.stroke), corner_radius(s, 8), false);
        gdi_text(
            hdc,
            f.body,
            label,
            r,
            p.text,
            DT_CENTER | DT_VCENTER | DT_SINGLELINE,
        );
    }
}

/// Double-buffered GDI WM_PAINT: `draw` gets a memory DC and the client size.
pub fn gdi_paint(hwnd: HWND, draw: impl FnOnce(HDC, RECT)) {
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
        let bg = CreateSolidBrush(COLORREF(colorref(palette().bg)));
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
    }
    if mica {
        set_backdrop(hwnd, DWMSBT_MAINWINDOW);
    }
}

/// Sets the DWM system backdrop; false where DWM refuses it (before
/// Windows 11 22H2), so the caller knows Mica is unavailable.
pub fn set_backdrop(hwnd: HWND, kind: DWM_SYSTEMBACKDROP_TYPE) -> bool {
    // SAFETY: one attribute write on the caller's window.
    unsafe {
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_SYSTEMBACKDROP_TYPE,
            &kind as *const _ as *const _,
            std::mem::size_of::<DWM_SYSTEMBACKDROP_TYPE>() as u32,
        )
        .is_ok()
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
    fn caption_glyphs_are_ten_px_like_windows() {
        let (face, size, _) = Font::IconSmall.spec();
        assert_eq!(size, 10.0);
        // SAFETY: a static wide string literal.
        assert_eq!(unsafe { face.to_string() }.unwrap(), "Segoe Fluent Icons");
    }

    #[test]
    fn palette_comes_from_tokens() {
        set_theme(1);
        assert_eq!(palette().accent, tokens::LIGHT.primary);
        assert_eq!(palette().amber, tokens::LIGHT.accent);
        set_theme(2);
        assert_eq!(palette().bg, tokens::DARK.surface);
        assert_eq!(palette().text2, tokens::DARK.text_muted);
        set_theme(0);
    }

    #[test]
    fn colorref_swaps_red_and_blue() {
        assert_eq!(colorref(0x3651C5), 0xC55136);
        assert_eq!(colorref(0xFFFFFF), 0xFFFFFF);
    }

    #[test]
    fn theme_change_repaints_registered_windows() {
        let (a, b) = (HWND(0x7110 as *mut _), HWND(0x7120 as *mut _));
        register_window(a);
        register_window(b);
        register_window(a);
        on_theme_change();
        let seen = invalidations();
        unregister_window(a);
        unregister_window(b);
        for h in [a, b] {
            let n = seen.iter().filter(|&&x| x == h.0 as isize).count();
            assert_eq!(n, 1, "{h:?} invalidated once");
        }
    }

    #[cfg(windows)]
    #[test]
    fn text_draws_with_ellipsis_in_a_hidden_window() {
        let hwnd = crate::d2d::test_window();
        let mut surface = Surface::new(hwnd, Backdrop::Opaque).expect("surface");
        let name = "a very long file name \u{1F4C4} \u{05E9}\u{05DC}\u{05D5}\u{05DD}.txt";
        let mut width = 0.0;
        let mut trimmed = f32::MAX;
        surface
            .draw(|dc| {
                let c = Canvas {
                    dc,
                    s: 1.0,
                    p: palette(),
                };
                let r = rectf(0.0, 0.0, 40.0, 20.0);
                text(&c, Font::Body, name, r, c.p.text, Align::Left);
                width = text_width(&c, Font::Body, name);
                trimmed = laid_out_width(&c, Font::Body, name, r, Align::Left);
            })
            .expect("draw");
        assert!(width > 40.0, "untrimmed width {width}");
        assert!(trimmed <= 40.5, "trimmed to the box: {trimmed}");
        drop(surface);
        // SAFETY: our own test window, made above on this thread.
        unsafe {
            let _ = windows::Win32::UI::WindowsAndMessaging::DestroyWindow(hwnd);
        }
    }

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

    /// GDI+ wants opaque 0xAARRGGBB.
    #[test]
    fn rgb_becomes_opaque_argb() {
        assert_eq!(argb(0x0067b8), 0xff0067b8);
        assert_eq!(argb(0x000000), 0xff000000);
        assert_eq!(argb(0xffffff), 0xffffffff);
        assert_eq!(argb(0xc42b1c), 0xffc42b1c);
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
            gdi_round_rect(hdc, &all, 0x000000, None, 0, false);
            let r = RECT {
                left: 4,
                top: 4,
                right: 36,
                bottom: 36,
            };
            assert!(smooth_round_rect(
                hdc,
                &r,
                0x0067b8,
                Some(0xc42b1c),
                16,
                false
            ));
            // GetPixel answers in COLORREF.
            assert_eq!(GetPixel(hdc, 20, 20).0, colorref(0x0067b8), "fill");
            assert_eq!(GetPixel(hdc, 20, 4).0, colorref(0xc42b1c), "top edge");
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
            gdi_text(hdc, f.small, "", r, 0, DT_LEFT | DT_END_ELLIPSIS);
            gdi_text(hdc, f.small, "", r, 0, DT_LEFT);
            let _ = DeleteDC(hdc);
        }
    }
}
