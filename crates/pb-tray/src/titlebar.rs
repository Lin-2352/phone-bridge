//! Custom title bar: caption layout, the pure `WM_NCHITTEST` mapping and the
//! caption-button drawing. The client area extends into the title area
//! (`WM_NCCALCSIZE`), so the window answers hit tests itself. Returning
//! `HTMAXBUTTON` over the maximise button keeps the Windows 11 Snap Layouts
//! flyout.

use windows::Win32::UI::WindowsAndMessaging::{
    HTBOTTOM, HTBOTTOMLEFT, HTBOTTOMRIGHT, HTCAPTION, HTCLIENT, HTCLOSE, HTLEFT, HTMAXBUTTON,
    HTMINBUTTON, HTRIGHT, HTTOP, HTTOPLEFT, HTTOPRIGHT,
};

use crate::fluent::{self, rectf, Canvas, Font, RectF};

/// Caption height, caption button width and resize border, in logical px.
const CAPTION_H: f32 = 40.0;
const BUTTON_W: f32 = 46.0;
const BORDER: f32 = 6.0;

/// The standard Windows close-button hover red; the palette's `danger` is a
/// lighter tint in dark mode.
const CLOSE_RED: u32 = 0xC42B1C;

/// Segoe Fluent Icons caption glyphs.
const GLYPH_MIN: char = '\u{E921}';
const GLYPH_MAX: char = '\u{E922}';
const GLYPH_RESTORE: char = '\u{E923}';
const GLYPH_CLOSE: char = '\u{E8BB}';

/// Caption rectangles in pixels, relative to the window's top-left.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Caption {
    pub drag: RectF,
    pub min: RectF,
    pub max: RectF,
    pub close: RectF,
}

pub fn layout(width: f32, s: f32) -> Caption {
    let (h, bw) = (CAPTION_H * s, BUTTON_W * s);
    Caption {
        drag: rectf(0.0, 0.0, width - 3.0 * bw, h),
        min: rectf(width - 3.0 * bw, 0.0, width - 2.0 * bw, h),
        max: rectf(width - 2.0 * bw, 0.0, width - bw, h),
        close: rectf(width - bw, 0.0, width, h),
    }
}

fn within(r: &RectF, x: f32, y: f32) -> bool {
    x >= r.left && x < r.right && y >= r.top && y < r.bottom
}

/// The `WM_NCHITTEST` result for a point in window pixels. Resize edges win
/// over the buttons, which win over the drag area.
pub fn hit_test(x: f32, y: f32, w: f32, h: f32, s: f32, maximized: bool) -> u32 {
    if !maximized {
        let b = BORDER * s;
        let (l, r) = (x < b, x >= w - b);
        let (t, bo) = (y < b, y >= h - b);
        let edge = match (l, r, t, bo) {
            (true, _, true, _) => Some(HTTOPLEFT),
            (_, true, true, _) => Some(HTTOPRIGHT),
            (true, _, _, true) => Some(HTBOTTOMLEFT),
            (_, true, _, true) => Some(HTBOTTOMRIGHT),
            (true, ..) => Some(HTLEFT),
            (_, true, ..) => Some(HTRIGHT),
            (_, _, true, _) => Some(HTTOP),
            (_, _, _, true) => Some(HTBOTTOM),
            _ => None,
        };
        if let Some(e) = edge {
            return e;
        }
    }
    let cap = layout(w, s);
    if within(&cap.close, x, y) {
        HTCLOSE
    } else if within(&cap.max, x, y) {
        HTMAXBUTTON
    } else if within(&cap.min, x, y) {
        HTMINBUTTON
    } else if within(&cap.drag, x, y) {
        HTCAPTION
    } else {
        HTCLIENT
    }
}

/// Draws the three caption buttons. `hover` and `pressed` are the `HT*`
/// value of the button under the pointer / held down.
pub fn draw(
    c: &mut Canvas,
    cap: &Caption,
    hover: Option<u32>,
    pressed: Option<u32>,
    maximized: bool,
    active: bool,
) {
    let buttons = [
        (&cap.min, HTMINBUTTON, GLYPH_MIN),
        (
            &cap.max,
            HTMAXBUTTON,
            if maximized { GLYPH_RESTORE } else { GLYPH_MAX },
        ),
        (&cap.close, HTCLOSE, GLYPH_CLOSE),
    ];
    for (r, ht, glyph) in buttons {
        let is_close = ht == HTCLOSE;
        let (fill, mut glyph_rgb) = match (pressed == Some(ht), hover == Some(ht)) {
            (true, _) if is_close => (Some((CLOSE_RED, 0.8)), 0xFFFFFF),
            (_, true) if is_close && pressed.is_none() => (Some((CLOSE_RED, 1.0)), 0xFFFFFF),
            (true, _) => (Some((c.p.stroke, 0.9)), c.p.text),
            (_, true) if pressed.is_none() => (Some((c.p.stroke, 0.6)), c.p.text),
            _ => (None, c.p.text),
        };
        if !active && fill.is_none() {
            glyph_rgb = c.p.text2;
        }
        if let Some(f) = fill {
            fluent::round_rect(c, r, Some(f), None, 0.0, false);
        }
        fluent::icon_sized(c, Font::Icon, glyph, *r, glyph_rgb);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn max_button_reports_htmaxbutton_for_snap_layouts() {
        let c = layout(800.0, 1.0);
        assert_eq!(
            hit_test(c.max.left + 5.0, 10.0, 800.0, 600.0, 1.0, false),
            HTMAXBUTTON
        );
    }

    #[test]
    fn caption_buttons_and_drag_area() {
        let c = layout(800.0, 1.0);
        let ht = |x, y| hit_test(x, y, 800., 600., 1.0, false);
        assert_eq!(ht(c.close.left + 5.0, 10.0), HTCLOSE);
        assert_eq!(ht(c.min.left + 5.0, 10.0), HTMINBUTTON);
        assert_eq!(ht(300.0, 20.0), HTCAPTION);
        assert_eq!(ht(300.0, 300.0), HTCLIENT);
    }

    #[test]
    fn edges_resize_unless_maximized() {
        assert_eq!(hit_test(2.0, 300.0, 800., 600., 1.0, false), HTLEFT);
        assert_eq!(
            hit_test(798.0, 598.0, 800., 600., 1.0, false),
            HTBOTTOMRIGHT
        );
        assert_eq!(hit_test(2.0, 300.0, 800., 600., 1.0, true), HTCLIENT);
    }

    #[test]
    fn scales_with_dpi() {
        let c = layout(1600.0, 2.0);
        assert_eq!(c.close.right - c.close.left, 92.0);
    }
}
