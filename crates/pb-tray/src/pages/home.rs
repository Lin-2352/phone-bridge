//! Home: the paired phone at a glance (hero card), quick actions that run
//! the flyout's commands (and Send clipboard), the live sessions with Stop,
//! the phone's recent notifications and the last few Timeline items.
//! `build` turns plain inputs into a [`HomeModel`] (pure); layout, draw and
//! click work on that model.

use pb_protocol::control::Notification;

pub use crate::netpath::Path;
use crate::{
    fluent::{self, rectf, round_rect, text, Align, Canvas, Font, RectF},
    flyout,
    main_window::Page,
    notify,
    pages::{Action, Hit},
    settings::Settings,
    settings_rows, timeline,
    tokens::{RADIUS, SPACE},
    widgets::{self, InfoKind},
};

/// At most this many notifications and Timeline items are listed.
pub const RECENT: usize = 5;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PhoneCard {
    pub name: String,
    /// Short fingerprint, for the "Reinstalled or reset" note.
    pub fingerprint: String,
    pub connected: bool,
    /// Unix seconds of the last contact this run.
    pub last_seen: Option<u64>,
    /// How the PC reaches it; shown only while connected.
    pub path: Path,
    /// Answered as a different device: reinstalled or reset.
    pub changed: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionKind {
    Screen,
    Camera,
    Control,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Session {
    pub kind: SessionKind,
    pub stop: Action,
    /// The picture is live; false while it waits for the phone again
    /// ("Reconnecting…").
    pub live: bool,
}

/// A phone notification as the toast would show it right now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NoteRow {
    pub app: String,
    pub title: String,
    pub line: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct HomeModel {
    pub phone: Option<PhoneCard>,
    pub sessions: Vec<Session>,
    pub notes: Vec<NoteRow>,
    /// Some notifications are held back because the PC is locked.
    pub notes_hidden: bool,
    pub recent: Vec<timeline::Entry>,
    /// Unix seconds when the model was built, for "5 min ago" (whole
    /// minutes, so the model only changes when a label can).
    pub now: u64,
}

/// The paired phone as the tray knows it.
pub struct PhoneIn {
    pub name: String,
    pub fingerprint: String,
    pub changed: bool,
    /// A session with it is open now.
    pub open: bool,
    pub last_seen: Option<u64>,
    /// How it was last reached this run.
    pub path: Path,
}

/// Everything `build` reads, gathered by the UI thread.
pub struct Inputs<'a> {
    pub phone: Option<PhoneIn>,
    /// The screen window is open, and its picture is live.
    pub screen: bool,
    pub screen_live: bool,
    /// The camera runs (or waits to reconnect), and is streaming.
    pub camera: bool,
    pub camera_live: bool,
    pub control: bool,
    /// Mirrored phone notifications, newest first.
    pub notes: &'a [Notification],
    pub settings: &'a Settings,
    pub locked: bool,
    /// Timeline, newest first.
    pub recent: &'a [timeline::Entry],
    pub now: u64,
}

/// Whether the sessions alone prove contact: a live screen, camera or
/// control session is an open connection too; one that waits to reconnect
/// is not.
pub fn sessions_live(
    screen: bool,
    screen_live: bool,
    camera: bool,
    camera_live: bool,
    control: bool,
) -> bool {
    (screen && screen_live) || (camera && camera_live) || control
}

/// The phone as the hero card (and the flyout header) shows it; `live`
/// from [`sessions_live`].
pub fn phone_card(p: PhoneIn, live: bool) -> PhoneCard {
    let connected = !p.changed && (p.open || live);
    PhoneCard {
        connected,
        // Shown only while not connected (it moves on every contact).
        last_seen: p.last_seen.filter(|_| !connected),
        path: p.path,
        changed: p.changed,
        name: p.name,
        fingerprint: p.fingerprint,
    }
}

pub fn build(i: Inputs) -> HomeModel {
    let live = sessions_live(i.screen, i.screen_live, i.camera, i.camera_live, i.control);
    let phone = i.phone.map(|p| phone_card(p, live));
    let sessions = [
        (i.screen, i.screen_live, SessionKind::Screen),
        (i.camera, i.camera_live, SessionKind::Camera),
        (i.control, true, SessionKind::Control),
    ]
    .into_iter()
    .filter(|(on, _, _)| *on)
    .map(|(_, live, kind)| Session {
        kind,
        stop: Action::Stop(kind),
        live,
    })
    .collect();
    let ctx = notify::Ctx {
        locked: i.locked,
        mirroring_focused: false,
    };
    let mut notes_hidden = false;
    let notes = i
        .notes
        .iter()
        .filter_map(|n| {
            // Exactly what the toast shows now; nothing while content is
            // held back for the lock screen.
            let p = notify::present(n, i.settings, &ctx)?;
            if notify::hidden_now(&p) {
                notes_hidden = true;
                return None;
            }
            let app = if n.app_name.is_empty() {
                n.package.clone()
            } else {
                n.app_name.clone()
            };
            Some(NoteRow {
                title: if p.title.is_empty() {
                    app.clone()
                } else {
                    p.title
                },
                // Message styles list the newest line last.
                line: p.lines.last().cloned().unwrap_or_default(),
                app,
            })
        })
        .take(RECENT)
        .collect();
    HomeModel {
        phone,
        sessions,
        notes,
        notes_hidden,
        recent: i.recent.iter().take(RECENT).cloned().collect(),
        now: i.now,
    }
}

/// The warning under the phone's name, when it was reinstalled or reset.
pub fn warning(c: &PhoneCard) -> Option<String> {
    c.changed
        .then(|| settings_rows::device_note(&c.fingerprint, true))
}

/// "5 min ago", "3 h ago": how long before `now` the time `at` was.
fn ago(at: u64, now: u64) -> String {
    let d = now.saturating_sub(at);
    match d {
        0..60 => "just now".into(),
        60..3600 => format!("{} min ago", d / 60),
        3600..86_400 => format!("{} h ago", d / 3600),
        _ if d < 2 * 86_400 => "yesterday".into(),
        _ => format!("{} days ago", d / 86_400),
    }
}

/// "Connected · USB", "Last seen 5 min ago" and the like. The path is
/// only named while connected: an old one may no longer be true.
pub fn status_line(c: &PhoneCard, now: u64) -> String {
    if !c.connected {
        return if c.changed {
            "Not connected".to_owned()
        } else {
            match c.last_seen {
                Some(at) => format!("Last seen {}", ago(at, now)),
                None => "Not seen since Relay started".to_owned(),
            }
        };
    }
    match c.path {
        Path::Wifi => "Connected \u{b7} Wi-Fi".to_owned(),
        Path::Usb => "Connected \u{b7} USB".to_owned(),
        Path::Network => "Connected \u{b7} Network".to_owned(),
        Path::Unknown => "Connected".to_owned(),
    }
}

/// What the notifications card says when it lists nothing.
pub fn notes_empty_text(m: &HomeModel) -> &'static str {
    if m.notes_hidden {
        "Hidden while this PC is locked"
    } else {
        "No notifications from your phone right now"
    }
}

/// The quick actions, in order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Quick {
    SendFile,
    Clipboard,
    Screen,
    Camera,
    Control,
}

impl Quick {
    pub const ALL: [Quick; 5] = [
        Quick::SendFile,
        Quick::Clipboard,
        Quick::Screen,
        Quick::Camera,
        Quick::Control,
    ];

    /// What this runs: the flyout tile's command, or for Send clipboard the
    /// tray's one-off send.
    pub fn action(self) -> Action {
        match self {
            Quick::SendFile => Action::Flyout(flyout::Action::SendFiles),
            Quick::Clipboard => Action::SendClipboard,
            Quick::Screen => Action::Flyout(flyout::Action::ShowScreen),
            Quick::Camera => Action::Flyout(flyout::Action::ToggleCamera),
            Quick::Control => Action::Flyout(flyout::Action::ControlPhone),
        }
    }

    fn label(self) -> &'static str {
        match self {
            Quick::SendFile => "Send files",
            Quick::Clipboard => "Send clipboard",
            Quick::Screen => "Phone screen",
            Quick::Camera => "Camera",
            Quick::Control => "Control phone",
        }
    }

    fn glyph(self) -> char {
        match self {
            Quick::SendFile => fluent::glyph::SEND,
            Quick::Clipboard => fluent::glyph::CLIPBOARD,
            Quick::Screen => '\u{E7F4}',
            Quick::Camera => '\u{E714}',
            Quick::Control => fluent::glyph::KEYBOARD,
        }
    }

    /// Drawn in the accent fill: its session runs.
    fn on(self, m: &HomeModel) -> bool {
        let running = |k| m.sessions.iter().any(|x| x.kind == k);
        match self {
            Quick::Screen => running(SessionKind::Screen),
            Quick::Camera => running(SessionKind::Camera),
            Quick::Control => running(SessionKind::Control),
            Quick::SendFile | Quick::Clipboard => false,
        }
    }
}

impl Session {
    fn text(&self) -> (char, &'static str, &'static str) {
        let (glyph, title, meta) = match self.kind {
            SessionKind::Screen => ('\u{E7F4}', "Phone screen", "Showing in a window"),
            SessionKind::Camera => ('\u{E714}', "Phone camera", "In use as this PC's webcam"),
            SessionKind::Control => (
                fluent::glyph::KEYBOARD,
                "Control phone",
                "This PC's mouse and keyboard drive the phone",
            ),
        };
        (
            glyph,
            title,
            if self.live {
                meta
            } else {
                "Reconnecting\u{2026}"
            },
        )
    }
}

/// Hit ids.
const PAIR: u32 = 1;
const QUICK: u32 = 10;
const STOP: u32 = 20;
const SEE_ALL: u32 = 30;

/// Sizes in logical px.
const PAD: f32 = 24.0;
const TITLE_H: f32 = 40.0;
/// A section heading: a 16 px gap above, its 20 px line, 8 px below.
const HEAD_H: f32 = 44.0;
const HERO_H: f32 = 88.0;
const WARN_H: f32 = 40.0;
const EMPTY_H: f32 = 184.0;
const TILE_H: f32 = 76.0;
const ROW_H: f32 = 56.0;
const NONE_H: f32 = 48.0;
const BUTTON_W: f32 = 88.0;
const BUTTON_H: f32 = 32.0;
const SEE_ALL_W: f32 = 64.0;

/// Where everything goes, in pixels.
struct Parts {
    title: RectF,
    hero: RectF,
    warn: Option<RectF>,
    pair: Option<RectF>,
    quick_head: Option<RectF>,
    quick: Vec<RectF>,
    live_head: Option<RectF>,
    live: Option<RectF>,
    /// Row and its Stop button, per session.
    stops: Vec<(RectF, RectF)>,
    notes_head: RectF,
    notes: RectF,
    activity_head: RectF,
    see_all: RectF,
    activity: RectF,
    bottom: f32,
}

fn rows(n: usize) -> f32 {
    if n == 0 {
        NONE_H
    } else {
        n as f32 * ROW_H
    }
}

fn parts(r: RectF, s: f32, m: &HomeModel) -> Parts {
    let (left, right) = (r.left + PAD * s, (r.right - PAD * s).max(r.left + PAD * s));
    let mut y = r.top + PAD * s;
    let mut take = |h: f32| {
        let b = rectf(left, y, right, y + h * s);
        y += h * s;
        b
    };
    let title = take(TITLE_H);
    let (hero, warn, pair) = match &m.phone {
        None => {
            let hero = take(EMPTY_H);
            let cx = (hero.left + hero.right) / 2.0;
            let pair = rectf(
                cx - BUTTON_W * s / 2.0,
                hero.bottom - (SPACE[4] + BUTTON_H) * s,
                cx + BUTTON_W * s / 2.0,
                hero.bottom - SPACE[4] * s,
            );
            (hero, None, Some(pair))
        }
        Some(p) => {
            let h = HERO_H + if p.changed { WARN_H + SPACE[3] } else { 0.0 };
            let hero = take(h);
            let warn = p.changed.then(|| {
                rectf(
                    hero.left + SPACE[3] * s,
                    hero.bottom - (SPACE[3] + WARN_H) * s,
                    hero.right - SPACE[3] * s,
                    hero.bottom - SPACE[3] * s,
                )
            });
            (hero, warn, None)
        }
    };
    let (mut quick_head, mut quick) = (None, Vec::new());
    if m.phone.is_some() {
        quick_head = Some(take(HEAD_H));
        let row = take(TILE_H);
        let gap = SPACE[1] * s;
        let n = Quick::ALL.len() as f32;
        let w = ((row.right - row.left - gap * (n - 1.0)) / n).max(0.0);
        quick = (0..Quick::ALL.len())
            .map(|k| {
                let l = row.left + k as f32 * (w + gap);
                rectf(l, row.top, l + w, row.bottom)
            })
            .collect();
    }
    let (mut live_head, mut live, mut stops) = (None, None, Vec::new());
    if !m.sessions.is_empty() {
        live_head = Some(take(HEAD_H));
        let card = take(rows(m.sessions.len()));
        stops = (0..m.sessions.len())
            .map(|k| {
                let top = card.top + k as f32 * ROW_H * s;
                let row = rectf(card.left, top, card.right, top + ROW_H * s);
                let cy = top + ROW_H * s / 2.0;
                let pad = SPACE[3] * s;
                let b = rectf(
                    (row.right - pad - BUTTON_W * s).max(row.left),
                    cy - BUTTON_H * s / 2.0,
                    row.right - pad,
                    cy + BUTTON_H * s / 2.0,
                );
                (row, b)
            })
            .collect();
        live = Some(card);
    }
    let notes_head = take(HEAD_H);
    let notes = take(rows(m.notes.len()));
    let activity_head = take(HEAD_H);
    let see_all = rectf(
        (activity_head.right - SEE_ALL_W * s).max(activity_head.left),
        activity_head.top + SPACE[3] * s,
        activity_head.right,
        activity_head.bottom - SPACE[1] * s,
    );
    let activity = take(rows(m.recent.len()));
    Parts {
        title,
        hero,
        warn,
        pair,
        quick_head,
        quick,
        live_head,
        live,
        stops,
        notes_head,
        notes,
        activity_head,
        see_all,
        activity,
        bottom: y + PAD * s,
    }
}

pub fn layout(r: RectF, s: f32, m: &HomeModel) -> Vec<Hit> {
    let p = parts(r, s, m);
    let mut hits = Vec::new();
    if let Some(r) = p.pair {
        hits.push(Hit { id: PAIR, r });
    }
    for (k, r) in p.quick.iter().enumerate() {
        hits.push(Hit {
            id: QUICK + k as u32,
            r: *r,
        });
    }
    for (k, (_, b)) in p.stops.iter().enumerate() {
        hits.push(Hit {
            id: STOP + k as u32,
            r: *b,
        });
    }
    hits.push(Hit {
        id: SEE_ALL,
        r: p.see_all,
    });
    hits
}

/// `width` in pixels; the height in logical px.
pub fn content_height(width: f32, s: f32, m: &HomeModel) -> f32 {
    parts(rectf(0.0, 0.0, width, 0.0), s, m).bottom / s
}

pub fn draw(c: &mut Canvas, r: RectF, m: &HomeModel, hits: &[Hit]) {
    let _ = hits;
    let (s, pal) = (c.s, c.p);
    let p = parts(r, s, m);
    text(c, Font::Title, "Home", p.title, pal.text, Align::Left);
    let heading = |c: &Canvas, r: RectF, label: &str| {
        let t = rectf(
            r.left,
            r.top + SPACE[3] * s,
            r.right,
            r.bottom - SPACE[1] * s,
        );
        text(c, Font::BodyStrong, label, t, pal.text, Align::Left);
    };

    // The phone hero card, or the empty state with Pair.
    widgets::card(c, p.hero);
    match &m.phone {
        None => {
            let top = rectf(
                p.hero.left,
                p.hero.top,
                p.hero.right,
                p.hero.bottom - (SPACE[4] + BUTTON_H) * s,
            );
            widgets::empty_state(
                c,
                top,
                fluent::glyph::PHONE,
                "No phone paired",
                "Pair your phone to send files, share the clipboard and more.",
            );
            if let Some(b) = p.pair {
                fluent::button(c, b, "Pair", true);
            }
        }
        Some(card) => {
            let pad = SPACE[3] * s;
            let body = rectf(
                p.hero.left + pad,
                p.hero.top,
                p.hero.right - pad,
                p.hero.top + HERO_H * s,
            );
            let cy = (body.top + body.bottom) / 2.0;
            let disc = rectf(
                body.left,
                cy - 24.0 * s,
                body.left + 48.0 * s,
                cy + 24.0 * s,
            );
            let tint = if card.connected {
                pal.accent
            } else {
                pal.text2
            };
            round_rect(c, &disc, Some((tint, 0.14)), None, 24.0 * s, false);
            fluent::icon_sized(c, Font::IconLarge, fluent::glyph::PHONE, disc, tint);
            let tx = disc.right + SPACE[3] * s;
            let badge_w = if card.connected { 96.0 * s } else { 0.0 };
            let tr = (body.right - badge_w - SPACE[2] * s).max(tx);
            let name = rectf(tx, cy - 24.0 * s, tr, cy);
            text(c, Font::Subtitle, &card.name, name, pal.text, Align::Left);
            let line = rectf(tx, cy, tr, cy + 22.0 * s);
            let status = status_line(card, m.now);
            text(c, Font::Body, &status, line, pal.text2, Align::Left);
            if card.connected {
                let b = rectf(
                    body.right - badge_w,
                    cy - 12.0 * s,
                    body.right,
                    cy + 12.0 * s,
                );
                widgets::badge(c, b, "Connected", pal.success);
            }
            if let (Some(w), Some(msg)) = (p.warn, warning(card)) {
                widgets::info_bar(c, w, InfoKind::Warning, &msg);
            }
        }
    }

    // Quick actions: the flyout's commands.
    if let Some(h) = p.quick_head {
        heading(c, h, "Quick actions");
    }
    for (q, t) in Quick::ALL.iter().zip(&p.quick) {
        let on = q.on(m);
        let radius = RADIUS[1] * s;
        let (fill, fg) = if on {
            ((pal.accent, 1.0), pal.on_accent)
        } else {
            ((pal.card, 1.0), pal.text)
        };
        round_rect(c, t, Some(fill), Some(pal.stroke), radius, false);
        let g = rectf(t.left, t.top + SPACE[3] * s, t.right, t.top + 40.0 * s);
        fluent::icon_sized(c, Font::IconLarge, q.glyph(), g, fg);
        let l = rectf(
            t.left + SPACE[0] * s,
            g.bottom + SPACE[0] * s,
            t.right - SPACE[0] * s,
            t.bottom - SPACE[2] * s,
        );
        text(c, Font::Caption, q.label(), l, fg, Align::Center);
    }

    // Live sessions, each with Stop.
    if let (Some(h), Some(card)) = (p.live_head, p.live) {
        heading(c, h, "Live now");
        widgets::card(c, card);
        for (x, (row, b)) in m.sessions.iter().zip(&p.stops) {
            let (glyph, title, meta) = x.text();
            let text_row = rectf(row.left, row.top, b.left, row.bottom);
            widgets::row(c, text_row, glyph, title, meta, "");
            fluent::button(c, *b, "Stop", false);
        }
    }

    // Recent notifications, as the toasts show them now.
    heading(c, p.notes_head, "Recent notifications");
    widgets::card(c, p.notes);
    if m.notes.is_empty() {
        let t = rectf(
            p.notes.left + SPACE[3] * s,
            p.notes.top,
            p.notes.right,
            p.notes.bottom,
        );
        text(
            c,
            Font::Body,
            notes_empty_text(m),
            t,
            pal.text2,
            Align::Left,
        );
    }
    for (k, n) in m.notes.iter().enumerate() {
        let top = p.notes.top + k as f32 * ROW_H * s;
        let row = rectf(p.notes.left, top, p.notes.right, top + ROW_H * s);
        let meta = if n.line.is_empty() {
            n.app.clone()
        } else {
            format!("{} \u{b7} {}", n.app, n.line)
        };
        widgets::row(c, row, '\u{EA8F}', &n.title, &meta, "");
    }

    // Recent activity: the last Timeline items, "See all" opens it.
    heading(c, p.activity_head, "Recent activity");
    text(
        c,
        Font::Body,
        "See all",
        p.see_all,
        pal.accent,
        Align::Right,
    );
    widgets::card(c, p.activity);
    if m.recent.is_empty() {
        let a = p.activity;
        let t = rectf(a.left + SPACE[3] * s, a.top, a.right, a.bottom);
        text(
            c,
            Font::Body,
            "Nothing has moved yet",
            t,
            pal.text2,
            Align::Left,
        );
    }
    for (k, e) in m.recent.iter().enumerate() {
        let top = p.activity.top + k as f32 * ROW_H * s;
        let row = rectf(p.activity.left, top, p.activity.right, top + ROW_H * s);
        let glyph = match e.kind {
            timeline::Kind::File => '\u{E8A5}',
            timeline::Kind::ClipText => fluent::glyph::CLIPBOARD,
            timeline::Kind::ClipImage => '\u{EB9F}',
            timeline::Kind::Notification => '\u{EA8F}',
        };
        let meta = match e.dir {
            timeline::Dir::ToPhone => format!("To {}", e.peer),
            timeline::Dir::ToPc => format!("From {}", e.peer),
        };
        widgets::row(
            c,
            row,
            glyph,
            &timeline::summary(e),
            &meta,
            &ago(e.at, m.now),
        );
    }
}

pub fn click(hit: &Hit, m: &HomeModel) -> Option<Action> {
    match hit.id {
        PAIR if m.phone.is_none() => Some(Action::Flyout(flyout::Action::Pair)),
        SEE_ALL => Some(Action::Go(Page::Timeline)),
        id if (QUICK..QUICK + Quick::ALL.len() as u32).contains(&id) && m.phone.is_some() => {
            Some(Quick::ALL[(id - QUICK) as usize].action())
        }
        id if (STOP..STOP + 3).contains(&id) => {
            m.sessions.get((id - STOP) as usize).map(|x| x.stop.clone())
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: u64 = 1_800_000_000;

    fn phone(changed: bool, open: bool) -> PhoneIn {
        PhoneIn {
            name: "Pixel 8".into(),
            fingerprint: "18CF-F6D8".into(),
            changed,
            open,
            last_seen: Some(NOW - 300),
            path: Path::Wifi,
        }
    }

    fn inputs<'a>(settings: &'a Settings, notes: &'a [Notification]) -> Inputs<'a> {
        Inputs {
            phone: Some(phone(false, true)),
            screen: false,
            screen_live: false,
            camera: false,
            camera_live: false,
            control: false,
            notes,
            settings,
            locked: false,
            recent: &[],
            now: NOW,
        }
    }

    fn note(key: &str) -> Notification {
        Notification {
            key: key.into(),
            package: "com.whatsapp".into(),
            app_name: "WhatsApp".into(),
            title: "Mom".into(),
            text: "See you at 6".into(),
            ..Default::default()
        }
    }

    fn page() -> RectF {
        rectf(0.0, 0.0, 800.0, 2000.0)
    }

    fn hit(m: &HomeModel, id: u32) -> Option<Hit> {
        layout(page(), 1.0, m).into_iter().find(|h| h.id == id)
    }

    #[test]
    fn nothing_paired_shows_empty_state_with_pair() {
        let s = Settings::default();
        let mut i = inputs(&s, &[]);
        i.phone = None;
        let m = build(i);
        assert!(m.phone.is_none());
        let pair = hit(&m, PAIR).expect("a Pair button");
        assert_eq!(click(&pair, &m), Some(Action::Flyout(flyout::Action::Pair)));
        // No quick actions without a phone.
        assert!(hit(&m, QUICK).is_none());
    }

    #[test]
    fn reinstalled_phone_shows_the_warning_not_connected() {
        let s = Settings::default();
        let mut i = inputs(&s, &[]);
        i.phone = Some(phone(true, true));
        let m = build(i);
        let card = m.phone.clone().unwrap();
        assert!(card.changed);
        assert!(!card.connected, "a reinstalled phone is not the paired one");
        assert_eq!(
            warning(&card),
            Some(settings_rows::device_note("18CF-F6D8", true))
        );
        assert!(!status_line(&card, NOW).contains("Connected"));
        // The healthy phone: connected, no warning.
        let m = build(inputs(&s, &[]));
        let card = m.phone.unwrap();
        assert!(card.connected);
        assert_eq!(warning(&card), None);
        assert_eq!(status_line(&card, NOW), "Connected \u{b7} Wi-Fi");
    }

    #[test]
    fn not_connected_shows_last_seen() {
        let s = Settings::default();
        let mut i = inputs(&s, &[]);
        i.phone = Some(phone(false, false));
        let card = build(i).phone.unwrap();
        assert!(!card.connected);
        // The path is named only while connected.
        assert_eq!(status_line(&card, NOW), "Last seen 5 min ago");
        let never = PhoneCard {
            last_seen: None,
            path: Path::Unknown,
            ..card
        };
        assert_eq!(status_line(&never, NOW), "Not seen since Relay started");
    }

    #[test]
    fn connected_names_the_path_of_the_adapter() {
        let s = Settings::default();
        for (path, want) in [
            (Path::Usb, "Connected \u{b7} USB"),
            (Path::Wifi, "Connected \u{b7} Wi-Fi"),
            (Path::Network, "Connected \u{b7} Network"),
            (Path::Unknown, "Connected"),
        ] {
            let mut i = inputs(&s, &[]);
            i.phone = Some(PhoneIn {
                path,
                ..phone(false, true)
            });
            assert_eq!(status_line(&build(i).phone.unwrap(), NOW), want);
        }
    }

    #[test]
    fn a_running_session_counts_as_connected() {
        let s = Settings::default();
        let mut i = inputs(&s, &[]);
        i.phone = Some(phone(false, false));
        i.screen = true;
        i.screen_live = true;
        let card = build(i).phone.unwrap();
        assert!(card.connected);
        // "Last seen" is not kept while connected, so contact stamps made
        // during a session do not change the model.
        assert_eq!(card.last_seen, None);
    }

    #[test]
    fn a_reconnecting_session_is_not_connected() {
        let s = Settings::default();
        for camera in [false, true] {
            let mut i = inputs(&s, &[]);
            i.phone = Some(phone(false, false));
            // Open, but the phone stopped sending: waiting to reconnect.
            if camera {
                i.camera = true;
            } else {
                i.screen = true;
            }
            let m = build(i);
            let card = m.phone.clone().unwrap();
            assert!(!card.connected);
            assert_eq!(status_line(&card, NOW), "Last seen 5 min ago");
            assert_eq!(m.sessions.len(), 1);
            assert!(!m.sessions[0].live);
            assert_eq!(m.sessions[0].text().2, "Reconnecting\u{2026}");
        }
    }

    #[test]
    fn only_running_sessions_get_a_stop_button() {
        let s = Settings::default();
        let m = build(inputs(&s, &[]));
        assert!(m.sessions.is_empty());
        assert!(hit(&m, STOP).is_none());
        let mut i = inputs(&s, &[]);
        i.camera = true;
        i.control = true;
        let m = build(i);
        let kinds: Vec<SessionKind> = m.sessions.iter().map(|x| x.kind).collect();
        assert_eq!(kinds, vec![SessionKind::Camera, SessionKind::Control]);
        let stops: Vec<Hit> = layout(page(), 1.0, &m)
            .into_iter()
            .filter(|h| (STOP..STOP + 3).contains(&h.id))
            .collect();
        assert_eq!(stops.len(), 2);
        assert_eq!(
            click(&stops[0], &m),
            Some(Action::Stop(SessionKind::Camera))
        );
        assert_eq!(
            click(&stops[1], &m),
            Some(Action::Stop(SessionKind::Control))
        );
    }

    #[test]
    fn recent_lists_at_most_five_of_each() {
        let s = Settings::default();
        let notes: Vec<Notification> = (0..8).map(|k| note(&k.to_string())).collect();
        let entries: Vec<timeline::Entry> = (0..9)
            .map(|k| timeline::clip_text_entry("Pixel 8", timeline::Dir::ToPc, &"x".repeat(k + 1)))
            .collect();
        let mut i = inputs(&s, &notes);
        i.recent = &entries;
        let m = build(i);
        assert_eq!(m.notes.len(), RECENT);
        assert_eq!(m.recent.len(), RECENT);
        // Newest first, as given.
        assert_eq!(m.recent[0], entries[0]);
        assert_eq!(m.notes[0].title, "Mom");
        assert_eq!(m.notes[0].line, "See you at 6");
        // "See all" opens the Timeline.
        let all = hit(&m, SEE_ALL).expect("See all");
        assert_eq!(click(&all, &m), Some(Action::Go(Page::Timeline)));
    }

    #[test]
    fn quick_actions_map_to_the_flyout_actions() {
        let s = Settings::default();
        let m = build(inputs(&s, &[]));
        let want = [
            Action::Flyout(flyout::Action::SendFiles),
            // A one-off send (the tray's Send clipboard), not the switch.
            Action::SendClipboard,
            Action::Flyout(flyout::Action::ShowScreen),
            Action::Flyout(flyout::Action::ToggleCamera),
            Action::Flyout(flyout::Action::ControlPhone),
        ];
        for (k, (q, a)) in Quick::ALL.iter().zip(want).enumerate() {
            assert_eq!(q.action(), a);
            let h = hit(&m, QUICK + k as u32).expect("a quick action");
            assert_eq!(click(&h, &m), Some(a));
        }
        assert_eq!(Quick::Clipboard.label(), "Send clipboard");
        assert!(!Quick::Clipboard.on(&m));
    }

    #[test]
    fn hidden_notifications_are_not_listed() {
        let notes = [note("a")];
        // Notifications off, or the app muted: nothing.
        let off = Settings {
            notify_enabled: false,
            ..Default::default()
        };
        assert!(build(inputs(&off, &notes)).notes.is_empty());
        let muted = Settings {
            notify_apps: vec![("com.whatsapp".into(), "WhatsApp".into(), true)],
            ..Default::default()
        };
        assert!(build(inputs(&muted, &notes)).notes.is_empty());
        // Locked with "hide when locked": not listed at all.
        let hide = Settings {
            notify_content: 1,
            ..Default::default()
        };
        let mut i = inputs(&hide, &notes);
        i.locked = true;
        let m = build(i);
        assert!(m.notes.is_empty());
        // The card says why it is empty.
        assert!(m.notes_hidden);
        assert_eq!(notes_empty_text(&m), "Hidden while this PC is locked");
        let none = build(inputs(&hide, &[]));
        assert_eq!(
            notes_empty_text(&none),
            "No notifications from your phone right now"
        );
        // Sender only: the title, never the text.
        let sender = Settings {
            notify_content: 2,
            ..Default::default()
        };
        let m = build(inputs(&sender, &notes));
        assert_eq!(m.notes.len(), 1);
        assert_eq!(m.notes[0].title, "Mom");
        assert!(m.notes[0].line.is_empty());
        assert!(!format!("{:?}", m).contains("See you"));
    }

    #[test]
    fn layout_fits_its_height_and_ids_are_unique() {
        let s = Settings::default();
        let notes: Vec<Notification> = (0..3).map(|k| note(&k.to_string())).collect();
        let mut i = inputs(&s, &notes);
        i.screen = true;
        let m = build(i);
        let hits = layout(rectf(0.0, 0.0, 560.0, 100.0), 1.5, &m);
        let mut ids: Vec<u32> = hits.iter().map(|h| h.id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), hits.len());
        let h = content_height(560.0, 1.5, &m);
        assert!(h > 0.0);
        for x in &hits {
            assert!(x.r.bottom <= h * 1.5 + 0.5, "{x:?} past {h}");
            assert!(x.r.left >= 0.0 && x.r.right <= 560.0);
        }
    }
}
