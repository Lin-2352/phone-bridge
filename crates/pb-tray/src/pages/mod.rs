//! Content pages of the main window. Each page lays out its clickable parts
//! as [`Hit`]s (pure, testable without a window), draws itself and the hits
//! over a `Canvas`, and turns a click on a hit into an [`Action`]. The window
//! owns the frame, the nav rail, scrolling, keyboard focus and the Settings
//! search box (a native child it places at `settings::search_rect`).

pub mod devices;
pub mod home;
pub mod settings;
pub mod timeline;
pub mod timeline_view;

use std::sync::Arc;

use crate::fluent::{Canvas, RectF};
use crate::main_window::Page;
use crate::{settings_groups, settings_rows};

/// What the pages show, rebuilt by the UI thread on every model change
/// (`main_window::refresh`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Model {
    pub home: home::HomeModel,
    /// Shared: the paint snapshot clones the model every frame.
    pub timeline: Arc<timeline::TimelineModel>,
    pub devices: devices::DevicesModel,
    /// The tray fills `rows`; the window keeps the sub-page and the query
    /// (they are its own state) and copies them in.
    pub settings: settings::SettingsModel,
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
    /// Resume Relay after a pause (the tray menu's Resume).
    Resume,
    /// End a running session the way the tray menu does.
    Stop(home::SessionKind),
    /// Remove the paired device at this index (the tray asks first).
    Unpair(usize),
    /// Open a received or sent file, or show it in Explorer; the tray
    /// checks first that it is still there.
    OpenFile(String),
    ShowInFolder(String),
    /// A Settings control, applied by the tray as the settings window did.
    Settings(settings_rows::Action),
    /// Show another Settings page (handled by the window, like `Go`).
    SettingsPage(settings_groups::Page),
    /// Timeline: open or close one day (its local day number), or all of
    /// them (`true` = open). Handled by the window, which owns the state.
    TimelineDay(i64),
    TimelineAll(bool),
}

/// The page's hits inside `r` (its content rect, already scrolled), at
/// scale `s`. `view` is the visible pane in pixels: long lists lay out only
/// what is in it.
pub fn layout(page: Page, r: RectF, view: RectF, s: f32, m: &Model) -> Vec<Hit> {
    match page {
        Page::Home => home::layout(r, s, &m.home),
        Page::Timeline => timeline::layout(r, view, s, &m.timeline),
        Page::Devices => devices::layout(r, s, &m.devices),
        Page::Settings => settings::layout(r, view, s, &m.settings),
    }
}

/// Height of the page's content in logical px, for scrolling; 0 when it
/// always fits.
pub fn content_height(page: Page, width: f32, s: f32, m: &Model) -> f32 {
    match page {
        Page::Home => home::content_height(width, s, &m.home),
        Page::Timeline => timeline::content_height(width, s, &m.timeline),
        Page::Devices => devices::content_height(width, s, &m.devices),
        Page::Settings => settings::content_height(width, s, &m.settings),
    }
}

/// Draws the page into `r`, the same scrolled content rect `layout` got
/// (the window clips to the visible pane `view`). `hits` is this frame's
/// `layout`.
pub fn draw(page: Page, c: &mut Canvas, r: RectF, view: RectF, m: &Model, hits: &[Hit]) {
    match page {
        Page::Home => home::draw(c, r, &m.home, hits),
        Page::Timeline => timeline::draw(c, r, view, &m.timeline, hits),
        Page::Devices => devices::draw(c, r, &m.devices),
        Page::Settings => settings::draw(c, r, view, &m.settings, hits),
    }
}

/// The action for a click (or Enter / Space) on `hit`.
pub fn click(page: Page, hit: &Hit, m: &Model) -> Option<Action> {
    match page {
        Page::Home => home::click(hit, &m.home),
        Page::Timeline => timeline::click(hit, &m.timeline),
        Page::Devices => devices::click(hit, &m.devices),
        Page::Settings => settings::click(hit, &m.settings),
    }
}
