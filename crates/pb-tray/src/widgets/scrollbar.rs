//! The scrollbar of the main window, as pure maths: thumb size and
//! position, dragging, track paging, and the hit area (wider than what is
//! painted, so a thin bar is still easy to grab). Lengths are pixels unless
//! a function says otherwise; `s` is the DPI scale. The window owns the
//! state (hover, drag) and paints with [`thumb_color`].

/// Painted width at rest and while hovered or dragged, in logical px.
pub const REST_W: f32 = 6.0;
pub const HOVER_W: f32 = 12.0;
/// Hit area width in logical px (never narrower than this, whatever is
/// painted). Measured from the pane edge.
pub const HIT_W: f32 = 16.0;
/// The thumb never gets shorter than this (logical px).
pub const MIN_THUMB: f32 = 40.0;
/// Gap between the painted bar and the pane edge (logical px).
pub const EDGE: f32 = 2.0;
/// A page click or PgUp/PgDn moves this fraction of the view (keeps context).
pub const PAGE_FRACTION: f32 = 0.9;
/// Holding the track: first repeat after, then every, in milliseconds.
pub const REPEAT_DELAY_MS: u64 = 400;
pub const REPEAT_MS: u64 = 90;

/// One bar: the scroll range (logical px, like `Scroll`) and its track on
/// screen (pixels).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bar {
    pub content: f32,
    /// Visible height in logical px.
    pub view: f32,
    pub offset: f32,
    /// Track top and length in pixels (length is `view` times `s`).
    pub top: f32,
    pub len: f32,
    pub s: f32,
}

/// What a point on the bar is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Part {
    Thumb,
    /// On the track above or below the thumb: a click pages that way.
    Above,
    Below,
}

/// Where the pointer is, for colour.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Look {
    Idle,
    Hover,
    Pressed,
}

fn travel(content: f32, view: f32, track: f32, min_thumb: f32) -> Option<(f32, f32)> {
    if content <= view || view <= 0.0 || track <= 0.0 {
        return None;
    }
    let len = (track * view / content).max(min_thumb).min(track);
    Some((len, track - len))
}

/// Thumb (top, length) inside a track of length `track`, or None when
/// everything fits. `view` and `offset` are in the content's unit; the
/// thumb and `min_thumb` in the track's.
pub fn thumb(
    content: f32,
    view: f32,
    offset: f32,
    track: f32,
    min_thumb: f32,
) -> Option<(f32, f32)> {
    let (len, room) = travel(content, view, track, min_thumb)?;
    let max = content - view;
    Some((offset.clamp(0.0, max) / max * room, len))
}

/// The content offset whose thumb top is `thumb_top`.
pub fn offset_for_thumb_top(
    content: f32,
    view: f32,
    track: f32,
    min_thumb: f32,
    thumb_top: f32,
) -> f32 {
    match travel(content, view, track, min_thumb) {
        Some((_, room)) if room > 0.0 => (thumb_top / room).clamp(0.0, 1.0) * (content - view),
        _ => 0.0,
    }
}

/// Offset while the thumb is dragged `delta` px from where it was grabbed
/// (`start_offset` then).
pub fn drag_offset(
    content: f32,
    view: f32,
    track: f32,
    min_thumb: f32,
    start_offset: f32,
    delta: f32,
) -> f32 {
    match travel(content, view, track, min_thumb) {
        Some((_, room)) if room > 0.0 => {
            let max = content - view;
            (start_offset.clamp(0.0, max) + delta / room * max).clamp(0.0, max)
        }
        _ => 0.0,
    }
}

/// Offset moved by `pages` pages (negative = up), clamped.
pub fn page_by(content: f32, view: f32, offset: f32, pages: f32) -> f32 {
    let max = (content - view).max(0.0);
    (offset + pages * view * PAGE_FRACTION).clamp(0.0, max)
}

/// The offset a track click at `y` (track coordinates) pages to, or None
/// when it is on the thumb or there is nothing to scroll.
pub fn page_target(
    content: f32,
    view: f32,
    offset: f32,
    track: f32,
    min_thumb: f32,
    y: f32,
) -> Option<f32> {
    let (top, len) = thumb(content, view, offset, track, min_thumb)?;
    if y < top {
        Some(page_by(content, view, offset, -1.0))
    } else if y >= top + len {
        Some(page_by(content, view, offset, 1.0))
    } else {
        None
    }
}

/// Painted width for hover fade `t` (0 = rest, 1 = fully hovered), scaled.
pub fn painted_width(t: f32, s: f32) -> f32 {
    (REST_W + (HOVER_W - REST_W) * t.clamp(0.0, 1.0)) * s
}

/// Thumb (alpha, rgb) for the pointer state: muted at rest (still visible),
/// stronger on hover, full text colour while pressed.
pub fn thumb_color(look: Look, text: u32, muted: u32) -> (f32, u32) {
    match look {
        Look::Idle => (0.7, muted),
        Look::Hover => (0.9, muted),
        Look::Pressed => (1.0, text),
    }
}

impl Bar {
    /// Thumb (top, length) in pixels from the track top, None when it all fits.
    pub fn thumb(&self) -> Option<(f32, f32)> {
        thumb(
            self.content * self.s,
            self.len,
            self.offset * self.s,
            self.len,
            MIN_THUMB * self.s,
        )
    }

    /// The painted track (left, right) at hover fade `t`, for a pane whose
    /// right edge is `pane_right`.
    pub fn painted_x(&self, pane_right: f32, t: f32) -> (f32, f32) {
        let right = pane_right - EDGE * self.s;
        (right - painted_width(t, self.s), right)
    }

    /// The hit area (left, right): at least `HIT_W` wide.
    pub fn hit_x(&self, pane_right: f32) -> (f32, f32) {
        (pane_right - HIT_W * self.s, pane_right)
    }

    /// What is under (x, y), if the bar is there.
    pub fn part_at(&self, pane_right: f32, x: f32, y: f32) -> Option<Part> {
        let (hl, hr) = self.hit_x(pane_right);
        if x < hl || x >= hr || y < self.top || y >= self.top + self.len {
            return None;
        }
        let (top, len) = self.thumb()?;
        let ty = y - self.top;
        Some(if ty < top {
            Part::Above
        } else if ty >= top + len {
            Part::Below
        } else {
            Part::Thumb
        })
    }

    /// Offset (logical px) for dragging the thumb, grabbed at `grab_y` with
    /// the offset at `start` then, to pointer `y`.
    pub fn drag_to(&self, start: f32, grab_y: f32, y: f32) -> f32 {
        drag_offset(
            self.content * self.s,
            self.len,
            self.len,
            MIN_THUMB * self.s,
            start * self.s,
            y - grab_y,
        ) / self.s
    }

    /// The offset (logical px) a press on the track at pixel `y` pages to.
    pub fn page_at(&self, y: f32) -> Option<f32> {
        page_target(
            self.content * self.s,
            self.len,
            self.offset * self.s,
            self.len,
            MIN_THUMB * self.s,
            y - self.top,
        )
        .map(|o| o / self.s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bar(content: f32, view: f32, offset: f32, s: f32) -> Bar {
        Bar {
            content,
            view,
            offset,
            top: 20.0,
            len: view * s,
            s,
        }
    }

    #[test]
    fn thumb_is_proportional_and_none_when_all_fits() {
        assert_eq!(thumb(300.0, 400.0, 0.0, 400.0, 40.0), None);
        assert_eq!(thumb(400.0, 400.0, 0.0, 400.0, 40.0), None);
        assert_eq!(thumb(2000.0, 400.0, 0.0, 400.0, 40.0), Some((0.0, 80.0)));
        assert_eq!(
            thumb(2000.0, 400.0, 1600.0, 400.0, 40.0),
            Some((320.0, 80.0))
        );
        assert_eq!(
            thumb(2000.0, 400.0, 800.0, 400.0, 40.0),
            Some((160.0, 80.0))
        );
    }

    #[test]
    fn thumb_has_a_minimum_length_and_stays_inside_the_track() {
        let (top, len) = thumb(100_000.0, 400.0, 0.0, 400.0, 40.0).unwrap();
        assert_eq!((top, len), (0.0, 40.0));
        let (top, len) = thumb(100_000.0, 400.0, 99_600.0, 400.0, 40.0).unwrap();
        assert_eq!(top + len, 400.0);
        let (top, len) = thumb(2000.0, 400.0, 9_999.0, 400.0, 40.0).unwrap();
        assert_eq!(top + len, 400.0);
        let (top, _) = thumb(2000.0, 400.0, -50.0, 400.0, 40.0).unwrap();
        assert_eq!(top, 0.0);
        // A track shorter than the minimum: the thumb fills it.
        let (_, len) = thumb(2000.0, 30.0, 0.0, 30.0, 40.0).unwrap();
        assert_eq!(len, 30.0);
    }

    #[test]
    fn thumb_position_and_offset_round_trip() {
        for off in [0.0, 1.0, 333.0, 1000.0, 1599.0, 1600.0] {
            let (top, _) = thumb(2000.0, 400.0, off, 400.0, 40.0).unwrap();
            let back = offset_for_thumb_top(2000.0, 400.0, 400.0, 40.0, top);
            assert!((back - off).abs() < 0.01, "{off} -> {back}");
        }
        assert_eq!(
            offset_for_thumb_top(100_000.0, 400.0, 400.0, 40.0, 0.0),
            0.0
        );
        assert_eq!(
            offset_for_thumb_top(100_000.0, 400.0, 400.0, 40.0, 360.0),
            99_600.0
        );
        assert_eq!(
            offset_for_thumb_top(100_000.0, 400.0, 400.0, 40.0, 9_000.0),
            99_600.0
        );
    }

    #[test]
    fn dragging_maps_pointer_delta_to_offset() {
        // Track 400, thumb 80: 320 px of travel covers 1600 of content.
        assert_eq!(drag_offset(2000.0, 400.0, 400.0, 40.0, 0.0, 32.0), 160.0);
        assert_eq!(drag_offset(2000.0, 400.0, 400.0, 40.0, 800.0, -32.0), 640.0);
        // Past either end (the pointer left the window) it clamps.
        assert_eq!(drag_offset(2000.0, 400.0, 400.0, 40.0, 0.0, -500.0), 0.0);
        assert_eq!(
            drag_offset(2000.0, 400.0, 400.0, 40.0, 0.0, 5_000.0),
            1600.0
        );
        assert_eq!(drag_offset(300.0, 400.0, 400.0, 40.0, 0.0, 50.0), 0.0);
    }

    #[test]
    fn track_clicks_page_up_or_down_and_not_on_the_thumb() {
        assert_eq!(
            page_target(2000.0, 400.0, 0.0, 400.0, 40.0, 300.0),
            Some(360.0)
        );
        assert_eq!(page_target(2000.0, 400.0, 0.0, 400.0, 40.0, 40.0), None);
        assert_eq!(
            page_target(2000.0, 400.0, 1600.0, 400.0, 40.0, 10.0),
            Some(1240.0)
        );
        assert_eq!(
            page_target(2000.0, 400.0, 1500.0, 400.0, 40.0, 399.0),
            Some(1600.0)
        );
        assert_eq!(
            page_target(2000.0, 400.0, 100.0, 400.0, 40.0, 399.0),
            Some(460.0)
        );
        assert_eq!(page_target(300.0, 400.0, 0.0, 400.0, 40.0, 10.0), None);
    }

    #[test]
    fn paging_keys_move_by_view_fraction_and_clamp() {
        assert_eq!(page_by(2000.0, 400.0, 0.0, 1.0), 360.0);
        assert_eq!(page_by(2000.0, 400.0, 100.0, -1.0), 0.0);
        assert_eq!(page_by(2000.0, 400.0, 1500.0, 1.0), 1600.0);
        assert_eq!(page_by(300.0, 400.0, 0.0, 1.0), 0.0);
    }

    #[test]
    fn painted_width_widens_with_hover_and_scales_with_dpi() {
        assert_eq!(painted_width(0.0, 1.0), 6.0);
        assert_eq!(painted_width(1.0, 1.0), 12.0);
        assert_eq!(painted_width(0.5, 1.0), 9.0);
        assert_eq!(painted_width(0.0, 1.5), 9.0);
        assert_eq!(painted_width(1.0, 2.0), 24.0);
        assert_eq!(painted_width(3.0, 1.0), 12.0);
        assert_eq!(painted_width(-1.0, 1.0), 6.0);
    }

    #[test]
    fn hit_area_is_wider_than_the_painted_bar_at_every_dpi() {
        for s in [1.0, 1.25, 1.5, 2.0, 3.0] {
            let b = bar(2000.0, 400.0, 0.0, s);
            let (hl, hr) = b.hit_x(1000.0);
            let (pl, pr) = b.painted_x(1000.0, 0.0);
            assert_eq!(hr, 1000.0);
            assert!(hr - hl >= 16.0 * s - 0.01, "hit {} at {s}", hr - hl);
            assert!(hl < pl, "hit area reaches left of the resting bar");
            assert!(pr <= hr && pr - pl >= 6.0 * s - 0.01);
            let (hovered_l, _) = b.painted_x(1000.0, 1.0);
            assert!(hovered_l >= hl, "hovered bar fits the hit area");
        }
    }

    #[test]
    fn part_at_finds_thumb_and_track_by_the_wide_hit_area() {
        let b = bar(2000.0, 400.0, 0.0, 1.0);
        // Track top is 20; thumb 20..100. x 990 is outside the painted 6 px
        // bar (992..998) but inside the 16 px hit area.
        assert_eq!(b.part_at(1000.0, 990.0, 50.0), Some(Part::Thumb));
        assert_eq!(b.part_at(1000.0, 990.0, 300.0), Some(Part::Below));
        let down = bar(2000.0, 400.0, 1600.0, 1.0);
        assert_eq!(down.part_at(1000.0, 999.0, 30.0), Some(Part::Above));
        assert_eq!(b.part_at(1000.0, 900.0, 50.0), None);
        assert_eq!(b.part_at(1000.0, 995.0, 5.0), None);
        assert_eq!(b.part_at(1000.0, 995.0, 421.0), None);
        assert_eq!(
            bar(300.0, 400.0, 0.0, 1.0).part_at(1000.0, 995.0, 50.0),
            None
        );
    }

    #[test]
    fn bar_drag_and_page_use_pixels_and_dpi() {
        let b = bar(2000.0, 400.0, 0.0, 2.0);
        // Track 800 px, thumb 160 px: 640 px of travel covers 1600 logical.
        assert_eq!(b.thumb().map(|t| t.1), Some(160.0));
        let off = b.drag_to(0.0, 30.0, 30.0 + 64.0);
        assert!((off - 160.0).abs() < 0.01, "{off}");
        assert!(b.page_at(b.top + 700.0).unwrap() > 0.0);
        assert_eq!(b.page_at(b.top + 10.0), None);
    }

    #[test]
    fn contrast_of_every_state_is_readable_in_both_themes() {
        use crate::tokens;
        fn lum(c: u32) -> f32 {
            let f = |v: u32| {
                let x = v as f32 / 255.0;
                if x <= 0.03928 {
                    x / 12.92
                } else {
                    ((x + 0.055) / 1.055).powf(2.4)
                }
            };
            0.2126 * f(c >> 16 & 255) + 0.7152 * f(c >> 8 & 255) + 0.0722 * f(c & 255)
        }
        fn mix(fg: u32, bg: u32, a: f32) -> u32 {
            let ch = |sh: u32| {
                let (f, b) = ((fg >> sh & 255) as f32, (bg >> sh & 255) as f32);
                (f * a + b * (1.0 - a)).round() as u32
            };
            ch(16) << 16 | ch(8) << 8 | ch(0)
        }
        fn ratio(a: u32, b: u32) -> f32 {
            let (x, y) = (lum(a), lum(b));
            (x.max(y) + 0.05) / (x.min(y) + 0.05)
        }
        for t in [&tokens::LIGHT, &tokens::DARK] {
            for look in [Look::Idle, Look::Hover, Look::Pressed] {
                let (alpha, rgb) = thumb_color(look, t.text, t.text_muted);
                let c = ratio(mix(rgb, t.surface_variant, alpha), t.surface_variant);
                let need = if look == Look::Idle { 3.0 } else { 4.0 };
                assert!(c >= need, "{look:?} contrast {c}");
            }
        }
    }
}
