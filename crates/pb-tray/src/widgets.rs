//! Widget kit for the Relay windows: scrolling, virtual list range, nav rail,
//! rows, cards, badges, info bars, empty states and skeletons. Layout is pure
//! (testable); the draw parts run over a Direct2D `Canvas`.
pub mod scrollbar;

use std::{ops::Range, sync::OnceLock, time::Instant};

use crate::{
    ease::Anim,
    fluent::{
        self, icon, icon_sized, rectf, round_rect, text, text_width, Align, Canvas, Font, RectF,
    },
    tokens::{self, motion, SPACE},
};

/// One mouse-wheel notch (120) scrolls this many logical px.
const WHEEL_STEP: f32 = 48.0;
/// The scrollbar thumb never gets shorter than this.
const MIN_THUMB: f32 = scrollbar::MIN_THUMB;

/// Scroll state of a list or page. Lengths share one unit chosen by the
/// caller; the wheel step of 48 is in logical px, so use logical px.
#[derive(Debug, Clone)]
pub struct Scroll {
    pub offset: f32,
    pub content: f32,
    pub view: f32,
    anim: Option<Anim>,
}

impl Scroll {
    /// `content` is the full height, `view` the visible height.
    pub fn new(content: f32, view: f32) -> Self {
        Self {
            offset: 0.0,
            content,
            view,
            anim: None,
        }
    }

    fn max_offset(&self) -> f32 {
        (self.content - self.view).max(0.0)
    }

    /// Keeps `offset` inside 0..=content-view.
    pub fn clamp(&mut self) {
        self.offset = self.offset.clamp(0.0, self.max_offset());
    }

    /// A wheel event (120 = one notch, positive = up): eases to the new
    /// target, adding to any scroll still in flight.
    pub fn wheel(&mut self, delta: i32, now: Instant) {
        let base = self.anim.map_or(self.offset, |a| a.to);
        let target = (base - delta as f32 / 120.0 * WHEEL_STEP).clamp(0.0, self.max_offset());
        if target == self.offset {
            // At a limit: nothing to ease, so no frame timer is needed.
            self.anim = None;
            return;
        }
        self.anim = Some(Anim::new(self.offset, target, now, motion::FAST_MS));
    }

    /// Where the offset is heading: the end of the ease in flight, else the
    /// offset itself.
    pub fn target(&self) -> f32 {
        self.anim.map_or(self.offset, |a| a.to)
    }

    /// Eases to `target` (clamped), like a wheel step of any size: paging
    /// keys and track clicks.
    pub fn ease_to(&mut self, target: f32, now: Instant) {
        let target = target.clamp(0.0, self.max_offset());
        if target == self.target() {
            return;
        }
        self.anim = Some(Anim::new(self.offset, target, now, motion::FAST_MS));
    }

    /// Jumps to `offset` (clamped) with no animation: dragging the thumb,
    /// or keeping the view still while the content changes.
    pub fn jump(&mut self, offset: f32) {
        self.anim = None;
        self.offset = offset;
        self.clamp();
    }

    /// True while a wheel ease still runs at `now` (read-only; `tick` advances).
    pub fn animating(&self, now: Instant) -> bool {
        self.anim.is_some_and(|a| a.running(now))
    }

    /// Advances the ease to `now`; true while it still runs (keep the timer).
    pub fn tick(&mut self, now: Instant) -> bool {
        let Some(a) = self.anim else {
            return false;
        };
        self.offset = a.value(now);
        if !a.running(now) {
            self.anim = None;
        }
        self.clamp();
        self.anim.is_some()
    }

    /// Scrollbar thumb as (top, height) in the view; None when all fits.
    pub fn thumb(&self) -> Option<(f32, f32)> {
        if self.content <= self.view || self.view <= 0.0 {
            return None;
        }
        let h = (self.view * self.view / self.content)
            .max(MIN_THUMB)
            .min(self.view);
        let top = self.offset.clamp(0.0, self.max_offset()) / self.max_offset() * (self.view - h);
        Some((top, h))
    }
}

/// Rows to build for a virtual list: those in view plus one of overscan
/// on each side, never past `count`.
pub fn visible_range(offset: f32, view: f32, row_h: f32, count: usize) -> Range<usize> {
    if row_h <= 0.0 || count == 0 {
        return 0..0;
    }
    let top = offset.max(0.0);
    let first = (top / row_h).floor() as usize;
    let last = ((top + view.max(0.0)) / row_h).ceil() as usize;
    first.saturating_sub(1).min(count)..(last + 1).min(count)
}

/// Item rectangles of the nav rail, top to bottom, as (rect, index). The
/// positions do not depend on `_selected` (selecting never shifts the
/// layout); callers pass the selection to `draw_nav_item`.
pub fn nav_rail(r: RectF, s: f32, items: &[&str], _selected: usize) -> Vec<(RectF, usize)> {
    let (margin, h, gap) = (SPACE[1] * s, 56.0 * s, SPACE[0] * s);
    (0..items.len())
        .map(|i| {
            let top = r.top + margin + i as f32 * (h + gap);
            (rectf(r.left + margin, top, r.right - margin, top + h), i)
        })
        .collect()
}

/// One nav rail item: a pill and accent bar when selected, glyph over caption.
pub fn draw_nav_item(c: &Canvas, r: RectF, glyph: char, label: &str, selected: bool) {
    let p = c.p;
    let radius = tokens::RADIUS[1] * c.s;
    if selected {
        round_rect(c, &r, Some((p.hover, 1.0)), None, radius, false);
        let bar = rectf(
            r.left,
            r.top + 16.0 * c.s,
            r.left + 3.0 * c.s,
            r.bottom - 16.0 * c.s,
        );
        round_rect(c, &bar, Some((p.accent, 1.0)), None, 1.5 * c.s, false);
    }
    let g = rectf(r.left, r.top + 8.0 * c.s, r.right, r.top + 30.0 * c.s);
    icon_sized(
        c,
        Font::IconLarge,
        glyph,
        g,
        if selected { p.accent } else { p.text2 },
    );
    let l = rectf(r.left, r.top + 32.0 * c.s, r.right, r.bottom - 4.0 * c.s);
    let rgb = if selected { p.text } else { p.text2 };
    text(c, Font::Caption, label, l, rgb, Align::Center);
}

/// Row parts as (icon, title, meta, trailing). The trailing column
/// (`trailing_w` px, already measured) is reserved first at the right, so
/// the title and meta stop 12 logical px short of it.
pub fn row_layout(r: RectF, s: f32, trailing_w: f32) -> (RectF, RectF, RectF, RectF) {
    let pad = fluent::ROW_PAD as f32 * s;
    let gap = SPACE[2] * s;
    let cy = (r.top + r.bottom) / 2.0;
    let icon_sz = 20.0 * s;
    let icon_r = rectf(
        r.left + pad,
        cy - icon_sz / 2.0,
        r.left + pad + icon_sz,
        cy + icon_sz / 2.0,
    );
    let trail_right = r.right - pad;
    // The trailing column shrinks before it may touch the icon's gap; the
    // text then stops `gap` short of it (zero width if there is no room).
    let min_left = icon_r.right + gap;
    let trail_left = (trail_right - trailing_w.max(0.0)).max(min_left);
    let trail_right = trail_right.max(trail_left);
    let text_right = if trailing_w > 0.0 {
        trail_left - gap
    } else {
        trail_right
    };
    let text_left = min_left.min(text_right);
    let (title_h, meta_h) = (20.0 * s, 16.0 * s);
    let top = cy - (title_h + meta_h) / 2.0;
    let title = rectf(text_left, top, text_right, top + title_h);
    let meta = rectf(text_left, top + title_h, text_right, top + title_h + meta_h);
    let trail = rectf(trail_left, r.top, trail_right, r.bottom);
    (icon_r, title, meta, trail)
}

/// A list row: glyph, ellipsized title over a caption line, and a trailing
/// text column (time, size, status) that is never trimmed.
pub fn row(c: &Canvas, r: RectF, glyph: char, title: &str, meta: &str, trailing: &str) {
    let p = c.p;
    let trailing_w = text_width(c, Font::Caption, trailing).ceil() + 1.0;
    let (icon_r, mut title_r, meta_r, trail_r) = row_layout(r, c.s, trailing_w);
    icon(c, glyph, icon_r, p.text2);
    if meta.is_empty() {
        // One line: the title takes the whole text block, centred.
        title_r.bottom = meta_r.bottom;
    }
    text(c, Font::Body, title, title_r, p.text, Align::Left);
    text(c, Font::Caption, meta, meta_r, p.text2, Align::Left);
    text(c, Font::Caption, trailing, trail_r, p.text2, Align::Right);
}

/// Card surface: token card fill, 1 px stroke, 8 px corners.
pub fn card(c: &Canvas, r: RectF) {
    let radius = tokens::RADIUS[1] * c.s;
    round_rect(
        c,
        &r,
        Some((c.p.card, 1.0)),
        Some(c.p.stroke),
        radius,
        false,
    );
}

/// Pill with a tinted fill and `rgb` text.
pub fn badge(c: &Canvas, r: RectF, label: &str, rgb: u32) {
    let radius = (r.bottom - r.top) / 2.0;
    round_rect(c, &r, Some((rgb, 0.16)), None, radius, false);
    text(c, Font::Caption, label, r, rgb, Align::Center);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InfoKind {
    Info,
    Warning,
    Error,
}

/// Inline message bar: tinted fill, kind glyph in the kind colour, text.
pub fn info_bar(c: &Canvas, r: RectF, kind: InfoKind, message: &str) {
    let p = c.p;
    let (rgb, glyph) = match kind {
        InfoKind::Info => (p.accent, '\u{E946}'),
        InfoKind::Warning => (p.warning, '\u{E7BA}'),
        InfoKind::Error => (p.danger, '\u{EA39}'),
    };
    let radius = tokens::RADIUS[1] * c.s;
    round_rect(c, &r, Some((rgb, 0.12)), Some(p.stroke), radius, false);
    let pad = SPACE[2] * c.s;
    let g = rectf(r.left + pad, r.top, r.left + pad + 20.0 * c.s, r.bottom);
    icon(c, glyph, g, rgb);
    let t = rectf(g.right + SPACE[1] * c.s, r.top, r.right - pad, r.bottom);
    text(c, Font::Body, message, t, p.text, Align::Left);
}

/// Centred glyph, title and body, as one block centred in `r`.
pub fn empty_state(c: &Canvas, r: RectF, glyph: char, title: &str, body: &str) {
    let p = c.p;
    let (ih, th, bh, gap) = (32.0 * c.s, 24.0 * c.s, 20.0 * c.s, SPACE[1] * c.s);
    let top = (r.top + r.bottom - (ih + gap + th + bh)) / 2.0;
    let g = rectf(r.left, top, r.right, top + ih);
    icon_sized(c, Font::IconLarge, glyph, g, p.text2);
    let t = rectf(r.left, g.bottom + gap, r.right, g.bottom + gap + th);
    text(c, Font::BodyStrong, title, t, p.text, Align::Center);
    let b = rectf(r.left, t.bottom, r.right, t.bottom + bh);
    text(c, Font::Body, body, b, p.text2, Align::Center);
}

/// Pulse period of loading placeholders.
const PULSE_MS: u128 = 1200;

/// Placeholder fill opacity `ms` after the start: a triangle wave, 0.06..0.14.
fn pulse_alpha(ms: u128) -> f32 {
    let phase = (ms % PULSE_MS) as f32 / PULSE_MS as f32;
    let tri = 1.0 - (phase * 2.0 - 1.0).abs();
    0.06 + 0.08 * tri
}

/// A page showing a skeleton must keep its frame timer running, because the
/// placeholder pulses until real content replaces it.
pub const SKELETON_ANIMATES: bool = true;

/// Loading placeholder block that pulses with `now` (it needs frames until
/// replaced by real content).
pub fn skeleton(c: &Canvas, r: RectF, now: Instant) {
    static START: OnceLock<Instant> = OnceLock::new();
    let start = *START.get_or_init(Instant::now);
    let ms = now.saturating_duration_since(start).as_millis();
    let radius = tokens::RADIUS[0] * c.s;
    round_rect(
        c,
        &r,
        Some((c.p.text, pulse_alpha(ms))),
        None,
        radius,
        false,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn only_visible_rows_plus_overscan() {
        assert_eq!(visible_range(0.0, 400.0, 48.0, 1000), 0..10);
        assert_eq!(visible_range(48.0 * 500.0, 400.0, 48.0, 1000), 499..510);
        assert_eq!(visible_range(0.0, 400.0, 48.0, 3), 0..3);
    }
    #[test]
    fn scroll_clamps_and_hides_thumb_when_content_fits() {
        let mut s = Scroll::new(300.0, 400.0);
        assert!(s.thumb().is_none());
        s.content = 2000.0;
        s.offset = 5000.0;
        s.clamp();
        assert_eq!(s.offset, 1600.0);
    }
    #[test]
    fn long_titles_never_reach_the_trailing_column() {
        let (_, title, _, trail) = row_layout(rectf(0., 0., 360., 48.), 1.0, 90.0);
        assert!(title.right <= trail.left - 8.0);
    }
    #[test]
    fn nav_rail_marks_the_selected_item() {
        let r = nav_rail(
            rectf(0., 0., 72., 600.),
            1.0,
            &["Home", "Timeline", "Devices", "Settings"],
            2,
        );
        assert_eq!(r.len(), 4);
        assert!(r[0].0.top < r[1].0.top);
    }

    #[test]
    fn wheel_eases_one_notch_down_and_stays_in_range() {
        let t0 = Instant::now();
        let mut s = Scroll::new(2000.0, 400.0);
        s.wheel(-120, t0);
        assert!(s.tick(t0 + Duration::from_millis(10)));
        assert!(s.offset > 0.0 && s.offset < 48.0);
        assert!(!s.tick(t0 + Duration::from_millis(500)));
        assert_eq!(s.offset, 48.0);
        s.wheel(120 * 5, t0 + Duration::from_millis(600));
        s.tick(t0 + Duration::from_millis(1200));
        assert_eq!(s.offset, 0.0);
    }

    #[test]
    fn thumb_tracks_the_offset() {
        let mut s = Scroll::new(2000.0, 400.0);
        assert_eq!(s.thumb(), Some((0.0, 80.0)));
        s.offset = 1600.0;
        let (top, h) = s.thumb().unwrap();
        assert_eq!(top + h, 400.0);
    }

    #[test]
    fn jump_and_ease_to_clamp_and_cancel_the_ease() {
        let t0 = Instant::now();
        let mut s = Scroll::new(2000.0, 400.0);
        s.ease_to(5000.0, t0);
        assert_eq!(s.target(), 1600.0);
        assert!(!s.tick(t0 + Duration::from_millis(500)));
        assert_eq!(s.offset, 1600.0);
        s.ease_to(0.0, t0);
        s.jump(300.0);
        assert!(!s.animating(t0), "a jump ends the ease");
        assert_eq!((s.offset, s.target()), (300.0, 300.0));
        s.jump(-10.0);
        assert_eq!(s.offset, 0.0);
    }

    #[test]
    fn pulse_stays_in_its_band() {
        for ms in (0..2400).step_by(50) {
            let a = pulse_alpha(ms);
            assert!((0.06..=0.14 + 1e-6).contains(&a), "{ms}: {a}");
        }
    }

    #[test]
    fn trailing_column_shrinks_instead_of_overlapping_the_text() {
        let gap = SPACE[2];
        for (w, h, trailing) in [
            (360.0, 48.0, 360.0),
            (40.0, 48.0, 90.0),
            (120.0, 48.0, 500.0),
        ] {
            let (icon, title, meta, trail) = row_layout(rectf(0., 0., w, h), 1.0, trailing);
            for t in [title, meta] {
                assert!(t.right >= t.left, "width >= 0 in {w}px row");
                assert!(
                    t.right <= trail.left - gap,
                    "{w}px row, trailing {trailing}"
                );
            }
            assert!(trail.right >= trail.left);
            assert!(trail.left >= icon.right, "trailing stays off the icon");
        }
    }

    #[test]
    fn scroll_animation_is_read_only_and_idle_at_the_limits() {
        let t0 = Instant::now();
        let mut s = Scroll::new(2000.0, 400.0);
        s.wheel(120, t0); // up at the top: nothing to ease
        assert!(!s.animating(t0));
        s.wheel(-120, t0);
        assert!(s.animating(t0 + Duration::from_millis(10)));
        assert_eq!(s.offset, 0.0, "animating() does not advance the ease");
        assert!(!s.animating(t0 + Duration::from_millis(500)));
        let mut end = Scroll::new(2000.0, 400.0);
        end.offset = 1600.0;
        end.wheel(-120, t0); // down at the bottom
        assert!(!end.animating(t0));
    }

    #[test]
    #[allow(unsafe_code)]
    fn long_mixed_titles_stay_inside_their_rect() {
        let hwnd = crate::d2d::test_window();
        let mut surface =
            crate::d2d::Surface::new(hwnd, crate::d2d::Backdrop::Opaque).expect("surface");
        let title = "photo \u{1F4F7} \u{05E9}\u{05DC}\u{05D5}\u{05DD} ".repeat(16);
        assert!(title.chars().count() >= 200);
        let mut widths = (0.0, f32::MAX);
        surface
            .draw(|dc| {
                let c = Canvas {
                    dc,
                    s: 1.0,
                    p: fluent::palette(),
                };
                let r = rectf(0.0, 0.0, 200.0, 48.0);
                let tw = text_width(&c, Font::Caption, "12:04").ceil() + 1.0;
                let (_, title_r, _, _) = row_layout(r, 1.0, tw);
                row(&c, r, '\u{E8A5}', &title, "meta", "12:04");
                widths = (
                    title_r.right - title_r.left,
                    fluent::laid_out_width(&c, Font::Body, &title, title_r, Align::Left),
                );
            })
            .expect("draw");
        assert!(widths.1 <= widths.0 + 0.5, "{} > {}", widths.1, widths.0);
        drop(surface);
        // SAFETY: our own test window, made above on this thread.
        unsafe {
            let _ = windows::Win32::UI::WindowsAndMessaging::DestroyWindow(hwnd);
        }
    }

    #[test]
    #[allow(unsafe_code)]
    fn widgets_draw_in_a_hidden_window() {
        use crate::{
            d2d::{Backdrop, Surface},
            fluent::palette,
        };
        let hwnd = crate::d2d::test_window();
        let mut surface = Surface::new(hwnd, Backdrop::Opaque).expect("surface");
        surface
            .draw(|dc| {
                let c = Canvas {
                    dc,
                    s: 1.0,
                    p: palette(),
                };
                let r = rectf(0.0, 0.0, 200.0, 48.0);
                card(&c, r);
                badge(&c, r, "Online", c.p.success);
                info_bar(&c, r, InfoKind::Warning, "Reconnecting\u{2026}");
                empty_state(&c, r, '\u{E8EA}', "Nothing here", "Send a file to start");
                row(
                    &c,
                    r,
                    '\u{E8A5}',
                    "a very long file name.txt",
                    "from phone",
                    "12:04",
                );
                skeleton(&c, r, Instant::now());
                for (rect, i) in nav_rail(r, 1.0, &["Home"], 0) {
                    draw_nav_item(&c, rect, '\u{E80F}', "Home", i == 0);
                }
            })
            .expect("draw");
        drop(surface);
        // SAFETY: our own test window, made above on this thread.
        unsafe {
            let _ = windows::Win32::UI::WindowsAndMessaging::DestroyWindow(hwnd);
        }
    }
}
