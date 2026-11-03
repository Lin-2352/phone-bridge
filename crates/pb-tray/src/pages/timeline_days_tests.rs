//! Tests of the Timeline's collapsible day groups (model, hits, scroll anchor).
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

fn page() -> (RectF, RectF) {
    (
        rectf(0.0, 0.0, 800.0, 100_000.0),
        rectf(0.0, 0.0, 800.0, 600.0),
    )
}

/// `n` days of three entries each, newest day = Today.
fn days_of_entries(n: u64, now: u64) -> Vec<Entry> {
    (0..n)
        .flat_map(|d| (0..3).map(move |k| text(now - d * 24 * H - k * 60)))
        .collect()
}

fn heads(m: &TimelineModel) -> Vec<(String, usize, bool)> {
    m.items
        .iter()
        .filter_map(|i| match i {
            Item::Day(d) => Some((d.label.clone(), d.count, d.open)),
            _ => None,
        })
        .collect()
}

#[test]
fn many_days_start_with_only_today_open_and_show_counts() {
    let now = DAY + 8 * H;
    let m = build(&days_of_entries(5, now), true, now, 0);
    let h = heads(&m);
    assert_eq!(h.len(), 5);
    assert_eq!(h[0], ("Today".to_string(), 3, true));
    assert_eq!(h[1], ("Yesterday".to_string(), 3, false));
    assert!(h[1..].iter().all(|(_, n, open)| *n == 3 && !open));
    assert_eq!(m.items.len(), 5 + 3);
    assert_eq!(m.cards.len(), 1);
    assert_eq!(m.height, 5 * HEAD_H + 3 * ROW_H);
}

#[test]
fn three_days_or_fewer_start_fully_open() {
    let now = DAY + 8 * H;
    let m = build(&days_of_entries(3, now), true, now, 0);
    assert!(heads(&m).iter().all(|(_, _, open)| *open));
    assert_eq!(m.items.len(), 3 + 9);
    assert_eq!(m.cards.len(), 3);
}

#[test]
fn toggling_a_day_recomputes_rows_height_and_cards() {
    let now = DAY + 8 * H;
    let m = build(&days_of_entries(5, now), true, now, 0);
    let day = m.groups[1].day;
    let mut exp = m.exp.clone();
    exp.toggle(&m.days(), day);
    let m2 = m.with_expansion(&exp);
    assert!(heads(&m2)[1].2);
    assert_eq!(m2.items.len(), m.items.len() + 3);
    assert_eq!(m2.height, m.height + 3 * ROW_H);
    assert_eq!(m2.cards.len(), 2);
    assert_eq!(m2.tops.len(), m2.items.len());
    assert!(m2.tops.windows(2).all(|w| w[0] < w[1]));
    let last = m2.items.len() - 1;
    assert_eq!(m2.tops[last] + item_h(&m2.items[last]), m2.height);
    assert!(content_height(800.0, 1.0, &m2) > content_height(800.0, 1.0, &m));
    // Collapsing again restores the first model exactly.
    exp.toggle(&m.days(), day);
    let back = m.with_expansion(&exp);
    assert_eq!(
        (back.items, back.height, back.cards),
        (m.items, m.height, m.cards)
    );
}

#[test]
fn collapse_all_and_expand_all_change_the_visible_rows() {
    let now = DAY + 8 * H;
    let m = build(&days_of_entries(5, now), true, now, 0);
    assert!(!m.all_open());
    let mut exp = m.exp.clone();
    exp.set_all(&m.days(), true);
    let open = m.with_expansion(&exp);
    assert!(open.all_open());
    assert_eq!(open.items.len(), 5 + 15);
    exp.set_all(&m.days(), false);
    let shut = m.with_expansion(&exp);
    assert_eq!(shut.items.len(), 5);
    assert!(shut.cards.is_empty());
    assert_eq!(shut.height, 5 * HEAD_H);
}

#[test]
fn day_headers_and_the_all_control_are_hits_that_toggle() {
    let now = DAY + 8 * H;
    let m = build(&days_of_entries(5, now), true, now, 0);
    let (r, view) = page();
    let hits = layout_with(r, view, 1.0, &m, &|_| FileState::Present);
    let ids: std::collections::HashSet<u32> = hits.iter().map(|h| h.id).collect();
    assert_eq!(ids.len(), hits.len(), "ids are unique");
    let yesterday = m.groups[1].day;
    let head = hits.iter().find(|h| h.id == day_id(yesterday)).unwrap();
    assert_eq!(click(head, &m), Some(Action::TimelineDay(yesterday)));
    // The header is a full-width row, tall enough to hit.
    assert!(head.r.right - head.r.left > 600.0 && head.r.bottom - head.r.top >= 32.0);
    let all = hits.iter().find(|h| h.id == ALL_ID).unwrap();
    assert_eq!(click(all, &m), Some(Action::TimelineAll(true)));
    let mut exp = m.exp.clone();
    exp.set_all(&m.days(), true);
    let open = m.with_expansion(&exp);
    assert_eq!(click(all, &open), Some(Action::TimelineAll(false)));
    // Day hits never collide with a file row's buttons.
    let files: Vec<Entry> = (0..3).map(|k| file(DAY - k, "C:\\x\\a.pdf")).collect();
    let fm = build(&files, true, DAY + H, 0);
    let fh = layout_with(r, view, 1.0, &fm, &|_| FileState::Present);
    let special = fh.iter().filter(|h| h.id >= DAY_BASE).count();
    assert_eq!(special, fm.groups.len() + 1);
    // Per row: Open, Show in folder and the whole row.
    assert_eq!(fh.len(), special + 9);
}

#[test]
fn scroll_position_holds_when_an_entry_arrives_above() {
    let now = DAY + 8 * H;
    let mut entries = days_of_entries(3, now);
    let m = build(&entries, true, now, 0);
    // The user is looking at the 6th item.
    let at = LIST_TOP + m.tops[5] as f32 + 10.0;
    let a = m.anchor(at).unwrap();
    entries.insert(0, text(now - 1));
    let m2 = build(&entries, true, now, 0);
    // Everything under the new row moved down by one row.
    assert_eq!(m2.restore(&a), Some(at + ROW_H as f32));
    // At the very top nothing is anchored (the new entry shows).
    assert!(m.anchor(0.0).is_none());
}

#[test]
fn scroll_position_holds_when_a_day_above_collapses() {
    let now = DAY + 8 * H;
    let m = build(&days_of_entries(3, now), true, now, 0);
    // Looking at a row of the last day, 7 px into it.
    let k = m.items.len() - 2;
    let at = LIST_TOP + m.tops[k] as f32 + 7.0;
    let a = m.anchor(at).unwrap();
    let mut exp = m.exp.clone();
    exp.toggle(&m.days(), m.groups[0].day);
    let m2 = m.with_expansion(&exp);
    assert_eq!(m2.restore(&a), Some(at - 3.0 * ROW_H as f32));
    // The anchored row's own day collapsing falls back to its header.
    let mut exp = m.exp.clone();
    exp.toggle(&m.days(), m.groups[2].day);
    let m3 = m.with_expansion(&exp);
    let head = m3.items.len() - 1;
    assert_eq!(m3.restore(&a), Some(LIST_TOP + m3.tops[head] as f32));
    // A vanished anchor says so; the caller clamps instead.
    let none = build(&[], true, now, 0);
    assert_eq!(none.restore(&a), None);
}

#[test]
fn offset_is_clamped_after_collapse_all_shrinks_the_content() {
    let now = DAY + 8 * H;
    let m = build(&days_of_entries(3, now), true, now, 0);
    let mut exp = m.exp.clone();
    let open_h = content_height(800.0, 1.0, &m);
    let mut sc = crate::widgets::Scroll::new(open_h, 300.0);
    sc.jump(open_h - 300.0);
    exp.set_all(&m.days(), false);
    let shut = m.with_expansion(&exp);
    sc.content = content_height(800.0, 1.0, &shut);
    sc.clamp();
    assert_eq!(sc.offset, (sc.content - 300.0).max(0.0));
    assert!(sc.offset < open_h - 300.0);
    // Collapsed content that fits the view: back to the top.
    sc.view = 10_000.0;
    sc.clamp();
    assert_eq!(sc.offset, 0.0);
}
