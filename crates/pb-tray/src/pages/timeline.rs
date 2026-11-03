//! Timeline: what moved between this PC and its phones, newest first and
//! grouped by local day. File rows offer Open and Show in folder, disabled
//! when the saved file is gone. `build` turns the store's entries into a
//! [`TimelineModel`] once per model change (labels, rows, offsets); layout
//! and draw then touch only the items in view.

use std::{ops::Range, sync::Arc};

use crate::{
    fluent::{self, rectf, round_rect, text, Align, Canvas, Font, RectF},
    pages::{timeline_view::Expansion, Action, Hit},
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
#[cfg(test)]
pub fn group_by_day(entries: &[Entry], now: u64, tz_offset_s: i64) -> Vec<(String, Vec<usize>)> {
    days_of(entries, now, tz_offset_s)
        .into_iter()
        .map(|(_, l, v)| (l, v))
        .collect()
}

/// `group_by_day` with each run's local day number first.
fn days_of(entries: &[Entry], now: u64, tz_offset_s: i64) -> Vec<(i64, String, Vec<usize>)> {
    let today = local_day(now, tz_offset_s);
    let mut out: Vec<(i64, String, Vec<usize>)> = Vec::new();
    for (k, e) in entries.iter().enumerate() {
        let day = local_day(e.at, tz_offset_s);
        match out.last_mut() {
            Some((d, _, v)) if *d == day => v.push(k),
            _ => out.push((day, day_label(day, today), vec![k])),
        }
    }
    out
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RowModel {
    pub kind: Kind,
    pub title: String,
    pub meta: String,
    /// A file with a saved path; the buttons act on it.
    pub file: Option<String>,
    /// Stable across inserts (from the entry's own fields): the row's hit
    /// ids, so focus and a click in flight stay on this entry.
    pub key: u32,
}

/// One local day's entries, newest first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Group {
    /// Days since 1970-01-01, local.
    pub day: i64,
    pub label: String,
    pub rows: Vec<RowModel>,
}

/// A day header as drawn: chevron, label and item count.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DayHead {
    pub day: i64,
    pub label: String,
    pub count: usize,
    pub open: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Item {
    Day(DayHead),
    Row(RowModel),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TimelineModel {
    pub history_on: bool,
    /// Every day with all its rows; `items` shows the open ones.
    pub groups: Arc<Vec<Group>>,
    /// Which days `items` has open.
    pub exp: Expansion,
    /// Day headers and rows, top to bottom.
    pub items: Vec<Item>,
    /// Top of each item in logical px, from the top of the list.
    pub tops: Vec<u32>,
    /// The list's height in logical px.
    pub height: u32,
    /// Each day's card behind its rows, as (top, bottom) like `tops`.
    pub cards: Vec<(u32, u32)>,
    /// The search text this list is filtered by (empty: everything).
    pub query: String,
}

/// Sizes in logical px.
const PAD: f32 = 24.0;
const TITLE_H: f32 = 40.0;
const HEAD_H: u32 = 32;
const ROW_H: u32 = 56;
const OPEN_W: f32 = 72.0;
const SHOW_W: f32 = 120.0;
const BUTTON_H: f32 = 32.0;
/// Top of the list in the page, logical px.
const LIST_TOP: f32 = PAD + TITLE_H;
const ALL_W: f32 = 112.0;
const MORE_W: f32 = 36.0;

/// Hit ids: file buttons use 2 key + 1 and 2 key + 2 (below 2^31 + 3);
/// day headers `DAY_BASE + day`; the collapse / expand all control `ALL_ID`.
const DAY_BASE: u32 = 0xC000_0000;
const ALL_ID: u32 = 0xFFFF_0000;
/// A whole row (below the day ids, above the 2 key + n button ids).
const ROW_BASE: u32 = 0x8000_1000;

fn row_id(key: u32) -> u32 {
    ROW_BASE + (key & 0x1FFF_FFFF)
}

/// A row's "..." button: above the whole-row ids, below the day ids.
const MORE_BASE: u32 = 0xA000_2000;

pub fn more_id(key: u32) -> u32 {
    MORE_BASE + (key & 0x0FFF_FFFF)
}

/// Every hit id that belongs to the row with `key`.
pub fn hit_ids(key: u32) -> [u32; 4] {
    [row_id(key), open_id(key), open_id(key) + 1, more_id(key)]
}

/// What the row's "..." menu and its context menu offer. Open and Show in
/// folder are the row's own buttons, so they are not repeated here, and
/// clipboard text is not stored (by design), so there is no "copy text".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowAct {
    CopyPath,
    Details,
    Delete,
}

impl RowAct {
    pub fn label(self) -> &'static str {
        match self {
            RowAct::CopyPath => "Copy path",
            RowAct::Details => "Details",
            RowAct::Delete => "Delete from history",
        }
    }
}

/// The menu entries for a row, in order.
pub fn row_actions(x: &RowModel) -> Vec<RowAct> {
    let mut v = Vec::new();
    if x.file.is_some() {
        v.push(RowAct::CopyPath);
    }
    v.push(RowAct::Details);
    v.push(RowAct::Delete);
    v
}

/// The text of the Details box for a row.
pub fn details(x: &RowModel) -> String {
    let mut s = format!("{}\n{}", x.title, x.meta);
    if let Some(p) = &x.file {
        s.push_str(&format!("\nSaved to: {p}"));
    }
    s
}

/// Whether `row` matches a search: every word in the query appears in the
/// title or the caption, ignoring case.
pub fn matches(row: &RowModel, query: &str) -> bool {
    let hay = format!("{} {}", row.title, row.meta).to_lowercase();
    query
        .to_lowercase()
        .split_whitespace()
        .all(|w| hay.contains(w))
}

fn day_id(day: i64) -> u32 {
    DAY_BASE + day.clamp(0, 0x3FFE_0000) as u32
}

fn item_h(i: &Item) -> u32 {
    match i {
        Item::Day(_) => HEAD_H,
        Item::Row(_) => ROW_H,
    }
}

/// 30 bits of a hash of the entry's fields, so `HIT_BASE + 2 * key + 2`
/// stays far inside u32.
pub fn entry_key(e: &Entry) -> u32 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (
        e.at,
        e.kind as u8,
        e.dir as u8,
        &e.peer,
        &e.name,
        e.size,
        &e.path,
    )
        .hash(&mut h);
    (h.finish() & 0x3FFF_FFFF) as u32
}

fn row(e: &Entry, tz_offset_s: i64) -> RowModel {
    let (arrow, way) = match e.dir {
        Dir::ToPhone => ('\u{2191}', "To"),
        Dir::ToPc => ('\u{2193}', "From"),
    };
    RowModel {
        kind: e.kind,
        title: timeline::summary(e),
        meta: format!(
            "{arrow} {way} {} \u{b7} {}",
            e.peer,
            clock(e.at, tz_offset_s)
        ),
        file: match (e.kind, &e.path) {
            (Kind::File, Some(p)) => Some(p.clone()),
            _ => None,
        },
        key: entry_key(e),
    }
}

/// Labels, rows and offsets; whether files are still there is asked when
/// rows are laid out (`layout`), not here.
pub fn build(entries: &[Entry], history_on: bool, now: u64, tz_offset_s: i64) -> TimelineModel {
    let m = TimelineModel {
        history_on,
        ..Default::default()
    };
    if !history_on {
        return m;
    }
    let groups = days_of(entries, now, tz_offset_s)
        .into_iter()
        .map(|(day, label, idx)| Group {
            day,
            label,
            rows: idx
                .into_iter()
                .map(|k| row(&entries[k], tz_offset_s))
                .collect(),
        })
        .collect();
    TimelineModel {
        groups: Arc::new(groups),
        ..m
    }
    .with_expansion(&Expansion::default())
}

/// Where the user is looking, so a rebuilt list can put the same thing back
/// at the same place (see [`TimelineModel::anchor`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Anchor {
    row: Option<u32>,
    day: i64,
    /// Page offset minus the item's top, logical px.
    delta: f32,
}

impl TimelineModel {
    /// The days, newest first.
    pub fn days(&self) -> Vec<i64> {
        self.groups.iter().map(|g| g.day).collect()
    }

    /// True when every day is open: the control then says "Collapse all".
    pub fn all_open(&self) -> bool {
        self.exp.all_open(&self.days())
    }

    /// The same entries with the days opened or closed as `exp` says:
    /// items, offsets, cards and height are recomputed.
    /// The same list narrowed to the rows matching `query` and not in
    /// `hidden` (deleted, waiting for the undo window to pass). Days left
    /// with no rows disappear; the days' open state is kept.
    pub fn filtered(&self, query: &str, hidden: &std::collections::HashSet<u32>) -> TimelineModel {
        let q = query.trim();
        let groups: Vec<Group> = self
            .groups
            .iter()
            .filter_map(|g| {
                let rows: Vec<RowModel> = g
                    .rows
                    .iter()
                    .filter(|r| !hidden.contains(&r.key) && (q.is_empty() || matches(r, q)))
                    .cloned()
                    .collect();
                (!rows.is_empty()).then(|| Group {
                    day: g.day,
                    label: g.label.clone(),
                    rows,
                })
            })
            .collect();
        TimelineModel {
            history_on: self.history_on,
            groups: Arc::new(groups),
            query: q.to_owned(),
            ..Default::default()
        }
        .with_expansion(&self.exp)
    }

    pub fn with_expansion(&self, exp: &Expansion) -> TimelineModel {
        let mut m = TimelineModel {
            history_on: self.history_on,
            groups: self.groups.clone(),
            exp: exp.clone(),
            query: self.query.clone(),
            ..Default::default()
        };
        let days = self.days();
        let mut y = 0;
        for (i, g) in self.groups.iter().enumerate() {
            let open = exp.is_open(&days, i);
            m.tops.push(y);
            y += HEAD_H;
            m.items.push(Item::Day(DayHead {
                day: g.day,
                label: g.label.clone(),
                count: g.rows.len(),
                open,
            }));
            if !open {
                continue;
            }
            let top = y;
            for r in &g.rows {
                m.tops.push(y);
                y += ROW_H;
                m.items.push(Item::Row(r.clone()));
            }
            m.cards.push((top, y));
        }
        m.height = y;
        m
    }

    /// The item at page offset `offset` (logical px), as something that
    /// survives a rebuild. None at the very top (nothing to hold: new
    /// entries should show) or when the list is empty.
    pub fn anchor(&self, offset: f32) -> Option<Anchor> {
        if offset <= 0.5 || self.items.is_empty() {
            return None;
        }
        let k = self
            .tops
            .partition_point(|&t| LIST_TOP + t as f32 <= offset)
            .saturating_sub(1);
        let day = self.items[..=k].iter().rev().find_map(|i| match i {
            Item::Day(d) => Some(d.day),
            Item::Row(_) => None,
        })?;
        let row = match &self.items[k] {
            Item::Row(x) => Some(x.key),
            Item::Day(_) => None,
        };
        Some(Anchor {
            row,
            day,
            delta: offset - (LIST_TOP + self.tops[k] as f32),
        })
    }

    /// The page offset that puts the anchored item back at the same place;
    /// its day header when the row is no longer listed; None when both are
    /// gone.
    pub fn restore(&self, a: &Anchor) -> Option<f32> {
        let top = |k: usize| LIST_TOP + self.tops[k] as f32;
        if let Some(key) = a.row {
            if let Some(k) = self
                .items
                .iter()
                .position(|i| matches!(i, Item::Row(x) if x.key == key))
            {
                return Some(top(k) + a.delta);
            }
        }
        let k = self
            .items
            .iter()
            .position(|i| matches!(i, Item::Day(d) if d.day == a.day))?;
        Some(top(k) + if a.row.is_some() { 0.0 } else { a.delta })
    }
}

/// The row's caption line as drawn: says so first when its file is gone,
/// so a narrow row still shows it.
pub fn shown_meta(x: &RowModel, present: bool) -> String {
    if x.file.is_some() && !present {
        format!("{MISSING} \u{b7} {}", x.meta)
    } else {
        x.meta.clone()
    }
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
    let n = m.items.len();
    // `tops` ascend, so both ends are binary searches: the item holding y0
    // (none when the view is past the end), through the last one starting
    // above y1.
    let first = if y0 >= m.height as f32 {
        n
    } else {
        m.tops
            .partition_point(|&t| t as f32 <= y0)
            .saturating_sub(1)
    };
    let end = m.tops.partition_point(|&t| (t as f32) < y1);
    let start = first.saturating_sub(1).min(n);
    start..(end + 1).min(n).max(start)
}

/// The search box between the title and the Collapse / Expand all control.
pub fn search_rect(r: RectF, s: f32) -> RectF {
    let all = all_rect(r, s);
    let left = (r.left + (PAD + 150.0) * s).min(all.left);
    rectf(
        left,
        all.top,
        (all.left - SPACE[2] * s).max(left),
        all.bottom,
    )
}

/// The Collapse all / Expand all control, right of the title.
fn all_rect(r: RectF, s: f32) -> RectF {
    let right = r.right - PAD * s;
    let left = (right - ALL_W * s).max(r.left + PAD * s);
    let mid = r.top + (PAD + TITLE_H / 2.0) * s;
    rectf(
        left,
        mid - BUTTON_H * s / 2.0,
        right,
        mid + BUTTON_H * s / 2.0,
    )
}

/// Item `k`'s rect in pixels.
fn item_rect(r: RectF, s: f32, m: &TimelineModel, k: usize) -> RectF {
    let top = list_top(r, s) + m.tops[k] as f32 * s;
    let h = item_h(&m.items[k]) as f32 * s;
    let left = r.left + PAD * s;
    rectf(left, top, (r.right - PAD * s).max(left), top + h)
}

/// The "..." button at the right end of a row.
fn more_rect(row: RectF, s: f32) -> RectF {
    let pad = SPACE[2] * s;
    let cy = (row.top + row.bottom) / 2.0;
    let r = (row.right - pad).max(row.left);
    rectf(
        (r - MORE_W * s).max(row.left),
        cy - BUTTON_H * s / 2.0,
        r,
        cy + BUTTON_H * s / 2.0,
    )
}

/// Open and Show in folder, left of the "..." button.
fn buttons(row: RectF, s: f32) -> (RectF, RectF) {
    let pad = SPACE[2] * s;
    let cy = (row.top + row.bottom) / 2.0;
    let (t, b) = (cy - BUTTON_H * s / 2.0, cy + BUTTON_H * s / 2.0);
    let show_r = (more_rect(row, s).left - SPACE[1] * s).max(row.left);
    let _ = pad;
    let show_l = (show_r - SHOW_W * s).max(row.left);
    let open_r = (show_l - SPACE[1] * s).max(row.left);
    let open_l = (open_r - OPEN_W * s).max(row.left);
    (rectf(open_l, t, open_r, b), rectf(show_l, t, show_r, b))
}

/// Hit ids: a row's Open is 2 key + 1, its Show in folder 2 key + 2.
fn open_id(key: u32) -> u32 {
    2 * key + 1
}

/// Hits of the rows in view whose file is there, as the window's file
/// cache knows it (`files::lookup`, never blocking).
pub fn layout(r: RectF, view: RectF, s: f32, m: &TimelineModel) -> Vec<Hit> {
    layout_with(r, view, s, m, &files::lookup)
}

/// `layout` with the file check given: a missing file's buttons are drawn
/// disabled and take no clicks.
pub fn layout_with(
    r: RectF,
    view: RectF,
    s: f32,
    m: &TimelineModel,
    state: &dyn Fn(&str) -> FileState,
) -> Vec<Hit> {
    let mut hits = Vec::new();
    let all = all_rect(r, s);
    if !m.items.is_empty() && all.bottom > view.top && all.top < view.bottom {
        hits.push(Hit { id: ALL_ID, r: all });
    }
    // Every item has hits, in view or not: Tab can walk the whole list (the
    // window scrolls the focused one into view) and so can a screen reader.
    // The Open / Show buttons exist only for rows near the view: each needs
    // a (cached, off-thread) check that its file is still there.
    let near = visible(r, view, s, m);
    for k in 0..m.items.len() {
        let x = match &m.items[k] {
            Item::Row(x) => x,
            Item::Day(d) => {
                hits.push(Hit {
                    id: day_id(d.day),
                    r: item_rect(r, s, m, k),
                });
                continue;
            }
        };
        if near.contains(&k)
            && x.file
                .as_deref()
                .is_some_and(|p| state(p) == FileState::Present)
        {
            let (open, show) = buttons(item_rect(r, s, m, k), s);
            hits.push(Hit {
                id: open_id(x.key),
                r: open,
            });
            hits.push(Hit {
                id: open_id(x.key) + 1,
                r: show,
            });
        }
        hits.push(Hit {
            id: more_id(x.key),
            r: more_rect(item_rect(r, s, m, k), s),
        });
        // After its buttons: a click on a button reaches the button first.
        hits.push(Hit {
            id: row_id(x.key),
            r: item_rect(r, s, m, k),
        });
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
    if m.history_on && !m.query.is_empty() {
        ("No matches", "Nothing in the history matches that search.")
    } else if m.history_on {
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

/// A day header row: chevron (down when open, right when closed), the day
/// and how many entries it holds.
fn day_header(c: &Canvas, r: RectF, d: &DayHead) {
    let (s, pal) = (c.s, c.p);
    let pad = SPACE[1] * s;
    let chev = rectf(r.left + pad, r.top, r.left + pad + 20.0 * s, r.bottom);
    let g = if d.open { '\u{E70D}' } else { '\u{E76C}' };
    fluent::icon(c, g, chev, pal.text2);
    let label = rectf(chev.right + pad, r.top, r.right - 96.0 * s, r.bottom);
    text(c, Font::BodyStrong, &d.label, label, pal.text, Align::Left);
    let n = if d.count == 1 {
        "1 item".to_string()
    } else {
        format!("{} items", d.count)
    };
    let cnt = rectf(r.right - 96.0 * s - pad, r.top, r.right - pad, r.bottom);
    text(c, Font::Caption, &n, cnt, pal.text2, Align::Right);
}

/// `hits` is this frame's `layout`: a file row without its hits is drawn
/// as missing, so the picture always matches what takes clicks.
pub fn draw(c: &mut Canvas, r: RectF, view: RectF, m: &TimelineModel, hits: &[Hit]) {
    let (s, pal) = (c.s, c.p);
    if m.items.is_empty() {
        let (title, body) = empty_text(m);
        if m.history_on && !m.query.is_empty() {
            // The search box stays: keep the page's title above the message.
            let t = rectf(
                r.left + PAD * s,
                r.top + PAD * s,
                r.right - PAD * s,
                r.top + (PAD + TITLE_H) * s,
            );
            text(c, Font::Title, "Timeline", t, pal.text, Align::Left);
            let body_r = rectf(
                r.left,
                list_top(r, s),
                r.right,
                r.bottom.max(list_top(r, s)),
            );
            return widgets::empty_state(c, body_r, '\u{E81C}', title, body);
        }
        return widgets::empty_state(c, r, '\u{E81C}', title, body);
    }
    let title = rectf(
        r.left + PAD * s,
        r.top + PAD * s,
        r.right - PAD * s,
        r.top + (PAD + TITLE_H) * s,
    );
    text(c, Font::Title, "Timeline", title, pal.text, Align::Left);
    let all = if m.all_open() {
        "Collapse all"
    } else {
        "Expand all"
    };
    fluent::button(c, all_rect(r, s), all, false);
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
            Item::Day(d) => day_header(c, ir, d),
            Item::Row(x) if x.file.is_none() => {
                let more = more_rect(ir, s);
                let text_r = rectf(ir.left, ir.top, more.left, ir.bottom);
                widgets::row(c, text_r, glyph(x.kind), &x.title, &x.meta, "");
                fluent::button(c, more, "\u{2026}", false);
            }
            Item::Row(x) => {
                let present = hits.iter().any(|h| h.id == open_id(x.key));
                let (open, show) = buttons(ir, s);
                fluent::button(c, more_rect(ir, s), "\u{2026}", false);
                let text_r = rectf(ir.left, ir.top, open.left, ir.bottom);
                let meta = shown_meta(x, present);
                widgets::row(c, text_r, glyph(x.kind), &x.title, &meta, "");
                if present {
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

/// The row is found by its key, so a click that spans an insert still
/// acts on the row that was pressed (or nothing, if it scrolled away).
/// The row model of a hit id (a row, its Open or its Show in folder).
fn row_of(id: u32, m: &TimelineModel) -> Option<(&RowModel, u8)> {
    m.items.iter().find_map(|i| match i {
        Item::Row(x) if row_id(x.key) == id => Some((x, 0)),
        Item::Row(x) if open_id(x.key) == id => Some((x, 1)),
        Item::Row(x) if open_id(x.key) + 1 == id => Some((x, 2)),
        Item::Row(x) if more_id(x.key) == id => Some((x, 3)),
        _ => None,
    })
}

pub fn describe(hit: &Hit, m: &TimelineModel) -> Option<super::Desc> {
    use super::{Desc, Role};
    if hit.id == ALL_ID {
        let name = if m.all_open() {
            "Collapse all days"
        } else {
            "Expand all days"
        };
        return Some(Desc::new(name, Role::Button));
    }
    if hit.id >= DAY_BASE {
        let g = m.groups.iter().find(|g| day_id(g.day) == hit.id)?;
        let open = m
            .items
            .iter()
            .any(|i| matches!(i, Item::Day(d) if d.day == g.day && d.open));
        let mut d = Desc::new(format!("{}, {} items", g.label, g.rows.len()), Role::Header);
        d.expanded = Some(open);
        return Some(d);
    }
    let (x, part) = row_of(hit.id, m)?;
    Some(match part {
        1 => Desc::new(format!("Open {}", x.title), Role::Button),
        2 => Desc::new(format!("Show {} in folder", x.title), Role::Button),
        3 => Desc::new(format!("More actions for {}", x.title), Role::Button),
        _ => Desc::new(format!("{}, {}", x.title, x.meta), Role::Item),
    })
}

pub fn click(hit: &Hit, m: &TimelineModel) -> Option<Action> {
    if let Some((x, part)) = row_of(hit.id, m) {
        match part {
            // Clicking a received file opens it (the tray re-checks it is there).
            0 => return x.file.clone().map(Action::OpenFile),
            3 => return Some(Action::TimelineMenu(x.key)),
            _ => {}
        }
    }
    if hit.id == ALL_ID {
        return Some(Action::TimelineAll(!m.all_open()));
    }
    if hit.id >= DAY_BASE {
        let g = m.groups.iter().find(|g| day_id(g.day) == hit.id)?;
        return Some(Action::TimelineDay(g.day));
    }
    let n = hit.id.checked_sub(1)?;
    let key = n / 2;
    let path = m.items.iter().find_map(|i| match i {
        Item::Row(x) if x.key == key => x.file.clone(),
        _ => None,
    })?;
    Some(if n % 2 == 0 {
        Action::OpenFile(path)
    } else {
        Action::ShowInFolder(path)
    })
}

/// Whether the rows' files are there, checked off the UI thread: a save
/// folder on a slow or offline share must never stall the window. Layout
/// asks for the rows in view; a path not checked yet counts as present (the
/// click re-checks before acting).
pub mod files {
    use std::{
        collections::{HashMap, HashSet},
        sync::{Arc, Mutex, OnceLock},
        time::{Duration, Instant},
    };

    use super::{file_state, FileState};

    /// A result is reused this long before the path is checked again.
    pub const FRESH: Duration = Duration::from_millis(2500);
    /// Checks running at once; a hung share holds at most this many threads.
    const MAX_RUNNING: usize = 8;

    type Notify = Arc<dyn Fn() + Send + Sync>;

    #[derive(Default)]
    struct Cache {
        known: HashMap<String, (FileState, Instant)>,
        running: HashSet<String>,
        notify: Option<Notify>,
    }

    fn cache() -> &'static Mutex<Cache> {
        static C: OnceLock<Mutex<Cache>> = OnceLock::new();
        C.get_or_init(Default::default)
    }

    fn lock() -> std::sync::MutexGuard<'static, Cache> {
        cache().lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Called (from a worker thread) when a path's state changed, so the
    /// window repaints. It must only post.
    pub fn set_notify(f: impl Fn() + Send + Sync + 'static) {
        lock().notify = Some(Arc::new(f));
    }

    /// The last known state (Present until checked); starts a check on a
    /// worker when it is unknown or older than `FRESH`. Never blocks.
    pub fn lookup(path: &str) -> FileState {
        let mut c = lock();
        let known = c.known.get(path).copied();
        if known.is_some_and(|(_, at)| at.elapsed() < FRESH) {
            return known.map_or(FileState::Present, |k| k.0);
        }
        if !c.running.contains(path) && c.running.len() < MAX_RUNNING {
            c.running.insert(path.to_owned());
            let p = path.to_owned();
            std::thread::spawn(move || {
                let st = file_state(&p);
                record(&p, st);
            });
        }
        known.map_or(FileState::Present, |k| k.0)
    }

    /// A fresh result for `path` (a worker's, or the click's re-check);
    /// tells the window when it changes what the row shows.
    pub fn record(path: &str, st: FileState) {
        let notify = {
            let mut c = lock();
            c.running.remove(path);
            let before = c
                .known
                .insert(path.to_owned(), (st, Instant::now()))
                .map_or(FileState::Present, |k| k.0);
            (before != st).then(|| c.notify.clone()).flatten()
        };
        if let Some(f) = notify {
            f();
        }
    }
}

#[cfg(test)]
#[path = "timeline_days_tests.rs"]
mod days_tests;

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
            ..timeline::clip_text_entry("Pixel 8", Dir::ToPc, "hello", false)
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

    #[test]
    fn every_hit_is_described_with_day_state_and_row_names() {
        let now = DAY + 8 * H;
        let entries = [
            file(now - 60, r"C:/x/a.pdf"),
            text(now - 120),
            text(now - 2 * 86_400),
        ];
        let m = build(&entries, true, now, 0);
        let (r, view) = page();
        let hits = layout_with(r, view, 1.0, &m, &|_| FileState::Present);
        let mut ids = std::collections::HashSet::new();
        for h in &hits {
            assert!(ids.insert(h.id), "duplicate id {}", h.id);
            let d = describe(h, &m).expect("described");
            assert!(!d.name.is_empty());
            if d.role == super::super::Role::Header {
                assert!(d.expanded.is_some());
            }
        }
        let rows = hits
            .iter()
            .filter_map(|h| describe(h, &m))
            .filter(|d| d.role == super::super::Role::Item)
            .count();
        assert_eq!(
            rows,
            3.min(m.items.iter().filter(|i| matches!(i, Item::Row(_))).count())
        );
        // A file row's click opens it.
        let row = hits
            .iter()
            .find(|h| matches!(click(h, &m), Some(Action::OpenFile(_))) && h.id >= ROW_BASE)
            .expect("the file row opens on click");
        assert!(describe(row, &m).is_some());
    }

    #[test]
    fn search_narrows_rows_and_drops_empty_days() {
        let now = DAY + 8 * H;
        let entries = [
            file(now - 60, r"C:/x/report.pdf"),
            text(now - 120),
            text(now - 2 * 86_400),
        ];
        let m = build(&entries, true, now, 0);
        let all: usize = m.groups.iter().map(|g| g.rows.len()).sum();
        assert_eq!(all, 3);
        let none = std::collections::HashSet::new();
        assert_eq!(m.filtered("", &none).groups.len(), m.groups.len());
        let f = m.filtered("REPORT", &none);
        assert_eq!(f.groups.len(), 1);
        assert_eq!(f.groups[0].rows.len(), 1);
        assert_eq!(f.query, "REPORT");
        assert!(m.filtered("zzz", &none).items.is_empty());
        // Several words all have to match; the caption counts.
        assert_eq!(m.filtered("text characters", &none).groups.len(), 2);
    }

    #[test]
    fn a_deleted_row_is_hidden_until_the_undo_passes() {
        let now = DAY + 8 * H;
        let entries = [text(now - 60), text(now - 120)];
        let m = build(&entries, true, now, 0);
        let key = m.groups[0].rows[0].key;
        let mut hidden = std::collections::HashSet::new();
        hidden.insert(key);
        let f = m.filtered("", &hidden);
        assert_eq!(f.groups[0].rows.len(), 1);
        assert!(f.groups[0].rows.iter().all(|r| r.key != key));
        hidden.clear();
        assert_eq!(m.filtered("", &hidden).groups[0].rows.len(), 2);
    }

    #[test]
    fn row_menu_offers_copy_path_only_for_files_and_never_repeats_the_buttons() {
        let now = DAY + 8 * H;
        let entries = [file(now - 60, r"C:/x/a.pdf"), text(now - 120)];
        let m = build(&entries, true, now, 0);
        let rows: Vec<&RowModel> = m.groups[0].rows.iter().collect();
        let file_row = rows.iter().find(|r| r.file.is_some()).unwrap();
        let text_row = rows.iter().find(|r| r.file.is_none()).unwrap();
        assert_eq!(
            row_actions(file_row),
            [RowAct::CopyPath, RowAct::Details, RowAct::Delete]
        );
        assert_eq!(row_actions(text_row), [RowAct::Details, RowAct::Delete]);
        assert!(details(file_row).contains("a.pdf"));
    }

    #[test]
    fn the_more_button_asks_for_the_row_menu() {
        let now = DAY + 8 * H;
        let m = build(&[text(now - 60)], true, now, 0);
        let (r, view) = page();
        let hits = layout_with(r, view, 1.0, &m, &|_| FileState::Present);
        let more = hits
            .iter()
            .find(|h| matches!(click(h, &m), Some(Action::TimelineMenu(_))))
            .expect("a more button");
        assert!(describe(more, &m).unwrap().name.starts_with("More actions"));
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
        let m = build(&entries, true, now, 0);
        let (r, view) = page();
        let hits = layout_with(r, view, 1.0, &m, &file_state);
        // Open and Show in folder on the file row; nothing on the text row.
        let hits: Vec<Hit> = hits.into_iter().filter(|h| h.id < ROW_BASE).collect();
        assert_eq!(hits.len(), 2);
        assert_eq!(file_state(&p), FileState::Present);
        // Gone between the layout and the click: the page cannot know yet,
        // so the click still asks, and the tray's re-check refuses.
        std::fs::remove_file(&path).unwrap();
        assert_eq!(click(&hits[0], &m), Some(Action::OpenFile(p.clone())));
        assert_eq!(click(&hits[1], &m), Some(Action::ShowInFolder(p.clone())));
        assert_eq!(file_state(&p), FileState::Missing);
        // The next model disables the buttons and says why.
        let m = build(&entries, true, now, 0);
        assert!(layout_with(r, view, 1.0, &m, &file_state)
            .iter()
            .all(|h| h.id >= ROW_BASE));
        let row = m
            .items
            .iter()
            .find_map(|i| match i {
                Item::Row(x) if x.kind == Kind::File => Some(x),
                _ => None,
            })
            .unwrap();
        // Drawn without its hits, the row says why.
        let shown = shown_meta(row, false);
        assert!(shown.starts_with(MISSING), "{shown}");
        assert_eq!(shown_meta(row, true), row.meta);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn thousand_entries_lay_out_only_visible_rows() {
        let now = DAY + 8 * H;
        // About 33 a day over 30 days: many day headers between rows.
        let entries: Vec<Entry> = (0..1000)
            .map(|k| file(now - k as u64 * 2600, &format!("C:\\x\\{k}.pdf")))
            .collect();
        let m = build(&entries, true, now, 0);
        let mut exp = m.exp.clone();
        exp.set_all(&m.days(), true);
        let m = m.with_expansion(&exp);
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
            let hits = layout_with(r, v, s, &m, &|_| FileState::Present);
            // Buttons only near the view; one whole-row hit per row.
            let buttons = hits.iter().filter(|h| h.id < ROW_BASE).count();
            assert!(buttons <= 2 * range.len());
            let rows = m.items.iter().filter(|i| matches!(i, Item::Row(_))).count();
            assert_eq!(
                hits.iter()
                    .filter(|h| (ROW_BASE..MORE_BASE).contains(&h.id))
                    .count(),
                rows
            );
            assert!(!hits.is_empty());
            // Every hit is in view or on the one overscan row beside it.
            let near = 2.0 * 56.0 * s;
            for h in hits.iter().filter(|h| h.id < ROW_BASE) {
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
        let off = build(&entries, false, now, 0);
        assert!(!off.history_on);
        assert!(off.items.is_empty());
        let (r, view) = page();
        assert!(layout_with(r, view, 1.0, &off, &|_| FileState::Present).is_empty());
        assert_eq!(content_height(800.0, 1.0, &off), 0.0);
        assert_eq!(empty_text(&off).0, "History is off");
        let none = build(&[], true, now, 0);
        assert_eq!(content_height(800.0, 1.0, &none), 0.0);
        assert_eq!(empty_text(&none).0, "Nothing has moved yet");
        let some = build(&entries, true, now, 0);
        assert!(content_height(800.0, 1.0, &some) > 0.0);
    }

    #[test]
    fn rows_show_direction_peer_and_time() {
        let now = DAY + 8 * H;
        let mut e = text(DAY + 14 * H + 5 * 60 - 12 * H);
        e.dir = Dir::ToPhone;
        let m = build(&[e], true, now, 19_800);
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
        let full = build(&entries, true, now, 19_800);
        let empty = build(&[], false, now, 0);
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
                    let hits = layout_with(r, view, 1.0, &full, &file_state);
                    draw(&mut c, r, view, &full, &hits);
                }
                draw(&mut c, view, view, &empty, &[]);
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

    #[test]
    fn a_new_entry_at_the_top_keeps_each_rows_action() {
        let now = DAY + 8 * H;
        let mut entries = vec![
            file(now - 60, "C:\\x\\a.pdf"),
            file(now - 120, "C:\\x\\b.pdf"),
        ];
        let m = build(&entries, true, now, 0);
        let (r, view) = page();
        let hits = layout_with(r, view, 1.0, &m, &|_| FileState::Present);
        // b.pdf's Open, as focused or pressed before the insert.
        let b_open = hits
            .iter()
            .find(|h| click(h, &m) == Some(Action::OpenFile("C:\\x\\b.pdf".into())))
            .copied()
            .unwrap();
        entries.insert(0, file(now - 10, "C:\\x\\new.pdf"));
        let m = build(&entries, true, now, 0);
        let hits = layout_with(r, view, 1.0, &m, &|_| FileState::Present);
        // The same id (focus follows it) still names b.pdf.
        let same = hits.iter().find(|h| h.id == b_open.id).unwrap();
        assert_eq!(
            click(same, &m),
            Some(Action::OpenFile("C:\\x\\b.pdf".into()))
        );
        assert_eq!(
            click(&b_open, &m),
            Some(Action::OpenFile("C:\\x\\b.pdf".into()))
        );
    }

    #[test]
    fn visible_at_the_edges() {
        let now = DAY + 8 * H;
        let entries: Vec<Entry> = (0..40).map(|k| text(now - k * 60)).collect();
        let m = build(&entries, true, now, 0);
        // Items: day header (top 0), then rows at 32, 88, 144, ...
        let list = PAD + TITLE_H;
        let s = 1.0;
        // The view's top exactly on row 3's top (item 3, at 32 + 2 * 56):
        // item 3 is the first in view, plus one overscan above.
        let y = list + 32.0 + 2.0 * 56.0;
        let r = rectf(0.0, -y, 800.0, 10_000.0);
        let v = rectf(0.0, 0.0, 800.0, 112.0);
        let range = visible(r, v, s, &m);
        assert_eq!(range.start, 2);
        // Rows 3 and 4 fill the view exactly; row 5 starts at its bottom
        // and is the overscan below.
        assert_eq!(range.end, 6);
        // The view entirely below the list: nothing past the end.
        let r = rectf(0.0, -100_000.0, 800.0, 0.0);
        let range = visible(r, v, s, &m);
        assert!(range.end <= m.items.len());
        assert!(range.len() <= 1, "{range:?}");
        // The view above the list (on the title): the first item or two.
        let r = rectf(0.0, 0.0, 800.0, 10_000.0);
        let v = rectf(0.0, 0.0, 800.0, 30.0);
        assert_eq!(visible(r, v, s, &m), 0..1);
        // A new entry at the top: the range covers the same pixels, and
        // the rows under it shift down by one.
        let mut more = entries.clone();
        more.insert(0, text(now - 1));
        let m2 = build(&more, true, now, 0);
        let r = rectf(0.0, -y, 800.0, 10_000.0);
        let v = rectf(0.0, 0.0, 800.0, 112.0);
        assert_eq!(visible(r, v, s, &m2), 2..6);
        assert_eq!(m2.items.len(), m.items.len() + 1);
    }

    #[test]
    fn file_checks_never_block_and_report_changes() {
        use std::sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        };
        let dir = std::env::temp_dir().join(format!("pb-tl-files-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("later.pdf");
        let _ = std::fs::remove_file(&path);
        let p = path.to_string_lossy().into_owned();
        let told = Arc::new(AtomicUsize::new(0));
        let t = told.clone();
        files::set_notify(move || {
            t.fetch_add(1, Ordering::SeqCst);
        });
        // Not checked yet: present (buttons on) at once, checked on a worker.
        assert_eq!(files::lookup(&p), FileState::Present);
        let t0 = std::time::Instant::now();
        while files::lookup(&p) == FileState::Present {
            assert!(t0.elapsed().as_secs() < 5, "the worker reports");
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert_eq!(told.load(Ordering::SeqCst), 1, "the window is told");
        // The same result again changes nothing on screen.
        files::record(&p, FileState::Missing);
        assert_eq!(told.load(Ordering::SeqCst), 1);
        // The click's re-check found it back.
        std::fs::write(&path, b"x").unwrap();
        files::record(&p, file_state(&p));
        assert_eq!(told.load(Ordering::SeqCst), 2);
        assert_eq!(files::lookup(&p), FileState::Present);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
