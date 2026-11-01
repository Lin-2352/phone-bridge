//! Content pages of the main window. Each page lays out its clickable parts
//! as [`Hit`]s (pure, testable without a window), draws itself and the hits
//! over a `Canvas`, and turns a click on a hit into an [`Action`]. The window
//! owns the frame, the nav rail, scrolling and keyboard focus.
//!
//! Pages without a module yet show a placeholder empty state until Tasks
//! 10-11 add the `timeline`, `devices` and `settings` modules, which export
//! the same functions as `home` (`layout`, `content_height`, `draw`,
//! `click`) and are dispatched from here.

pub mod home;

use crate::fluent::{Canvas, RectF};
use crate::main_window::Page;
use crate::widgets;

/// What the pages show, rebuilt by the UI thread on every model change
/// (`main_window::refresh`). Tasks 10-11 add their fields.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Model {
    pub home: home::HomeModel,
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
    /// End a running session the way the tray menu does.
    Stop(home::SessionKind),
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
/// scale `s`.
pub fn layout(page: Page, r: RectF, s: f32, m: &Model) -> Vec<Hit> {
    match page {
        Page::Home => home::layout(r, s, &m.home),
        _ => Vec::new(),
    }
}

/// Height of the page's content in logical px, for scrolling; 0 when it
/// always fits.
pub fn content_height(page: Page, width: f32, s: f32, m: &Model) -> f32 {
    match page {
        Page::Home => home::content_height(width, s, &m.home),
        _ => 0.0,
    }
}

/// Draws the page into `r`, the same scrolled content rect `layout` got
/// (the window clips to the visible pane). `hits` is this frame's `layout`.
pub fn draw(page: Page, c: &mut Canvas, r: RectF, m: &Model, hits: &[Hit]) {
    if page == Page::Home {
        return home::draw(c, r, &m.home, hits);
    }
    let (glyph, title, body) = placeholder(page);
    widgets::empty_state(c, r, glyph, title, body);
}

/// The action for a click (or Enter / Space) on `hit`.
pub fn click(page: Page, hit: &Hit, m: &Model) -> Option<Action> {
    match page {
        Page::Home => home::click(hit, &m.home),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fluent::rectf;

    #[test]
    fn placeholders_have_no_hits_and_fit() {
        let m = Model::default();
        for page in Page::ALL.into_iter().filter(|p| *p != Page::Home) {
            let r = rectf(0.0, 0.0, 800.0, 600.0);
            assert!(layout(page, r, 1.0, &m).is_empty());
            assert_eq!(content_height(page, 800.0, 1.0, &m), 0.0);
            assert!(!placeholder(page).1.is_empty());
        }
    }
}
