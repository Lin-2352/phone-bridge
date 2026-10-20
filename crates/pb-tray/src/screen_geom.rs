//! Geometry and decisions for the phone-screen window, kept pure so they are
//! tested on every platform.

use pb_video::Codec;

/// Where the video sits inside the client area (letterboxed): x, y, w, h.
pub fn video_rect(client: (u32, u32), video: (u32, u32)) -> (i32, i32, u32, u32) {
    if client.0 == 0 || client.1 == 0 || video.0 == 0 || video.1 == 0 {
        return (0, 0, 0, 0);
    }
    let scale =
        (f64::from(client.0) / f64::from(video.0)).min(f64::from(client.1) / f64::from(video.1));
    let w = (f64::from(video.0) * scale).round() as u32;
    let h = (f64::from(video.1) * scale).round() as u32;
    (
        ((client.0 - w) / 2) as i32,
        ((client.1 - h) / 2) as i32,
        w,
        h,
    )
}

/// A click in client coordinates → the phone's 0..=65535 coordinates, or
/// `None` in the bars outside the video.
pub fn map_point(p: (i32, i32), client: (u32, u32), video: (u32, u32)) -> Option<(u16, u16)> {
    let (x, y, w, h) = video_rect(client, video);
    if w < 2 || h < 2 {
        return None;
    }
    let (dx, dy) = (p.0 - x, p.1 - y);
    if dx < 0 || dy < 0 || dx >= w as i32 || dy >= h as i32 {
        return None;
    }
    let scale = |d: i32, n: u32| (f64::from(d) / f64::from(n - 1) * 65535.0).round() as u16;
    Some((scale(dx, w), scale(dy, h)))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edge {
    Left,
    Right,
    Top,
    Bottom,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

/// Resizing (WM_SIZING): keep the phone's aspect, led by the dragged edge.
pub fn fit_window(proposed: (u32, u32), video: (u32, u32), edge: Edge) -> (u32, u32) {
    if video.0 == 0 || video.1 == 0 {
        return proposed;
    }
    let (vw, vh) = (f64::from(video.0), f64::from(video.1));
    match edge {
        Edge::Top | Edge::Bottom => ((f64::from(proposed.1) * vw / vh).round() as u32, proposed.1),
        _ => (proposed.0, (f64::from(proposed.0) * vh / vw).round() as u32),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Next {
    Retry(Codec),
    GiveUp,
}

/// Decoding failed with `current`: the PC's next codec, or give up.
pub fn on_decode_error(current: Codec, pc: &[Codec]) -> Next {
    pb_video::codec::next_after(current, pc).map_or(Next::GiveUp, Next::Retry)
}

/// Whether a window state (active, dim) must be sent to the phone: only
/// when it differs from the last one sent (every resize would otherwise
/// make the phone restart its key frames).
pub fn state_changed(prev: Option<(bool, bool)>, next: (bool, bool)) -> bool {
    prev != Some(next)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn video_rect_letterboxes_both_ways() {
        assert_eq!(video_rect((1000, 1000), (864, 1920)), (275, 0, 450, 1000));
        assert_eq!(video_rect((1000, 1000), (1920, 864)), (0, 275, 1000, 450));
        assert_eq!(video_rect((0, 0), (864, 1920)), (0, 0, 0, 0));
    }

    #[test]
    fn clicks_outside_the_video_send_nothing() {
        let (c, v) = ((1000, 1000), (864, 1920));
        assert_eq!(map_point((100, 500), c, v), None, "left bar");
        assert_eq!(map_point((900, 500), c, v), None, "right bar");
        assert_eq!(map_point((500, -1), c, v), None);
        assert_eq!(map_point((500, 1000), c, v), None);
    }

    #[test]
    fn corners_map_to_the_full_range() {
        let (c, v) = ((1000, 1000), (864, 1920));
        assert_eq!(map_point((275, 0), c, v), Some((0, 0)));
        assert_eq!(map_point((275 + 449, 999), c, v), Some((65535, 65535)));
    }

    #[test]
    fn touch_mapping_follows_rotation() {
        // Landscape video in a wide window.
        let (c, v) = ((1000, 600), (1920, 864));
        let (x, y) = map_point((500, 300), c, v).unwrap();
        assert!((32767 - 100..=32767 + 100).contains(&i32::from(x)), "{x}");
        assert!((32767 - 200..=32767 + 200).contains(&i32::from(y)), "{y}");
        let (x, _) = map_point((10, 300), c, v).unwrap();
        assert!(x < 1000, "{x}");
    }

    #[test]
    fn fit_window_keeps_aspect_from_the_dragged_edge() {
        let v = (864, 1920);
        assert_eq!(fit_window((500, 900), v, Edge::Right), (500, 1111));
        assert_eq!(fit_window((500, 900), v, Edge::Bottom), (405, 900));
        assert_eq!(fit_window((500, 900), v, Edge::BottomRight), (500, 1111));
        assert_eq!(fit_window((500, 900), (0, 0), Edge::Right), (500, 900));
    }

    #[test]
    fn state_sent_only_on_change() {
        assert!(
            state_changed(None, (true, false)),
            "the first state is sent"
        );
        assert!(
            !state_changed(Some((true, false)), (true, false)),
            "resizes send nothing"
        );
        assert!(
            state_changed(Some((true, false)), (false, false)),
            "minimised"
        );
        assert!(
            state_changed(Some((true, false)), (true, true)),
            "dim toggled"
        );
    }

    #[test]
    fn decoder_failure_asks_next_codec() {
        let pc = [Codec::H265Main10, Codec::H265, Codec::H264];
        assert_eq!(on_decode_error(Codec::H265, &pc), Next::Retry(Codec::H264));
        assert_eq!(on_decode_error(Codec::H264, &pc), Next::GiveUp);
    }
}
