//! Transfers: what is moving between the PC and a phone, then what finished. Each running or waiting
//! transfer has a Cancel; a failed one has a Retry. Built from the same registry the transfer window
//! draws (`transfers::rows`), so the two cannot disagree.

use crate::{
    fluent::{self, rectf, round_rect, text, Align, Canvas, Font, RectF},
    pages::{Action, Desc, Hit, Role},
    tokens::SPACE,
    transfers::{self, State},
    widgets,
};

/// Where a transfer is in its life, for ordering and drawing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Running,
    Waiting,
    Done,
    Failed,
    Cancelled,
}

/// One transfer as the page shows it (plain data, so the page tests need no registry).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TransferRow {
    pub id: u64,
    pub name: String,
    pub peer: String,
    /// PC to phone.
    pub outgoing: bool,
    pub kind: Kind,
    pub done: u64,
    pub total: u64,
    pub percent: u32,
    /// The line under the name (the registry's wording).
    pub status: String,
    pub can_cancel: bool,
    pub can_retry: bool,
}

impl From<&transfers::Row> for TransferRow {
    fn from(r: &transfers::Row) -> Self {
        TransferRow {
            id: r.id,
            name: r.name.clone(),
            peer: r.peer.clone(),
            outgoing: r.dir == transfers::Dir::Send,
            kind: match r.state {
                State::Active => Kind::Running,
                State::Waiting => Kind::Waiting,
                State::Done => Kind::Done,
                State::Failed(_) => Kind::Failed,
                State::Cancelled(_) => Kind::Cancelled,
            },
            done: r.done,
            total: r.total,
            percent: r.percent(),
            status: r.status_line(),
            can_cancel: r.can_cancel,
            can_retry: r.can_retry,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TransfersModel {
    pub rows: Vec<TransferRow>,
}

impl TransfersModel {
    /// The registry's rows, ordered for the page.
    pub fn from_registry() -> Self {
        let rows = transfers::rows().iter().map(TransferRow::from).collect();
        TransfersModel { rows: order(rows) }
    }
}

/// Running first, then waiting (oldest first, the order they will finish in); then everything that has
/// ended, newest first.
pub fn order(mut rows: Vec<TransferRow>) -> Vec<TransferRow> {
    let live = |k: Kind| matches!(k, Kind::Running | Kind::Waiting);
    rows.sort_by(|a, b| match (live(a.kind), live(b.kind)) {
        (true, true) => (a.kind == Kind::Waiting, a.id).cmp(&(b.kind == Kind::Waiting, b.id)),
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
        (false, false) => b.id.cmp(&a.id),
    });
    rows
}

/// "42% \u{b7} 12 MB / 30 MB": the percentage, then done of total, always in this order.
pub fn progress_text(r: &TransferRow) -> String {
    format!(
        "{}% \u{b7} {} / {}",
        r.percent,
        transfers::fmt_bytes(r.done),
        transfers::fmt_bytes(r.total)
    )
}

/// The title and hint of the empty page.
pub fn empty_text() -> (&'static str, &'static str) {
    (
        crate::strings::TRANSFERS_EMPTY,
        crate::strings::TRANSFERS_EMPTY_HINT,
    )
}

/// Hit ids: one button per row, by position.
const BUTTON: u32 = 10;

/// Sizes in logical px.
const PAD: f32 = 24.0;
const TITLE_H: f32 = 40.0;
const EMPTY_H: f32 = 184.0;
const ROW_H: f32 = 76.0;
const BUTTON_W: f32 = 88.0;
const BUTTON_H: f32 = 32.0;
const BAR_H: f32 = 4.0;

struct Parts {
    title: RectF,
    card: RectF,
    /// Each row and its button.
    rows: Vec<(RectF, RectF)>,
    bottom: f32,
}

fn parts(r: RectF, s: f32, m: &TransfersModel) -> Parts {
    let (left, right) = (r.left + PAD * s, (r.right - PAD * s).max(r.left + PAD * s));
    let top = r.top + PAD * s;
    let title = rectf(left, top, right, top + TITLE_H * s);
    if m.rows.is_empty() {
        let card = rectf(left, title.bottom, right, title.bottom + EMPTY_H * s);
        return Parts {
            title,
            card,
            rows: Vec::new(),
            bottom: card.bottom + PAD * s,
        };
    }
    let card = rectf(
        left,
        title.bottom,
        right,
        title.bottom + m.rows.len() as f32 * ROW_H * s,
    );
    let rows = (0..m.rows.len())
        .map(|k| {
            let t = card.top + k as f32 * ROW_H * s;
            let row = rectf(left, t, right, t + ROW_H * s);
            let cy = t + ROW_H * s / 2.0;
            let pad = SPACE[3] * s;
            let b = rectf(
                (right - pad - BUTTON_W * s).max(left),
                cy - BUTTON_H * s / 2.0,
                (right - pad).max(left),
                cy + BUTTON_H * s / 2.0,
            );
            (row, b)
        })
        .collect();
    Parts {
        title,
        card,
        rows,
        bottom: card.bottom + PAD * s,
    }
}

fn has_button(r: &TransferRow) -> bool {
    r.can_cancel || r.can_retry
}

pub fn layout(r: RectF, s: f32, m: &TransfersModel) -> Vec<Hit> {
    let p = parts(r, s, m);
    m.rows
        .iter()
        .zip(&p.rows)
        .enumerate()
        .filter(|(_, (row, _))| has_button(row))
        .map(|(k, (_, (_, b)))| Hit {
            id: BUTTON + k as u32,
            r: *b,
        })
        .collect()
}

/// `width` in pixels; the height in logical px.
pub fn content_height(width: f32, s: f32, m: &TransfersModel) -> f32 {
    parts(rectf(0.0, 0.0, width, 0.0), s, m).bottom / s
}

pub fn draw(c: &mut Canvas, r: RectF, m: &TransfersModel) {
    let (s, pal) = (c.s, c.p);
    let p = parts(r, s, m);
    text(c, Font::Title, "Transfers", p.title, pal.text, Align::Left);
    widgets::card(c, p.card);
    if m.rows.is_empty() {
        let (title, hint) = empty_text();
        widgets::empty_state(c, p.card, fluent::glyph::SEND, title, hint);
        return;
    }
    for (t, (row, b)) in m.rows.iter().zip(&p.rows) {
        let pad = fluent::ROW_PAD as f32 * s;
        let cy = (row.top + row.bottom) / 2.0;
        let g = rectf(
            row.left + pad,
            cy - 10.0 * s,
            row.left + pad + 20.0 * s,
            cy + 10.0 * s,
        );
        let glyph = if t.outgoing {
            fluent::glyph::UPLOAD
        } else {
            fluent::glyph::SEND
        };
        let bad = t.kind == Kind::Failed;
        fluent::icon(c, glyph, g, if bad { pal.danger } else { pal.text2 });
        let tx = g.right + SPACE[2] * s;
        let tr = (b.left - SPACE[2] * s).max(tx);
        let name = rectf(tx, row.top + 10.0 * s, tr, row.top + 32.0 * s);
        text(c, Font::Body, &t.name, name, pal.text, Align::Left);
        let meta = rectf(tx, name.bottom, tr, name.bottom + 18.0 * s);
        let line = if t.kind == Kind::Running {
            progress_text(t)
        } else {
            t.status.clone()
        };
        text(
            c,
            Font::Caption,
            &line,
            meta,
            if bad { pal.danger } else { pal.text2 },
            Align::Left,
        );
        if matches!(t.kind, Kind::Running | Kind::Waiting) {
            let bar = rectf(
                tx,
                meta.bottom + 6.0 * s,
                tr,
                meta.bottom + (6.0 + BAR_H) * s,
            );
            round_rect(
                c,
                &bar,
                Some((pal.stroke, 1.0)),
                None,
                BAR_H * s / 2.0,
                false,
            );
            let w = (bar.right - bar.left) * t.percent.min(100) as f32 / 100.0;
            if w > 0.0 {
                let fill = rectf(bar.left, bar.top, bar.left + w, bar.bottom);
                round_rect(
                    c,
                    &fill,
                    Some((pal.primary, 1.0)),
                    None,
                    BAR_H * s / 2.0,
                    false,
                );
            }
        }
        if t.can_cancel {
            fluent::button(c, *b, "Cancel", false);
        } else if t.can_retry {
            fluent::button(c, *b, "Retry", true);
        }
    }
}

pub fn describe(hit: &Hit, m: &TransfersModel) -> Option<Desc> {
    let t = m.rows.get(hit.id.checked_sub(BUTTON)? as usize)?;
    if t.can_cancel {
        Some(Desc::new(format!("Cancel {}", t.name), Role::Button))
    } else if t.can_retry {
        Some(Desc::new(format!("Retry {}", t.name), Role::Button))
    } else {
        None
    }
}

pub fn click(hit: &Hit, m: &TransfersModel) -> Option<Action> {
    let t = m.rows.get(hit.id.checked_sub(BUTTON)? as usize)?;
    if t.can_cancel {
        Some(Action::TransferCancel(t.id))
    } else if t.can_retry {
        Some(Action::TransferRetry(t.id))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fluent::rectf;

    fn row(id: u64, kind: Kind, done: u64, total: u64) -> TransferRow {
        TransferRow {
            id,
            name: format!("file{id}.bin"),
            peer: "Pixel 8".into(),
            outgoing: true,
            kind,
            done,
            total,
            percent: (done * 100).checked_div(total).unwrap_or(0) as u32,
            status: "status".into(),
            can_cancel: matches!(kind, Kind::Running | Kind::Waiting),
            can_retry: kind == Kind::Failed,
        }
    }

    fn page() -> crate::fluent::RectF {
        rectf(0.0, 0.0, 800.0, 2000.0)
    }

    #[test]
    fn transfers_page_lists_active_then_finished() {
        let rows = order(vec![
            row(1, Kind::Done, 10, 10),
            row(2, Kind::Running, 5, 10),
            row(3, Kind::Done, 10, 10),
            row(4, Kind::Waiting, 0, 10),
            row(5, Kind::Failed, 1, 10),
        ]);
        let ids: Vec<u64> = rows.iter().map(|r| r.id).collect();
        // Running, then waiting; then everything finished, newest first.
        assert_eq!(ids, vec![2, 4, 5, 3, 1]);
    }

    #[test]
    fn an_empty_transfers_page_says_so() {
        let (title, hint) = empty_text();
        assert_eq!(title, crate::strings::TRANSFERS_EMPTY);
        assert!(!hint.is_empty());
        assert!(layout(page(), 1.0, &TransfersModel::default()).is_empty());
    }

    #[test]
    fn transfer_progress_format_has_stable_field_order() {
        let r = row(1, Kind::Running, 13_002_342, 30_932_992);
        assert_eq!(progress_text(&r), "42% \u{b7} 12 MB / 30 MB");
    }

    #[test]
    fn every_hit_has_a_name() {
        let m = TransfersModel {
            rows: vec![
                row(1, Kind::Running, 1, 10),
                row(2, Kind::Failed, 1, 10),
                row(3, Kind::Done, 10, 10),
            ],
        };
        let hits = layout(page(), 1.0, &m);
        assert_eq!(
            hits.len(),
            2,
            "Cancel for the running one, Retry for the failed one"
        );
        for h in &hits {
            let d = describe(h, &m).expect("described");
            assert!(!d.name.is_empty());
        }
    }

    #[test]
    fn cancel_and_retry_use_the_transfer_actions() {
        let m = TransfersModel {
            rows: vec![row(7, Kind::Running, 1, 10), row(9, Kind::Failed, 1, 10)],
        };
        let hits = layout(page(), 1.0, &m);
        assert_eq!(click(&hits[0], &m), Some(Action::TransferCancel(7)));
        assert_eq!(click(&hits[1], &m), Some(Action::TransferRetry(9)));
    }

    #[test]
    fn hits_stay_inside_the_content_height() {
        let m = TransfersModel {
            rows: (1..=6).map(|i| row(i, Kind::Done, 10, 10)).collect(),
        };
        let h = content_height(800.0, 1.5, &m);
        for x in layout(rectf(0.0, 0.0, 800.0, 10.0), 1.5, &m) {
            assert!(x.r.bottom <= h * 1.5 + 0.5);
        }
    }
}
