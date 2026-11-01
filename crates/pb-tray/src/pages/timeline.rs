//! Timeline: what moved between this PC and its phones, newest first and
//! grouped by local day. File rows offer Open and Show in folder, disabled
//! when the saved file is gone. `build` turns the store's entries into a
//! [`TimelineModel`] once per model change (labels, rows, offsets); layout
//! and draw then touch only the items in view.

use std::ops::Range;

use crate::{
    fluent::{self, rectf, round_rect, text, Align, Canvas, Font, RectF},
    pages::{Action, Hit},
    timeline::{self, Dir, Entry, Kind},
    tokens::{RADIUS, SPACE},
    widgets,
};

/// Shown on a row whose file is gone, and toasted when its button is used.
pub const MISSING: &str = "File moved or deleted";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileState {
    Present,
    Missing,
}

/// Whether the saved file is still there.
pub fn file_state(path: &str) -> FileState {
    if std::path::Path::new(path).exists() {
        FileState::Present
    } else {
        FileState::Missing
    }
}

/// The re-check before Open or Show in folder: the file may have gone
/// since the row was drawn. Err carries the toast text.
pub fn still_there(path: &str) -> Result<(), &'static str> {
    match file_state(path) {
        FileState::Present => Ok(()),
        FileState::Missing => Err(MISSING),
    }
}

const DAY_SECS: i64 = 86_400;

/// The local day number (days since 1970-01-01) of Unix time `at`.
fn local_day(at: u64, tz_offset_s: i64) -> i64 {
    (at as i64 + tz_offset_s).div_euclid(DAY_SECS)
}

/// (year, month 1-12, day 1-31) of a day number (proleptic Gregorian).
fn civil(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

/// "Today", "Yesterday", else "Mon 27 Oct".
fn day_label(day: i64, today: i64) -> String {
    const WEEK: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
    const MONTH: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    match today - day {
        0 => "Today".into(),
        1 => "Yesterday".into(),
        _ => {
            let (_, m, d) = civil(day);
            // 1970-01-01 was a Thursday.
            let wd = (day + 4).rem_euclid(7) as usize;
            format!("{} {d} {}", WEEK[wd], MONTH[m as usize - 1])
        }
    }
}

/// Local "14:05" of Unix time `at`.
fn clock(at: u64, tz_offset_s: i64) -> String {
    let secs = (at as i64 + tz_offset_s).rem_euclid(DAY_SECS);
    format!("{:02}:{:02}", secs / 3600, secs / 60 % 60)
}

/// Entries (newest first) as runs of one local day: ("Today", indices),
/// ("Yesterday", ...), ("Mon 27 Oct", ...). `tz_offset_s` is local minus
/// UTC.
pub fn group_by_day(entries: &[Entry], now: u64, tz_offset_s: i64) -> Vec<(String, Vec<usize>)> {
    let today = local_day(now, tz_offset_s);
    let mut out: Vec<(i64, String, Vec<usize>)> = Vec::new();
    for (k, e) in entries.iter().enumerate() {
        let day = local_day(e.at, tz_offset_s);
        match out.last_mut() {
            Some((d, _, v)) if *d == day => v.push(k),
            _ => out.push((day, day_label(day, today), vec![k])),
        }
    }
    out.into_iter().map(|(_, l, v)| (l, v)).collect()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileRow {
    pub path: String,
    pub state: FileState,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RowModel {
    pub kind: Kind,
    pub title: String,
    pub meta: String,
    /// A file with a saved path; the buttons act on it.
    pub file: Option<FileRow>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Item {
    Day(String),
    Row(RowModel),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TimelineModel {
    pub history_on: bool,
    /// Day headers and rows, top to bottom.
    pub items: Vec<Item>,
    /// Top of each item in logical px, from the top of the list.
    pub tops: Vec<u32>,
    /// The list's height in logical px.
    pub height: u32,
    /// Each day's card behind its rows, as (top, bottom) like `tops`.
    pub cards: Vec<(u32, u32)>,
}

/// Sizes in logical px.
const PAD: f32 = 24.0;
const TITLE_H: f32 = 40.0;
const HEAD_H: u32 = 32;
const ROW_H: u32 = 56;
const OPEN_W: f32 = 72.0;
const SHOW_W: f32 = 120.0;
const BUTTON_H: f32 = 32.0;

fn item_h(i: &Item) -> u32 {
    match i {
        Item::Day(_) => HEAD_H,
        Item::Row(_) => ROW_H,
    }
}

fn row(e: &Entry, tz_offset_s: i64, state: &impl Fn(&str) -> FileState) -> RowModel {
    let (arrow, way) = match e.dir {
        Dir::ToPhone => ('\u{2191}', "To"),
        Dir::ToPc => ('\u{2193}', "From"),
    };
    let mut meta = format!(
        "{arrow} {way} {} \u{b7} {}",
        e.peer,
        clock(e.at, tz_offset_s)
    );
    let file = match (e.kind, &e.path) {
        (Kind::File, Some(p)) => Some(FileRow {
            path: p.clone(),
            state: state(p),
        }),
        _ => None,
    };
    if file.as_ref().is_some_and(|f| f.state == FileState::Missing) {
        // First, so a narrow row still says it.
        meta = format!("{MISSING} \u{b7} {meta}");
    }
    RowModel {
        kind: e.kind,
        title: timeline::summary(e),
        meta,
        file,
    }
}

/// `state` tells whether a saved file is still there (`file_state`).
pub fn build(
    entries: &[Entry],
    history_on: bool,
    now: u64,
    tz_offset_s: i64,
    state: impl Fn(&str) -> FileState,
) -> TimelineModel {
    let mut m = TimelineModel {
        history_on,
        ..Default::default()
    };
    if !history_on {
        return m;
    }
    let mut y = 0;
    for (label, idx) in group_by_day(entries, now, tz_offset_s) {
        m.tops.push(y);
        y += HEAD_H;
        m.items.push(Item::Day(label));
        let top = y;
        for k in idx {
            m.tops.push(y);
            y += ROW_H;
            m.items
                .push(Item::Row(row(&entries[k], tz_offset_s, &state)));
        }
        m.cards.push((top, y));
    }
    m.height = y;
    m
}

/// Top of the list in pixels.
fn list_top(r: RectF, s: f32) -> f32 {
    r.top + (PAD + TITLE_H) * s
}

/// Items to lay out and draw: those overlapping `view` (the visible pane,
/// in pixels) plus one on each side. `r` is the scrolled content rect.
pub fn visible(r: RectF, view: RectF, s: f32, m: &TimelineModel) -> Range<usize> {
    let top = list_top(r, s);
    let (y0, y1) = ((view.top - top) / s, (view.bottom - top) / s);
    // `tops` ascend, so both ends are binary searches: the item holding y0,
    // through the last one starting above y1.
    let n = m.items.len();
    let first = m
        .tops
        .partition_point(|&t| t as f32 <= y0)
        .saturating_sub(1);
    let end = m.tops.partition_point(|&t| (t as f32) < y1);
    let start = first.saturating_sub(1).min(n);
    start..(end + 1).min(n).max(start)
}

/// Item `k`'s rect in pixels.
fn item_rect(r: RectF, s: f32, m: &TimelineModel, k: usize) -> RectF {
    let top = list_top(r, s) + m.tops[k] as f32 * s;
    let h = item_h(&m.items[k]) as f32 * s;
    let left = r.left + PAD * s;
    rectf(left, top, (r.right - PAD * s).max(left), top + h)
}

/// Open and Show in folder at the right of a row.
fn buttons(row: RectF, s: f32) -> (RectF, RectF) {
    let pad = SPACE[2] * s;
    let cy = (row.top + row.bottom) / 2.0;
    let (t, b) = (cy - BUTTON_H * s / 2.0, cy + BUTTON_H * s / 2.0);
    let show_r = (row.right - pad).max(row.left);
    let show_l = (show_r - SHOW_W * s).max(row.left);
    let open_r = (show_l - SPACE[1] * s).max(row.left);
    let open_l = (open_r - OPEN_W * s).max(row.left);
    (rectf(open_l, t, open_r, b), rectf(show_l, t, show_r, b))
}

/// Hit ids: item `k`'s Open is 2k + 1, its Show in folder 2k + 2.
fn open_id(k: usize) -> u32 {
    2 * k as u32 + 1
}

pub fn layout(r: RectF, view: RectF, s: f32, m: &TimelineModel) -> Vec<Hit> {
    let mut hits = Vec::new();
    for k in visible(r, view, s, m) {
        let Item::Row(x) = &m.items[k] else {
            continue;
        };
        // A missing file's buttons are drawn disabled and take no clicks.
        if x.file
            .as_ref()
            .is_some_and(|f| f.state == FileState::Present)
        {
            let (open, show) = buttons(item_rect(r, s, m, k), s);
            hits.push(Hit {
                id: open_id(k),
                r: open,
            });
            hits.push(Hit {
                id: open_id(k) + 1,
                r: show,
            });
        }
    }
    hits
}

/// `width` in pixels; the height in logical px (0 for the empty state).
pub fn content_height(width: f32, s: f32, m: &TimelineModel) -> f32 {
    let _ = (width, s);
    if m.items.is_empty() {
        0.0
    } else {
        PAD + TITLE_H + m.height as f32 + PAD
    }
}

/// Title and body of the empty state: history off, or nothing yet.
pub fn empty_text(m: &TimelineModel) -> (&'static str, &'static str) {
    if m.history_on {
        (
            "Nothing has moved yet",
            "Files, clipboard and notifications you share will appear here.",
        )
    } else {
        (
            "History is off",
            "Turn on history in Settings to see what moved between your devices.",
        )
    }
}

fn glyph(k: Kind) -> char {
    match k {
        Kind::File => '\u{E8A5}',
        Kind::ClipText => fluent::glyph::CLIPBOARD,
        Kind::ClipImage => '\u{EB9F}',
        Kind::Notification => '\u{EA8F}',
    }
}

/// A button that takes no clicks: outline only, muted label.
fn disabled_button(c: &Canvas, r: RectF, label: &str) {
    let radius = RADIUS[0] * c.s;
    round_rect(c, &r, None, Some(c.p.stroke), radius, false);
    text(c, Font::Body, label, r, c.p.text2, Align::Center);
}

pub fn draw(c: &mut Canvas, r: RectF, view: RectF, m: &TimelineModel) {
    let (s, pal) = (c.s, c.p);
    if m.items.is_empty() {
        let (title, body) = empty_text(m);
        return widgets::empty_state(c, r, '\u{E81C}', title, body);
    }
    let title = rectf(
        r.left + PAD * s,
        r.top + PAD * s,
        r.right - PAD * s,
        r.top + (PAD + TITLE_H) * s,
    );
    text(c, Font::Title, "Timeline", title, pal.text, Align::Left);
    let range = visible(r, view, s, m);
    if range.is_empty() {
        return;
    }
    // The cards of the days in view (one per day, so at most about 30).
    let top = list_top(r, s);
    let (from, to) = (m.tops[range.start], m.tops[range.end - 1] + ROW_H);
    for &(a, b) in m.cards.iter().filter(|(a, b)| *b > from && *a < to) {
        let left = r.left + PAD * s;
        let card = rectf(
            left,
            top + a as f32 * s,
            (r.right - PAD * s).max(left),
            top + b as f32 * s,
        );
        widgets::card(c, card);
    }
    for k in range {
        let ir = item_rect(r, s, m, k);
        match &m.items[k] {
            Item::Day(label) => {
                let t = rectf(ir.left, ir.top + SPACE[1] * s, ir.right, ir.bottom);
                text(c, Font::BodyStrong, label, t, pal.text, Align::Left);
            }
            Item::Row(x) => {
                let Some(f) = &x.file else {
                    widgets::row(c, ir, glyph(x.kind), &x.title, &x.meta, "");
                    continue;
                };
                let (open, show) = buttons(ir, s);
                let text_r = rectf(ir.left, ir.top, open.left, ir.bottom);
                widgets::row(c, text_r, glyph(x.kind), &x.title, &x.meta, "");
                if f.state == FileState::Present {
                    fluent::button(c, open, "Open", false);
                    fluent::button(c, show, "Show in folder", false);
                } else {
                    disabled_button(c, open, "Open");
                    disabled_button(c, show, "Show in folder");
                }
            }
        }
    }
}

pub fn click(hit: &Hit, m: &TimelineModel) -> Option<Action> {
    let k = (hit.id.checked_sub(1)? / 2) as usize;
    let Some(Item::Row(x)) = m.items.get(k) else {
        return None;
    };
    let f = x.file.as_ref().filter(|f| f.state == FileState::Present)?;
    Some(if hit.id == open_id(k) {
        Action::OpenFile(f.path.clone())
    } else {
        Action::ShowInFolder(f.path.clone())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::timeline::{self, Dir};

    /// Fri 15 Jan 2027 00:00 UTC.
    const DAY: u64 = 1_799_971_200;
    const H: u64 = 3600;

    fn text(at: u64) -> Entry {
        Entry {
            at,
            ..timeline::clip_text_entry("Pixel 8", Dir::ToPc, "hello")
        }
    }

    fn file(at: u64, path: &str) -> Entry {
        Entry {
            at,
            ..timeline::file_entry(
                "Pixel 8",
                Dir::ToPc,
                "report.pdf",
                1200,
                Some(std::path::Path::new(path)),
            )
        }
    }

    fn labels(g: &[(String, Vec<usize>)]) -> Vec<&str> {
        g.iter().map(|(l, _)| l.as_str()).collect()
    }

    #[test]
    fn groups_today_yesterday_and_dates() {
        let now = DAY + 8 * H;
        let entries = [
            text(now - 60),
            text(now - 2 * H),
            text(DAY - 12 * H),
            text(now - 3 * 24 * H),
            text(now - 13 * 24 * H),
        ];
        let g = group_by_day(&entries, now, 0);
        assert_eq!(
            labels(&g),
            ["Today", "Yesterday", "Tue 12 Jan", "Sat 2 Jan"]
        );
        assert_eq!(g[0].1, vec![0, 1]);
        assert_eq!(g[1].1, vec![2]);
        assert_eq!(g[3].1, vec![4]);
        // India (+5:30): 18:00 UTC is 23:30 the same day there, and 19:00
        // UTC is already 00:30 the next day.
        let now = DAY + 19 * H;
        let late = [text(DAY + 18 * H)];
        assert_eq!(labels(&group_by_day(&late, now, 0)), ["Today"]);
        assert_eq!(labels(&group_by_day(&late, now, 19_800)), ["Yesterday"]);
        // West of UTC (-5:00): 03:00 UTC is still the day before.
        let early = [text(DAY + 3 * H)];
        assert_eq!(
            labels(&group_by_day(&early, DAY + 8 * H, -18_000)),
            ["Yesterday"]
        );
        assert!(group_by_day(&[], now, 0).is_empty());
    }

    fn page() -> (RectF, RectF) {
        // Content rect not scrolled, and the visible pane.
        (
            rectf(0.0, 0.0, 800.0, 100_000.0),
            rectf(0.0, 0.0, 800.0, 600.0),
        )
    }

    #[test]
    fn missing_file_rows_disable_open_and_say_so() {
        let dir = std::env::temp_dir().join(format!("pb-tl-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("report.pdf");
        std::fs::write(&path, b"x").unwrap();
        let p = path.to_string_lossy().into_owned();
        let now = DAY + 8 * H;
        let entries = [file(now - 60, &p), text(now - 120)];
        let m = build(&entries, true, now, 0, file_state);
        let (r, view) = page();
        let hits = layout(r, view, 1.0, &m);
        // Open and Show in folder on the file row; nothing on the text row.
        assert_eq!(hits.len(), 2);
        assert_eq!(still_there(&p), Ok(()));
        // Gone between the layout and the click: the page cannot know yet,
        // so the click still asks, and the tray's re-check refuses.
        std::fs::remove_file(&path).unwrap();
        assert_eq!(click(&hits[0], &m), Some(Action::OpenFile(p.clone())));
        assert_eq!(click(&hits[1], &m), Some(Action::ShowInFolder(p.clone())));
        assert_eq!(file_state(&p), FileState::Missing);
        assert_eq!(still_there(&p), Err(MISSING));
        // The next model disables the buttons and says why.
        let m = build(&entries, true, now, 0, file_state);
        assert!(layout(r, view, 1.0, &m).is_empty());
        let row = m
            .items
            .iter()
            .find_map(|i| match i {
                Item::Row(x) if x.kind == Kind::File => Some(x),
                _ => None,
            })
            .unwrap();
        assert!(row.meta.contains(MISSING), "{}", row.meta);
        assert_eq!(row.file.as_ref().unwrap().state, FileState::Missing);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn thousand_entries_lay_out_only_visible_rows() {
        let now = DAY + 8 * H;
        // About 33 a day over 30 days: many day headers between rows.
        let entries: Vec<Entry> = (0..1000)
            .map(|k| file(now - k as u64 * 2600, &format!("C:\\x\\{k}.pdf")))
            .collect();
        let m = build(&entries, true, now, 0, |_| FileState::Present);
        assert!(m.items.len() > 1000, "rows and day headers");
        assert_eq!(m.tops.len(), m.items.len());
        let (full, view) = page();
        let view_h = 600.0;
        // In view: at most view / 32 (the shortest item) + 2 overscan + 1.
        let most = (view_h / 32.0) as usize + 3;
        for (s, offset) in [(1.0, 0.0), (1.0, 30_000.0), (1.5, 20_000.0)] {
            let r = rectf(full.left, -offset * s, full.right, full.bottom * s);
            let v = rectf(view.left, 0.0, view.right, view_h * s);
            let range = visible(r, v, s, &m);
            assert!(!range.is_empty());
            assert!(range.len() <= most, "{range:?} at {offset}");
            let hits = layout(r, v, s, &m);
            assert!(hits.len() <= 2 * range.len());
            assert!(!hits.is_empty());
            // Every hit is in view or on the one overscan row beside it.
            let near = 2.0 * 56.0 * s;
            for h in &hits {
                assert!(
                    h.r.bottom >= v.top - near && h.r.top <= v.bottom + near,
                    "{h:?} outside {v:?} at {offset}"
                );
            }
        }
        let h = content_height(800.0, 1.0, &m);
        assert!(h >= m.height as f32);
        assert!(m.height >= 1000 * 56);
    }

    #[test]
    fn history_off_shows_the_empty_state() {
        let now = DAY + 8 * H;
        let entries = [text(now - 60)];
        let off = build(&entries, false, now, 0, |_| FileState::Present);
        assert!(!off.history_on);
        assert!(off.items.is_empty());
        let (r, view) = page();
        assert!(layout(r, view, 1.0, &off).is_empty());
        assert_eq!(content_height(800.0, 1.0, &off), 0.0);
        assert_eq!(empty_text(&off).0, "History is off");
        let none = build(&[], true, now, 0, |_| FileState::Present);
        assert_eq!(content_height(800.0, 1.0, &none), 0.0);
        assert_eq!(empty_text(&none).0, "Nothing has moved yet");
        let some = build(&entries, true, now, 0, |_| FileState::Present);
        assert!(content_height(800.0, 1.0, &some) > 0.0);
    }

    #[test]
    fn rows_show_direction_peer_and_time() {
        let now = DAY + 8 * H;
        let mut e = text(DAY + 14 * H + 5 * 60 - 12 * H);
        e.dir = Dir::ToPhone;
        let m = build(&[e], true, now, 19_800, |_| FileState::Present);
        let Item::Row(row) = &m.items[1] else {
            panic!("a row after its day")
        };
        assert_eq!(row.title, "Text, 5 characters");
        // 02:05 UTC is 07:35 in India.
        assert_eq!(row.meta, "\u{2191} To Pixel 8 \u{b7} 07:35");
        assert_eq!(row.file, None);
    }

    #[test]
    #[allow(unsafe_code)]
    fn timeline_and_devices_draw_in_a_hidden_window() {
        use crate::{
            d2d::{Backdrop, Surface},
            fluent::{palette, Canvas},
            pages::devices,
        };
        let now = DAY + 8 * H;
        let entries: Vec<Entry> = (0..300)
            .map(|k| file(now - k as u64 * 2600, &format!("C:\\gone\\{k}.pdf")))
            .collect();
        let full = build(&entries, true, now, 19_800, file_state);
        let empty = build(&[], false, now, 0, file_state);
        let hwnd = crate::d2d::test_window();
        let mut surface = Surface::new(hwnd, Backdrop::Opaque).expect("surface");
        surface
            .draw(|dc| {
                let mut c = Canvas {
                    dc,
                    s: 1.0,
                    p: palette(),
                };
                let view = rectf(0.0, 0.0, 600.0, 400.0);
                for offset in [0.0, 5_000.0, full.height as f32] {
                    let r = rectf(0.0, -offset, 600.0, 20_000.0);
                    draw(&mut c, r, view, &full);
                }
                draw(&mut c, view, view, &empty);
                let d = devices::DevicesModel {
                    devices: vec![("Galaxy".into(), "AB12-CD34".into(), true)],
                };
                devices::draw(&mut c, view, &d);
                devices::draw(&mut c, view, &devices::DevicesModel::default());
            })
            .expect("draw");
        drop(surface);
        // SAFETY: our own test window, made above on this thread.
        unsafe {
            let _ = windows::Win32::UI::WindowsAndMessaging::DestroyWindow(hwnd);
        }
    }
}
