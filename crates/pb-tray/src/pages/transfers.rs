//! Transfers: what is moving between the PC and a phone, then what finished. Every row has a checkbox; a
//! running file can be paused or cancelled, a waiting one paused, resumed or removed, a failed one retried,
//! a finished one removed from the list. Ticked rows are acted on together from the bar under the title.
//! Built from the same registry the transfer window draws (`transfers::rows`), so the two cannot disagree.

use std::{path::PathBuf, sync::Mutex};

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

/// One row the person can tick: a transfer in the registry, or a file waiting in a phone's queue.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum SelKey {
    Transfer(u64),
    Queued(String, u64),
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
    /// The file being sent (outgoing rows): what Pause finds its place in the queue by.
    pub path: Option<PathBuf>,
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
            path: r.path.clone(),
        }
    }
}

/// A file waiting in a phone's send queue (not started, paused, or its source is missing).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QueuedRow {
    /// The phone's queue key, for Remove.
    pub peer: String,
    pub id: u64,
    pub name: String,
    pub status: String,
    /// Held on its own (Resume applies to it).
    pub held: bool,
    /// The source file is gone (only Remove applies).
    pub needs_file: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TransfersModel {
    pub rows: Vec<TransferRow>,
    /// Files waiting behind the ones sending, from the send queue.
    pub queued: Vec<QueuedRow>,
    /// The send queue is paused: the next file waits (Resume is offered).
    pub paused: bool,
    /// The ticked rows.
    pub selected: Vec<SelKey>,
}

static SELECTION: Mutex<Vec<SelKey>> = Mutex::new(Vec::new());

/// Ticks or unticks one row.
pub fn toggle_selection(k: SelKey) {
    if let Ok(mut s) = SELECTION.lock() {
        match s.iter().position(|x| *x == k) {
            Some(at) => {
                s.remove(at);
            }
            None => s.push(k),
        }
    }
}

/// Replaces the ticked rows (Select all).
pub fn set_selection(keys: Vec<SelKey>) {
    if let Ok(mut s) = SELECTION.lock() {
        *s = keys;
    }
}

pub fn clear_selection() {
    set_selection(Vec::new());
}

impl TransfersModel {
    /// The registry's rows, ordered for the page.
    pub fn from_registry() -> Self {
        let rows = transfers::rows().iter().map(TransferRow::from).collect();
        let mut m = TransfersModel {
            rows: order(rows),
            queued: crate::core::queue_rows(),
            paused: crate::core::queue_paused(),
            selected: Vec::new(),
        };
        // A ticked row that is gone (finished and dismissed, sent) is no longer ticked.
        if let Ok(mut s) = SELECTION.lock() {
            s.retain(|k| m.has(k));
            m.selected = s.clone();
        }
        m
    }

    fn has(&self, k: &SelKey) -> bool {
        match k {
            SelKey::Transfer(id) => self.rows.iter().any(|r| r.id == *id),
            SelKey::Queued(p, id) => self.queued.iter().any(|q| q.peer == *p && q.id == *id),
        }
    }

    /// Every row's key, in page order.
    pub fn keys(&self) -> Vec<SelKey> {
        self.rows
            .iter()
            .map(|r| SelKey::Transfer(r.id))
            .chain(
                self.queued
                    .iter()
                    .map(|q| SelKey::Queued(q.peer.clone(), q.id)),
            )
            .collect()
    }

    fn count(&self) -> usize {
        self.rows.len() + self.queued.len()
    }

    fn key_at(&self, p: usize) -> Option<SelKey> {
        self.keys().get(p).cloned()
    }

    fn all_selected(&self) -> bool {
        let k = self.keys();
        !k.is_empty() && k.iter().all(|x| self.selected.contains(x))
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

/// A button on one row, left to right as drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowBtn {
    Pause,
    Resume,
    Cancel,
    Retry,
    Remove,
}

impl RowBtn {
    fn label(self) -> &'static str {
        match self {
            RowBtn::Pause => "Pause",
            RowBtn::Resume => "Resume",
            RowBtn::Cancel => "Cancel",
            RowBtn::Retry => "Retry",
            RowBtn::Remove => "Remove",
        }
    }
}

/// What a transfer row offers. Pause is only for a file this PC is sending (it goes back to the queue,
/// held, and starts again at Resume).
pub fn transfer_buttons(r: &TransferRow) -> Vec<RowBtn> {
    let mut v = Vec::new();
    match r.kind {
        Kind::Running => {
            if r.outgoing && r.path.is_some() {
                v.push(RowBtn::Pause);
            }
            if r.can_cancel {
                v.push(RowBtn::Cancel);
            }
        }
        Kind::Waiting => {
            if r.can_cancel {
                v.push(RowBtn::Cancel);
            }
        }
        Kind::Failed | Kind::Cancelled => {
            if r.can_retry {
                v.push(RowBtn::Retry);
            }
            v.push(RowBtn::Remove);
        }
        Kind::Done => v.push(RowBtn::Remove),
    }
    v
}

/// What a queued file offers. While the whole queue is paused a waiting file is already paused.
pub fn queued_buttons(q: &QueuedRow, queue_paused: bool) -> Vec<RowBtn> {
    if q.held {
        vec![RowBtn::Resume, RowBtn::Remove]
    } else if q.needs_file || queue_paused {
        vec![RowBtn::Remove]
    } else {
        vec![RowBtn::Pause, RowBtn::Remove]
    }
}

fn buttons_at(m: &TransfersModel, p: usize) -> Vec<RowBtn> {
    match m.rows.get(p) {
        Some(t) => transfer_buttons(t),
        None => m
            .queued
            .get(p - m.rows.len())
            .map(|q| queued_buttons(q, m.paused))
            .unwrap_or_default(),
    }
}

fn name_at(m: &TransfersModel, p: usize) -> String {
    match m.rows.get(p) {
        Some(t) => t.name.clone(),
        None => m
            .queued
            .get(p - m.rows.len())
            .map(|q| q.name.clone())
            .unwrap_or_default(),
    }
}

/// The action of one row button.
fn row_action(m: &TransfersModel, p: usize, b: RowBtn) -> Option<Action> {
    if let Some(t) = m.rows.get(p) {
        return match b {
            RowBtn::Pause => t.path.clone().map(Action::TransferHold),
            RowBtn::Cancel => Some(Action::TransferCancel(t.id)),
            RowBtn::Retry => Some(Action::TransferRetry(t.id)),
            RowBtn::Remove => Some(Action::TransferDismiss(t.id)),
            RowBtn::Resume => None,
        };
    }
    let q = m.queued.get(p - m.rows.len())?;
    let (peer, id) = (q.peer.clone(), q.id);
    Some(match b {
        RowBtn::Pause => Action::QueueHold(peer, id),
        RowBtn::Resume => Action::QueueRelease(peer, id),
        RowBtn::Remove | RowBtn::Cancel => Action::QueueRemove(peer, id),
        RowBtn::Retry => return None,
    })
}

/// What the bar under the title does to the ticked rows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Bulk {
    Pause,
    Resume,
    Cancel,
    Remove,
}

const BULK: [Bulk; 4] = [Bulk::Pause, Bulk::Resume, Bulk::Cancel, Bulk::Remove];

impl Bulk {
    fn label(self) -> &'static str {
        match self {
            Bulk::Pause => "Pause",
            Bulk::Resume => "Resume",
            Bulk::Cancel => "Cancel",
            Bulk::Remove => "Remove",
        }
    }

    fn name(self) -> &'static str {
        match self {
            Bulk::Pause => "Pause selected",
            Bulk::Resume => "Resume selected",
            Bulk::Cancel => "Cancel selected",
            Bulk::Remove => "Remove selected",
        }
    }

    fn wants(self) -> RowBtn {
        match self {
            Bulk::Pause => RowBtn::Pause,
            Bulk::Resume => RowBtn::Resume,
            Bulk::Cancel => RowBtn::Cancel,
            Bulk::Remove => RowBtn::Remove,
        }
    }
}

/// The actions of one bulk button over the ticked rows: each row only when it offers that button
/// (a queued file's Cancel is its Remove).
pub fn bulk_actions(m: &TransfersModel, b: Bulk) -> Vec<Action> {
    let mut out = Vec::new();
    for (p, k) in m.keys().iter().enumerate() {
        if !m.selected.contains(k) {
            continue;
        }
        let offers = buttons_at(m, p);
        let want = b.wants();
        let hit = offers.contains(&want)
            || (want == RowBtn::Cancel && offers.contains(&RowBtn::Remove) && p >= m.rows.len());
        if !hit {
            continue;
        }
        let btn = if offers.contains(&want) {
            want
        } else {
            RowBtn::Remove
        };
        if let Some(a) = row_action(m, p, btn) {
            out.push(a);
        }
    }
    out
}

/// Hit ids.
const QUEUE_BUTTON: u32 = 5;
const CANCEL_ALL: u32 = 6;
const SELECT_ALL: u32 = 7;
/// One per bulk button, in [`BULK`] order.
const BULK_BASE: u32 = 20;
/// One checkbox per row, by position.
const CHECK_BASE: u32 = 2000;
/// Up to [`SLOTS`] buttons per row, by position and slot.
const ROW_BASE: u32 = 4000;
const SLOTS: u32 = 4;

/// Sizes in logical px.
const PAD: f32 = 24.0;
const TITLE_H: f32 = 40.0;
const BAR_ROW_H: f32 = 44.0;
const EMPTY_H: f32 = 184.0;
const ROW_H: f32 = 76.0;
const BUTTON_W: f32 = 88.0;
const BUTTON_H: f32 = 32.0;
const HEAD_BTN_W: f32 = 120.0;
const BAR_H: f32 = 4.0;
const CHECK: f32 = 20.0;

struct RowParts {
    row: RectF,
    check: RectF,
    /// Left to right, as [`buttons_at`] lists them.
    btns: Vec<RectF>,
}

struct Parts {
    title: RectF,
    /// Select all, Pause or Resume queue and Cancel all, right of the title, left to right.
    select_all: Option<RectF>,
    queue: Option<RectF>,
    cancel_all: Option<RectF>,
    /// "N selected" and the bulk buttons, shown while rows are ticked.
    bar: Option<RectF>,
    bulk: Vec<(Bulk, RectF)>,
    card: RectF,
    rows: Vec<RowParts>,
    bottom: f32,
}

fn parts(r: RectF, s: f32, m: &TransfersModel) -> Parts {
    let (left, right) = (r.left + PAD * s, (r.right - PAD * s).max(r.left + PAD * s));
    let top = r.top + PAD * s;
    let title = rectf(left, top, right, top + TITLE_H * s);
    let count = m.count();
    let gap = SPACE[2] * s;
    let (mut select_all, mut queue, mut cancel_all) = (None, None, None);
    if count > 0 || m.paused {
        let y0 = title.top + (TITLE_H - BUTTON_H) * s / 2.0;
        let y1 = title.top + (TITLE_H + BUTTON_H) * s / 2.0;
        let mut x = right;
        let mut next = |w: f32| {
            let l = (x - w * s).max(left);
            let rect = rectf(l, y0, x, y1);
            x = (l - gap).max(left);
            rect
        };
        cancel_all = (count > 0).then(|| next(HEAD_BTN_W));
        queue = Some(next(HEAD_BTN_W));
        select_all = (count > 0).then(|| next(HEAD_BTN_W));
    }
    let mut y = title.bottom;
    let (mut bar, mut bulk) = (None, Vec::new());
    if !m.selected.is_empty() && count > 0 {
        let b = rectf(left, y, right, y + BAR_ROW_H * s);
        let mut x = right;
        for kind in BULK.iter().rev() {
            if bulk_actions(m, *kind).is_empty() {
                continue;
            }
            let l = (x - BUTTON_W * s).max(left);
            bulk.push((
                *kind,
                rectf(
                    l,
                    b.top + (BAR_ROW_H - BUTTON_H) * s / 2.0,
                    x,
                    b.top + (BAR_ROW_H + BUTTON_H) * s / 2.0,
                ),
            ));
            x = (l - gap).max(left);
        }
        bulk.reverse();
        bar = Some(b);
        y = b.bottom;
    }
    if count == 0 {
        let card = rectf(left, y, right, y + EMPTY_H * s);
        return Parts {
            title,
            select_all,
            queue,
            cancel_all,
            bar,
            bulk,
            card,
            rows: Vec::new(),
            bottom: card.bottom + PAD * s,
        };
    }
    let card = rectf(left, y, right, y + count as f32 * ROW_H * s);
    let pad = SPACE[3] * s;
    let rows = (0..count)
        .map(|k| {
            let t = card.top + k as f32 * ROW_H * s;
            let row = rectf(left, t, right, t + ROW_H * s);
            let cy = t + ROW_H * s / 2.0;
            let check = rectf(
                left + pad,
                cy - CHECK * s / 2.0,
                (left + pad + CHECK * s).min(right),
                cy + CHECK * s / 2.0,
            );
            let n = buttons_at(m, k).len();
            let mut x = (right - pad).max(left);
            let mut btns = Vec::with_capacity(n);
            for _ in 0..n {
                let l = (x - BUTTON_W * s).max(left);
                btns.push(rectf(
                    l,
                    cy - BUTTON_H * s / 2.0,
                    x,
                    cy + BUTTON_H * s / 2.0,
                ));
                x = (l - SPACE[1] * s).max(left);
            }
            btns.reverse();
            RowParts { row, check, btns }
        })
        .collect();
    Parts {
        title,
        select_all,
        queue,
        cancel_all,
        bar,
        bulk,
        card,
        rows,
        bottom: card.bottom + PAD * s,
    }
}

pub fn layout(r: RectF, s: f32, m: &TransfersModel) -> Vec<Hit> {
    let p = parts(r, s, m);
    let mut hits = Vec::new();
    for (k, rp) in p.rows.iter().enumerate() {
        hits.push(Hit {
            id: CHECK_BASE + k as u32,
            r: rp.check,
        });
        for (slot, b) in rp.btns.iter().enumerate().take(SLOTS as usize) {
            hits.push(Hit {
                id: ROW_BASE + k as u32 * SLOTS + slot as u32,
                r: *b,
            });
        }
    }
    for (i, kind) in BULK.iter().enumerate() {
        if let Some((_, rect)) = p.bulk.iter().find(|(b, _)| b == kind) {
            hits.push(Hit {
                id: BULK_BASE + i as u32,
                r: *rect,
            });
        }
    }
    for (id, rect) in [
        (SELECT_ALL, p.select_all),
        (QUEUE_BUTTON, p.queue),
        (CANCEL_ALL, p.cancel_all),
    ] {
        if let Some(r) = rect {
            hits.push(Hit { id, r });
        }
    }
    hits
}

/// `width` in pixels; the height in logical px.
pub fn content_height(width: f32, s: f32, m: &TransfersModel) -> f32 {
    parts(rectf(0.0, 0.0, width, 0.0), s, m).bottom / s
}

fn checkbox(c: &Canvas, r: RectF, on: bool) {
    let (s, pal) = (c.s, c.p);
    if on {
        round_rect(c, &r, Some((pal.primary, 1.0)), None, 4.0 * s, false);
        fluent::icon(c, fluent::glyph::CHECK, r, pal.on_primary);
    } else {
        round_rect(c, &r, None, Some(pal.outline_strong), 4.0 * s, false);
    }
}

pub fn draw(c: &mut Canvas, r: RectF, m: &TransfersModel) {
    let (s, pal) = (c.s, c.p);
    let p = parts(r, s, m);
    text(c, Font::Title, "Transfers", p.title, pal.text, Align::Left);
    if let Some(b) = p.select_all {
        let label = if m.all_selected() {
            "Clear selection"
        } else {
            "Select all"
        };
        fluent::button(c, b, label, false);
    }
    if let Some(q) = p.queue {
        fluent::button(c, q, queue_label(m), m.paused);
    }
    if let Some(b) = p.cancel_all {
        fluent::button(c, b, "Cancel all", false);
    }
    if let Some(bar) = p.bar {
        let label = format!("{} selected", m.selected.len());
        let t = rectf(
            bar.left,
            bar.top,
            p.bulk.first().map_or(bar.right, |(_, b)| b.left),
            bar.bottom,
        );
        text(c, Font::Body, &label, t, pal.text, Align::Left);
        for (kind, rect) in &p.bulk {
            fluent::button(c, *rect, kind.label(), false);
        }
    }
    widgets::card(c, p.card);
    if m.count() == 0 {
        let (title, hint) = empty_text();
        widgets::empty_state(c, p.card, fluent::glyph::SEND, title, hint);
        return;
    }
    let keys = m.keys();
    for (k, rp) in p.rows.iter().enumerate() {
        let selected = keys.get(k).is_some_and(|key| m.selected.contains(key));
        checkbox(c, rp.check, selected);
        let (name, line, outgoing, bad, progress) = match m.rows.get(k) {
            Some(t) => (
                t.name.clone(),
                if t.kind == Kind::Running {
                    progress_text(t)
                } else {
                    t.status.clone()
                },
                t.outgoing,
                t.kind == Kind::Failed,
                matches!(t.kind, Kind::Running | Kind::Waiting).then_some(t.percent),
            ),
            None => match m.queued.get(k - m.rows.len()) {
                Some(q) => (q.name.clone(), q.status.clone(), true, false, None),
                None => continue,
            },
        };
        let row = rp.row;
        let pad = fluent::ROW_PAD as f32 * s;
        let cy = (row.top + row.bottom) / 2.0;
        let g = rectf(
            rp.check.right + SPACE[2] * s,
            cy - 10.0 * s,
            rp.check.right + SPACE[2] * s + 20.0 * s,
            cy + 10.0 * s,
        );
        let _ = pad;
        let glyph = if outgoing {
            fluent::glyph::UPLOAD
        } else {
            fluent::glyph::SEND
        };
        fluent::icon(c, glyph, g, if bad { pal.danger } else { pal.text2 });
        let tx = g.right + SPACE[2] * s;
        let tr = (rp.btns.first().map_or(row.right, |b| b.left) - SPACE[2] * s).max(tx);
        let name_r = rectf(tx, row.top + 10.0 * s, tr, row.top + 32.0 * s);
        text(c, Font::Body, &name, name_r, pal.text, Align::Left);
        let meta = rectf(tx, name_r.bottom, tr, name_r.bottom + 18.0 * s);
        text(
            c,
            Font::Caption,
            &line,
            meta,
            if bad { pal.danger } else { pal.text2 },
            Align::Left,
        );
        if let Some(percent) = progress {
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
            let w = (bar.right - bar.left) * percent.min(100) as f32 / 100.0;
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
        for (b, rect) in buttons_at(m, k).iter().zip(&rp.btns) {
            fluent::button(c, *rect, b.label(), *b == RowBtn::Retry);
        }
    }
}

fn queue_label(m: &TransfersModel) -> &'static str {
    if m.paused {
        "Resume queue"
    } else {
        "Pause queue"
    }
}

pub fn describe(hit: &Hit, m: &TransfersModel) -> Option<Desc> {
    match hit.id {
        QUEUE_BUTTON => return Some(Desc::new(queue_label(m).to_owned(), Role::Button)),
        CANCEL_ALL => return Some(Desc::new("Cancel all", Role::Button)),
        SELECT_ALL => {
            let mut d = Desc::new("Select all", Role::Toggle);
            d.toggled = Some(m.all_selected());
            return Some(d);
        }
        id if (BULK_BASE..BULK_BASE + 4).contains(&id) => {
            return Some(Desc::new(
                BULK[(id - BULK_BASE) as usize].name(),
                Role::Button,
            ));
        }
        id if (CHECK_BASE..ROW_BASE).contains(&id) => {
            let p = (id - CHECK_BASE) as usize;
            let key = m.key_at(p)?;
            let mut d = Desc::new(format!("Select {}", name_at(m, p)), Role::Toggle);
            d.toggled = Some(m.selected.contains(&key));
            return Some(d);
        }
        _ => {}
    }
    let (p, slot) = row_slot(hit.id)?;
    let b = *buttons_at(m, p).get(slot)?;
    Some(Desc::new(
        format!("{} {}", b.label(), name_at(m, p)),
        Role::Button,
    ))
}

fn row_slot(id: u32) -> Option<(usize, usize)> {
    let n = id.checked_sub(ROW_BASE)?;
    Some(((n / SLOTS) as usize, (n % SLOTS) as usize))
}

/// Everything the bar does ends by unticking the rows it acted on.
fn then_clear(mut v: Vec<Action>) -> Option<Action> {
    if v.is_empty() {
        return None;
    }
    v.push(Action::ClearSelection);
    Some(Action::Many(v))
}

pub fn click(hit: &Hit, m: &TransfersModel) -> Option<Action> {
    match hit.id {
        QUEUE_BUTTON => {
            return Some(if m.paused {
                Action::QueueResume
            } else {
                Action::QueuePause
            })
        }
        CANCEL_ALL => return Some(Action::QueueCancelAll),
        SELECT_ALL => {
            return Some(if m.all_selected() {
                Action::ClearSelection
            } else {
                Action::SelectAll(m.keys())
            })
        }
        id if (BULK_BASE..BULK_BASE + 4).contains(&id) => {
            return then_clear(bulk_actions(m, BULK[(id - BULK_BASE) as usize]));
        }
        id if (CHECK_BASE..ROW_BASE).contains(&id) => {
            return m
                .key_at((id - CHECK_BASE) as usize)
                .map(Action::ToggleSelect);
        }
        _ => {}
    }
    let (p, slot) = row_slot(hit.id)?;
    let b = *buttons_at(m, p).get(slot)?;
    row_action(m, p, b)
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
            path: Some(PathBuf::from(format!(r"C:\file{id}.bin"))),
        }
    }

    fn page() -> crate::fluent::RectF {
        rectf(0.0, 0.0, 800.0, 2000.0)
    }

    fn queued(id: u64, status: &str) -> QueuedRow {
        QueuedRow {
            peer: "phone-1".into(),
            id,
            name: format!("queued{id}.bin"),
            status: status.into(),
            held: status == "Paused",
            needs_file: status == "Needs the file again",
        }
    }

    fn hit_for(m: &TransfersModel, name: &str) -> Hit {
        layout(page(), 1.0, m)
            .into_iter()
            .find(|h| describe(h, m).is_some_and(|d| d.name == name))
            .unwrap_or_else(|| panic!("no control named {name}"))
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
            queued: vec![queued(5, "Waiting"), queued(6, "Paused")],
            selected: vec![SelKey::Transfer(1)],
            ..Default::default()
        };
        let hits = layout(page(), 1.0, &m);
        assert!(hits.len() > 10);
        for h in &hits {
            let d = describe(h, &m).expect("described");
            assert!(!d.name.is_empty());
        }
    }

    #[test]
    fn each_row_offers_what_fits_its_state() {
        use RowBtn::*;
        assert_eq!(
            transfer_buttons(&row(1, Kind::Running, 1, 10)),
            vec![Pause, Cancel]
        );
        let mut incoming = row(2, Kind::Running, 1, 10);
        incoming.outgoing = false;
        assert_eq!(
            transfer_buttons(&incoming),
            vec![Cancel],
            "no Pause for what arrives"
        );
        assert_eq!(
            transfer_buttons(&row(3, Kind::Waiting, 0, 10)),
            vec![Cancel]
        );
        assert_eq!(
            transfer_buttons(&row(4, Kind::Failed, 1, 10)),
            vec![Retry, Remove]
        );
        assert_eq!(transfer_buttons(&row(5, Kind::Done, 10, 10)), vec![Remove]);
        assert_eq!(
            queued_buttons(&queued(1, "Waiting"), false),
            vec![Pause, Remove]
        );
        assert_eq!(
            queued_buttons(&queued(2, "Paused"), false),
            vec![Resume, Remove]
        );
        assert_eq!(
            queued_buttons(&queued(3, "Waiting"), true),
            vec![Remove],
            "the whole queue is paused already"
        );
        assert_eq!(
            queued_buttons(&queued(4, "Needs the file again"), false),
            vec![Remove]
        );
    }

    #[test]
    fn row_buttons_do_what_they_say() {
        let m = TransfersModel {
            rows: vec![row(7, Kind::Running, 1, 10), row(9, Kind::Failed, 1, 10)],
            queued: vec![queued(5, "Waiting"), queued(6, "Paused")],
            ..Default::default()
        };
        let a = |name: &str| click(&hit_for(&m, name), &m);
        assert_eq!(
            a("Pause file7.bin"),
            Some(Action::TransferHold(PathBuf::from(r"C:\file7.bin")))
        );
        assert_eq!(a("Cancel file7.bin"), Some(Action::TransferCancel(7)));
        assert_eq!(a("Retry file9.bin"), Some(Action::TransferRetry(9)));
        assert_eq!(a("Remove file9.bin"), Some(Action::TransferDismiss(9)));
        assert_eq!(
            a("Pause queued5.bin"),
            Some(Action::QueueHold("phone-1".into(), 5))
        );
        assert_eq!(
            a("Resume queued6.bin"),
            Some(Action::QueueRelease("phone-1".into(), 6))
        );
        assert_eq!(
            a("Remove queued6.bin"),
            Some(Action::QueueRemove("phone-1".into(), 6))
        );
    }

    #[test]
    fn the_queue_button_pauses_then_resumes_and_cancel_all_clears() {
        let mut m = TransfersModel {
            rows: vec![row(7, Kind::Running, 1, 10)],
            ..Default::default()
        };
        assert_eq!(
            click(&hit_for(&m, "Pause queue"), &m),
            Some(Action::QueuePause)
        );
        assert_eq!(
            click(&hit_for(&m, "Cancel all"), &m),
            Some(Action::QueueCancelAll)
        );
        m.paused = true;
        assert_eq!(
            click(&hit_for(&m, "Resume queue"), &m),
            Some(Action::QueueResume)
        );
        assert!(
            layout(page(), 1.0, &TransfersModel::default()).is_empty(),
            "nothing to pause, no buttons"
        );
    }

    #[test]
    fn a_page_with_only_queued_files_is_not_the_empty_state() {
        let m = TransfersModel {
            queued: vec![queued(1, "Paused")],
            paused: true,
            ..Default::default()
        };
        assert!(content_height(800.0, 1.0, &m) > 0.0);
        for x in layout(page(), 1.0, &m) {
            assert!(x.r.bottom <= content_height(page().right, 1.0, &m) + 0.5);
        }
        hit_for(&m, "Resume queued1.bin");
        hit_for(&m, "Remove queued1.bin");
    }

    #[test]
    fn ticking_rows_shows_the_bar_and_it_acts_on_exactly_those_rows() {
        let mut m = TransfersModel {
            rows: vec![
                row(1, Kind::Running, 1, 10),
                row(2, Kind::Waiting, 0, 10),
                row(3, Kind::Done, 10, 10),
            ],
            queued: vec![queued(5, "Waiting"), queued(6, "Paused")],
            ..Default::default()
        };
        let no_bar = layout(page(), 1.0, &m);
        assert!(
            !no_bar
                .iter()
                .any(|h| describe(h, &m).is_some_and(|d| d.name.ends_with(" selected"))),
            "nothing ticked, no bar"
        );
        assert_eq!(
            click(&hit_for(&m, "Select file1.bin"), &m),
            Some(Action::ToggleSelect(SelKey::Transfer(1)))
        );
        m.selected = vec![
            SelKey::Transfer(1),
            SelKey::Transfer(3),
            SelKey::Queued("phone-1".into(), 5),
            SelKey::Queued("phone-1".into(), 6),
        ];
        assert_eq!(
            bulk_actions(&m, Bulk::Pause),
            vec![
                Action::TransferHold(PathBuf::from(r"C:\file1.bin")),
                Action::QueueHold("phone-1".into(), 5)
            ]
        );
        assert_eq!(
            bulk_actions(&m, Bulk::Resume),
            vec![Action::QueueRelease("phone-1".into(), 6)]
        );
        assert_eq!(
            bulk_actions(&m, Bulk::Cancel),
            vec![
                Action::TransferCancel(1),
                Action::QueueRemove("phone-1".into(), 5),
                Action::QueueRemove("phone-1".into(), 6)
            ],
            "a queued file's Cancel is its Remove"
        );
        assert_eq!(
            bulk_actions(&m, Bulk::Remove),
            vec![
                Action::TransferDismiss(3),
                Action::QueueRemove("phone-1".into(), 5),
                Action::QueueRemove("phone-1".into(), 6)
            ]
        );
        // The bar's buttons exist and each ends by unticking.
        let pause = click(&hit_for(&m, "Pause selected"), &m);
        match pause {
            Some(Action::Many(v)) => {
                assert_eq!(v.last(), Some(&Action::ClearSelection));
                assert_eq!(v.len(), 3);
            }
            other => panic!("{other:?}"),
        }
        // Unticked rows are never touched.
        m.selected = vec![SelKey::Transfer(2)];
        assert_eq!(
            bulk_actions(&m, Bulk::Pause),
            vec![],
            "a waiting transfer has no Pause"
        );
        assert_eq!(
            bulk_actions(&m, Bulk::Cancel),
            vec![Action::TransferCancel(2)]
        );
    }

    #[test]
    fn select_all_ticks_everything_then_clears_and_checkboxes_tell_their_state() {
        let mut m = TransfersModel {
            rows: vec![row(1, Kind::Running, 1, 10)],
            queued: vec![queued(5, "Waiting")],
            ..Default::default()
        };
        assert_eq!(
            click(&hit_for(&m, "Select all"), &m),
            Some(Action::SelectAll(vec![
                SelKey::Transfer(1),
                SelKey::Queued("phone-1".into(), 5)
            ]))
        );
        m.selected = m.keys();
        assert_eq!(
            click(&hit_for(&m, "Select all"), &m),
            Some(Action::ClearSelection)
        );
        let h = hit_for(&m, "Select file1.bin");
        assert_eq!(describe(&h, &m).unwrap().toggled, Some(true));
        m.selected.clear();
        let h = hit_for(&m, "Select file1.bin");
        assert_eq!(describe(&h, &m).unwrap().toggled, Some(false));
    }

    #[test]
    fn hits_stay_inside_the_content_height() {
        let m = TransfersModel {
            rows: (1..=6).map(|i| row(i, Kind::Done, 10, 10)).collect(),
            selected: vec![SelKey::Transfer(2)],
            ..Default::default()
        };
        let h = content_height(800.0, 1.5, &m);
        for x in layout(rectf(0.0, 0.0, 800.0, 10.0), 1.5, &m) {
            assert!(x.r.bottom <= h * 1.5 + 0.5);
        }
    }
}
