//! Devices: the paired devices, each with its fingerprint (or the
//! "Reinstalled or reset" note) and Remove, and "Pair a device". Remove and
//! Pair take the same paths as the tray (Remove asks first).

use crate::{
    fluent::{self, rectf, text, Align, Canvas, Font, RectF},
    flyout,
    pages::{Action, Hit},
    settings_rows,
    tokens::SPACE,
    widgets,
};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DevicesModel {
    /// (name, short fingerprint, answers as a different device now), in
    /// the tray's peer order (Remove names a device by its index).
    pub devices: Vec<(String, String, bool)>,
    /// Index of the default device (sends go there when none is chosen), if any.
    pub default: Option<usize>,
}

impl DevicesModel {
    /// Whether row `k` offers "Make default": there is a choice, and it is not the default already.
    fn can_make_default(&self, k: usize) -> bool {
        self.devices.len() > 1 && self.default != Some(k)
    }
}

/// The line under a device's name, and whether it is a warning (drawn in
/// the danger colour).
pub fn note(d: &(String, String, bool)) -> (String, bool) {
    (settings_rows::device_note(&d.1, d.2), d.2)
}

/// Hit ids.
const PAIR: u32 = 1;
const REMOVE: u32 = 10;
const MAKE_DEFAULT: u32 = 1000;

/// Sizes in logical px.
const PAD: f32 = 24.0;
const TITLE_H: f32 = 40.0;
const EMPTY_H: f32 = 184.0;
const ROW_H: f32 = 64.0;
const REMOVE_W: f32 = 88.0;
const DEFAULT_W: f32 = 120.0;
const PAIR_W: f32 = 136.0;
const BUTTON_H: f32 = 32.0;

/// Where everything goes, in pixels.
struct Parts {
    title: RectF,
    /// The card: the device rows, or the empty state.
    card: RectF,
    /// Row, its Remove button and its Make default button, per device.
    rows: Vec<(RectF, RectF, RectF)>,
    pair: RectF,
    bottom: f32,
}

fn parts(r: RectF, s: f32, m: &DevicesModel) -> Parts {
    let (left, right) = (r.left + PAD * s, (r.right - PAD * s).max(r.left + PAD * s));
    let top = r.top + PAD * s;
    let title = rectf(left, top, right, top + TITLE_H * s);
    if m.devices.is_empty() {
        let card = rectf(left, title.bottom, right, title.bottom + EMPTY_H * s);
        let cx = (left + right) / 2.0;
        let pair = rectf(
            cx - PAIR_W * s / 2.0,
            card.bottom - (SPACE[4] + BUTTON_H) * s,
            cx + PAIR_W * s / 2.0,
            card.bottom - SPACE[4] * s,
        );
        return Parts {
            title,
            card,
            rows: Vec::new(),
            pair,
            bottom: card.bottom + PAD * s,
        };
    }
    let n = m.devices.len() as f32;
    let card = rectf(left, title.bottom, right, title.bottom + n * ROW_H * s);
    let rows = (0..m.devices.len())
        .map(|k| {
            let t = card.top + k as f32 * ROW_H * s;
            let row = rectf(left, t, right, t + ROW_H * s);
            let cy = t + ROW_H * s / 2.0;
            let pad = SPACE[3] * s;
            let b = rectf(
                (right - pad - REMOVE_W * s).max(left),
                cy - BUTTON_H * s / 2.0,
                (right - pad).max(left),
                cy + BUTTON_H * s / 2.0,
            );
            let d = rectf(
                (b.left - SPACE[2] * s - DEFAULT_W * s).max(left),
                b.top,
                (b.left - SPACE[2] * s).max(left),
                b.bottom,
            );
            (row, b, d)
        })
        .collect();
    let pt = card.bottom + SPACE[3] * s;
    let pair = rectf(left, pt, left + PAIR_W * s, pt + BUTTON_H * s);
    Parts {
        title,
        card,
        rows,
        pair,
        bottom: pair.bottom + PAD * s,
    }
}

pub fn layout(r: RectF, s: f32, m: &DevicesModel) -> Vec<Hit> {
    let p = parts(r, s, m);
    let mut hits: Vec<Hit> = Vec::new();
    for (k, (_, b, d)) in p.rows.iter().enumerate() {
        hits.push(Hit {
            id: REMOVE + k as u32,
            r: *b,
        });
        if m.can_make_default(k) {
            hits.push(Hit {
                id: MAKE_DEFAULT + k as u32,
                r: *d,
            });
        }
    }
    hits.push(Hit {
        id: PAIR,
        r: p.pair,
    });
    hits
}

/// `width` in pixels; the height in logical px.
pub fn content_height(width: f32, s: f32, m: &DevicesModel) -> f32 {
    parts(rectf(0.0, 0.0, width, 0.0), s, m).bottom / s
}

pub fn draw(c: &mut Canvas, r: RectF, m: &DevicesModel) {
    let (s, pal) = (c.s, c.p);
    let p = parts(r, s, m);
    text(c, Font::Title, "Devices", p.title, pal.text, Align::Left);
    widgets::card(c, p.card);
    if m.devices.is_empty() {
        let top = rectf(
            p.card.left,
            p.card.top,
            p.card.right,
            p.card.bottom - (SPACE[4] + BUTTON_H) * s,
        );
        widgets::empty_state(
            c,
            top,
            fluent::glyph::PHONE,
            "No devices paired",
            "Pair your phone to send files, share the clipboard and more.",
        );
    }
    for (k, (d, (row, b, mk))) in m.devices.iter().zip(&p.rows).enumerate() {
        let (mut line, warn) = note(d);
        if m.default == Some(k) && m.devices.len() > 1 {
            line = format!("Default \u{b7} {line}");
        }
        let pad = fluent::ROW_PAD as f32 * s;
        let cy = (row.top + row.bottom) / 2.0;
        let g = rectf(
            row.left + pad,
            cy - 10.0 * s,
            row.left + pad + 20.0 * s,
            cy + 10.0 * s,
        );
        fluent::icon(
            c,
            fluent::glyph::PHONE,
            g,
            if warn { pal.danger } else { pal.text2 },
        );
        let tx = g.right + SPACE[2] * s;
        let tr = if m.can_make_default(k) {
            mk.left
        } else {
            b.left
        }
        .max(tx + 1.0)
            - SPACE[2] * s;
        let tr = tr.max(tx);
        let name = rectf(tx, cy - 20.0 * s, tr, cy);
        text(c, Font::Body, &d.0, name, pal.text, Align::Left);
        let meta = rectf(tx, cy, tr, cy + 18.0 * s);
        let rgb = if warn { pal.danger } else { pal.text2 };
        text(c, Font::Caption, &line, meta, rgb, Align::Left);
        if m.can_make_default(k) {
            fluent::button(c, *mk, "Make default", false);
        }
        fluent::button(c, *b, "Remove", false);
    }
    fluent::button(c, p.pair, crate::strings::PAIR_DEVICE, true);
}

pub fn describe(hit: &Hit, m: &DevicesModel) -> Option<super::Desc> {
    use super::{Desc, Role};
    match hit.id {
        PAIR => Some(Desc::new(crate::strings::PAIR_DEVICE, Role::Button)),
        id if id >= MAKE_DEFAULT => {
            let d = m.devices.get((id - MAKE_DEFAULT) as usize)?;
            Some(Desc::new(
                format!("Make {} the default device", d.0),
                Role::Button,
            ))
        }
        id if id >= REMOVE => {
            let d = m.devices.get((id - REMOVE) as usize)?;
            Some(Desc::new(format!("Remove {}", d.0), Role::Button))
        }
        _ => None,
    }
}

pub fn click(hit: &Hit, m: &DevicesModel) -> Option<Action> {
    match hit.id {
        PAIR => Some(Action::Flyout(flyout::Action::Pair)),
        id if id >= MAKE_DEFAULT && ((id - MAKE_DEFAULT) as usize) < m.devices.len() => {
            Some(Action::MakeDefault((id - MAKE_DEFAULT) as usize))
        }
        id if id >= REMOVE && ((id - REMOVE) as usize) < m.devices.len() => {
            Some(Action::Unpair((id - REMOVE) as usize))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{fluent::rectf, flyout, settings_rows};

    #[test]
    fn every_hit_has_a_name() {
        for m in [model(), DevicesModel::default()] {
            for h in layout(page(), 1.0, &m) {
                let d = describe(&h, &m).expect("described");
                assert!(!d.name.is_empty());
            }
        }
    }

    fn model() -> DevicesModel {
        DevicesModel {
            devices: vec![
                ("Pixel 8".into(), "18CF-F6D8".into(), false),
                ("Galaxy".into(), "AB12-CD34".into(), true),
            ],
            default: Some(0),
        }
    }

    fn page() -> RectF {
        rectf(0.0, 0.0, 800.0, 2000.0)
    }

    #[test]
    fn device_row_uses_the_reinstalled_note() {
        let m = model();
        assert_eq!(
            note(&m.devices[0]),
            (settings_rows::device_note("18CF-F6D8", false), false)
        );
        let (n, warn) = note(&m.devices[1]);
        assert_eq!(n, settings_rows::device_note("AB12-CD34", true));
        assert!(warn, "drawn in the danger colour");
        assert!(!n.contains("AB12"));
    }

    #[test]
    fn remove_and_pair_use_the_tray_actions() {
        let m = model();
        let hits = layout(page(), 1.0, &m);
        // Remove per device, Make default for the one that is not the default, then Pair a device.
        assert_eq!(hits.len(), 4);
        assert_eq!(click(&hits[0], &m), Some(Action::Unpair(0)));
        assert_eq!(click(&hits[1], &m), Some(Action::Unpair(1)));
        assert_eq!(click(&hits[2], &m), Some(Action::MakeDefault(1)));
        assert_eq!(
            click(&hits[3], &m),
            Some(Action::Flyout(flyout::Action::Pair))
        );
        let h = content_height(800.0, 1.5, &m);
        for x in layout(rectf(0.0, 0.0, 800.0, 10.0), 1.5, &m) {
            assert!(x.r.bottom <= h * 1.5 + 0.5);
        }
    }

    #[test]
    fn the_default_device_offers_no_make_default() {
        let mut m = model();
        m.default = Some(1);
        let ids: Vec<Action> = layout(page(), 1.0, &m)
            .iter()
            .filter_map(|h| click(h, &m))
            .collect();
        assert!(ids.contains(&Action::MakeDefault(0)));
        assert!(!ids.contains(&Action::MakeDefault(1)));
        let one = DevicesModel {
            devices: vec![("Pixel 8".into(), "18CF-F6D8".into(), false)],
            default: Some(0),
        };
        assert!(!layout(page(), 1.0, &one)
            .iter()
            .any(|h| h.id >= MAKE_DEFAULT));
    }

    #[test]
    fn nothing_paired_offers_pair() {
        let m = DevicesModel::default();
        let hits = layout(page(), 1.0, &m);
        assert_eq!(hits.len(), 1);
        assert_eq!(
            click(&hits[0], &m),
            Some(Action::Flyout(flyout::Action::Pair))
        );
    }
}
