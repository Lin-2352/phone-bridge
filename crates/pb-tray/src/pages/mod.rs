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
pub mod transfers;

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
    pub transfers: transfers::TransfersModel,
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
    /// Start audio sharing in this direction, or cancel its open question
    /// (the tray menu's entry).
    Audio(crate::audio_state::Dir),
    /// Send the clipboard's text once (the tray's "Send clipboard").
    SendClipboard,
    /// Start Relay again after its core stopped (the tray restarts itself).
    Restart,
    /// Resume Relay after a pause (the tray menu's Resume).
    Resume,
    /// End a running session the way the tray menu does.
    Stop(home::SessionKind),
    /// Remove the paired device at this index (the tray asks first).
    Unpair(usize),
    /// Make the paired device at this index the one sends go to when none is chosen.
    MakeDefault(usize),
    /// Cancel or retry the transfer with this id (the registry's own cancel and retry).
    TransferCancel(u64),
    TransferRetry(u64),
    /// Pause or resume the whole send queue (the file sending stops too and keeps its place).
    QueuePause,
    QueueResume,
    /// Take a waiting file out of a phone's queue (its queue key and the item id).
    QueueRemove(String, u64),
    /// Hold one waiting file, or let a held one go again (queue key and item id).
    QueueHold(String, u64),
    QueueRelease(String, u64),
    /// Cancel everything: what waits and what sends.
    QueueCancelAll,
    /// Pause the file being sent from this path (it goes back to the queue, held).
    TransferHold(std::path::PathBuf),
    /// Take a finished, failed or cancelled transfer off the list.
    TransferDismiss(u64),
    /// Tick or untick one Transfers row; tick every row; untick all.
    ToggleSelect(transfers::SelKey),
    SelectAll(Vec<transfers::SelKey>),
    ClearSelection,
    /// Several actions from one click (the bar over ticked rows).
    Many(Vec<Action>),
    /// Files dropped on the main window: sent to a phone (asking which when several are paired).
    DropFiles(Vec<std::path::PathBuf>),
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
    /// Timeline: the row with this key asked for its "..." menu (handled by
    /// the window, which owns the menu).
    TimelineMenu(u32),
    /// Put this text on the clipboard (a row's path); not shared with the phone.
    CopyText(String),
    /// Timeline: remove the entries with these keys from the history (a
    /// delete whose undo window passed).
    TimelineDelete(Vec<u32>),
}

/// What kind of control a hit is, for screen readers (UI Automation).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Button,
    /// On / off (switch).
    Toggle,
    /// One of a group of choices; `selected` says which.
    Radio,
    /// A day header: expands and collapses.
    Header,
    /// A list entry (a Timeline row).
    Item,
    /// A nav rail item; `selected` marks the page shown.
    Tab,
}

/// The accessible description of a hit: derived from the same model that
/// draws and hit-tests it, so the screen reader's tree cannot drift.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Desc {
    pub name: String,
    pub role: Role,
    pub toggled: Option<bool>,
    pub expanded: Option<bool>,
    pub selected: bool,
}

impl Desc {
    pub fn new(name: impl Into<String>, role: Role) -> Desc {
        Desc {
            name: name.into(),
            role,
            toggled: None,
            expanded: None,
            selected: false,
        }
    }
}

/// The description of `hit` on `page`.
pub fn describe(page: Page, hit: &Hit, m: &Model) -> Option<Desc> {
    match page {
        Page::Home => home::describe(hit, &m.home),
        Page::Timeline => timeline::describe(hit, &m.timeline),
        Page::Devices => devices::describe(hit, &m.devices),
        Page::Transfers => transfers::describe(hit, &m.transfers),
        Page::Settings => settings::describe(hit, &m.settings),
    }
}

/// Where `hit` really is when its rect is empty (a Settings control out of
/// view), so keyboard focus can scroll to it.
pub fn full_rect(page: Page, id: u32, r: RectF, s: f32, m: &Model) -> Option<RectF> {
    match page {
        Page::Settings => settings::full_rect(id, r, s, &m.settings),
        _ => None,
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
        Page::Transfers => transfers::layout(r, s, &m.transfers),
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
        Page::Transfers => transfers::content_height(width, s, &m.transfers),
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
        Page::Transfers => transfers::draw(c, r, &m.transfers),
        Page::Settings => settings::draw(c, r, view, &m.settings, hits),
    }
}

/// The action for a click (or Enter / Space) on `hit`.
pub fn click(page: Page, hit: &Hit, m: &Model) -> Option<Action> {
    match page {
        Page::Home => home::click(hit, &m.home),
        Page::Timeline => timeline::click(hit, &m.timeline),
        Page::Devices => devices::click(hit, &m.devices),
        Page::Transfers => transfers::click(hit, &m.transfers),
        Page::Settings => settings::click(hit, &m.settings),
    }
}
