//! Small Fluent (Windows 11) drawing kit shared by the tray windows: theme,
//! token palette, fonts, Segoe Fluent Icons glyphs, toggles and buttons, DWM
//! window frame. Every window draws through a Direct2D `Canvas` (`paint_with`).
#![allow(unsafe_code)]

use std::{
    cell::RefCell,
    collections::HashMap,
    sync::{
        atomic::{AtomicU8, Ordering},
        Mutex,
    },
};

use windows::{
    core::{w, Interface, PCWSTR},
    Win32::{
        Foundation::{HWND, RECT},
        Graphics::{
            Direct2D::{
                Common::D2D_RECT_F, ID2D1DeviceContext, ID2D1Factory, ID2D1StrokeStyle,
                D2D1_ANTIALIAS_MODE_PER_PRIMITIVE, D2D1_CAP_STYLE_FLAT, D2D1_DASH_STYLE_CUSTOM,
                D2D1_DRAW_TEXT_OPTIONS_CLIP, D2D1_DRAW_TEXT_OPTIONS_ENABLE_COLOR_FONT,
                D2D1_LINE_JOIN_MITER, D2D1_ROUNDED_RECT, D2D1_STROKE_STYLE_PROPERTIES,
                D2D1_TEXT_ANTIALIAS_MODE_DEFAULT, D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE,
            },
            DirectWrite::{
                IDWriteFontCollection, IDWriteTextFormat, IDWriteTextLayout1,
                DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_WEIGHT,
                DWRITE_MEASURING_MODE_NATURAL, DWRITE_PARAGRAPH_ALIGNMENT_CENTER,
                DWRITE_PARAGRAPH_ALIGNMENT_NEAR, DWRITE_TEXT_ALIGNMENT,
                DWRITE_TEXT_ALIGNMENT_CENTER, DWRITE_TEXT_ALIGNMENT_LEADING,
                DWRITE_TEXT_ALIGNMENT_TRAILING, DWRITE_TEXT_METRICS, DWRITE_TEXT_RANGE,
                DWRITE_TRIMMING, DWRITE_TRIMMING_GRANULARITY_CHARACTER,
                DWRITE_WORD_WRAPPING_NO_WRAP, DWRITE_WORD_WRAPPING_WRAP,
            },
            Dwm::{
                DwmSetWindowAttribute, DWMSBT_MAINWINDOW, DWMWA_SYSTEMBACKDROP_TYPE,
                DWMWA_USE_IMMERSIVE_DARK_MODE, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND,
                DWM_SYSTEMBACKDROP_TYPE,
            },
            Gdi::{BeginPaint, EndPaint, PAINTSTRUCT},
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
    pub const CHECK: char = '\u{E73E}';
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
        amber: t.attention,
    }
}

/// 0xRRGGBB to a GDI COLORREF value (0x00BBGGRR), for the native EDIT
/// control's colours.
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

/// A pixel-snapped GDI-style `RECT` (as the pure layouts make) as a `RectF`.
pub fn from_rect(r: RECT) -> RectF {
    rectf(r.left as f32, r.top as f32, r.right as f32, r.bottom as f32)
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
    /// 32 px monospaced: the pairing code in its dialog card.
    Code,
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
            Font::Code => (w!("Consolas"), 32.0, 600),
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
    /// Word-wrapping, top-aligned formats per (font, scale).
    static WRAPPED: RefCell<HashMap<(Font, u32), IDWriteTextFormat>> =
        RefCell::new(HashMap::new());
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

/// One line of `s` in `r` like `text`, with `trailing` extra pixels after
/// every character (letter spacing, e.g. for a code to compare).
pub fn text_spaced(
    c: &Canvas,
    font: Font,
    s: &str,
    r: RectF,
    rgb: u32,
    align: Align,
    trailing: f32,
) {
    if s.is_empty() || r.right <= r.left || r.bottom <= r.top {
        return;
    }
    let Some(f) = format(font, c.s, align) else {
        return;
    };
    let (_, dwrite) = d2d::factories();
    let chars = utf16(s);
    // SAFETY: a text layout made here and drawn inside the surface's
    // BeginDraw/EndDraw.
    unsafe {
        let Ok(layout) = dwrite.CreateTextLayout(&chars, &f, r.right - r.left, r.bottom - r.top)
        else {
            return;
        };
        if let Ok(l1) = layout.cast::<IDWriteTextLayout1>() {
            let all = DWRITE_TEXT_RANGE {
                startPosition: 0,
                length: chars.len() as u32,
            };
            let _ = l1.SetCharacterSpacing(0.0, trailing, 0.0, all);
        }
        let Ok(brush) = c.dc.CreateSolidColorBrush(&color(rgb, 1.0), None) else {
            return;
        };
        c.dc.DrawTextLayout(
            windows_numerics::Vector2 {
                X: r.left,
                Y: r.top,
            },
            &layout,
            &brush,
            D2D1_DRAW_TEXT_OPTIONS_ENABLE_COLOR_FONT | D2D1_DRAW_TEXT_OPTIONS_CLIP,
        );
    }
}

/// The width `s` takes laid out with `trailing` letter spacing.
#[cfg(test)]
fn spaced_width(font: Font, s: f32, t: &str, trailing: f32) -> f32 {
    let Some(f) = format(font, s, Align::Left) else {
        return 0.0;
    };
    let (_, dwrite) = d2d::factories();
    let chars = utf16(t);
    let mut m = DWRITE_TEXT_METRICS::default();
    // SAFETY: a text layout made and measured here.
    unsafe {
        let Ok(layout) = dwrite.CreateTextLayout(&chars, &f, 10_000.0, 1_000.0) else {
            return 0.0;
        };
        if let Ok(l1) = layout.cast::<IDWriteTextLayout1>() {
            let all = DWRITE_TEXT_RANGE {
                startPosition: 0,
                length: chars.len() as u32,
            };
            let _ = l1.SetCharacterSpacing(0.0, trailing, 0.0, all);
        }
        let _ = layout.GetMetrics(&mut m);
    }
    m.widthIncludingTrailingWhitespace
}

/// The word-wrapping, top-aligned format for `font` at scale `s`.
fn wrapped_format(font: Font, s: f32) -> Option<IDWriteTextFormat> {
    let key = (font, s.to_bits());
    WRAPPED.with(|m| {
        let mut m = m.borrow_mut();
        if let Some(f) = m.get(&key) {
            return Some(f.clone());
        }
        let f = make_format(font, s, Align::Left).ok()?;
        // SAFETY: setters on the fresh format made above (not shared yet).
        unsafe {
            f.SetWordWrapping(DWRITE_WORD_WRAPPING_WRAP).ok()?;
            f.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_NEAR)
                .ok()?;
        }
        m.insert(key, f.clone());
        Some(f)
    })
}

/// Height in pixels `text_wrapped` needs for `s` at `width`, and the
/// height of one line (to cap a box at whole lines). Zero for empty text.
/// Call only once a `Surface` exists (it needs the DirectWrite factory).
pub fn wrapped_height(font: Font, s: f32, text: &str, width: f32) -> (f32, f32) {
    let Some(f) = wrapped_format(font, s) else {
        return (0.0, 0.0);
    };
    let (_, dwrite) = d2d::factories();
    let measure = |t: &str| {
        let mut m = DWRITE_TEXT_METRICS::default();
        // SAFETY: a text layout made and measured here.
        unsafe {
            let layout = dwrite
                .CreateTextLayout(&utf16(t), &f, width.max(1.0), f32::MAX / 4.0)
                .ok()?;
            layout.GetMetrics(&mut m).ok()?;
        }
        Some(m.height)
    };
    let line = measure("X").unwrap_or(0.0);
    if text.is_empty() {
        return (0.0, line);
    }
    (measure(text).unwrap_or(0.0), line)
}

/// `s` wrapped onto as many lines as fit in `r`, from its top-left; what
/// does not fit is clipped.
pub fn text_wrapped(c: &Canvas, font: Font, s: &str, r: RectF, rgb: u32) {
    if s.is_empty() || r.right <= r.left || r.bottom <= r.top {
        return;
    }
    let Some(f) = wrapped_format(font, c.s) else {
        return;
    };
    // SAFETY: drawing calls inside the surface's BeginDraw/EndDraw.
    unsafe {
        let Ok(brush) = c.dc.CreateSolidColorBrush(&color(rgb, 1.0), None) else {
            return;
        };
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
/// Windows' "Show animations" setting is off (Settings > Accessibility >
/// Visual effects): animations should jump to their end. Asked each time
/// (a cheap system call), so a change applies at once.
pub fn reduced_motion() -> bool {
    use windows::Win32::UI::WindowsAndMessaging::{
        SystemParametersInfoW, SPI_GETCLIENTAREAANIMATION, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS,
    };
    let mut on = windows::core::BOOL(1);
    // SAFETY: SPI_GETCLIENTAREAANIMATION writes one BOOL through the pointer.
    let ok = unsafe {
        SystemParametersInfoW(
            SPI_GETCLIENTAREAANIMATION,
            0,
            Some(std::ptr::addr_of_mut!(on).cast()),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
    };
    ok.is_ok() && !on.as_bool()
}

/// The keyboard focus ring: two accent outlines just outside `r`.
pub fn focus_ring(c: &Canvas, r: RectF) {
    let radius = crate::tokens::RADIUS[1] * c.s;
    for grow in [2.0, 1.0] {
        let o = rectf(r.left - grow, r.top - grow, r.right + grow, r.bottom + grow);
        round_rect(c, &o, None, Some(c.p.accent), radius + grow, false);
    }
}

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
/// (0..=1), `stroke` a 1 px outline drawn inside the edge. `radius` is the
/// corner radius in pixels.
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
        assert_eq!(palette().amber, tokens::LIGHT.attention);
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

    #[cfg(windows)]
    #[test]
    fn wrapped_height_counts_the_lines() {
        let hwnd = crate::d2d::test_window();
        let surface = Surface::new(hwnd, Backdrop::Opaque).expect("surface");
        let (empty, line) = wrapped_height(Font::Body, 1.0, "", 200.0);
        assert_eq!(empty, 0.0);
        assert!(line > 14.0 && line < 30.0, "one Body line: {line}");
        let (one, _) = wrapped_height(Font::Body, 1.0, "Accept", 200.0);
        assert!((one - line).abs() < 0.5, "{one} vs {line}");
        let long = "word ".repeat(60);
        let (many, _) = wrapped_height(Font::Body, 1.0, &long, 200.0);
        assert!(many > 5.0 * line, "wraps onto many lines: {many}");
        let (big, big_line) = wrapped_height(Font::Body, 2.0, &long, 400.0);
        assert!((big_line - 2.0 * line).abs() < 1.0 && big > 1.5 * many);
        drop(surface);
        // SAFETY: our own test window, made above on this thread.
        unsafe {
            let _ = windows::Win32::UI::WindowsAndMessaging::DestroyWindow(hwnd);
        }
    }

    #[cfg(windows)]
    #[test]
    fn letter_spacing_widens_the_code() {
        let hwnd = crate::d2d::test_window();
        let mut surface = Surface::new(hwnd, Backdrop::Opaque).expect("surface");
        let plain = spaced_width(Font::Code, 1.5, "628 183", 0.0);
        let spaced = spaced_width(Font::Code, 1.5, "628 183", 6.0);
        // Seven characters, 6 px after each.
        assert!((spaced - plain - 42.0).abs() < 1.0, "{plain} -> {spaced}");
        surface
            .draw(|dc| {
                let c = Canvas {
                    dc,
                    s: 1.5,
                    p: palette(),
                };
                let r = rectf(0.0, 0.0, 200.0, 100.0);
                text_spaced(&c, Font::Code, "628 183", r, c.p.text, Align::Center, 6.0);
            })
            .expect("draw");
        drop(surface);
        // SAFETY: our own test window, made above on this thread.
        unsafe {
            let _ = windows::Win32::UI::WindowsAndMessaging::DestroyWindow(hwnd);
        }
    }

    #[test]
    fn a_layout_rect_converts_edge_for_edge() {
        let r = from_rect(RECT {
            left: 1,
            top: 2,
            right: 30,
            bottom: 40,
        });
        assert_eq!((r.left, r.top, r.right, r.bottom), (1.0, 2.0, 30.0, 40.0));
    }

    /// Every `.rs` file under `dir`, with its path relative to `root`
    /// ('/'-separated).
    fn sources(root: &std::path::Path, dir: &std::path::Path, out: &mut Vec<(String, String)>) {
        for entry in std::fs::read_dir(dir).expect("source dir") {
            let path = entry.expect("entry").path();
            if path.is_dir() {
                sources(root, &path, out);
            } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
                let rel = path.strip_prefix(root).expect("under src");
                let rel = rel.to_string_lossy().replace('\\', "/");
                out.push((rel, std::fs::read_to_string(&path).expect("source")));
            }
        }
    }

    /// GDI drawing calls found in `files` ((path, text) pairs), except where
    /// allowed: the native EDIT control (font and text colour) and the
    /// paint pass in `paint_with`. The needles are split so this test does
    /// not match itself; fluent.rs is scanned too (the GDI kit lived here).
    fn gdi_calls(files: &[(String, String)]) -> Vec<String> {
        // (needle, files allowed to use it)
        let rules: [(&str, &[&str]); 12] = [
            (concat!("gdi_", "text("), &[]),
            (concat!("gdi_", "round_rect("), &[]),
            (concat!("gdi_", "paint("), &[]),
            (concat!("gdi_", "icon("), &[]),
            (concat!("Stretch", "DIBits("), &[]),
            (concat!("Bit", "Blt("), &[]),
            (concat!("Draw", "TextW("), &[]),
            (concat!("Fill", "Rect("), &[]),
            (concat!("Round", "Rect("), &[]),
            // WM_CTLCOLOREDIT: the EDIT control's own text colour.
            (concat!("SetText", "Color("), &["search_box.rs"]),
            // The native EDIT control needs a GDI font.
            (concat!("CreateFont", "W("), &["search_box.rs"]),
            // `paint_with` validates the window with a paint pass.
            (concat!("Begin", "Paint("), &["fluent.rs"]),
        ];
        let mut offending = Vec::new();
        for (name, body) in files {
            for (needle, allowed) in rules {
                if body.contains(needle) && !allowed.contains(&name.as_str()) {
                    offending.push(format!("{name}: {needle}"));
                }
            }
        }
        offending.sort();
        offending
    }

    /// Every window draws through Direct2D: no GDI text, shapes, blits or
    /// paint passes are left anywhere under src/ (subfolders included).
    #[test]
    fn no_gdi_drawing_left() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut files = Vec::new();
        sources(&root, &root, &mut files);
        assert!(
            files.iter().any(|(n, _)| n.starts_with("pages/")),
            "subfolders are scanned"
        );
        let offending = gdi_calls(&files);
        assert!(offending.is_empty(), "GDI drawing left in: {offending:#?}");
    }

    #[test]
    fn the_gdi_guard_catches_each_call_outside_its_allow_list() {
        let file = |n: &str, body: String| (n.to_owned(), body);
        let found = gdi_calls(&[
            file("pages/home.rs", concat!("Bit", "Blt(hdc, 0)").into()),
            file("a.rs", concat!("Fill", "Rect(hdc, &r, b)").into()),
            file("search_box.rs", concat!("SetText", "Color(hdc, c)").into()),
            file("b.rs", concat!("SetText", "Color(hdc, c)").into()),
            file("fluent.rs", concat!("Begin", "Paint(hwnd, &mut ps)").into()),
        ]);
        assert_eq!(
            found,
            [
                concat!("a.rs: Fill", "Rect("),
                concat!("b.rs: SetText", "Color("),
                concat!("pages/home.rs: Bit", "Blt("),
            ]
        );
    }

    #[test]
    fn spacing_constants_are_on_the_4px_grid() {
        assert_eq!(GROUP_GAP % 4, 0);
        assert_eq!(ROW_PAD % 4, 0);
    }
}
