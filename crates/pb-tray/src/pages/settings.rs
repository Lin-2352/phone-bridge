//! Settings: the pages of `settings_groups` in a second column, the search
//! box at the top and the option rows of `settings_rows` to the right. The
//! title, the page links and the search box stay put; only the rows scroll.
//! While the query is not empty the matches from every page replace the
//! page. Controls report `settings_rows::Action`s, which the tray applies as
//! the old settings window's were.
#![allow(unsafe_code)]

use crate::{
    app_name::APP_NAME,
    camera_status::{CameraButton, Mark},
    fluent::{self, rectf, round_rect, text, text_width, text_wrapped, Align, Canvas, Font, RectF},
    pages::{Action, Hit},
    settings_groups::{label_of, page_of, page_title_of, pages, search},
    settings_rows::{self, rows, Action as SAction, CameraAct, ChoiceKey, Key, Model, Row},
    tokens::{RADIUS, SPACE},
    widgets,
};

pub use crate::settings_groups::Page;

/// Longest query kept (characters).
pub const QUERY_MAX: usize = 64;

/// The query as kept: at most `QUERY_MAX` characters (never half of one).
pub fn clamp_query(q: &str) -> String {
    q.chars().take(QUERY_MAX).collect()
}

#[derive(Clone, Debug, PartialEq)]
pub struct SettingsModel {
    pub rows: Model,
    /// The page picked in the second column.
    pub sub: Page,
    /// The search box text.
    pub query: String,
}

impl Default for SettingsModel {
    fn default() -> Self {
        Self {
            rows: Model::default(),
            sub: Page::Devices,
            query: String::new(),
        }
    }
}

/// What the list shows: the page title and the option rows on the page.
#[derive(Clone, Debug, PartialEq)]
pub enum Item {
    /// Page title.
    Caption(&'static str),
    /// Small page name above the search results that sit on that page.
    PageName(&'static str),
    /// Nothing to show.
    Message(&'static str),
    Row(Row),
}

/// The rows of one page, under its title. A section header is dropped: the
/// page link already names the section. While `query` is not empty the
/// matches from every page replace the page, each under its page name.
pub fn items(m: &Model, page: Page, query: &str) -> Vec<Item> {
    if !query.trim().is_empty() {
        let all = rows(m);
        let hits = search(&all, m, query);
        if hits.is_empty() {
            return vec![Item::Message("No settings match")];
        }
        let mut out = Vec::new();
        let mut last = "";
        for i in hits {
            let r = &all[i];
            let t = page_title_of(r);
            if t != last {
                out.push(Item::PageName(t));
                last = t;
            }
            out.push(Item::Row(r.clone()));
        }
        return out;
    }
    let title = pages()
        .iter()
        .find(|(p, _)| *p == page)
        .map_or("", |(_, t)| *t);
    let mut out = vec![Item::Caption(title)];
    out.extend(
        rows(m)
            .into_iter()
            .filter(|r| label_of(r).map(page_of) == Some(page))
            .map(Item::Row),
    );
    out
}

/// `path` as it fits (`fits` measures): whole, or with folders from the
/// middle replaced by "…" so the drive (or `\\server\share`) and the last
/// folders stay; at worst "…\" and the last folder alone.
pub fn shorten_path(path: &str, fits: impl Fn(&str) -> bool) -> String {
    if fits(path) || !path.contains('\\') {
        return path.to_owned();
    }
    let parts: Vec<&str> = path.split('\\').collect();
    let head_n = if path.starts_with(r"\\") {
        4.min(parts.len() - 1)
    } else {
        1
    };
    let (head, body) = parts.split_at(head_n);
    let head = head.join("\\");
    let last = body.last().copied().unwrap_or_default();
    for keep in (1..body.len()).rev() {
        let t = format!("{head}\\\u{2026}\\{}", body[body.len() - keep..].join("\\"));
        if fits(&t) {
            return t;
        }
    }
    format!("\u{2026}\\{last}")
}

/// Sizes in logical px. `PANE_PAD` is the window's padding around the
/// content rect (`main_window::frame`).
const PANE_PAD: f32 = SPACE[4];
const PAD: f32 = 24.0;
const TITLE_H: f32 = 40.0;
const COL_W: f32 = 168.0;
const COL_GAP: f32 = 16.0;
const LINK_H: f32 = 40.0;
const LINK_GAP: f32 = 4.0;
/// The search box sits in the title's line, 2 px down.
const SEARCH_TOP: f32 = 2.0;
const SEARCH_H: f32 = 36.0;
/// From the search box to the first row.
const LIST_GAP: f32 = 12.0;
const HEAD_H: f32 = SEARCH_TOP + SEARCH_H + LIST_GAP;
const ITEM_GAP: f32 = 4.0;
const INSET: f32 = 16.0;
const BUTTON_H: f32 = 32.0;

fn item_height(i: &Item) -> f32 {
    match i {
        Item::Caption(..) => 44.0,
        Item::PageName(..) => 32.0,
        Item::Message(..) => 56.0,
        Item::Row(r) => match r {
            Row::Header(_) => 40.0,
            Row::Theme | Row::Look => 64.0,
            Row::Toggle(..) => 68.0,
            Row::Choice(..) => 100.0,
            Row::App(_) => 56.0,
            Row::Folder => 76.0,
            Row::Device(_) => 64.0,
            Row::Pair => 60.0,
            Row::About => 68.0,
            Row::CameraStatus => 120.0,
            Row::CameraWindow => 68.0,
            Row::ClearHistory => 68.0,
            Row::LogFolder => 76.0,
            Row::ClearLogs => 68.0,
            Row::Updates => 68.0,
        },
    }
}

/// Hit ids. Page links, the theme, toggles and choices by position in fixed
/// lists; an app or a device by a hash of what it is, so focus or a click in
/// flight stays on it when the list changes around it.
const LINK: u32 = 1;
const THEME: u32 = 10;
const LOOK: u32 = 13;
const TOGGLE: u32 = 20;
const CHOICE: u32 = 44;
const OPEN_FOLDER: u32 = 50;
const CHANGE_FOLDER: u32 = 51;
const PAIR: u32 = 52;
const CAMERA: u32 = 53;
const REPAIR: u32 = 54;
const CAMERA_WINDOW: u32 = 55;
const CLEAR_HISTORY: u32 = 56;
const OPEN_LOGS: u32 = 57;
const CLEAR_LOGS: u32 = 58;
const CHECK_UPDATES: u32 = 59;
const APP_BIT: u32 = 0x1000_0000;
const DEVICE_BIT: u32 = 0x2000_0000;
const KEY_MASK: u32 = 0x0FFF_FFFF;

const KEYS: [Key; 23] = [
    Key::Share,
    Key::ShareImages,
    Key::Passwords,
    Key::Keep,
    Key::AutoCopyText,
    Key::ToastShowText,
    Key::Auto,
    Key::SendTo,
    Key::Startup,
    Key::Control,
    Key::UnlockGate,
    Key::GateFiles,
    Key::GateAudio,
    Key::GateControl,
    Key::Notify,
    Key::NotifyLock,
    Key::NotifyClear,
    Key::NotifyOpen,
    Key::NotifyOtp,
    Key::NotifyQuietMirror,
    Key::KeepHistory,
    Key::KeepLogs,
    Key::AudioSkipConfirm,
];

/// Every key is in `KEYS` (a test checks), so this never falls back.
fn toggle_id(k: Key) -> u32 {
    let i = KEYS.iter().position(|x| *x == k);
    debug_assert!(i.is_some(), "{k:?} missing from KEYS");
    TOGGLE + i.unwrap_or(KEYS.len()) as u32
}

fn choice_id(k: ChoiceKey, i: u8) -> u32 {
    let base = match k {
        ChoiceKey::Content => 0,
        ChoiceKey::Sound => 3,
    };
    CHOICE + base + u32::from(i)
}

fn hash28(v: impl std::hash::Hash) -> u32 {
    use std::hash::Hasher;
    let mut h = std::collections::hash_map::DefaultHasher::new();
    v.hash(&mut h);
    (h.finish() as u32) & KEY_MASK
}

/// The app at `i`: its name, and which of the apps with that name it is.
fn app_id(m: &Model, i: usize) -> u32 {
    let name = &m.apps[i].0;
    let nth = m.apps[..i].iter().filter(|a| a.0 == *name).count();
    APP_BIT | hash28((name, nth))
}

/// The device at `i`: its name and fingerprint (and which of the same).
fn device_id(m: &Model, i: usize) -> u32 {
    let (name, fp, _) = &m.devices[i];
    let nth = m.devices[..i]
        .iter()
        .filter(|d| d.0 == *name && d.1 == *fp)
        .count();
    DEVICE_BIT | hash28((name, fp, nth))
}

/// Whether a toggle can be flipped now (sub-options wait for their switch).
fn toggle_enabled(m: &Model, k: Key) -> bool {
    match k {
        Key::UnlockGate => m.unlock_available,
        Key::GateFiles | Key::GateAudio | Key::GateControl => m.unlock_available && m.unlock_gate,
        Key::Passwords | Key::ShareImages => m.share,
        Key::NotifyLock
        | Key::NotifyClear
        | Key::NotifyOpen
        | Key::NotifyOtp
        | Key::NotifyQuietMirror => m.notify,
        _ => true,
    }
}

/// Where the fixed parts begin: the content rect's unscrolled top plus the
/// page padding, from the pane (`view`) so scrolling never moves them.
fn fixed_top(view: RectF, s: f32) -> f32 {
    view.top + (PANE_PAD + PAD) * s
}

/// (links left, links right, rows left, rows right) in pixels.
fn columns(r: RectF, s: f32) -> (f32, f32, f32, f32) {
    let col_l = r.left + PAD * s;
    let col_r = col_l + COL_W * s;
    let list_l = col_r + COL_GAP * s;
    let list_r = (r.right - PAD * s).max(list_l);
    (col_l, col_r, list_l, list_r)
}

/// The search box's frame in client pixels: above the rows, in the title's
/// line. `r` is the content rect (scrolled or not: only its sides count).
pub fn search_rect(r: RectF, view: RectF, s: f32) -> RectF {
    let (_, _, l, rr) = columns(r, s);
    let top = fixed_top(view, s) + SEARCH_TOP * s;
    rectf(l, top, rr, top + SEARCH_H * s)
}

/// What the rows show through: below the search box, to the pane's bottom.
fn list_area(r: RectF, view: RectF, s: f32) -> RectF {
    let sb = search_rect(r, view, s);
    rectf(sb.left, sb.bottom + LIST_GAP * s, sb.right, view.bottom)
}

fn link_rect(r: RectF, view: RectF, s: f32, k: usize) -> RectF {
    let (l, rr, _, _) = columns(r, s);
    let top = fixed_top(view, s) + (TITLE_H + SPACE[1] + k as f32 * (LINK_H + LINK_GAP)) * s;
    rectf(l, top, rr, top + LINK_H * s)
}

/// The list's items with their cards, scrolled with `r`.
fn placed(r: RectF, s: f32, m: &SettingsModel) -> Vec<(Item, RectF)> {
    let (_, _, l, rr) = columns(r, s);
    let mut y = r.top + (PAD + HEAD_H) * s;
    items(&m.rows, m.sub, &m.query)
        .into_iter()
        .map(|i| {
            let h = item_height(&i) * s;
            let card = rectf(l, y, rr, y + h);
            y += h + ITEM_GAP * s;
            (i, card)
        })
        .collect()
}

/// `width` in pixels; the height of the scrolling rows in logical px.
pub fn content_height(width: f32, s: f32, m: &SettingsModel) -> f32 {
    let _ = (width, s);
    let list: f32 = items(&m.rows, m.sub, &m.query)
        .iter()
        .map(|i| item_height(i) + ITEM_GAP)
        .sum();
    PAD + HEAD_H + list + PAD
}

/// Inside a card: (left, right) of its contents.
fn inner(card: RectF, s: f32) -> (f32, f32) {
    let l = card.left + INSET * s;
    (l, (card.right - INSET * s).max(l))
}

/// A `w` x 32 button at the right of the card, `top` px down.
fn right_button(card: RectF, s: f32, w: f32, top: f32) -> RectF {
    let (l, rr) = inner(card, s);
    let t = card.top + top * s;
    rectf((rr - w * s).max(l), t, rr, t + BUTTON_H * s)
}

/// The three theme buttons, right-aligned in the card's middle.
fn theme_buttons(card: RectF, s: f32) -> [RectF; 3] {
    let (l, rr) = inner(card, s);
    let seg = (88.0 * s).min((rr - l) * 0.66 / 3.0);
    let cy = (card.top + card.bottom) / 2.0;
    std::array::from_fn(|k| {
        let x = rr - seg * (3 - k) as f32;
        rectf(x, cy - 16.0 * s, x + seg - 4.0 * s, cy + 16.0 * s)
    })
}

/// The two look buttons (native glass, Relay glass), right-aligned in the card's middle.
fn look_buttons(card: RectF, s: f32) -> [RectF; 2] {
    let (l, rr) = inner(card, s);
    let seg = (128.0 * s).min((rr - l) * 0.66 / 2.0);
    let cy = (card.top + card.bottom) / 2.0;
    std::array::from_fn(|k| {
        let x = rr - seg * (2 - k) as f32;
        rectf(x, cy - 16.0 * s, x + seg - 4.0 * s, cy + 16.0 * s)
    })
}

/// The three choice buttons across the card's lower half.
fn choice_buttons(card: RectF, s: f32) -> [RectF; 3] {
    let (l, rr) = inner(card, s);
    let seg = (rr - l) / 3.0;
    let t = card.top + 50.0 * s;
    std::array::from_fn(|k| {
        let x = l + seg * k as f32;
        rectf(x, t, (x + seg - 6.0 * s).max(x), t + 34.0 * s)
    })
}

/// (Open, Change) of the folder row.
fn folder_buttons(card: RectF, s: f32) -> (RectF, RectF) {
    let change = right_button(card, s, 76.0, 22.0);
    let (l, _) = inner(card, s);
    let open = rectf(
        (change.left - 84.0 * s).max(l),
        change.top,
        (change.left - 8.0 * s).max(l),
        change.bottom,
    );
    (open, change)
}

/// (Start / Cancel / Stop, Repair) of the camera status card.
fn camera_buttons(card: RectF, s: f32, repair: bool) -> (RectF, Option<RectF>) {
    let top = if repair { 18.0 } else { 44.0 };
    let b = right_button(card, s, 132.0, top);
    let r = repair.then(|| rectf(b.left, card.top + 66.0 * s, b.right, card.top + 98.0 * s));
    (b, r)
}

fn camera_repair(m: &Model) -> bool {
    m.camera.as_ref().is_some_and(|c| c.repair)
}

/// The clickable parts of a row in `card`, as (id, rect); a disabled
/// control has none.
fn controls(row: &Row, card: RectF, s: f32, m: &Model) -> Vec<(u32, RectF)> {
    match row {
        Row::Header(_) | Row::About => Vec::new(),
        Row::Theme => theme_buttons(card, s)
            .iter()
            .enumerate()
            .map(|(k, b)| (THEME + k as u32, *b))
            .collect(),
        Row::Look => look_buttons(card, s)
            .iter()
            .enumerate()
            .map(|(k, b)| (LOOK + k as u32, *b))
            .collect(),
        Row::Toggle(k, ..) if toggle_enabled(m, *k) => vec![(toggle_id(*k), card)],
        Row::Toggle(..) => Vec::new(),
        Row::Choice(k, ..) if m.notify => choice_buttons(card, s)
            .iter()
            .enumerate()
            .map(|(i, b)| (choice_id(*k, i as u8), *b))
            .collect(),
        Row::Choice(..) => Vec::new(),
        Row::App(i) if m.notify && *i < m.apps.len() => vec![(app_id(m, *i), card)],
        Row::App(_) => Vec::new(),
        Row::Folder => {
            let (open, change) = folder_buttons(card, s);
            vec![(OPEN_FOLDER, open), (CHANGE_FOLDER, change)]
        }
        Row::Device(i) if *i < m.devices.len() => {
            vec![(device_id(m, *i), right_button(card, s, 88.0, 16.0))]
        }
        Row::Device(_) => Vec::new(),
        Row::Pair => vec![(PAIR, right_button(card, s, 128.0, 14.0))],
        Row::CameraStatus => {
            let (b, r) = camera_buttons(card, s, camera_repair(m));
            let mut v = vec![(CAMERA, b)];
            v.extend(r.map(|r| (REPAIR, r)));
            v
        }
        Row::CameraWindow => vec![(CAMERA_WINDOW, right_button(card, s, 100.0, 18.0))],
        Row::ClearHistory => vec![(CLEAR_HISTORY, right_button(card, s, 100.0, 18.0))],
        Row::LogFolder => vec![(OPEN_LOGS, right_button(card, s, 100.0, 18.0))],
        Row::ClearLogs => vec![(CLEAR_LOGS, right_button(card, s, 100.0, 18.0))],
        Row::Updates => vec![(CHECK_UPDATES, right_button(card, s, 100.0, 18.0))],
    }
}

/// The page links, then the controls of the rows in view (cut to the part
/// below the search box).
pub fn layout(r: RectF, view: RectF, s: f32, m: &SettingsModel) -> Vec<Hit> {
    let mut hits: Vec<Hit> = (0..pages().len())
        .map(|k| Hit {
            id: LINK + k as u32,
            r: link_rect(r, view, s, k),
        })
        .collect();
    let area = list_area(r, view, s);
    for (item, card) in placed(r, s, m) {
        let Item::Row(row) = item else { continue };
        let off = card.bottom <= area.top || card.top >= area.bottom;
        for (id, b) in controls(&row, card, s, &m.rows) {
            let cut = rectf(
                b.left,
                b.top.max(area.top),
                b.right,
                b.bottom.min(area.bottom),
            );
            // Every control has a hit so Tab reaches rows below the fold (the
            // window scrolls the focused one into view); one out of view
            // keeps its real rect only when it is outside the pane, else it
            // is empty and takes no clicks.
            if !off && cut.bottom > cut.top {
                hits.push(Hit { id, r: cut });
            } else {
                let y = if b.top >= area.bottom {
                    b.top
                } else {
                    area.top
                };
                hits.push(Hit {
                    id,
                    r: rectf(b.left, y, b.right, y),
                });
            }
        }
    }
    hits
}

/// Where control `id` really is, scrolled with `r` and not clipped (its
/// hit is empty while it is out of view).
pub fn full_rect(id: u32, r: RectF, s: f32, m: &SettingsModel) -> Option<RectF> {
    placed(r, s, m).into_iter().find_map(|(item, card)| {
        let Item::Row(row) = item else { return None };
        controls(&row, card, s, &m.rows)
            .into_iter()
            .find(|(i, _)| *i == id)
            .map(|(_, b)| b)
    })
}

/// The accessible description of a Settings hit: found by walking the
/// same rows and controls the layout uses.
pub fn describe(hit: &Hit, m: &SettingsModel) -> Option<super::Desc> {
    use super::{Desc, Role};
    if (LINK..THEME).contains(&hit.id) {
        let (p, label) = pages().get((hit.id - LINK) as usize).copied()?;
        let mut d = Desc::new(label, Role::Radio);
        d.selected = m.sub == p;
        return Some(d);
    }
    let card = rectf(0.0, 0.0, 800.0, 120.0);
    for item in items(&m.rows, m.sub, &m.query) {
        let Item::Row(row) = item else { continue };
        for (k, (id, _)) in controls(&row, card, 1.0, &m.rows).into_iter().enumerate() {
            if id != hit.id {
                continue;
            }
            let rm = &m.rows;
            return Some(match &row {
                Row::Theme => {
                    let n = ["System", "Light", "Dark"][k.min(2)];
                    let mut d = Desc::new(format!("Theme: {n}"), Role::Radio);
                    d.selected = usize::from(rm.theme) == k;
                    d
                }
                Row::Look => {
                    let n = ["Native glass", "Relay glass"][k.min(1)];
                    let mut d = Desc::new(format!("Look: {n}"), Role::Radio);
                    d.selected = usize::from(rm.look.min(1)) == k;
                    d
                }
                Row::Toggle(key, title, _) => {
                    let mut d = Desc::new(*title, Role::Toggle);
                    d.toggled = Some(rm.get(*key));
                    d
                }
                Row::Choice(key, title, labels) => {
                    let mut d = Desc::new(format!("{title}: {}", labels[k.min(2)]), Role::Radio);
                    d.selected = usize::from(rm.choice(*key)) == k;
                    d
                }
                Row::App(i) => {
                    let (name, shown) = rm.apps.get(*i)?;
                    let mut d = Desc::new(format!("Show notifications from {name}"), Role::Toggle);
                    d.toggled = Some(*shown);
                    d
                }
                Row::Folder => Desc::new(
                    if k == 0 {
                        "Open received files folder"
                    } else {
                        "Change received files folder"
                    },
                    Role::Button,
                ),
                Row::Device(i) => {
                    Desc::new(format!("Remove {}", rm.devices.get(*i)?.0), Role::Button)
                }
                Row::Pair => Desc::new(crate::strings::PAIR_DEVICE, Role::Button),
                Row::CameraStatus => Desc::new(
                    if k == 0 {
                        "Camera: start or stop"
                    } else {
                        "Repair camera"
                    },
                    Role::Button,
                ),
                Row::CameraWindow => Desc::new("Open camera window", Role::Button),
                Row::ClearHistory => Desc::new("Clear history", Role::Button),
                Row::LogFolder => Desc::new("Open log folder", Role::Button),
                Row::ClearLogs => Desc::new("Clear all logs", Role::Button),
                Row::Updates => Desc::new("Check for updates", Role::Button),
                Row::Header(_) | Row::About => return None,
            });
        }
    }
    None
}

/// The action for a click on `hit`; an app or a device is looked up by its
/// id, so a list that changed since the press acts on the same one (or none).
pub fn click(hit: &Hit, m: &SettingsModel) -> Option<Action> {
    let id = hit.id;
    let rm = &m.rows;
    let a = match id {
        _ if id & APP_BIT != 0 => SAction::Mute((0..rm.apps.len()).find(|&i| app_id(rm, i) == id)?),
        _ if id & DEVICE_BIT != 0 => {
            SAction::Remove((0..rm.devices.len()).find(|&i| device_id(rm, i) == id)?)
        }
        LINK..THEME => {
            let (p, _) = pages().get((id - LINK) as usize).copied()?;
            return Some(Action::SettingsPage(p));
        }
        THEME..LOOK => SAction::Theme(u8::try_from(id - THEME).ok().filter(|t| *t <= 2)?),
        LOOK..TOGGLE => SAction::Look(u8::try_from(id - LOOK).ok().filter(|l| *l <= 1)?),
        TOGGLE..CHOICE => SAction::Toggle(*KEYS.get((id - TOGGLE) as usize)?),
        CHOICE..OPEN_FOLDER => {
            let n = id - CHOICE;
            let k = match n {
                0..=2 => ChoiceKey::Content,
                3..=5 => ChoiceKey::Sound,
                _ => return None,
            };
            SAction::Choice(k, (n % 3) as u8)
        }
        OPEN_FOLDER => SAction::OpenFolder,
        CHANGE_FOLDER => SAction::ChangeFolder,
        PAIR => SAction::Pair,
        CAMERA => SAction::Camera(CameraAct::Toggle),
        REPAIR => SAction::Camera(CameraAct::Repair),
        CAMERA_WINDOW => SAction::Camera(CameraAct::OpenWindow),
        CLEAR_HISTORY => SAction::ClearHistory,
        OPEN_LOGS => SAction::OpenLogs,
        CLEAR_LOGS => SAction::ClearLogs,
        CHECK_UPDATES => SAction::CheckUpdates,
        _ => return None,
    };
    Some(Action::Settings(a))
}

/// Title over a caption line, in the card's upper part between `l` and `r`.
fn two_lines(c: &Canvas, card: RectF, l: f32, r: f32, title: &str, sub: &str, rgb: u32) {
    let s = c.s;
    let t = rectf(l, card.top + 13.0 * s, r, card.top + 35.0 * s);
    text(c, Font::Body, title, t, rgb, Align::Left);
    let b = rectf(l, card.top + 35.0 * s, r, card.top + 55.0 * s);
    text(c, Font::Caption, sub, b, c.p.text2, Align::Left);
}

/// A button that takes no clicks: outline only, muted label.
fn disabled_button(c: &Canvas, r: RectF, label: &str) {
    round_rect(c, &r, None, Some(c.p.stroke), RADIUS[0] * c.s, false);
    text(c, Font::Body, label, r, c.p.text2, Align::Center);
}

/// The text column left of a right-hand control at `x`.
fn left_of(il: f32, x: f32, s: f32) -> f32 {
    (x - SPACE[2] * s).max(il)
}

fn draw_row(c: &Canvas, row: &Row, card: RectF, m: &Model) {
    let (s, p) = (c.s, c.p);
    let (il, ir) = inner(card, s);
    let cy = (card.top + card.bottom) / 2.0;
    if !matches!(row, Row::Header(_)) {
        widgets::card(c, card);
    }
    match row {
        Row::Header(t) => {
            let tr = rectf(card.left, card.top + 14.0 * s, card.right, card.bottom);
            text(c, Font::BodyStrong, t, tr, p.text, Align::Left);
        }
        Row::Theme => {
            let b = theme_buttons(card, s);
            let tr = rectf(il, card.top, left_of(il, b[0].left, s), card.bottom);
            text(c, Font::Body, "Theme", tr, p.text, Align::Left);
            for (k, label) in ["System", "Light", "Dark"].iter().enumerate() {
                fluent::button(c, b[k], label, m.theme == k as u8);
            }
        }
        Row::Look => {
            let b = look_buttons(card, s);
            let tr = rectf(il, card.top, left_of(il, b[0].left, s), card.bottom);
            text(c, Font::Body, "Look", tr, p.text, Align::Left);
            for (k, label) in ["Native glass", "Relay glass"].iter().enumerate() {
                fluent::button(c, b[k], label, m.look.min(1) == k as u8);
            }
        }
        Row::Toggle(k, title, sub) => {
            let enabled = toggle_enabled(m, *k);
            let rgb = if enabled { p.text } else { p.text2 };
            let right = left_of(il, ir - 40.0 * s, s);
            two_lines(c, card, il, right, title, sub, rgb);
            fluent::toggle(c, ir, cy, m.get(*k), enabled);
        }
        Row::Choice(k, title, labels) => {
            let t = rectf(il, card.top + 12.0 * s, ir, card.top + 38.0 * s);
            let rgb = if m.notify { p.text } else { p.text2 };
            text(c, Font::Body, title, t, rgb, Align::Left);
            for (i, b) in choice_buttons(card, s).iter().enumerate() {
                if m.notify {
                    fluent::button(c, *b, labels[i], m.choice(*k) == i as u8);
                } else {
                    disabled_button(c, *b, labels[i]);
                }
            }
        }
        Row::App(i) => {
            let Some((name, shown)) = m.apps.get(*i) else {
                return;
            };
            let tr = rectf(il, card.top, left_of(il, ir - 40.0 * s, s), card.bottom);
            let rgb = if m.notify { p.text } else { p.text2 };
            text(c, Font::Body, name, tr, rgb, Align::Left);
            fluent::toggle(c, ir, cy, *shown, m.notify);
        }
        Row::Folder => {
            let (open, change) = folder_buttons(card, s);
            let right = left_of(il, open.left, s);
            let t = rectf(il, card.top + 14.0 * s, right, card.top + 36.0 * s);
            text(
                c,
                Font::Body,
                "Save received files to",
                t,
                p.text,
                Align::Left,
            );
            let pr = rectf(il, card.top + 36.0 * s, right, card.top + 58.0 * s);
            let w = pr.right - pr.left;
            let shown = shorten_path(&m.save_dir, |t| text_width(c, Font::Caption, t) <= w);
            text(c, Font::Caption, &shown, pr, p.text2, Align::Left);
            fluent::button(c, open, "Open", false);
            fluent::button(c, change, "Change", true);
        }
        Row::Device(i) => {
            let Some((name, fp, changed)) = m.devices.get(*i) else {
                return;
            };
            let b = right_button(card, s, 88.0, 16.0);
            let g = rectf(il, cy - 10.0 * s, il + 20.0 * s, cy + 10.0 * s);
            let tint = if *changed { p.danger } else { p.primary };
            fluent::icon(c, fluent::glyph::PHONE, g, tint);
            let (l, right) = (g.right + SPACE[2] * s, left_of(g.right, b.left, s));
            let t = rectf(l, card.top + 11.0 * s, right, card.top + 33.0 * s);
            text(c, Font::Body, name, t, p.text, Align::Left);
            let note = settings_rows::device_note(fp, *changed);
            let n = rectf(l, card.top + 33.0 * s, right, card.top + 53.0 * s);
            let rgb = if *changed { p.danger } else { p.text2 };
            text(c, Font::Caption, &note, n, rgb, Align::Left);
            let radius = RADIUS[0] * s;
            round_rect(c, &b, Some((p.card, 1.0)), Some(p.stroke), radius, false);
            text(c, Font::Body, "Remove", b, p.danger, Align::Center);
        }
        Row::Pair => {
            let b = right_button(card, s, 128.0, 14.0);
            let label = if m.devices.is_empty() {
                "No device paired yet"
            } else {
                "Add another device"
            };
            let tr = rectf(il, card.top, left_of(il, b.left, s), card.bottom);
            text(c, Font::Body, label, tr, p.text, Align::Left);
            fluent::button(c, b, crate::strings::PAIR_DEVICE, true);
        }
        Row::About => {
            let version = format!("{} {}", APP_NAME, env!("CARGO_PKG_VERSION"));
            let fp = format!("This PC's fingerprint {}", m.my_fingerprint);
            two_lines(c, card, il, ir, &version, &fp, p.text);
        }
        Row::CameraStatus => {
            let (mark, title, detail, button, repair) = match &m.camera {
                Some(x) => (
                    x.mark,
                    x.title.as_str(),
                    x.detail.as_str(),
                    x.button,
                    x.repair,
                ),
                None => (
                    Mark::Idle,
                    "Camera is off",
                    "Use your phone as a webcam in any app",
                    CameraButton::Start,
                    false,
                ),
            };
            let (b, rb) = camera_buttons(card, s, repair);
            let dot = rectf(il, card.top + 19.0 * s, il + 10.0 * s, card.top + 29.0 * s);
            let colour = match mark {
                Mark::Ok => p.primary,
                Mark::Bad => p.danger,
                Mark::Wait | Mark::Idle => p.text2,
            };
            round_rect(c, &dot, Some((colour, 1.0)), None, 5.0 * s, false);
            let l = il + 22.0 * s;
            let right = left_of(l, b.left, s);
            let t = rectf(l, card.top + 12.0 * s, right, card.top + 36.0 * s);
            text(c, Font::Body, title, t, p.text, Align::Left);
            let d = rectf(l, card.top + 40.0 * s, right, card.bottom - 10.0 * s);
            text_wrapped(c, Font::Caption, detail, d, p.text2);
            let label = match button {
                CameraButton::Start => "Start camera",
                CameraButton::Cancel => "Cancel",
                CameraButton::Stop => "Stop camera",
            };
            fluent::button(c, b, label, true);
            if let Some(rb) = rb {
                fluent::button(c, rb, "Repair camera", false);
            }
        }
        Row::CameraWindow => {
            let b = right_button(card, s, 100.0, 18.0);
            let right = left_of(il, b.left, s);
            let sub = "Preview and the full checklist";
            two_lines(c, card, il, right, "Camera window", sub, p.text);
            fluent::button(c, b, "Open", false);
        }
        Row::ClearHistory => {
            let b = right_button(card, s, 100.0, 18.0);
            let right = left_of(il, b.left, s);
            let sub = "Delete everything in the Timeline";
            two_lines(c, card, il, right, "Clear history", sub, p.text);
            fluent::button(c, b, "Clear", false);
        }
        Row::LogFolder => {
            let b = right_button(card, s, 100.0, 18.0);
            let right = left_of(il, b.left, s);
            let w = right - il;
            let shown = shorten_path(&m.log_folder_text(), |t| {
                text_width(c, Font::Caption, t) <= w
            });
            two_lines(c, card, il, right, "Log folder", &shown, p.text);
            fluent::button(c, b, "Open", false);
        }
        Row::ClearLogs => {
            let b = right_button(card, s, 100.0, 18.0);
            let right = left_of(il, b.left, s);
            let sub = format!(
                "Delete every log file ({})",
                settings_rows::log_size_text(m.log_bytes)
            );
            two_lines(c, card, il, right, "Clear all logs", &sub, p.text);
            fluent::button(c, b, "Clear", false);
        }
        Row::Updates => {
            let b = right_button(card, s, 100.0, 18.0);
            let right = left_of(il, b.left, s);
            two_lines(
                c,
                card,
                il,
                right,
                "Check for updates",
                &m.update_note,
                p.text,
            );
            fluent::button(
                c,
                b,
                if m.update_open {
                    "Open page"
                } else {
                    "Check now"
                },
                false,
            );
        }
    }
}

/// The title, the page links and the rows in view. The window draws the
/// search box's frame (it knows the focus) and owns the edit inside it.
pub fn draw(c: &mut Canvas, r: RectF, view: RectF, m: &SettingsModel, _hits: &[Hit]) {
    let (s, p) = (c.s, c.p);
    let (col_l, col_r, _, _) = columns(r, s);
    let top = fixed_top(view, s);
    let title = rectf(col_l, top, col_r, top + TITLE_H * s);
    text(c, Font::Title, "Settings", title, p.text, Align::Left);
    let searching = !m.query.trim().is_empty();
    for (k, (page, name)) in pages().iter().enumerate() {
        let lr = link_rect(r, view, s, k);
        let on = *page == m.sub && !searching;
        if on {
            round_rect(c, &lr, Some((p.hover, 1.0)), None, RADIUS[1] * s, false);
            let bar = rectf(
                lr.left,
                lr.top + 10.0 * s,
                lr.left + 3.0 * s,
                lr.bottom - 10.0 * s,
            );
            round_rect(c, &bar, Some((p.primary, 1.0)), None, 1.5 * s, false);
        }
        let t = rectf(lr.left + 16.0 * s, lr.top, lr.right, lr.bottom);
        let rgb = if on { p.text } else { p.text2 };
        text(c, Font::Body, name, t, rgb, Align::Left);
    }
    let area = list_area(r, view, s);
    if area.bottom <= area.top {
        return;
    }
    // SAFETY: clip push/pop pair inside the surface's BeginDraw/EndDraw.
    unsafe {
        c.dc.PushAxisAlignedClip(
            &area,
            windows::Win32::Graphics::Direct2D::D2D1_ANTIALIAS_MODE_ALIASED,
        )
    };
    for (item, card) in placed(r, s, m) {
        if card.bottom <= area.top || card.top >= area.bottom {
            continue;
        }
        match item {
            Item::Caption(t) => text(c, Font::Subtitle, t, card, p.text, Align::Left),
            Item::PageName(t) => {
                let tr = rectf(card.left, card.top + SPACE[1] * s, card.right, card.bottom);
                text(c, Font::Caption, t, tr, p.text2, Align::Left);
            }
            Item::Message(t) => text(c, Font::Body, t, card, p.text2, Align::Left),
            Item::Row(row) => draw_row(c, &row, card, &m.rows),
        }
    }
    // SAFETY: pops the clip pushed above.
    unsafe { c.dc.PopAxisAlignedClip() };
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        fluent::rectf,
        settings_groups::{label_of, pages, OPTIONS},
        settings_rows::{rows, Action as SAction, CameraAct, ChoiceKey, Key, Model, Row},
    };

    #[test]
    fn every_hit_on_every_page_has_a_unique_name_and_role() {
        use std::collections::HashSet;
        for (p, _) in pages() {
            let m = on(p, model());
            let r = rectf(104.0, 56.0, 976.0, 3000.0);
            let view = rectf(80.0, 32.0, 1000.0, 532.0);
            let hits = layout(r, view, 1.0, &m);
            assert!(!hits.is_empty());
            let mut names = HashSet::new();
            let mut ids = HashSet::new();
            for h in hits {
                assert!(ids.insert(h.id), "duplicate hit id {}", h.id);
                let d =
                    describe(&h, &m).unwrap_or_else(|| panic!("hit {} of {p:?} undescribed", h.id));
                assert!(!d.name.is_empty());
                assert!(
                    names.insert(d.name.clone()),
                    "two controls named {:?}",
                    d.name
                );
            }
        }
    }

    #[test]
    fn rows_below_the_fold_have_hits_and_a_real_rect() {
        let m = on(Page::Notifications, model());
        let r = rectf(104.0, 56.0, 976.0, 3000.0);
        let view = rectf(80.0, 32.0, 1000.0, 300.0);
        let hits = layout(r, view, 1.0, &m);
        let below: Vec<_> = hits.iter().filter(|h| h.r.bottom <= h.r.top).collect();
        assert!(!below.is_empty());
        for h in below {
            let full = full_rect(h.id, r, 1.0, &m).expect("real rect");
            assert!(full.bottom > full.top);
        }
    }

    fn model() -> Model {
        Model {
            share: true,
            notify: true,
            // Windows Hello can run: the switch is a control, not a greyed row.
            unlock_available: true,
            unlock_reason: "Ask for Windows Hello before sensitive actions",
            apps: vec![("WhatsApp".into(), true), ("Gmail".into(), false)],
            devices: vec![
                ("Pixel 8".into(), "18CF-F6D8".into(), false),
                ("Galaxy".into(), "AB12-CD34".into(), true),
            ],
            ..Default::default()
        }
    }

    fn on(sub: Page, rows: Model) -> SettingsModel {
        SettingsModel {
            rows,
            sub,
            query: String::new(),
        }
    }

    /// A tall pane, so every row is in view.
    fn pane() -> RectF {
        rectf(80.0, 32.0, 1000.0, 3032.0)
    }

    /// The content rect the window passes (pane minus its padding), unscrolled.
    fn content() -> RectF {
        let p = pane();
        rectf(p.left + 24.0, p.top + 24.0, p.right - 24.0, p.bottom - 24.0)
    }

    fn actions(m: &SettingsModel) -> Vec<Action> {
        layout(content(), pane(), 1.0, m)
            .iter()
            .filter_map(|h| click(h, m))
            .collect()
    }

    fn set(a: SAction) -> Action {
        Action::Settings(a)
    }

    fn labels(it: &[Item]) -> Vec<&'static str> {
        it.iter()
            .filter_map(|i| match i {
                Item::Row(r) => label_of(r),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn general_page_has_keep_history_and_clear() {
        let it = items(&model(), Page::General, "");
        let l = labels(&it);
        assert!(l.contains(&"Keep history"), "{l:?}");
        assert!(l.contains(&"Clear history"), "{l:?}");
        assert!(it
            .iter()
            .any(|i| matches!(i, Item::Row(Row::Toggle(Key::KeepHistory, ..)))));
        let rows_model = Model {
            keep_history: true,
            ..model()
        };
        assert!(rows_model.get(Key::KeepHistory));
        let a = actions(&on(Page::General, rows_model));
        assert!(a.contains(&set(SAction::Toggle(Key::KeepHistory))), "{a:?}");
        assert!(a.contains(&set(SAction::ClearHistory)), "{a:?}");
        // Search finds them too.
        assert!(labels(&items(&model(), Page::Devices, "history")).contains(&"Clear history"));
    }

    #[test]
    fn the_hello_area_switches_work_only_while_the_gate_is_on() {
        for (k, name) in [
            (Key::GateFiles, Page::Files),
            (Key::GateAudio, Page::Devices),
            (Key::GateControl, Page::Devices),
        ] {
            let h = Hit {
                id: toggle_id(k),
                r: rectf(0.0, 0.0, 1.0, 1.0),
            };
            let gated = Model {
                unlock_gate: true,
                unlock_available: true,
                ..model()
            };
            assert_eq!(
                click(&h, &on(name, gated.clone())),
                Some(set(SAction::Toggle(k)))
            );
        }
        assert!(!toggle_enabled(
            &Model {
                unlock_gate: false,
                unlock_available: true,
                ..model()
            },
            Key::GateFiles
        ));
    }

    #[test]
    fn every_option_is_still_reachable() {
        // Every labelled row is on exactly one page, and every option the
        // old window offered is on some page.
        let m = model();
        let mut shown: Vec<&str> = Vec::new();
        for (p, _) in pages() {
            shown.extend(labels(&items(&m, p, "")));
        }
        let mut all: Vec<&str> = rows(&m).iter().filter_map(label_of).collect();
        shown.sort();
        all.sort();
        assert_eq!(shown, all, "each row drawn exactly once");
        for o in OPTIONS {
            assert!(shown.contains(o), "unreachable option {o}");
        }
    }

    #[test]
    fn every_control_maps_to_the_old_action() {
        let mut got = Vec::new();
        for (p, _) in pages() {
            got.extend(actions(&on(p, model())));
        }
        let mut want = vec![
            set(SAction::Theme(0)),
            set(SAction::Theme(1)),
            set(SAction::Theme(2)),
            set(SAction::Look(0)),
            set(SAction::Look(1)),
            set(SAction::ChangeFolder),
            set(SAction::OpenFolder),
            set(SAction::Pair),
            set(SAction::Remove(0)),
            set(SAction::Remove(1)),
            set(SAction::Mute(0)),
            set(SAction::Mute(1)),
            set(SAction::Camera(CameraAct::Toggle)),
            set(SAction::Camera(CameraAct::OpenWindow)),
            set(SAction::ClearHistory),
            set(SAction::OpenLogs),
            set(SAction::ClearLogs),
            set(SAction::CheckUpdates),
        ];
        for k in [ChoiceKey::Content, ChoiceKey::Sound] {
            want.extend((0..3).map(|i| set(SAction::Choice(k, i))));
        }
        for k in [
            Key::Share,
            Key::ShareImages,
            Key::Passwords,
            Key::Keep,
            Key::AutoCopyText,
            Key::ToastShowText,
            Key::Auto,
            Key::SendTo,
            Key::Startup,
            Key::Control,
            Key::UnlockGate,
            Key::Notify,
            Key::NotifyLock,
            Key::NotifyClear,
            Key::NotifyOpen,
            Key::NotifyOtp,
            Key::NotifyQuietMirror,
            Key::KeepHistory,
            Key::KeepLogs,
            Key::AudioSkipConfirm,
        ] {
            want.push(set(SAction::Toggle(k)));
        }
        for w in &want {
            assert!(got.contains(w), "missing {w:?}");
        }
        // Plus a link to every page on every page.
        let links = got
            .iter()
            .filter(|a| matches!(a, Action::SettingsPage(_)))
            .count();
        assert_eq!(links, pages().len() * pages().len());
    }

    #[test]
    fn repair_shows_only_when_the_camera_offers_it() {
        use crate::camera_status::{CameraButton, Mark, Summary};
        let mut m = model();
        m.camera = Some(Summary {
            mark: Mark::Bad,
            title: "Camera needs repair".into(),
            detail: "Something broke".into(),
            button: CameraButton::Start,
            repair: true,
        });
        let a = actions(&on(Page::Camera, m));
        assert!(a.contains(&set(SAction::Camera(CameraAct::Repair))));
        let a = actions(&on(Page::Camera, model()));
        assert!(!a.contains(&set(SAction::Camera(CameraAct::Repair))));
    }

    #[test]
    fn disabled_rows_take_no_clicks() {
        let m = Model {
            share: false,
            notify: false,
            ..model()
        };
        let clip = actions(&on(Page::Clipboard, m.clone()));
        assert!(clip.contains(&set(SAction::Toggle(Key::Share))));
        assert!(!clip.contains(&set(SAction::Toggle(Key::Passwords))));
        assert!(!clip.contains(&set(SAction::Toggle(Key::ShareImages))));
        let n = actions(&on(Page::Notifications, m.clone()));
        let set_only: Vec<_> = n
            .iter()
            .filter(|a| matches!(a, Action::Settings(_)))
            .collect();
        assert_eq!(set_only, [&set(SAction::Toggle(Key::Notify))]);
        let apps = actions(&on(Page::Apps, m));
        assert!(apps.iter().all(|a| matches!(a, Action::SettingsPage(_))));
    }

    #[test]
    fn page_links_open_each_page_and_stay_during_a_search() {
        let mut m = on(Page::Devices, model());
        m.query = "camera".into();
        let a = actions(&m);
        for (p, _) in pages() {
            assert!(a.contains(&Action::SettingsPage(p)), "{p:?}");
        }
    }

    #[test]
    fn search_lists_matches_from_every_page() {
        let it = items(&model(), Page::Devices, "camera");
        assert!(it.iter().any(|i| matches!(i, Item::PageName("Camera"))));
        assert!(labels(&it).contains(&"Camera status"));
        assert!(labels(&it).contains(&"Camera window"));
        let none = items(&model(), Page::Devices, "zzzz");
        assert!(matches!(none[..], [Item::Message(_)]));
        // A blank query shows the page itself, under its title.
        let page = items(&model(), Page::Files, "   ");
        assert!(matches!(page[0], Item::Caption("Files")));
        // The search result rows take clicks.
        let mut m = on(Page::Devices, model());
        m.query = "camera".into();
        assert!(actions(&m).contains(&set(SAction::Camera(CameraAct::OpenWindow))));
    }

    #[test]
    fn hit_ids_follow_their_app_or_device() {
        let m = on(Page::Apps, model());
        let gmail = layout(content(), pane(), 1.0, &m)
            .into_iter()
            .find(|h| click(h, &m) == Some(set(SAction::Mute(1))))
            .expect("Gmail");
        // A new app sorts in before it: the same id still means Gmail.
        let mut rows = model();
        rows.apps.insert(0, ("Calendar".into(), true));
        let m2 = on(Page::Apps, rows);
        assert_eq!(click(&gmail, &m2), Some(set(SAction::Mute(2))));
        // Same for a device.
        let m = on(Page::Devices, model());
        let galaxy = layout(content(), pane(), 1.0, &m)
            .into_iter()
            .find(|h| click(h, &m) == Some(set(SAction::Remove(1))))
            .expect("Galaxy");
        let mut rows = model();
        rows.devices.remove(0);
        let m2 = on(Page::Devices, rows);
        assert_eq!(click(&galaxy, &m2), Some(set(SAction::Remove(0))));
        // Gone: nothing.
        let mut rows = model();
        rows.devices.clear();
        assert_eq!(click(&galaxy, &on(Page::Devices, rows)), None);
    }

    #[test]
    fn two_apps_with_one_name_get_distinct_ids() {
        let mut rows = model();
        rows.apps = vec![("Mail".into(), true), ("Mail".into(), true)];
        let m = on(Page::Apps, rows);
        let hits = layout(content(), pane(), 1.0, &m);
        let mut ids: Vec<u32> = hits.iter().map(|h| h.id).collect();
        let n = ids.len();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), n, "hit ids are unique");
        let a = actions(&m);
        assert!(a.contains(&set(SAction::Mute(0))) && a.contains(&set(SAction::Mute(1))));
    }

    #[test]
    fn search_box_and_page_links_stay_put_while_the_rows_scroll() {
        let m = on(Page::Notifications, model());
        let view = rectf(80.0, 32.0, 1000.0, 532.0);
        let c = rectf(104.0, 56.0, 976.0, 508.0);
        let off = 200.0;
        let scrolled = rectf(c.left, c.top - off, c.right, c.top - off + 2000.0);
        assert_eq!(search_rect(c, view, 1.0), search_rect(scrolled, view, 1.0));
        let sb = search_rect(c, view, 1.0);
        assert!(sb.top >= view.top && sb.bottom < view.bottom && sb.right <= c.right);
        let links = |r| -> Vec<Hit> {
            layout(r, view, 1.0, &m)
                .into_iter()
                .filter(|h| matches!(click(h, &m), Some(Action::SettingsPage(_))))
                .collect()
        };
        assert_eq!(links(c), links(scrolled));
        // A row scrolled up under the search box takes no clicks there.
        for h in layout(scrolled, view, 1.0, &m) {
            // (A control out of view has an empty hit: it takes no clicks.)
            if h.r.bottom > h.r.top && matches!(click(&h, &m), Some(Action::Settings(_))) {
                assert!(h.r.top > sb.bottom, "{h:?} under the search box");
                assert!(h.r.bottom <= view.bottom + 0.5);
                assert!(h.r.left >= sb.left - 0.5, "rows sit right of the links");
            }
        }
    }

    #[test]
    fn content_height_covers_every_row() {
        for (p, _) in pages() {
            let m = on(p, model());
            for s in [1.0, 1.5] {
                let r = rectf(0.0, 0.0, 900.0, 10.0);
                let h = content_height(900.0, s, &m);
                let tall = rectf(-24.0, -24.0, 924.0, 10_000.0);
                // The rows scroll; the page links are fixed in the pane.
                for x in layout(r, tall, s, &m) {
                    if matches!(click(&x, &m), Some(Action::Settings(_))) {
                        assert!(x.r.bottom <= h * s + 0.5, "{p:?} at {s}");
                    }
                }
            }
        }
    }

    #[test]
    fn narrow_rows_keep_their_controls_inside() {
        // The narrowest window: 640 px minus the rail and padding.
        let view = rectf(80.0, 32.0, 640.0, 3032.0);
        let c = rectf(104.0, 56.0, 616.0, 3008.0);
        for (p, _) in pages() {
            let m = on(p, model());
            for h in layout(c, view, 1.0, &m) {
                assert!(h.r.left >= c.left && h.r.right <= c.right, "{p:?} {h:?}");
                assert!(h.r.right > h.r.left);
            }
        }
    }

    #[test]
    fn every_page_and_a_search_draw_in_a_hidden_window() {
        use crate::d2d::{Backdrop, Surface};
        let hwnd = crate::d2d::test_window();
        let mut surface = Surface::new(hwnd, Backdrop::Opaque).expect("surface");
        let mut rows = model();
        rows.camera = Some(crate::camera_status::Summary {
            mark: crate::camera_status::Mark::Bad,
            title: "Camera needs repair".into(),
            detail: "A long explanation that wraps onto more than one line ".repeat(4),
            button: crate::camera_status::CameraButton::Stop,
            repair: true,
        });
        let mut models: Vec<SettingsModel> =
            pages().iter().map(|(p, _)| on(*p, rows.clone())).collect();
        let mut q = on(Page::Devices, rows);
        q.query = "camera".into();
        models.push(q);
        surface
            .draw(|dc| {
                let mut c = Canvas {
                    dc,
                    s: 1.5,
                    p: fluent::palette(),
                };
                let (view, r) = (
                    rectf(80.0, 32.0, 900.0, 600.0),
                    rectf(104.0, -100.0, 876.0, 2000.0),
                );
                for m in &models {
                    let hits = layout(r, view, 1.5, m);
                    draw(&mut c, r, view, m, &hits);
                }
            })
            .expect("draw");
        drop(surface);
        // SAFETY: our own test window, made above on this thread.
        unsafe {
            let _ = windows::Win32::UI::WindowsAndMessaging::DestroyWindow(hwnd);
        }
    }

    #[test]
    fn keys_cover_every_toggle() {
        // Fails to compile when a Key is added: add it to KEYS too.
        let numbered = |k: Key| match k {
            Key::Share
            | Key::ShareImages
            | Key::Passwords
            | Key::Keep
            | Key::AutoCopyText
            | Key::ToastShowText
            | Key::Auto
            | Key::SendTo
            | Key::Startup
            | Key::Control
            | Key::UnlockGate
            | Key::GateFiles
            | Key::GateAudio
            | Key::GateControl
            | Key::Notify
            | Key::NotifyLock
            | Key::NotifyClear
            | Key::NotifyOpen
            | Key::NotifyOtp
            | Key::NotifyQuietMirror
            | Key::KeepHistory
            | Key::KeepLogs
            | Key::AudioSkipConfirm => KEYS.contains(&k),
        };
        let m = on(Page::General, model());
        for (i, k) in KEYS.iter().enumerate() {
            assert!(numbered(*k));
            assert!(!KEYS[..i].contains(k), "{k:?} twice");
            let h = Hit {
                id: toggle_id(*k),
                r: rectf(0.0, 0.0, 1.0, 1.0),
            };
            assert_eq!(click(&h, &m), Some(set(SAction::Toggle(*k))));
        }
        assert_eq!(KEYS.len(), 23);
        for r in rows(&model()) {
            if let Row::Toggle(k, ..) = r {
                assert!(KEYS.contains(&k), "{k:?} missing from KEYS");
            }
        }
    }

    #[test]
    fn long_save_paths_keep_the_folder_name() {
        let fits = |n: usize| move |t: &str| t.chars().count() <= n;
        let p = r"C:\Users\Alex Kim\Documents\Relay\Received";
        assert_eq!(shorten_path(p, fits(100)), p);
        assert_eq!(shorten_path(p, fits(28)), "C:\\\u{2026}\\Relay\\Received");
        assert_eq!(shorten_path(p, fits(18)), "C:\\\u{2026}\\Received");
        // Not even that fits: the folder name alone, after the ellipsis.
        assert_eq!(shorten_path(p, fits(5)), "\u{2026}\\Received");
        // A UNC share keeps its server and share.
        let u = r"\\server\share\a\b\Folder";
        assert_eq!(
            shorten_path(u, fits(24)),
            "\\\\server\\share\\\u{2026}\\Folder"
        );
        assert_eq!(shorten_path("Folder", fits(3)), "Folder");
    }

    #[test]
    fn the_query_is_capped() {
        let q = "x".repeat(QUERY_MAX + 10);
        assert_eq!(clamp_query(&q).chars().count(), QUERY_MAX);
        assert_eq!(clamp_query("cam"), "cam");
    }

    #[test]
    fn a_character_outside_the_bmp_is_never_split() {
        let q = format!("{}\u{1f4f7}x", "a".repeat(QUERY_MAX - 1));
        let c = clamp_query(&q);
        assert_eq!(c.chars().count(), QUERY_MAX);
        assert!(c.ends_with('\u{1f4f7}'));
    }
}
