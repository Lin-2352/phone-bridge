//! Content pages of the main window. Each page lays out its clickable parts
//! as [`Hit`]s (pure, testable without a window), draws itself and the hits
//! over a `Canvas`, and turns a click on a hit into an [`Action`]. The window
//! owns the frame, the nav rail, scrolling and keyboard focus.
//!
//! Settings shows a placeholder empty state until Task 11 adds its module,
//! which exports the same functions as `home` (`layout`, `content_height`,
//! `draw`, `click`) and is dispatched from here.

pub mod devices;
pub mod home;
pub mod timeline;

use std::sync::Arc;

use crate::fluent::{Canvas, RectF};
use crate::main_window::Page;
use crate::widgets;

/// What the pages show, rebuilt by the UI thread on every model change
/// (`main_window::refresh`). Tasks 10-11 add their fields.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Model {
    pub home: home::HomeModel,
    /// Shared: the paint snapshot clones the model every frame.
    pub timeline: Arc<timeline::TimelineModel>,
    pub devices: devices::DevicesModel,
}

/// A clickable, focusable part of a page. `id` is unique within its page and
/// stable across layouts (keyboard focus follows it); `r` is in pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hit {
    pub id: u32,
    pub r: RectF,
}

/// What a click on a page asks of the window.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    /// Switch to another page ("See all" on Home opens the Timeline).
    Go(Page),
    /// Run a flyout command (Home's quick actions and Pair), through the
    /// same handler the flyout uses.
    Flyout(crate::flyout::Action),
    /// Send the clipboard's text once (the tray's "Send clipboard").
    SendClipboard,
    /// End a running session the way the tray menu does.
    Stop(home::SessionKind),
    /// Remove the paired device at this index (the tray asks first).
    Unpair(usize),
    /// Open a received or sent file, or show it in Explorer; the tray
    /// checks first that it is still there.
    OpenFile(String),
    ShowInFolder(String),
}

/// Glyph, title and body of a page's placeholder.
fn placeholder(page: Page) -> (char, &'static str, &'static str) {
    match page {
        Page::Home => (
            '\u{E80F}',
            "Home",
            "Your phone at a glance will appear here.",
        ),
        Page::Timeline => (
            '\u{E81C}',
            "Timeline",
            "What moved between your devices will appear here.",
        ),
        Page::Devices => (
            '\u{E8EA}',
            "Devices",
            "Your paired devices will appear here.",
        ),
        Page::Settings => ('\u{E713}', "Settings", "Relay settings will appear here."),
    }
}

/// The page's hits inside `r` (its content rect, already scrolled), at
/// scale `s`. `view` is the visible pane in pixels: long lists lay out only
/// what is in it.
pub fn layout(page: Page, r: RectF, view: RectF, s: f32, m: &Model) -> Vec<Hit> {
    match page {
        Page::Home => home::layout(r, s, &m.home),
        Page::Timeline => timeline::layout(r, view, s, &m.timeline),
        Page::Devices => devices::layout(r, s, &m.devices),
        Page::Settings => Vec::new(),
    }
}

/// Height of the page's content in logical px, for scrolling; 0 when it
/// always fits.
pub fn content_height(page: Page, width: f32, s: f32, m: &Model) -> f32 {
    match page {
        Page::Home => home::content_height(width, s, &m.home),
        Page::Timeline => timeline::content_height(width, s, &m.timeline),
        Page::Devices => devices::content_height(width, s, &m.devices),
        Page::Settings => 0.0,
    }
}

/// Draws the page into `r`, the same scrolled content rect `layout` got
/// (the window clips to the visible pane `view`). `hits` is this frame's
/// `layout`.
pub fn draw(page: Page, c: &mut Canvas, r: RectF, view: RectF, m: &Model, hits: &[Hit]) {
    match page {
        Page::Home => home::draw(c, r, &m.home, hits),
        Page::Timeline => timeline::draw(c, r, view, &m.timeline),
        Page::Devices => devices::draw(c, r, &m.devices),
        Page::Settings => {
            let (glyph, title, body) = placeholder(page);
            widgets::empty_state(c, r, glyph, title, body);
        }
    }
}

/// The action for a click (or Enter / Space) on `hit`.
pub fn click(page: Page, hit: &Hit, m: &Model) -> Option<Action> {
    match page {
        Page::Home => home::click(hit, &m.home),
        Page::Timeline => timeline::click(hit, &m.timeline),
        Page::Devices => devices::click(hit, &m.devices),
        Page::Settings => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fluent::rectf;

    #[test]
    fn placeholders_have_no_hits_and_fit() {
        let (m, page) = (Model::default(), Page::Settings);
        let r = rectf(0.0, 0.0, 800.0, 600.0);
        assert!(layout(page, r, r, 1.0, &m).is_empty());
        assert_eq!(content_height(page, 800.0, 1.0, &m), 0.0);
        assert!(!placeholder(page).1.is_empty());
    }
}
