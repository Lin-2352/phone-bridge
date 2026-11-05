//! Win32 tray UI: notification-area icon, context menu, balloons and modal
//! prompts. All `unsafe` of the tray lives here.
#![allow(unsafe_code)]

use std::{
    collections::VecDeque,
    sync::{Arc, Mutex, OnceLock},
};

use pb_platform_win::clipboard::{self, ClipText};
use tokio::sync::mpsc::UnboundedSender;
use windows::{
    core::{w, PCWSTR},
    Win32::{
        Foundation::{HWND, LPARAM, LRESULT, WPARAM},
        System::LibraryLoader::GetModuleHandleW,
        UI::{
            Shell::{
                Shell_NotifyIconW, NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_TIP, NIIF_INFO, NIM_ADD,
                NIM_DELETE, NIM_MODIFY, NIM_SETVERSION, NOTIFYICONDATAW, NOTIFYICON_VERSION_4,
            },
            WindowsAndMessaging::{
                CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, LoadIconW,
                PostMessageW, PostQuitMessage, RegisterClassW, TranslateMessage, IDI_APPLICATION,
                MSG, WM_APP, WM_COMMAND, WM_CONTEXTMENU, WM_DESTROY, WM_ENDSESSION,
                WM_SETTINGCHANGE, WNDCLASSW,
            },
        },
    },
};

use crate::{
    core::{Cmd, Peers, UiEvent},
    dialog_window::Dialog,
};

/// Window class name; test tooling finds the window by it (hidden top-level window).
pub const CLASS: PCWSTR = w!("PhoneBridgeTray");
const WM_TRAY: u32 = WM_APP + 1;
pub const WM_CORE_EVENT: u32 = WM_APP + 2;

pub const ID_PAIR: usize = 1;
pub const ID_SEND_CLIPBOARD: usize = 2;
pub const ID_QUIT: usize = 3;
/// Why the core stopped (it never came up, or died): Home, the flyout and
/// the tooltip say so, and Restart starts the tray again.
static CORE_ERROR: Mutex<Option<String>> = Mutex::new(None);

fn core_error() -> Option<String> {
    CORE_ERROR.lock().ok().and_then(|g| g.clone())
}

/// Called by the core thread when `core::run` ends with an error.
pub fn core_failed(hwnd_raw: isize, msg: String) {
    if let Ok(mut g) = CORE_ERROR.lock() {
        *g = Some(msg.clone());
    }
    post_event(hwnd_raw, UiEvent::Info(format!("Relay stopped: {msg}")));
}

/// Starts a new tray (same data folder) a moment after this one is gone
/// (the single-instance mutex is released then), and quits this one.
fn restart_app(raw: isize) {
    if let Ok(exe) = std::env::current_exe() {
        use std::os::windows::process::CommandExt;
        let _ = std::process::Command::new("cmd")
            .raw_arg(crate::relaunch::restart_line(&exe, crate::relaunch::args()))
            .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
            .spawn();
    }
    post_command(raw, ID_QUIT);
}

/// The user said yes to "a transfer is running, quit anyway?".
static QUIT_CONFIRMED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
const ID_UNPAIR_BASE: usize = 100;

struct State {
    cmds: UnboundedSender<Cmd>,
    peers: Peers,
    auto_accept: bool,
    log: crate::core::Log,
    settings: crate::core::SharedSettings,
    settings_path: std::path::PathBuf,
}

pub const ID_SEND_FILES: usize = 4;
pub const ID_TOGGLE_CLIP: usize = 5;
const ID_TOGGLE_AUTO: usize = 6;
const ID_OPEN_FOLDER: usize = 7;
const ID_SENDTO: usize = 8;
const ID_TOGGLE_PRIVATE: usize = 9;
pub const ID_OPEN_FLYOUT: usize = 10;
pub const ID_SETTINGS: usize = 11;
pub const ID_MENU: usize = 12;
const ID_CONTROL: usize = 13;
pub const ID_SCREEN: usize = 14;
pub const ID_SCREEN_SHOW: usize = 15;
pub const ID_SCREEN_HIDE: usize = 16;
pub const ID_CAMERA: usize = 17;
pub const ID_CAMERA_START: usize = 18;
pub const ID_CAMERA_STOP: usize = 19;
const ID_CAMERA_LENS: usize = 20;
const ID_CAMERA_REMOVE: usize = 21;
pub const ID_CAMERA_WINDOW: usize = 22;
pub const ID_CAMERA_REPAIR: usize = 23;
/// The main window on Home (menu "Open Relay", IPC `--open`).
pub const ID_OPEN_MAIN: usize = 24;
/// Ends PC -> phone control if it still runs (Home's Stop; never starts it).
const ID_CONTROL_STOP: usize = 25;
/// The tray's hidden window, for the main window's actions.
static TRAY_HWND: std::sync::atomic::AtomicIsize = std::sync::atomic::AtomicIsize::new(0);
const TIMER_CONTROL: usize = 8;
const HOTKEY_CONTROL: i32 = 1;
const WM_SETTINGS_ACTION: u32 = WM_APP + 5;
/// Each peer check dials every paired device (5 s timeout each): at most
/// one per this long, whoever asks.
const PEER_CHECK_EVERY: std::time::Duration = std::time::Duration::from_secs(30);
static LAST_PEER_CHECK: Mutex<Option<std::time::Instant>> = Mutex::new(None);

/// Whether a peer check may run at `now` after the one at `last`.
fn peer_check_due(last: Option<std::time::Instant>, now: std::time::Instant) -> bool {
    last.is_none_or(|t| {
        now.checked_duration_since(t)
            .is_some_and(|d| d >= PEER_CHECK_EVERY)
    })
}

/// Asks which devices still answer as the paired one (Settings and Devices
/// show the "Reinstalled or reset" note from it), debounced.
fn check_peers(cmds: &UnboundedSender<Cmd>) {
    let now = std::time::Instant::now();
    let due = LAST_PEER_CHECK
        .lock()
        .map(|mut last| {
            let due = peer_check_due(*last, now);
            if due {
                *last = Some(now);
            }
            due
        })
        .unwrap_or(true);
    if due {
        let _ = cmds.send(Cmd::CheckPeers);
    }
}

/// Settings page controls, applied on the UI thread (`WM_SETTINGS_ACTION`).
static SETTINGS_ACTIONS: Mutex<VecDeque<crate::settings_rows::Action>> =
    Mutex::new(VecDeque::new());
/// A Timeline file action finished (toasts queued in `PAGE_FILES`), or a
/// file's state changed: toast and repaint the main window.
const WM_PAGE_FILE: u32 = WM_APP + 8;
static PAGE_FILES: Mutex<VecDeque<String>> = Mutex::new(VecDeque::new());
static MY_FINGERPRINT: Mutex<String> = Mutex::new(String::new());
const WM_CLIPBOARDUPDATE: u32 = 0x031D;
const NIN_BALLOONUSERCLICK: u32 = 0x0405;
/// Version-4 tray callbacks: left click / keyboard Enter on the icon.
const NIN_SELECT: u32 = 0x0400;
const NIN_KEYSELECT: u32 = 0x0401;
const TIMER_CLIP: usize = 7;
/// Checks whether mirrored toasts were cleared in the notification centre.
const TIMER_NOTIFY: usize = 9;
/// Clears a copied one-time code after 60 s.
const TIMER_OTP: usize = 10;
/// Writes held-back timeline entries (a trailing write within about a
/// second) and refreshes the open main window and flyout. Runs only while
/// one of those needs it (`timeline_timer_needed`).
const TIMER_TIMELINE: usize = 11;
/// 30 s after the last Direct2D surface on the UI thread went (the main
/// window, flyout, menu, QR, camera window and pill): release the shared
/// device and trim the working set. Dialogs and the screen window run on
/// their own threads; their devices go when those threads end.
const TIMER_RELEASE: usize = 12;
/// How long the device stays after the last window, for a quick reopen.
const RELEASE_AFTER_MS: u32 = 30_000;
/// The timeline timer is armed (set before SetTimer, cleared after KillTimer).
static TIMELINE_ARMED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// Another thread added a timeline entry: arm the timeline timer.
const WM_TIMELINE_WAKE: u32 = WM_APP + 9;
const WM_TOAST: u32 = WM_APP + 7;
const WM_WTSSESSION_CHANGE: u32 = 0x02B1;
const WTS_SESSION_LOCK: usize = 7;
const WTS_SESSION_UNLOCK: usize = 8;

/// A click, button, reply or dismissal on one of our toasts.
enum ToastEvent {
    /// Raw arguments (parsed on the UI thread) and the reply text.
    Activated(String, Option<String>),
    Dismissed(String),
}
static TOAST_EVENTS: Mutex<VecDeque<ToastEvent>> = Mutex::new(VecDeque::new());

/// Phone Bridge's own messages kept in the notification centre at once.
const OWN_TOASTS: u64 = 5;

/// Toasts for phone notifications and Phone Bridge's own messages.
#[derive(Default)]
struct NotifyUi {
    mirror: crate::notify::Mirror,
    /// tag -> (group, index of the reply action, picture file)
    meta: std::collections::HashMap<String, (String, Option<u32>, Option<std::path::PathBuf>)>,
    locked: bool,
    /// (clipboard sequence right after copying, the code)
    otp_pending: Option<(u32, String)>,
    /// Real toasts work; otherwise the old balloons are used.
    toasts_ok: bool,
    own_counter: u64,
    /// The own toast a click acts on (latest file or link).
    last_own_tag: String,
    /// Toasts shown without content because the PC was locked, kept to be
    /// shown in full (quietly) at unlock.
    hidden: std::collections::HashMap<String, HiddenToast>,
}

/// (phone, notification, app icon, picture)
type HiddenToast = (
    pb_identity::PeerId,
    Box<pb_protocol::control::Notification>,
    Option<std::path::PathBuf>,
    Option<std::path::PathBuf>,
);
static NOTIFY: std::sync::LazyLock<Mutex<NotifyUi>> = std::sync::LazyLock::new(Mutex::default);

fn notify_ui() -> std::sync::MutexGuard<'static, NotifyUi> {
    NOTIFY.lock().unwrap_or_else(|p| p.into_inner())
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Clipboard sequence number right after our own write, so the listener
/// does not send back what we just received.
static OWN_SEQ: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
/// Last received file, opened when the user clicks the notification.
static LAST_FILE: Mutex<Option<std::path::PathBuf>> = Mutex::new(None);
/// Last received link; when set, a notification click opens it instead.
static LAST_LINK: Mutex<Option<String>> = Mutex::new(None);

/// Open a received link in the default browser. Web links only: anything else
/// (file:, ms-settings:, custom schemes) could launch arbitrary handlers.
fn open_link(url: &str) {
    let lower = url.to_ascii_lowercase();
    if !(lower.starts_with("https://") || lower.starts_with("http://")) {
        return;
    }
    let wide_url = wide(url);
    // SAFETY: valid NUL-terminated strings for the duration of the call.
    unsafe {
        windows::Win32::UI::Shell::ShellExecuteW(
            None,
            w!("open"),
            PCWSTR(wide_url.as_ptr()),
            PCWSTR::null(),
            PCWSTR::null(),
            windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL,
        );
    }
}

fn clipboard_seq() -> u32 {
    // SAFETY: plain query with no arguments.
    unsafe { windows::Win32::System::DataExchange::GetClipboardSequenceNumber() }
}

fn set_clipboard_own(hwnd: HWND, text: &str, sensitive: bool) -> std::io::Result<()> {
    clipboard::set_text(text, sensitive, Some(hwnd))?;
    OWN_SEQ.store(clipboard_seq(), std::sync::atomic::Ordering::Relaxed);
    Ok(())
}

/// Put a received PNG on the clipboard (as a picture, plus the PNG format) and
/// remember our own write so the listener does not send it straight back.
/// `private` keeps it out of Win+V history and the cloud clipboard.
fn set_clipboard_image_own(hwnd: HWND, png: &[u8], private: bool) -> Result<(), String> {
    let dib = pb_platform_win::dib::png_to_dibv5(png)?;
    clipboard::set_image(png, &dib, private, Some(hwnd)).map_err(|e| e.to_string())?;
    OWN_SEQ.store(clipboard_seq(), std::sync::atomic::Ordering::Relaxed);
    Ok(())
}

/// Send the image on the clipboard to the core as a PNG, converting a DIB on a worker
/// thread (a large screenshot takes a moment to encode). Nothing happens if the
/// clipboard has no shareable image.
fn read_clipboard_png(hwnd: HWND, cmds: UnboundedSender<Cmd>) {
    use clipboard::ClipImage;
    // A private image (from a password manager) is never shared.
    let Ok((Some(img), false)) = clipboard::get_image(Some(hwnd)) else {
        return;
    };
    std::thread::spawn(move || {
        let png = match img {
            ClipImage::Png(p) => Some(p),
            ClipImage::Dib(d) => pb_platform_win::dib::dib_to_png(&d).ok(),
        };
        if let Some(png) = png {
            let _ = cmds.send(Cmd::ClipboardImageChanged { png });
        }
    });
}

fn sendto_folder() -> Option<std::path::PathBuf> {
    std::env::var_os("APPDATA").map(|a| {
        std::path::PathBuf::from(a)
            .join("Microsoft")
            .join("Windows")
            .join("SendTo")
    })
}

fn sendto_link() -> Option<std::path::PathBuf> {
    sendto_folder().map(|d| d.join(crate::app_name::LINK_NAME))
}

/// The shortcut an older build made under its old name.
fn legacy_sendto_link() -> Option<std::path::PathBuf> {
    sendto_folder().map(|d| d.join(crate::app_name::LEGACY_LINK_NAME))
}

/// Whether the Send to entry exists (under either name).
fn sendto_enabled() -> bool {
    [sendto_link(), legacy_sendto_link()]
        .into_iter()
        .flatten()
        .any(|l| l.exists())
}

/// Create (or remove) the Explorer "Send to > Phone Bridge" shortcut. Explorer
/// runs it as `pb-tray.exe --send <selected files>`, which reaches the running
/// tray over the local pipe.
fn toggle_sendto() -> Result<bool, String> {
    use windows::{
        core::{Interface, HSTRING},
        Win32::{
            System::Com::{
                CoCreateInstance, CoInitializeEx, IPersistFile, CLSCTX_INPROC_SERVER,
                COINIT_APARTMENTTHREADED,
            },
            UI::Shell::{IShellLinkW, ShellLink},
        },
    };
    let link = sendto_link().ok_or("no APPDATA folder")?;
    if sendto_enabled() {
        for l in [Some(link.clone()), legacy_sendto_link()]
            .into_iter()
            .flatten()
        {
            if l.exists() {
                std::fs::remove_file(&l).map_err(|e| e.to_string())?;
            }
        }
        return Ok(false);
    }
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    // SAFETY: standard IShellLink usage on this thread's STA; strings are
    // HSTRINGs that outlive each call.
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let sl: IShellLinkW =
            CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).map_err(|e| e.to_string())?;
        sl.SetPath(&HSTRING::from(exe.as_os_str()))
            .map_err(|e| e.to_string())?;
        sl.SetArguments(&HSTRING::from(send_to_arguments()))
            .map_err(|e| e.to_string())?;
        sl.SetDescription(&HSTRING::from("Send to your phone with Relay"))
            .map_err(|e| e.to_string())?;
        sl.SetIconLocation(&HSTRING::from(exe.as_os_str()), 0)
            .map_err(|e| e.to_string())?;
        let pf: IPersistFile = sl.cast().map_err(|e| e.to_string())?;
        pf.Save(&HSTRING::from(link.as_os_str()), true)
            .map_err(|e| e.to_string())?;
    }
    Ok(true)
}

/// `--dir`/`--name` first (so a portable copy keeps its folder), then `--send`,
/// which takes every following argument as a file.
fn send_to_arguments() -> String {
    let a = crate::relaunch::quoted(crate::relaunch::args());
    if a.is_empty() {
        "--send".into()
    } else {
        format!("{a} --send")
    }
}

/// Last thing that happened, shown at the bottom of the flyout.
static LAST_ACTIVITY: Mutex<String> = Mutex::new(String::new());

fn note_activity(s: &str) {
    if let Ok(mut l) = LAST_ACTIVITY.lock() {
        *l = s.to_owned();
    }
}

/// The session flags Home and the flyout header read: the screen window is
/// open and live, the camera runs and streams, control is active. A camera
/// request still waiting for the phone is not a session (the Camera tile
/// cancels it); the screen and camera stay open while they wait to
/// reconnect, and only a live picture is contact.
fn session_flags() -> (bool, bool, bool, bool, bool) {
    (
        crate::screen_window::is_open(),
        crate::screen_window::is_streaming(),
        crate::camera::is_running(),
        crate::camera::is_streaming(),
        crate::control::is_active(),
    )
}

/// The paired phone as Home's builder takes it.
fn phone_in((id, name): (pb_identity::PeerId, String)) -> crate::pages::home::PhoneIn {
    let c = crate::core::contact(&id);
    crate::pages::home::PhoneIn {
        name,
        fingerprint: id.fingerprint()[..9].to_owned(),
        changed: crate::core::is_changed(&id),
        open: c.open > 0,
        last_seen: c.last_seen,
        path: c.path,
    }
}

/// The flyout's peer name and header card, from one look at the peers so
/// the tiles and the header never disagree; built like Home's hero card.
fn flyout_phone(st: &State) -> (Option<String>, Option<crate::pages::home::PhoneCard>) {
    use crate::pages::home;
    let first = st.peers.lock().ok().and_then(|g| g.first().cloned());
    let (screen, screen_live, camera, camera_live, control) = session_flags();
    let audio = [
        crate::audio::phase(crate::audio_state::Dir::PhoneToPc),
        crate::audio::phase(crate::audio_state::Dir::PcToPhone),
    ];
    let live = home::sessions_live(screen, screen_live, camera, camera_live, control, audio);
    let name = first.as_ref().map(|(_, n)| n.clone());
    (name, first.map(|p| home::phone_card(phone_in(p), live)))
}

/// Unix seconds in whole minutes: "5 min ago" changes no faster, and an
/// unchanged model is not repainted.
fn minute_now() -> u64 {
    crate::timeline::now_unix() / 60 * 60
}

/// Everything the flyout shows, from one look at the peers, sessions,
/// clipboard setting and last activity.
fn flyout_model(st: &State) -> crate::flyout::Model {
    let (peer, phone) = flyout_phone(st);
    crate::flyout::Model {
        peer,
        phone,
        now: minute_now(),
        clipboard_on: st
            .settings
            .lock()
            .map(|s| s.universal_clipboard)
            .unwrap_or(false),
        screen_on: crate::screen_window::is_open(),
        camera_on: crate::camera::is_running(),
        camera_fps: crate::camera::fps(),
        camera_pending: crate::camera::pending(),
        screen_pending: crate::core::screen_pending(),
        audio: [
            crate::audio::phase(crate::audio_state::Dir::PhoneToPc),
            crate::audio::phase(crate::audio_state::Dir::PcToPhone),
        ],
        last: LAST_ACTIVITY.lock().map(|l| l.clone()).unwrap_or_default(),
        paused: st.settings.lock().map(|s| s.paused).unwrap_or(false),
        stopped: core_error().is_some(),
    }
}

/// Keeps an open flyout (header, tiles, drop zone, footer) in step with
/// the tray: a phone paired or removed, a session started or ended.
fn refresh_flyout(st: &State) {
    if crate::flyout::is_open() {
        crate::flyout::update(flyout_model(st));
    }
}

fn open_flyout(hwnd: HWND, st: &State) {
    let model = flyout_model(st);
    let raw = hwnd.0 as isize;
    let cmds = st.cmds.clone();
    // Runs on the UI thread from the flyout's window procedure: only post
    // messages here, never call back into the flyout.
    crate::flyout::toggle(model, Box::new(move |a| flyout_action(raw, &cmds, a)));
    // Opened from the tray icon or a command alike: refresh it every second.
    if crate::flyout::is_open() {
        arm_timeline(hwnd);
    }
}

/// Runs a flyout command (from the flyout or Home's quick actions). Only
/// posts: the caller may be inside another window's procedure.
fn flyout_action(raw: isize, cmds: &UnboundedSender<Cmd>, a: crate::flyout::Action) {
    match a {
        crate::flyout::Action::ToggleClipboard => post_command(raw, ID_TOGGLE_CLIP),
        crate::flyout::Action::SendFiles => post_command(raw, ID_SEND_FILES),
        crate::flyout::Action::Pair => post_command(raw, ID_PAIR),
        crate::flyout::Action::Settings => post_command(raw, ID_SETTINGS),
        crate::flyout::Action::ControlPhone => post_command(raw, ID_CONTROL),
        crate::flyout::Action::ShowScreen => post_command(raw, ID_SCREEN),
        crate::flyout::Action::ToggleCamera => post_command(raw, ID_CAMERA),
        crate::flyout::Action::TogglePhoneAudio => {
            post_command(raw, crate::menu_model::ID_AUDIO_PHONE)
        }
        crate::flyout::Action::CameraWindow => post_command(raw, ID_CAMERA_WINDOW),
        // Opens the main window on Home.
        crate::flyout::Action::OpenRelay => post_command(raw, ID_OPEN_MAIN),
        crate::flyout::Action::Dropped(files) => {
            let _ = cmds.send(Cmd::SendFiles(files));
        }
    }
}

/// A main window page action other than switching pages. Stop takes the
/// same path as the tray menu's stop items.
fn page_action(a: crate::pages::Action) {
    use crate::pages::{home::SessionKind, Action};
    let raw = TRAY_HWND.load(std::sync::atomic::Ordering::Relaxed);
    let Some(st) = STATE.get().filter(|_| raw != 0) else {
        return;
    };
    match a {
        Action::Go(_)
        | Action::TimelineDay(_)
        | Action::TimelineAll(_)
        | Action::TimelineMenu(_) => {}
        Action::CopyText(t) => {
            let _ = set_clipboard_own(HWND(raw as *mut _), &t, false);
        }
        // A Timeline delete whose undo window passed: gone for good.
        Action::TimelineDelete(keys) => {
            crate::timeline::with(|t| {
                t.remove_where(|e| keys.contains(&crate::pages::timeline::entry_key(e)));
            });
        }
        Action::Flyout(f) => flyout_action(raw, &st.cmds, f),
        // The same path as the hotkey and IPC: private clips are refused.
        Action::SendClipboard => post_command(raw, ID_SEND_CLIPBOARD),
        Action::Restart => restart_app(raw),
        // The page only offers Resume while paused, so the menu's toggle resumes.
        Action::Resume => post_command(raw, crate::menu_model::ID_PAUSE),
        Action::Stop(SessionKind::Screen) => post_command(raw, ID_SCREEN_HIDE),
        Action::Stop(SessionKind::Camera) => post_command(raw, ID_CAMERA_STOP),
        Action::Stop(SessionKind::Control) => post_command(raw, ID_CONTROL_STOP),
        Action::Stop(SessionKind::PhoneAudio)
        | Action::Audio(crate::audio_state::Dir::PhoneToPc) => {
            post_command(raw, crate::menu_model::ID_AUDIO_PHONE)
        }
        Action::Stop(SessionKind::PcAudio) | Action::Audio(crate::audio_state::Dir::PcToPhone) => {
            post_command(raw, crate::menu_model::ID_AUDIO_PC)
        }
        // The tray's own Remove: it asks first, then unpairs.
        Action::Unpair(i) => post_command(raw, ID_UNPAIR_BASE + i),
        // On a worker: the check and the shell may wait on a slow share.
        Action::OpenFile(path) => page_file(raw, path, true),
        Action::ShowInFolder(path) => page_file(raw, path, false),
        // Queued for the tray window, as the settings window's were.
        Action::Settings(a) => {
            if let Ok(mut q) = SETTINGS_ACTIONS.lock() {
                q.push_back(a);
            }
            post_settings_action(raw);
        }
        // The window switches its own Settings page.
        Action::SettingsPage(_) => {}
    }
}

/// Wakes the tray window to apply queued Settings actions (or just to
/// refresh the main window when the queue is empty).
fn post_settings_action(raw: isize) {
    // SAFETY: posting to our own message window.
    let _ = unsafe {
        PostMessageW(
            Some(HWND(raw as *mut _)),
            WM_SETTINGS_ACTION,
            WPARAM(0),
            LPARAM(0),
        )
    };
}

/// Open or Show in folder on a Timeline file, on a worker thread so a slow
/// or offline share never stalls the window. The file is checked again
/// first (it may have gone since the row was drawn); a toast says when it
/// is gone or the shell could not act.
fn page_file(raw: isize, path: String, open: bool) {
    use crate::pages::timeline::{file_state, files, FileState, MISSING};
    std::thread::spawn(move || {
        let st = file_state(&path);
        // The row shows it now, and the next layout skips a re-check.
        files::record(&path, st);
        let toast = if st == FileState::Missing {
            Some(MISSING.to_owned())
        } else {
            // SAFETY: COM for the shell on this fresh thread; paired below.
            let com = unsafe {
                windows::Win32::System::Com::CoInitializeEx(
                    None,
                    windows::Win32::System::Com::COINIT_APARTMENTTHREADED
                        | windows::Win32::System::Com::COINIT_DISABLE_OLE1DDE,
                )
            };
            let ok = if open {
                shell_open(&path, None)
            } else {
                show_in_folder(&path)
            };
            if com.is_ok() {
                // SAFETY: pairs the successful CoInitializeEx above.
                unsafe { windows::Win32::System::Com::CoUninitialize() };
            }
            (!ok).then(|| failed_text(&path, open))
        };
        if let Some(t) = toast {
            if let Ok(mut q) = PAGE_FILES.lock() {
                q.push_back(t);
            }
            post_page_file(raw);
        }
    });
}

/// Wakes the tray window to show queued toasts and repaint the main window.
fn post_page_file(raw: isize) {
    // SAFETY: posting to our own message window.
    let _ = unsafe {
        PostMessageW(
            Some(HWND(raw as *mut _)),
            WM_PAGE_FILE,
            WPARAM(0),
            LPARAM(0),
        )
    };
}

/// The toast when the shell could not open or show the file.
fn failed_text(path: &str, open: bool) -> String {
    let name = std::path::Path::new(path)
        .file_name()
        .map_or_else(|| path.to_owned(), |n| n.to_string_lossy().into_owned());
    if open {
        format!("Couldn't open {name}")
    } else {
        format!("Couldn't show {name} in its folder")
    }
}

/// `ShellExecuteW` "open" on `file` (with `params`); false when the shell
/// reports an error (a result of 32 or less).
pub(crate) fn shell_open(file: &str, params: Option<&str>) -> bool {
    use windows::Win32::UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL};
    let file = wide(file);
    let params = params.map(wide);
    let p = params
        .as_ref()
        .map_or(PCWSTR::null(), |p| PCWSTR(p.as_ptr()));
    // SAFETY: valid NUL-terminated strings for the duration of the call.
    let h = unsafe {
        ShellExecuteW(
            None,
            w!("open"),
            PCWSTR(file.as_ptr()),
            p,
            PCWSTR::null(),
            SW_SHOWNORMAL,
        )
    };
    h.0 as isize > 32
}

/// Explorer with the file selected. The one way Relay shows a file in its
/// folder (Timeline, toasts, balloons).
pub(crate) fn show_in_folder(path: &str) -> bool {
    select_arg(path).is_some_and(|p| shell_open("explorer.exe", Some(&p)))
}

/// Explorer's `/select,"<path>"`. Windows paths cannot hold a quote, so one
/// that does is refused rather than allowed to end the quoting early.
fn select_arg(path: &str) -> Option<String> {
    (!path.contains('"')).then(|| format!("/select,\"{path}\""))
}

/// This PC's offset from UTC now (DST included), in seconds.
fn local_offset_s() -> i64 {
    use windows::Win32::{Foundation::FILETIME, Storage::FileSystem::FileTimeToLocalFileTime};
    // 100 ns ticks since 1601.
    let utc = (crate::timeline::now_unix() + 11_644_473_600) * 10_000_000;
    let ft = FILETIME {
        dwLowDateTime: utc as u32,
        dwHighDateTime: (utc >> 32) as u32,
    };
    let mut local = FILETIME::default();
    // SAFETY: both point to live FILETIMEs for the duration of the call.
    if unsafe { FileTimeToLocalFileTime(&ft, &mut local) }.is_err() {
        return 0;
    }
    let local = (u64::from(local.dwHighDateTime) << 32) | u64::from(local.dwLowDateTime);
    (local as i64 - utc as i64) / 10_000_000
}

/// The paired devices as (name, short fingerprint, reinstalled or reset),
/// in peer order (Remove names one by its index).
fn paired_devices(st: &State) -> Vec<(String, String, bool)> {
    st.peers
        .lock()
        .map(|g| {
            g.iter()
                .map(|(id, n)| {
                    (
                        n.clone(),
                        id.fingerprint()[..9].to_owned(),
                        crate::core::is_changed(id),
                    )
                })
                .collect()
        })
        .unwrap_or_default()
}

/// What the Timeline model was built from: the store's version, history
/// on, the local day and the time zone.
type TimelineKey = (u64, bool, i64, i64);
/// The last Timeline model and Home's recent entries, with their key.
type TimelineCache = Option<(
    TimelineKey,
    Arc<crate::pages::timeline::TimelineModel>,
    Vec<crate::timeline::Entry>,
)>;
static TIMELINE_MODEL: Mutex<TimelineCache> = Mutex::new(None);

/// The Timeline model and Home's last few entries. The entries are copied
/// and the model rebuilt only when the store, the day or the time zone
/// changed, so the main window's once-a-second model check stays cheap.
fn timeline_model(
    history_on: bool,
    now: u64,
) -> (
    Arc<crate::pages::timeline::TimelineModel>,
    Vec<crate::timeline::Entry>,
) {
    let tz = local_offset_s();
    let today = (now as i64 + tz).div_euclid(86_400);
    let mut cache = TIMELINE_MODEL.lock().unwrap_or_else(|p| p.into_inner());
    let cached = cache.as_ref().map(|(k, _, _)| *k);
    let mut fresh = None;
    crate::timeline::with(|t| {
        let key = (t.version(), history_on, today, tz);
        if cached != Some(key) {
            fresh = Some((key, t.entries().to_vec()));
        }
    });
    if let Some((key, entries)) = fresh {
        let m = Arc::new(crate::pages::timeline::build(&entries, history_on, now, tz));
        let recent = entries
            .into_iter()
            .take(crate::pages::home::RECENT)
            .collect();
        *cache = Some((key, m, recent));
    }
    cache
        .as_ref()
        .map(|(_, m, r)| (m.clone(), r.clone()))
        .unwrap_or_default()
}

/// Everything the main window's pages show. Each lock is taken and
/// released on its own.
fn page_model(st: &State) -> crate::pages::Model {
    use crate::pages::home;
    let phone = st.peers.lock().ok().and_then(|g| g.first().cloned());
    let settings = st.settings.lock().map(|s| s.clone()).unwrap_or_default();
    let (screen, screen_live, camera, camera_live, control) = session_flags();
    let (notes, locked) = {
        let ns = notify_ui();
        let notes: Vec<pb_protocol::control::Notification> = ns
            .mirror
            .recent(usize::MAX)
            .into_iter()
            .filter_map(|e| ns.mirror.note(e).cloned())
            .collect();
        (notes, ns.locked)
    };
    let now = crate::timeline::now_unix();
    let (timeline, recent) = timeline_model(settings.keep_history, now);
    let phone = phone.map(phone_in);
    let mut home_model = home::build(home::Inputs {
        phone,
        screen,
        screen_live,
        camera,
        camera_live,
        control,
        camera_pending: crate::camera::pending(),
        screen_pending: crate::core::screen_pending(),
        audio: [
            crate::audio::phase(crate::audio_state::Dir::PhoneToPc),
            crate::audio::phase(crate::audio_state::Dir::PcToPhone),
        ],
        notes: &notes,
        settings: &settings,
        locked,
        recent: &recent,
        // Whole minutes: the labels change no faster, and an unchanged
        // model is not repainted.
        now: now / 60 * 60,
    });
    home_model.error = core_error();
    crate::pages::Model {
        home: home_model,
        timeline,
        devices: crate::pages::devices::DevicesModel {
            devices: paired_devices(st),
        },
        // The window adds its own sub-page and query. The rows (a registry
        // read and two file checks) are built only while Settings shows.
        settings: crate::pages::settings::SettingsModel {
            rows: if crate::main_window::shows_settings() {
                settings_model(st)
            } else {
                Default::default()
            },
            ..Default::default()
        },
    }
}

static SAVE_FAILING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// True when a save result should tell the user: the first failure of a streak
/// (a locked or full disk fails every toggle, one message is enough), and not again
/// until a save succeeds.
fn tell_save_failure(ok: bool, failing: &std::sync::atomic::AtomicBool) -> bool {
    use std::sync::atomic::Ordering::SeqCst;
    if ok {
        failing.store(false, SeqCst);
        false
    } else {
        !failing.swap(true, SeqCst)
    }
}

fn save_settings(st: &State) {
    let Ok(s) = st.settings.lock() else { return };
    let res = s.save(&st.settings_path);
    drop(s);
    if let Err(e) = &res {
        st.log.line(&format!("SETTINGS save failed: {e}"));
    }
    if tell_save_failure(res.is_ok(), &SAVE_FAILING) {
        pill(
            "Couldn't save settings. They apply now but will be lost when Relay restarts.",
            8,
        );
    }
}

fn reveal(path: &std::path::Path) {
    let _ = show_in_folder(&path.to_string_lossy());
}

/// Multi-select file picker; runs on its own thread (COM STA).
fn pick_files() -> Vec<std::path::PathBuf> {
    use windows::Win32::{
        System::Com::{
            CoCreateInstance, CoInitializeEx, CoTaskMemFree, CLSCTX_INPROC_SERVER,
            COINIT_APARTMENTTHREADED,
        },
        UI::Shell::{
            FileOpenDialog, IFileOpenDialog, FOS_ALLOWMULTISELECT, FOS_FORCEFILESYSTEM,
            SIGDN_FILESYSPATH,
        },
    };
    let mut out = Vec::new();
    // SAFETY: standard COM usage on a fresh STA thread; every returned string
    // is freed with CoTaskMemFree after copying.
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let Ok(dlg) =
            CoCreateInstance::<_, IFileOpenDialog>(&FileOpenDialog, None, CLSCTX_INPROC_SERVER)
        else {
            return out;
        };
        if let Ok(opts) = dlg.GetOptions() {
            let _ = dlg.SetOptions(opts | FOS_ALLOWMULTISELECT | FOS_FORCEFILESYSTEM);
        }
        let _ = dlg.SetTitle(w!("Send files with Relay"));
        if dlg.Show(None).is_err() {
            return out; // cancelled
        }
        let Ok(items) = dlg.GetResults() else {
            return out;
        };
        let n = items.GetCount().unwrap_or(0);
        for i in 0..n {
            if let Ok(item) = items.GetItemAt(i) {
                if let Ok(name) = item.GetDisplayName(SIGDN_FILESYSPATH) {
                    if let Ok(s) = name.to_string() {
                        out.push(std::path::PathBuf::from(s));
                    }
                    CoTaskMemFree(Some(name.0 as *const _));
                }
            }
        }
    }
    out
}

static STATE: OnceLock<State> = OnceLock::new();
static EVENTS: Mutex<VecDeque<UiEvent>> = Mutex::new(VecDeque::new());

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn copy_into(dst: &mut [u16], s: &str) {
    let src: Vec<u16> = s.encode_utf16().take(dst.len() - 1).collect();
    dst[..src.len()].copy_from_slice(&src);
    dst[src.len()] = 0;
}

/// Where the tray icon is on screen, when the shell says (it may sit in
/// the overflow, or on another monitor than the cursor).
pub fn tray_icon_rect(hwnd: HWND) -> Option<windows::Win32::Foundation::RECT> {
    use windows::Win32::UI::Shell::{Shell_NotifyIconGetRect, NOTIFYICONIDENTIFIER};
    let id = NOTIFYICONIDENTIFIER {
        cbSize: std::mem::size_of::<NOTIFYICONIDENTIFIER>() as u32,
        hWnd: hwnd,
        uID: 1,
        ..Default::default()
    };
    // SAFETY: a query into a valid struct.
    unsafe { Shell_NotifyIconGetRect(&id).ok() }
}

fn icon_data(hwnd: HWND) -> NOTIFYICONDATAW {
    NOTIFYICONDATAW {
        cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: 1,
        ..Default::default()
    }
}

/// A message from Phone Bridge itself: a toast (it stays in the notification
/// centre), or a tray balloon when toasts are not available.
fn balloon(hwnd: HWND, title: &str, text: &str) {
    use crate::toast_xml::{args, ToastArgs, ToastSpec};
    let tag = {
        let mut n = notify_ui();
        n.toasts_ok.then(|| {
            // A few rotating tags: newer messages replace older ones, so
            // Phone Bridge's own toasts never crowd out phone notifications
            // (Windows keeps 20 per app).
            n.own_counter += 1;
            let t = format!("pb-{}", n.own_counter % OWN_TOASTS);
            n.last_own_tag = t.clone();
            t
        })
    };
    if let Some(tag) = tag {
        let spec = ToastSpec {
            group: "pb".into(),
            title: title.into(),
            lines: vec![text.into()],
            when_ms: now_ms(),
            launch: args(&ToastArgs::Open { tag: tag.clone() }),
            tag,
            popup: true,
            ..Default::default()
        };
        if crate::toast::show(&spec).is_ok() {
            return;
        }
    }
    shell_balloon(hwnd, title, text);
}

fn shell_balloon(hwnd: HWND, title: &str, text: &str) {
    let mut nid = icon_data(hwnd);
    nid.uFlags = NIF_INFO;
    nid.dwInfoFlags = NIIF_INFO;
    copy_into(&mut nid.szInfoTitle, title);
    copy_into(&mut nid.szInfo, text);
    // SAFETY: `nid` is fully initialised and refers to our icon.
    let _ = unsafe { Shell_NotifyIconW(NIM_MODIFY, &nid) };
}

/// What the tray tooltip says now, from the real core, pause and audio state.
fn current_tip(st: &State) -> String {
    let fp = MY_FINGERPRINT.lock().map(|f| f.clone()).unwrap_or_default();
    let paused = st.settings.lock().map(|s| s.paused).unwrap_or(false);
    let audio = crate::audio_state::Dir::ALL
        .iter()
        .any(|d| crate::audio::phase(*d) == crate::audio_state::Phase::Running);
    crate::pause::current_tooltip(core_error().as_deref(), paused, &fp, audio)
}

fn set_tip(hwnd: HWND, tip: &str) {
    let mut nid = icon_data(hwnd);
    nid.uFlags = NIF_TIP;
    copy_into(&mut nid.szTip, tip);
    // SAFETY: as in `balloon`.
    let _ = unsafe { Shell_NotifyIconW(NIM_MODIFY, &nid) };
}

/// Questions and notices are Fluent dialogs on their own thread, so the tray
/// keeps working while one is open.
fn ask_yes_no(d: Dialog, done: impl FnOnce(bool) + Send + 'static) {
    crate::dialog_window::ask(d, done);
}

/// Pairing comparison prompts on screen, by pairing id: the flag is the
/// dialog's "still alive" test.
static SAS_OPEN: std::sync::Mutex<Vec<(u64, std::sync::Arc<std::sync::atomic::AtomicBool>)>> =
    std::sync::Mutex::new(Vec::new());

fn sas_open(id: u64) -> std::sync::Arc<std::sync::atomic::AtomicBool> {
    let flag = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
    if let Ok(mut v) = SAS_OPEN.lock() {
        v.push((id, flag.clone()));
    }
    flag
}

/// The prompt was answered or closed by the user.
fn sas_closed(id: u64) {
    if let Ok(mut v) = SAS_OPEN.lock() {
        v.retain(|(i, _)| *i != id);
    }
}

/// Closes the prompt for `id`; false when it was already gone.
fn sas_withdraw(id: u64) -> bool {
    let Ok(mut v) = SAS_OPEN.lock() else {
        return false;
    };
    let Some(at) = v.iter().position(|(i, _)| *i == id) else {
        return false;
    };
    v.remove(at)
        .1
        .store(false, std::sync::atomic::Ordering::SeqCst);
    true
}

fn show_info_pill(text: &str) {
    crate::status::show(text, 8);
}

fn show_info(d: Dialog) {
    crate::dialog_window::info(d);
}

/// Run a menu command on the UI thread (used by the pipe server).
pub fn post_command(hwnd_raw: isize, id: usize) {
    // SAFETY: posting to our own window.
    let _ = unsafe {
        PostMessageW(
            Some(HWND(hwnd_raw as *mut _)),
            WM_COMMAND,
            WPARAM(id),
            LPARAM(0),
        )
    };
}

/// Queue an event from the core and wake the UI thread.
/// Shows a status pill from any thread: the pill window must be made on the UI thread, which is
/// the only one with a message loop, so the request is queued to it.
pub fn pill(text: &str, secs: u32) {
    post_event(
        TRAY_HWND.load(std::sync::atomic::Ordering::Relaxed),
        UiEvent::Pill(text.to_owned(), secs),
    );
}

pub fn post_event(hwnd_raw: isize, e: UiEvent) {
    if let Ok(mut q) = EVENTS.lock() {
        q.push_back(e);
    }
    // SAFETY: posting to our own window; harmless if it is already gone.
    let _ = unsafe {
        PostMessageW(
            Some(HWND(hwnd_raw as *mut _)),
            WM_CORE_EVENT,
            WPARAM(0),
            LPARAM(0),
        )
    };
}

fn handle_event(hwnd: HWND, st: &State, e: UiEvent) {
    // A balloon click acts on the latest file or link only if that balloon
    // is the one showing; any other balloon clears the targets.
    if !matches!(
        e,
        UiEvent::FileReceived { .. }
            | UiEvent::Link { .. }
            | UiEvent::Notify { .. }
            | UiEvent::NotifyRemoved { .. }
            | UiEvent::NotifySync { .. }
    ) {
        if let Ok(mut l) = LAST_LINK.lock() {
            *l = None;
        }
        if let Ok(mut f) = LAST_FILE.lock() {
            *f = None;
        }
    }
    match e {
        UiEvent::Ready { fingerprint } => {
            if let Ok(mut f) = MY_FINGERPRINT.lock() {
                *f = fingerprint[..9].to_owned();
            }
            set_tip(hwnd, &current_tip(st));
            // Toasts restored from the notification centre whose phone is no
            // longer paired go now.
            let paired: Vec<pb_identity::PeerId> = st
                .peers
                .lock()
                .map(|g| g.iter().map(|(id, _)| *id).collect())
                .unwrap_or_default();
            let gone = {
                let mut ns = notify_ui();
                let mut tags = Vec::new();
                for p in ns.mirror.peers() {
                    if !paired.contains(&p) {
                        tags.extend(ns.mirror.remove_peer(&p));
                    }
                }
                forget(&mut ns, tags)
            };
            remove_toasts(gone);
        }
        UiEvent::Code {
            code,
            refresh_in,
            closes_in,
        } => {
            if let Err(e) = crate::qr_window::show(&code, refresh_in, closes_in) {
                show_info(Dialog::notice(
                    "Pair a new device".into(),
                    format!(
                        "Pairing code (valid 30 seconds):

{code}

({e})"
                    ),
                ));
            }
        }
        UiEvent::PairingClosed => {
            crate::qr_window::close();
            balloon(hwnd, "Relay", "Pairing window expired");
        }
        UiEvent::Sas { id, peer_name, sas } => {
            crate::qr_window::close();
            let cmds = st.cmds.clone();
            if st.auto_accept {
                st.log.line("SAS auto-accepted (test mode)");
                let _ = cmds.send(Cmd::Decide { id, accept: true });
                return;
            }
            let open = sas_open(id);
            crate::dialog_window::ask_while(
                Dialog::pairing(&peer_name, &sas),
                {
                    let open = open.clone();
                    move || open.load(std::sync::atomic::Ordering::SeqCst)
                },
                move |yes| {
                    sas_closed(id);
                    let _ = cmds.send(Cmd::Decide { id, accept: yes });
                },
            );
        }
        UiEvent::SasWithdrawn { id } => {
            if sas_withdraw(id) {
                show_info_pill("The phone cancelled pairing");
            }
        }
        UiEvent::Clipboard {
            from,
            text,
            sensitive,
        } => match set_clipboard_own(hwnd, &text, sensitive) {
            Ok(()) if sensitive => balloon(
                hwnd,
                &format!("Clipboard from {from}"),
                "Sensitive text copied (hidden, not kept in history)",
            ),
            Ok(()) => balloon(
                hwnd,
                &format!("Clipboard from {from}"),
                &text.chars().take(120).collect::<String>(),
            ),
            Err(e) => balloon(hwnd, "Relay", &format!("Could not set clipboard: {e}")),
        },
        UiEvent::SharedText { from, text } => {
            let (auto, keep) = st
                .settings
                .lock()
                .map(|s| (s.auto_copy_shared_text, s.keep_in_history))
                .unwrap_or((false, false));
            let plan = crate::shared_text::plan(auto, keep);
            if !plan.copy_now {
                show_shared_text_toast(hwnd, &from, text);
            } else if let Err(e) = set_clipboard_own(hwnd, &text, plan.exclude_from_history) {
                balloon(hwnd, "Relay", &format!("Could not set clipboard: {e}"));
            } else {
                balloon(
                    hwnd,
                    &crate::shared_text::title(&from),
                    crate::shared_text::body(true),
                );
            }
        }
        UiEvent::RemoteClipboard {
            from,
            text,
            sensitive,
        } => {
            let keep = st
                .settings
                .lock()
                .map(|s| s.keep_in_history)
                .unwrap_or(false);
            // Sensitive text never enters Win+V history, whatever the setting.
            match set_clipboard_own(hwnd, &text, sensitive || !keep) {
                Ok(()) if sensitive => balloon(
                    hwnd,
                    &format!("Copied on {from}"),
                    "Password or code, ready to paste",
                ),
                Ok(()) => balloon(hwnd, &format!("Copied on {from}"), "Ready to paste"),
                Err(e) => balloon(hwnd, "Relay", &format!("Could not set clipboard: {e}")),
            }
        }
        UiEvent::RemoteImage { from, png } => {
            let keep = st
                .settings
                .lock()
                .map(|s| s.keep_in_history)
                .unwrap_or(false);
            match set_clipboard_image_own(hwnd, &png, !keep) {
                Ok(()) => balloon(hwnd, &format!("Copied on {from}"), "Picture ready to paste"),
                Err(e) => balloon(hwnd, "Relay", &format!("Could not copy the picture: {e}")),
            }
        }
        UiEvent::ControlReady { name, sender } => {
            if crate::control::start(hwnd, sender) {
                // SAFETY: timer on our own window; checks the session is alive.
                unsafe {
                    windows::Win32::UI::WindowsAndMessaging::SetTimer(
                        Some(hwnd),
                        TIMER_CONTROL,
                        1000,
                        None,
                    )
                };
                crate::status::show(
                    &format!("You're now controlling {name}. Ctrl+Alt+P returns to this PC"),
                    6,
                );
                balloon(
                    hwnd,
                    &format!("Controlling {name}"),
                    "Your mouse and keyboard now drive the phone. Press Ctrl+Alt+P to return.",
                );
            } else {
                crate::status::show("Could not take over this PC's mouse and keyboard", 6);
                // The session ends (sender dropped) and the phone hides its cursor.
                balloon(
                    hwnd,
                    "Relay",
                    "Could not take over this PC's mouse and keyboard. Try again.",
                );
            }
        }
        UiEvent::Link { from, url } => {
            if let Ok(mut l) = LAST_LINK.lock() {
                *l = Some(url.clone());
            }
            note_activity(&format!("Last: link from {from}"));
            let shown: String = url.chars().take(120).collect();
            balloon(
                hwnd,
                &format!("Link from {from}"),
                &format!("{shown}\nClick to open."),
            );
        }
        UiEvent::FileReceived { from, path, size } => {
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            if let Ok(mut l) = LAST_FILE.lock() {
                *l = Some(path.clone());
            }
            if let Ok(mut l) = LAST_LINK.lock() {
                *l = None;
            }
            note_activity(&format!("Last: {name} received"));
            balloon(
                hwnd,
                &format!("{from} sent {name}"),
                &format!(
                    "{}, saved to {}. Click to show.",
                    crate::transfers::fmt_bytes(size),
                    path.parent()
                        .map(|d| d.display().to_string())
                        .unwrap_or_default()
                ),
            );
        }
        UiEvent::AskControl { from, reply } => {
            ask_yes_no(
                Dialog::question(
                    format!("Allow {from} to control this PC?"),
                    "It will be able to use this PC's mouse and keyboard. You can turn this off any time in Relay settings.".into(),
                    "Allow",
                    "Don't allow",
                ),
                move |yes| {
                    // An answer after the request timed out changes nothing.
                    let in_time = reply.send(yes).is_ok();
                    if yes && in_time {
                        // Remember the choice; Settings can turn it off again.
                        if let Some(st) = STATE.get() {
                            let _ = st.settings.lock().map(|mut s| s.allow_control = true);
                            save_settings(st);
                        }
                    }
                },
            );
        }
        UiEvent::IncomingOffer {
            from,
            name,
            size,
            folder,
            reply,
        } => {
            let alive = reply.clone();
            crate::dialog_window::ask_while(
                Dialog::question(
                    format!("{from} wants to send a file"),
                    format!(
                        "{name} ({})\n\nSave it to {}?",
                        crate::transfers::fmt_bytes(size),
                        folder.display()
                    ),
                    "Save",
                    "Decline",
                ),
                move || alive.alive(),
                move |yes| {
                    // The offer may have timed out while the dialog was open.
                    if !reply.answer(yes) && yes {
                        crate::ui::pill("That request expired. Send it again from the phone.", 8);
                    }
                },
            );
        }
        // Nothing to do here: the settings window is rebuilt after every event batch.
        UiEvent::PeersChecked => {}
        UiEvent::Pill(text, secs) => crate::status::show(&text, secs),
        // The menu, Home and the flyout read the state when they draw.
        UiEvent::AudioChanged => {
            set_tip(hwnd, &current_tip(st));
            refresh_flyout(st);
        }
        UiEvent::AudioAsking(text) => crate::status::show(&text, 40),
        UiEvent::AudioAskEnded => crate::status::close_if_asking(),
        UiEvent::AskAudioShare { from, reply } => {
            let alive = reply.clone();
            let (title, body, yes, no) = crate::audio_state::confirm_text(&from);
            crate::dialog_window::ask_while(
                Dialog::question(title, body, yes, no),
                move || alive.alive(),
                move |ok| {
                    let _ = reply.answer(ok);
                },
            );
        }
        UiEvent::Info(msg) => {
            if msg.starts_with("Sent ") {
                note_activity(&format!("Last: {}", msg));
            }
            // Answers to the user's own actions: shown in the pill (Do Not
            // Disturb hides balloons), and as a balloon for the history.
            let secs = if msg.starts_with("Asking ") || msg.starts_with("Connecting ") {
                35
            } else {
                6
            };
            crate::status::show(&msg, secs);
            balloon(hwnd, "Relay", &msg)
        }
        UiEvent::Notify {
            peer,
            from,
            n,
            logo,
            picture,
        } => show_phone_notification(hwnd, st, peer, &from, *n, logo, picture, false),
        UiEvent::NotifyRemoved { peer, key } => {
            let gone = {
                let mut ns = notify_ui();
                let tags: Vec<String> = ns.mirror.remove_key(&peer, &key).into_iter().collect();
                forget(&mut ns, tags)
            };
            remove_toasts(gone);
        }
        UiEvent::NotifySync { peer, keys } => {
            let gone = {
                let mut ns = notify_ui();
                let tags = if keys.is_empty() {
                    ns.mirror.remove_peer(&peer)
                } else {
                    ns.mirror.retain_live(&peer, &keys)
                };
                forget(&mut ns, tags)
            };
            remove_toasts(gone);
        }
        UiEvent::ScreenShown => crate::status::close_if_asking(),
        UiEvent::ScreenPinned(p) => {
            let _ = st.settings.lock().map(|mut s| s.screen_pinned = p);
            save_settings(st);
        }
        UiEvent::ScreenDim(d) => {
            let _ = st.settings.lock().map(|mut s| s.screen_dim = d);
            save_settings(st);
        }
        UiEvent::ScreenFiles(files) => {
            if !files.is_empty() {
                let _ = st.cmds.send(Cmd::SendFiles(files));
            }
        }
        UiEvent::NotifyActionFailed { reason, reply } => {
            crate::status::show(&reason, 6);
            let text = match reply {
                Some(r) => format!("{reason}\nYour reply: {r}"),
                None => reason,
            };
            balloon(hwnd, "Relay", &text);
        }
    }
}

/// Forget these toasts; returns (tag, group, picture) to remove with
/// [`remove_toasts`] once the lock is released (WinRT calls must not run
/// under the lock: COM may dispatch our timers while waiting).
fn forget(
    ns: &mut NotifyUi,
    tags: Vec<String>,
) -> Vec<(String, String, Option<std::path::PathBuf>)> {
    tags.into_iter()
        .filter_map(|tag| {
            ns.hidden.remove(&tag);
            ns.meta
                .remove(&tag)
                .map(|(group, _, picture)| (tag, group, picture))
        })
        .collect()
}

fn remove_toasts(list: Vec<(String, String, Option<std::path::PathBuf>)>) {
    for (tag, group, picture) in list {
        crate::toast::remove(&tag, &group);
        if let Some(p) = picture {
            let _ = std::fs::remove_file(p);
        }
    }
}

fn arm_timer(hwnd: HWND, id: usize, ms: u32) {
    // SAFETY: timer on our own window.
    unsafe {
        windows::Win32::UI::WindowsAndMessaging::SetTimer(Some(hwnd), id, ms, None);
    }
}

/// The timeline timer has work: a write is owed, or a window shows times
/// and states that change without a message.
fn timeline_timer_needed(wants_flush: bool, main_open: bool, flyout_open: bool) -> bool {
    wants_flush || main_open || flyout_open
}

/// Arms the timeline timer unless it runs already (UI thread).
fn arm_timeline(hwnd: HWND) {
    use std::sync::atomic::Ordering;
    if !TIMELINE_ARMED.swap(true, Ordering::SeqCst) {
        arm_timer(hwnd, TIMER_TIMELINE, 1000);
    }
}

/// A timeline entry was added (any thread): make sure it gets written.
pub fn wake_timeline() {
    use std::sync::atomic::Ordering;
    let raw = TRAY_HWND.load(Ordering::Relaxed);
    if raw != 0 && !TIMELINE_ARMED.load(Ordering::SeqCst) {
        // SAFETY: posting to our own tray window from any thread.
        let _ = unsafe {
            PostMessageW(
                Some(HWND(raw as *mut _)),
                WM_TIMELINE_WAKE,
                WPARAM(0),
                LPARAM(0),
            )
        };
    }
}

/// Hands unused pages back to Windows (after the device is released).
fn trim_working_set() {
    use windows::Win32::System::Threading::{GetCurrentProcess, SetProcessWorkingSetSize};
    // SAFETY: (-1, -1) on our own process only trims; it sets no limits.
    let _ = unsafe { SetProcessWorkingSetSize(GetCurrentProcess(), usize::MAX, usize::MAX) };
}

fn kill_timer(hwnd: HWND, id: usize) {
    // SAFETY: timer on our own window.
    let _ = unsafe { windows::Win32::UI::WindowsAndMessaging::KillTimer(Some(hwnd), id) };
}

/// Copy a one-time code, kept out of clipboard history; cleared after 60 s
/// if nothing else was copied meanwhile.
/// Texts shared from the phone, waiting for their toast's Copy button.
static SHARED_TEXTS: Mutex<crate::shared_text::Pending> =
    Mutex::new(crate::shared_text::Pending::new());

/// A toast with Copy for text shared from the phone. The text itself stays
/// in memory: toasts are kept in Windows' notification history.
fn show_shared_text_toast(hwnd: HWND, from: &str, text: String) {
    use crate::toast_xml::{args, ToastArgs, ToastSpec};
    let title = crate::shared_text::title(from);
    let tag = SHARED_TEXTS
        .lock()
        .map(|mut p| p.insert(text))
        .unwrap_or_default();
    if !tag.is_empty() && crate::toast::enabled() {
        let copy = args(&ToastArgs::CopyText { tag: tag.clone() });
        let spec = ToastSpec {
            group: "pb".into(),
            title: title.clone(),
            lines: vec![crate::shared_text::body(false).into()],
            when_ms: now_ms(),
            launch: copy.clone(),
            buttons: vec![("Copy".into(), copy)],
            tag,
            popup: true,
            ..Default::default()
        };
        if crate::toast::show(&spec).is_ok() {
            return;
        }
    }
    // A balloon has no button: without toasts the text can't be offered.
    balloon(
        hwnd,
        &title,
        "Turn on notifications for Relay in Windows to copy shared text",
    );
}

/// The toast's Copy: the text once, kept out of history unless allowed.
fn copy_shared_text(hwnd: HWND, st: &State, tag: &str) {
    let Some(text) = SHARED_TEXTS.lock().ok().and_then(|mut p| p.take(tag)) else {
        return;
    };
    let keep = st
        .settings
        .lock()
        .map(|s| s.keep_in_history)
        .unwrap_or(false);
    let plan = crate::shared_text::plan(false, keep);
    if let Err(e) = set_clipboard_own(hwnd, &text, plan.exclude_from_history) {
        balloon(hwnd, "Relay", &format!("Could not set clipboard: {e}"));
    }
}

fn copy_otp(hwnd: HWND, code: &str) {
    if code.is_empty() {
        return;
    }
    if set_clipboard_own(hwnd, code, true).is_ok() {
        notify_ui().otp_pending = Some((clipboard_seq(), code.to_owned()));
        arm_timer(hwnd, TIMER_OTP, 60_000);
    }
}

/// Rebuild the list of phone toasts from the notification centre (after a
/// restart), so their buttons work and the phone's sync can remove them.
fn restore_from_history() -> (usize, usize) {
    use crate::{notify::Entry, toast_xml::parse_ctx};
    let history = crate::toast::history();
    let mut ns = notify_ui();
    let mut restored = 0;
    for (tag, group, launch) in &history {
        let (tag, group) = (tag.clone(), group.clone());
        if tag.starts_with("pb-") {
            continue;
        }
        let Some(ctx) = parse_ctx(launch) else {
            continue;
        };
        let Ok(peer) = pb_identity::PeerId::from_hex(&ctx.peer) else {
            continue;
        };
        ns.mirror.restore(
            tag.clone(),
            Entry {
                peer,
                key: ctx.key,
                package: ctx.package,
                otp: ctx.otp,
            },
        );
        ns.meta.insert(tag, (group, ctx.reply, None));
        restored += 1;
    }
    (restored, history.len())
}

#[allow(clippy::too_many_arguments)]
fn show_phone_notification(
    hwnd: HWND,
    st: &State,
    peer: pb_identity::PeerId,
    from: &str,
    n: pb_protocol::control::Notification,
    logo: Option<std::path::PathBuf>,
    picture: Option<std::path::PathBuf>,
    refresh: bool,
) {
    use crate::{
        notify::{present, Ctx, Entry},
        toast_xml::{args_with, ToastArgs, ToastCtx, ToastSpec},
    };
    let app = if n.app_name.is_empty() {
        n.package.clone()
    } else {
        n.app_name.clone()
    };
    // Remember the app for the per-app switches in Settings.
    let new_app = st
        .settings
        .lock()
        .map(|mut s| {
            let known = s.notify_apps.iter().any(|(p, _, _)| *p == n.package);
            if !known && !n.package.is_empty() {
                s.notify_apps.push((n.package.clone(), app.clone(), false));
            }
            !known
        })
        .unwrap_or(false);
    if new_app {
        save_settings(st);
    }
    let settings = st.settings.lock().map(|s| s.clone()).unwrap_or_default();
    let (locked, toasts_ok) = {
        let ns = notify_ui();
        (ns.locked, ns.toasts_ok)
    };
    let ctx = Ctx {
        locked,
        mirroring_focused: crate::screen_window::is_focused(),
    };
    let Some(p) = present(&n, &settings, &ctx) else {
        return;
    };
    if !toasts_ok {
        // No toasts on this PC: a balloon, and nothing to keep in step.
        if p.popup {
            shell_balloon(hwnd, &p.title, &p.lines.join("\n"));
        }
        return;
    }
    let tag = crate::notify::tag(&peer, &n.key);
    let group = crate::notify::group(&peer, &n);
    let reply_index = n.actions.iter().position(|a| a.reply).map(|i| i as u32);
    let tctx = ToastCtx {
        peer: peer.to_string(),
        key: n.key.clone(),
        package: n.package.clone(),
        otp: n.otp.clone(),
        reply: reply_index,
    };
    let a = |args: ToastArgs| args_with(&args, &tctx);
    let mut buttons = Vec::new();
    if let Some(label) = &p.otp_label {
        buttons.push((label.clone(), a(ToastArgs::CopyOtp { tag: tag.clone() })));
    }
    for (i, act) in n.actions.iter().enumerate() {
        if !act.reply && !act.label.is_empty() {
            buttons.push((
                act.label.clone(),
                a(ToastArgs::Button {
                    tag: tag.clone(),
                    index: i as u32,
                }),
            ));
        }
    }
    if n.call {
        // Answer and Decline are the dialer's own buttons, above.
        buttons.push(("Silence".into(), a(ToastArgs::Silence { tag: tag.clone() })));
    }
    let spec = ToastSpec {
        tag: tag.clone(),
        group: group.clone(),
        attribution: app.clone(),
        title: p.title.clone(),
        lines: p.lines.clone(),
        logo: logo.clone(),
        picture: picture.clone().filter(|_| p.show_picture),
        when_ms: n.when_ms,
        launch: a(ToastArgs::Open { tag: tag.clone() }),
        buttons,
        reply: reply_index.map(|_| {
            (
                format!("Reply to {}", p.title),
                a(ToastArgs::Reply { tag: tag.clone() }),
            )
        }),
        menu: vec![(
            format!("Stop showing from {app}"),
            a(ToastArgs::Mute { tag: tag.clone() }),
        )],
        // A refresh at unlock replaces the toast in place, without a pop-up.
        popup: p.popup && !refresh,
        silent: p.silent || refresh,
        call: n.call,
    };
    // An update, a quiet re-send (the sync at start) or a refresh at unlock
    // of a toast already shown is not a new Timeline event. Checked before
    // the mirror takes this one.
    let is_new = crate::notify::is_new_event(&notify_ui().mirror, &tag, refresh);
    // Windows does not show it at all (turned off for Phone Bridge): do not
    // keep it in step, so its absence is never taken for a clear.
    let shown = crate::toast::enabled() && crate::toast::show(&spec).is_ok();
    if shown {
        // App name only.
        if is_new {
            crate::core::record(
                &st.settings,
                crate::timeline::notification_entry(crate::timeline::notification_peer(from), &app),
            );
        }
        let old_picture = {
            let mut ns = notify_ui();
            if crate::notify::hidden_now(&p) {
                ns.hidden.insert(
                    tag.clone(),
                    (peer, Box::new(n.clone()), logo.clone(), picture.clone()),
                );
            } else {
                ns.hidden.remove(&tag);
            }
            ns.mirror.insert(
                tag.clone(),
                Entry {
                    peer,
                    key: n.key.clone(),
                    package: n.package.clone(),
                    otp: n.otp.clone(),
                },
            );
            // Home presents it again under the settings of the moment; it
            // needs the text only, not the picture bytes.
            ns.mirror.set_note(
                &tag,
                pb_protocol::control::Notification {
                    picture: Vec::new(),
                    ..n.clone()
                },
            );
            ns.meta
                .insert(tag, (group, reply_index, picture.clone()))
                .and_then(|(_, _, old)| old)
        };
        if let Some(old) = old_picture.filter(|o| Some(o) != picture.as_ref()) {
            let _ = std::fs::remove_file(old);
        }
        if settings.notify_clear_on_phone {
            arm_timer(hwnd, TIMER_NOTIFY, 5000);
        }
    } else {
        st.log.line(&format!(
            "NOTIFY toast not shown key={}",
            pb_transport::notify::key_hash(&n.key)
        ));
        if let Some(p) = picture {
            let _ = std::fs::remove_file(p);
        }
    }
    // A quiet copy (re-send, sync at start) is old news: never auto-copy it.
    if settings.notify_otp_autocopy && !locked && !refresh && !n.quiet && !n.otp.is_empty() {
        copy_otp(hwnd, &n.otp);
    }
}

/// At unlock: show in full the toasts that arrived while the PC was locked.
fn refresh_hidden(hwnd: HWND, st: &State) {
    let list: Vec<HiddenToast> = notify_ui().hidden.drain().map(|(_, h)| h).collect();
    for (peer, n, logo, picture) in list {
        // A refresh records nothing, so it needs no device name.
        show_phone_notification(hwnd, st, peer, "", *n, logo, picture, true);
    }
}

/// A click, button, reply or dismissal on one of our toasts.
fn on_toast(hwnd: HWND, st: &State, ev: ToastEvent) {
    use crate::{
        notify::Entry,
        toast_xml::{parse_args, parse_ctx, ToastArgs},
    };
    use pb_protocol::control::NotifyActionKind as K;
    let settings = st.settings.lock().map(|s| s.clone()).unwrap_or_default();
    let paired = |p: &pb_identity::PeerId| {
        st.peers
            .lock()
            .map(|g| g.iter().any(|(id, _)| id == p))
            .unwrap_or(false)
    };
    match ev {
        ToastEvent::Activated(raw, reply) => {
            let Some(a) = parse_args(&raw) else { return };
            let tag = match &a {
                ToastArgs::Open { tag }
                | ToastArgs::Button { tag, .. }
                | ToastArgs::Reply { tag }
                | ToastArgs::CopyOtp { tag }
                | ToastArgs::CopyText { tag }
                | ToastArgs::Mute { tag }
                | ToastArgs::Silence { tag } => tag.clone(),
            };
            if matches!(a, ToastArgs::CopyText { .. }) {
                copy_shared_text(hwnd, st, &tag);
                return;
            }
            if tag.starts_with("pb-") {
                // Phone Bridge's own message: like a balloon click.
                if notify_ui().last_own_tag == tag {
                    if let Some(u) = LAST_LINK.lock().ok().and_then(|mut g| g.take()) {
                        open_link(&u);
                    } else if let Some(p) = LAST_FILE.lock().ok().and_then(|g| g.clone()) {
                        reveal(&p);
                    }
                }
                return;
            }
            // Which phone and notification: from the list, or (after a
            // restart, before the phone synced) from the toast itself.
            let ctx = parse_ctx(&raw);
            let (entry, reply_index) = {
                let mut ns = notify_ui();
                // Windows removed the clicked toast: not a clear.
                ns.mirror.unsee(&tag);
                let known = ns.mirror.get(&tag).cloned();
                let reply_index = ns.meta.get(&tag).and_then(|m| m.1);
                (known, reply_index.or(ctx.as_ref().and_then(|c| c.reply)))
            };
            let entry = entry.or_else(|| {
                let c = ctx?;
                Some(Entry {
                    peer: pb_identity::PeerId::from_hex(&c.peer).ok()?,
                    key: c.key,
                    package: c.package,
                    otp: c.otp,
                })
            });
            // Unknown, or the phone was removed: nothing to do.
            let Some(e) = entry.filter(|e| paired(&e.peer)) else {
                return;
            };
            let send = |kind: K, index: u32, text: String| {
                let _ = st.cmds.send(Cmd::NotifyAction {
                    peer: e.peer,
                    key: e.key.clone(),
                    kind,
                    index,
                    text,
                });
            };
            match a {
                ToastArgs::Open { .. } => {
                    if settings.notify_open_on_phone {
                        send(K::Open, 0, String::new());
                    }
                }
                ToastArgs::Button { index, .. } => send(K::Button, index, String::new()),
                ToastArgs::Reply { .. } => {
                    if let (Some(i), Some(text)) =
                        (reply_index, reply.filter(|t| !t.trim().is_empty()))
                    {
                        send(K::Reply, i, text);
                    }
                }
                ToastArgs::Silence { .. } => send(K::Silence, 0, String::new()),
                ToastArgs::CopyOtp { .. } => copy_otp(hwnd, &e.otp),
                ToastArgs::CopyText { .. } => {}
                ToastArgs::Mute { .. } => {
                    let _ = st.settings.lock().map(|mut s| {
                        match s.notify_apps.iter_mut().find(|(p, _, _)| *p == e.package) {
                            Some(app) => app.2 = true,
                            None => {
                                s.notify_apps
                                    .push((e.package.clone(), e.package.clone(), true))
                            }
                        }
                    });
                    save_settings(st);
                    let gone = {
                        let mut ns = notify_ui();
                        let tags = ns.mirror.remove_package(&e.package);
                        forget(&mut ns, tags)
                    };
                    remove_toasts(gone);
                }
            }
        }
        ToastEvent::Dismissed(tag) => {
            if tag.starts_with("pb-") {
                return;
            }
            let entry = {
                let mut ns = notify_ui();
                let e = ns.mirror.get(&tag).cloned();
                if let Some(e) = &e {
                    ns.mirror.remove_key(&e.peer, &e.key);
                    ns.hidden.remove(&tag);
                    if let Some((_, _, Some(p))) = ns.meta.remove(&tag) {
                        let _ = std::fs::remove_file(p);
                    }
                }
                e
            };
            if let (Some(e), true) = (entry, settings.notify_clear_on_phone) {
                let _ = st.cmds.send(Cmd::NotifyAction {
                    peer: e.peer,
                    key: e.key,
                    kind: K::Dismiss,
                    index: 0,
                    text: String::new(),
                });
            }
        }
    }
}

/// Toasts the user cleared from the notification centre: clear them on the
/// phone too (when that setting is on).
fn check_cleared(hwnd: HWND, st: &State) {
    let clear = st
        .settings
        .lock()
        .map(|s| s.notify_clear_on_phone)
        .unwrap_or(false);
    if notify_ui().mirror.is_empty() || !clear {
        kill_timer(hwnd, TIMER_NOTIFY);
        return;
    }
    // Read outside the lock (a WinRT call).
    let Ok(history) = crate::toast::history_tags() else {
        return;
    };
    // Windows keeps at most 20 toasts per app: at the cap, a missing toast
    // may have been pushed out rather than cleared by the user.
    let at_cap = history.len() >= 20;
    let gone = {
        let mut ns = notify_ui();
        let gone = ns.mirror.vanished(&history);
        for e in &gone {
            let tag = crate::notify::tag(&e.peer, &e.key);
            ns.hidden.remove(&tag);
            if let Some((_, _, Some(p))) = ns.meta.remove(&tag) {
                let _ = std::fs::remove_file(p);
            }
        }
        gone
    };
    if at_cap {
        return;
    }
    for e in gone {
        let _ = st.cmds.send(Cmd::NotifyAction {
            peer: e.peer,
            key: e.key,
            kind: pb_protocol::control::NotifyActionKind::Dismiss,
            index: 0,
            text: String::new(),
        });
    }
}

fn toggle_control(hwnd: HWND, st: &State) {
    if crate::control::is_active() {
        end_control(hwnd, "Back to this PC");
    } else {
        let _ = st.cmds.send(Cmd::ControlPhone);
    }
}

fn end_control(hwnd: HWND, why: &str) {
    // Control ran until now (every caller ends a session that was set up):
    // that was contact with the phone, whether or not Home is open.
    if let Some((id, _)) = STATE
        .get()
        .and_then(|st| st.peers.lock().ok().and_then(|g| g.first().cloned()))
    {
        crate::core::touch(&id);
    }
    crate::control::stop();
    // SAFETY: timer on our own window.
    let _ =
        unsafe { windows::Win32::UI::WindowsAndMessaging::KillTimer(Some(hwnd), TIMER_CONTROL) };
    crate::status::show(why, 4);
    balloon(hwnd, "Relay", why);
}

/// The menu entry and the Home tile: start when idle, otherwise stop or
/// cancel the question.
fn toggle_audio(st: &State, d: crate::audio_state::Dir) {
    let cmd = if crate::audio::phase(d) == crate::audio_state::Phase::Idle {
        Cmd::StartAudio(d)
    } else {
        Cmd::StopAudio(d)
    };
    let _ = st.cmds.send(cmd);
}

fn show_menu(hwnd: HWND) {
    use crate::menu_model::{build, Camera, Ids, Inputs};
    let Some(st) = STATE.get() else { return };
    // Read fresh every time: the rows show the real settings and sessions.
    let settings = st.settings.lock().map(|s| s.clone()).unwrap_or_default();
    let has_peer = st.peers.lock().map(|g| !g.is_empty()).unwrap_or(false);
    let camera = if crate::camera::pending() {
        Camera::Pending
    } else if crate::camera::is_running() {
        Camera::Running
    } else {
        Camera::Idle
    };
    let items = build(
        &Inputs {
            settings: &settings,
            has_peer,
            screen_open: crate::screen_window::is_open(),
            camera,
            audio: [
                crate::audio::phase(crate::audio_state::Dir::PhoneToPc),
                crate::audio::phase(crate::audio_state::Dir::PcToPhone),
            ],
        },
        &Ids {
            open: ID_OPEN_MAIN,
            send_files: ID_SEND_FILES,
            send_clipboard: ID_SEND_CLIPBOARD,
            screen: ID_SCREEN,
            camera: ID_CAMERA,
            settings: ID_SETTINGS,
            quit: ID_QUIT,
        },
    );
    crate::menu::show(hwnd, items);
}

/// Pause or resume Relay: saved, logged, and what is open is closed.
fn set_paused(hwnd: HWND, st: &State, on: bool) {
    let was = st
        .settings
        .lock()
        .map(|mut s| std::mem::replace(&mut s.paused, on))
        .unwrap_or(false);
    save_settings(st);
    let Some(t) = crate::pause::transition(was, on) else {
        return;
    };
    crate::applog::emit(crate::applog::pause(on));
    if t == crate::pause::Transition::Paused {
        st.log.line("PAUSE Relay paused: sessions closed");
        // The setting is already on, so nothing new slips in while closing.
        crate::core::pause_now();
        if crate::control::is_active() {
            end_control(hwnd, "Relay is paused");
        }
        crate::screen_window::close();
        crate::camera_window::close();
        // Tearing the virtual camera down waits on the Frame Server: not on this thread.
        std::thread::spawn(crate::camera::stop);
        crate::audio::stop_all();
        // A pairing code on screen would be refused by the core: take it down.
        let _ = st.cmds.send(crate::core::Cmd::CancelPair);
        crate::qr_window::close();
    } else {
        st.log.line("PAUSE Relay resumed");
    }
    set_tip(hwnd, &current_tip(st));
    // The main window and an open flyout show it now.
    refresh_flyout(st);
    post_settings_action(hwnd.0 as isize);
}

/// Snapshot of everything the Settings page shows.
fn settings_model(st: &State) -> crate::settings_rows::Model {
    let n = st.settings.lock().map(|s| s.clone()).unwrap_or_default();
    let (theme, share, share_images, passwords, keep, auto, save_dir, control) = st
        .settings
        .lock()
        .map(|s| {
            (
                s.theme,
                s.universal_clipboard,
                s.share_images,
                !s.skip_private,
                s.keep_in_history,
                s.auto_accept,
                s.save_dir.display().to_string(),
                s.allow_control,
            )
        })
        .unwrap_or_default();
    crate::settings_rows::Model {
        theme,
        share,
        share_images,
        passwords,
        keep,
        auto_copy_text: n.auto_copy_shared_text,
        auto,
        save_dir,
        sendto: sendto_enabled(),
        startup: startup_enabled(),
        control,
        keep_history: n.keep_history,
        keep_logs: n.keep_logs,
        audio_skip_confirm: n.audio_skip_confirm_pc_to_phone,
        log_dir: crate::applog::dir().display().to_string(),
        log_bytes: crate::applog::size_bytes(),
        logs_failed: crate::applog::handle().is_none(),
        devices: paired_devices(st),
        my_fingerprint: MY_FINGERPRINT.lock().map(|f| f.clone()).unwrap_or_default(),
        notify: n.notify_enabled,
        notify_lock: n.notify_lock_screen,
        notify_clear: n.notify_clear_on_phone,
        notify_open: n.notify_open_on_phone,
        notify_otp: n.notify_otp_autocopy,
        notify_quiet_mirror: n.notify_quiet_mirroring,
        content: n.notify_content,
        sound: n.notify_sound,
        apps: n
            .notify_apps
            .iter()
            .map(|(_, name, muted)| (name.clone(), !muted))
            .collect(),
        camera: crate::main_window::wants_camera().then(|| camera_summary(st)),
    }
}

/// What the camera is doing, from the same model the camera window uses
/// (the slow measurements are left to that window).
fn camera_summary(st: &State) -> crate::camera_status::Summary {
    let streaming = crate::camera::is_running();
    let installed = crate::camera_register::is_registered();
    let inputs = crate::camera_status::Inputs {
        phone: st
            .peers
            .lock()
            .ok()
            .and_then(|g| g.first().map(|(_, n)| n.clone())),
        installed,
        update_ready: installed && crate::camera_register::needs_install(),
        access: crate::camera_register::camera_access(),
        streaming,
        fps: crate::camera::fps(),
        listed: None,
        dark: crate::camera::picture_dark(),
        preview_fps: None,
        pending: crate::camera::pending(),
        start_error: crate::camera::last_error(),
        lens: streaming.then(|| crate::camera::lens() as u8),
    };
    crate::camera_status::summary(&inputs, crate::camera::playback_error())
}

fn apply_settings_action(hwnd: HWND, st: &State, a: crate::settings_rows::Action) {
    use crate::settings_rows::{Action, CameraAct, ChoiceKey, Key};
    match a {
        Action::Theme(t) => {
            let _ = st.settings.lock().map(|mut s| s.theme = t);
            crate::fluent::set_theme(t);
            crate::fluent::on_theme_change();
            save_settings(st);
        }
        Action::Toggle(Key::Share) => command(hwnd, st, ID_TOGGLE_CLIP),
        Action::Toggle(Key::Passwords) => command(hwnd, st, ID_TOGGLE_PRIVATE),
        Action::Toggle(Key::Auto) => command(hwnd, st, ID_TOGGLE_AUTO),
        Action::Toggle(Key::SendTo) => command(hwnd, st, ID_SENDTO),
        Action::Toggle(Key::Control) => {
            let now_on = st
                .settings
                .lock()
                .map(|mut s| {
                    s.allow_control = !s.allow_control;
                    s.allow_control
                })
                .unwrap_or(false);
            save_settings(st);
            if !now_on {
                crate::core::stop_phone_control();
            }
        }
        Action::Toggle(
            k @ (Key::Notify
            | Key::NotifyLock
            | Key::NotifyClear
            | Key::NotifyOpen
            | Key::NotifyOtp
            | Key::NotifyQuietMirror),
        ) => {
            let _ = st.settings.lock().map(|mut s| {
                let field = match k {
                    Key::Notify => &mut s.notify_enabled,
                    Key::NotifyLock => &mut s.notify_lock_screen,
                    Key::NotifyClear => &mut s.notify_clear_on_phone,
                    Key::NotifyOpen => &mut s.notify_open_on_phone,
                    Key::NotifyOtp => &mut s.notify_otp_autocopy,
                    _ => &mut s.notify_quiet_mirroring,
                };
                *field = !*field;
            });
            save_settings(st);
        }
        Action::Choice(k, i) => {
            let _ = st.settings.lock().map(|mut s| match k {
                ChoiceKey::Content => s.notify_content = i.min(2),
                ChoiceKey::Sound => s.notify_sound = i.min(2),
            });
            save_settings(st);
        }
        Action::Mute(i) => {
            let muted_now = st
                .settings
                .lock()
                .ok()
                .and_then(|mut s| {
                    s.notify_apps.get_mut(i).map(|app| {
                        app.2 = !app.2;
                        app.2.then(|| app.0.clone())
                    })
                })
                .flatten();
            save_settings(st);
            // Muting takes the app's toasts away now.
            if let Some(package) = muted_now {
                let mut ns = notify_ui();
                let gone = ns.mirror.remove_package(&package);
                forget(&mut ns, gone);
            }
        }
        Action::Toggle(Key::ShareImages) => {
            let _ = st
                .settings
                .lock()
                .map(|mut s| s.share_images = !s.share_images);
            save_settings(st);
        }
        Action::Toggle(Key::Keep) => {
            let _ = st
                .settings
                .lock()
                .map(|mut s| s.keep_in_history = !s.keep_in_history);
            save_settings(st);
        }
        Action::Toggle(Key::AutoCopyText) => {
            let _ = st
                .settings
                .lock()
                .map(|mut s| s.auto_copy_shared_text = !s.auto_copy_shared_text);
            save_settings(st);
        }
        // Off stops recording and deletes the stored history: asked first.
        Action::Toggle(Key::KeepHistory) => {
            let on = st.settings.lock().map(|s| s.keep_history).unwrap_or(true);
            if !on {
                set_keep_history(st, true);
            } else {
                let raw = hwnd.0 as isize;
                ask_yes_no(
                    Dialog::question(
                        "Turn off history?".into(),
                        "This deletes the history on this PC.".into(),
                        "Turn off",
                        "Cancel",
                    ),
                    move |yes| {
                        if yes {
                            if let Some(st) = STATE.get() {
                                set_keep_history(st, false);
                            }
                            // The switch and the Timeline show it now.
                            post_settings_action(raw);
                        }
                    },
                );
            }
        }
        Action::ClearHistory => {
            let raw = hwnd.0 as isize;
            ask_yes_no(
                Dialog::question(
                    "Clear history?".into(),
                    "Everything in the Timeline will be deleted.".into(),
                    "Clear",
                    "Cancel",
                ),
                move |yes| {
                    if yes {
                        crate::timeline::with(|t| t.clear());
                        // The Timeline page shows it now.
                        post_settings_action(raw);
                    }
                },
            );
        }
        // Read by the next PC audio request: no restart needed.
        Action::Toggle(Key::AudioSkipConfirm) => {
            let _ = st
                .settings
                .lock()
                .map(|mut s| s.audio_skip_confirm_pc_to_phone = !s.audio_skip_confirm_pc_to_phone);
            save_settings(st);
        }
        Action::Toggle(Key::KeepLogs) => {
            let on = st
                .settings
                .lock()
                .map(|mut s| {
                    s.keep_logs = !s.keep_logs;
                    s.keep_logs
                })
                .unwrap_or(true);
            crate::applog::set_enabled(on);
            save_settings(st);
        }
        Action::OpenLogs => {
            let dir = crate::applog::dir();
            let _ = std::fs::create_dir_all(&dir);
            let _ = std::process::Command::new("explorer").arg(&dir).spawn();
        }
        Action::ClearLogs => {
            let raw = hwnd.0 as isize;
            ask_yes_no(
                Dialog::question(
                    "Clear all logs?".into(),
                    format!(
                        "This deletes all {} of logs on this PC.",
                        crate::settings_rows::log_size_text(crate::applog::size_bytes())
                    ),
                    "Clear",
                    "Cancel",
                ),
                move |yes| {
                    if yes {
                        crate::applog::clear_all();
                        // The row shows the new size.
                        post_settings_action(raw);
                    }
                },
            );
        }
        Action::Toggle(Key::Startup) => {
            if let Err(e) = set_startup(!startup_enabled()) {
                balloon(hwnd, "Relay", &format!("Could not change startup: {e}"));
            }
        }
        Action::ChangeFolder => {
            let raw = hwnd.0 as isize;
            std::thread::spawn(move || {
                let Some(dir) = pick_folder() else { return };
                if let Some(st) = STATE.get() {
                    let _ = st.settings.lock().map(|mut s| s.save_dir = dir);
                    save_settings(st);
                }
                post_settings_action(raw);
            });
        }
        Action::OpenFolder => command(hwnd, st, ID_OPEN_FOLDER),
        Action::Remove(i) => command(hwnd, st, ID_UNPAIR_BASE + i),
        Action::Pair => command(hwnd, st, ID_PAIR),
        // The same commands as the tray entry and the camera window.
        Action::Camera(CameraAct::Toggle) => command(hwnd, st, ID_CAMERA),
        Action::Camera(CameraAct::Repair) => {
            let _ = st.cmds.send(Cmd::RepairCamera);
        }
        Action::Camera(CameraAct::OpenWindow) => command(hwnd, st, ID_CAMERA_WINDOW),
    }
}

/// "Keep history": saved, and the Timeline records (or is deleted) to match.
fn set_keep_history(st: &State, on: bool) {
    let _ = st.settings.lock().map(|mut s| s.keep_history = on);
    save_settings(st);
    crate::timeline::with(|t| t.set_enabled(on));
}

const RUN_KEY: PCWSTR = w!(r"Software\Microsoft\Windows\CurrentVersion\Run");

fn startup_enabled() -> bool {
    use windows::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_SZ};
    let mut size = 0u32;
    // SAFETY: size query only.
    unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            RUN_KEY,
            w!("Phone Bridge"),
            RRF_RT_REG_SZ,
            None,
            None,
            Some(&mut size),
        )
    }
    .is_ok()
}

/// "Start with Windows": a per-user Run entry for this exe (no admin, removable).
fn set_startup(on: bool) -> Result<(), String> {
    use windows::Win32::System::Registry::{
        RegDeleteKeyValueW, RegSetKeyValueW, HKEY_CURRENT_USER, REG_SZ,
    };
    if on {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let value = wide(&crate::relaunch::command_line(
            &exe,
            crate::relaunch::args(),
        ));
        // SAFETY: NUL-terminated UTF-16 value with its byte length.
        unsafe {
            RegSetKeyValueW(
                HKEY_CURRENT_USER,
                RUN_KEY,
                w!("Phone Bridge"),
                REG_SZ.0,
                Some(value.as_ptr() as *const _),
                (value.len() * 2) as u32,
            )
        }
        .ok()
        .map_err(|e| e.to_string())
    } else {
        // SAFETY: deletes our own value.
        unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, RUN_KEY, w!("Phone Bridge")) }
            .ok()
            .map_err(|e| e.to_string())
    }
}

/// Folder picker for "Save received files to"; runs on its own thread.
fn pick_folder() -> Option<std::path::PathBuf> {
    use windows::Win32::{
        System::Com::{
            CoCreateInstance, CoInitializeEx, CoTaskMemFree, CLSCTX_INPROC_SERVER,
            COINIT_APARTMENTTHREADED,
        },
        UI::Shell::{FileOpenDialog, IFileOpenDialog, FOS_PICKFOLDERS, SIGDN_FILESYSPATH},
    };
    // SAFETY: standard COM usage on a fresh STA thread; the string is freed.
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let dlg: IFileOpenDialog =
            CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER).ok()?;
        let opts = dlg.GetOptions().ok()?;
        dlg.SetOptions(opts | FOS_PICKFOLDERS).ok()?;
        let _ = dlg.SetTitle(w!("Save received files to"));
        dlg.Show(None).ok()?;
        let item = dlg.GetResult().ok()?;
        let name = item.GetDisplayName(SIGDN_FILESYSPATH).ok()?;
        let path = name.to_string().ok().map(std::path::PathBuf::from);
        CoTaskMemFree(Some(name.0 as *const _));
        path
    }
}

fn command(hwnd: HWND, st: &State, id: usize) {
    match id {
        ID_PAIR => {
            let _ = st.cmds.send(Cmd::Pair);
        }
        ID_SEND_CLIPBOARD => match clipboard::get_text(Some(hwnd)) {
            Ok(ClipText::Text(t)) => {
                let _ = st.cmds.send(Cmd::SendClipboard(t));
            }
            Ok(ClipText::Excluded) => {
                st.log
                    .line("SEND refused: clipboard marked private by its source app");
                balloon(
                    hwnd,
                    "Relay",
                    "Not sent: the app that copied this marked it private",
                );
            }
            Ok(ClipText::Empty) => balloon(hwnd, "Relay", "Clipboard has no text"),
            Err(e) => balloon(hwnd, "Relay", &format!("Could not read clipboard: {e}")),
        },
        ID_SEND_FILES => {
            let cmds = st.cmds.clone();
            std::thread::spawn(move || {
                let files = pick_files();
                if !files.is_empty() {
                    let _ = cmds.send(Cmd::SendFiles(files));
                }
            });
        }
        ID_SENDTO => match toggle_sendto() {
            Ok(true) => balloon(hwnd, "Relay", "Added: right-click files, Send to, Relay"),
            Ok(false) => balloon(hwnd, "Relay", "Removed from the Send to menu"),
            Err(e) => balloon(
                hwnd,
                "Relay",
                &format!("Could not change the Send to menu: {e}"),
            ),
        },
        ID_OPEN_FOLDER => {
            // Copy the path out: no disk access while holding the settings lock.
            if let Some(dir) = st.settings.lock().ok().map(|s| s.save_dir.clone()) {
                let _ = std::fs::create_dir_all(&dir);
                let _ = std::process::Command::new("explorer").arg(&dir).spawn();
            }
        }
        ID_TOGGLE_CLIP => {
            let on = st
                .settings
                .lock()
                .map(|mut s| {
                    s.universal_clipboard = !s.universal_clipboard;
                    s.universal_clipboard
                })
                .unwrap_or(false);
            save_settings(st);
            st.log.line(&format!("SETTING universal_clipboard={on}"));
            balloon(
                hwnd,
                "Share clipboard",
                if on {
                    "On: what you copy is shared with your devices"
                } else {
                    "Off"
                },
            );
        }
        ID_TOGGLE_PRIVATE => {
            let _ = st
                .settings
                .lock()
                .map(|mut s| s.skip_private = !s.skip_private);
            save_settings(st);
        }
        ID_TOGGLE_AUTO => {
            let _ = st
                .settings
                .lock()
                .map(|mut s| s.auto_accept = !s.auto_accept);
            save_settings(st);
        }
        ID_OPEN_FLYOUT => open_flyout(hwnd, st),
        ID_OPEN_MAIN => crate::main_window::open(crate::main_window::Page::Home),
        ID_SETTINGS => {
            // Which devices still answer as the paired one (the page updates
            // when it ends); the window asks too, under the same debounce.
            check_peers(&st.cmds);
            crate::main_window::open(crate::main_window::Page::Settings);
        }
        ID_MENU => show_menu(hwnd),
        crate::menu_model::ID_PAUSE => {
            let now = st.settings.lock().map(|s| s.paused).unwrap_or(false);
            set_paused(hwnd, st, !now);
        }
        // The menu's quick settings run the Settings page's own setters.
        crate::menu_model::ID_TOGGLE_ALLOW_CONTROL => apply_settings_action(
            hwnd,
            st,
            crate::settings_rows::Action::Toggle(crate::settings_rows::Key::Control),
        ),
        ID_CONTROL => toggle_control(hwnd, st),
        crate::menu_model::ID_AUDIO_PHONE => toggle_audio(st, crate::audio_state::Dir::PhoneToPc),
        crate::menu_model::ID_AUDIO_PC => toggle_audio(st, crate::audio_state::Dir::PcToPhone),
        ID_CONTROL_STOP => {
            if crate::control::is_active() {
                end_control(hwnd, "Back to this PC");
            }
        }
        ID_SCREEN_SHOW => {
            if !crate::screen_window::is_open() && !crate::core::screen_pending() {
                let _ = st.cmds.send(Cmd::ShowScreen);
            }
        }
        ID_SCREEN_HIDE => crate::screen_window::close(),
        ID_CAMERA_START => {
            let _ = st.cmds.send(Cmd::StartCamera);
        }
        ID_CAMERA_STOP => {
            let _ = st.cmds.send(Cmd::StopCamera);
        }
        ID_CAMERA_LENS => {
            let _ = st.cmds.send(Cmd::SwitchCameraLens);
        }
        ID_CAMERA_REPAIR => {
            let _ = st.cmds.send(Cmd::RepairCamera);
        }
        ID_CAMERA_REMOVE => {
            let _ = st.cmds.send(Cmd::RemoveCamera);
        }
        ID_CAMERA_WINDOW => {
            let peers = st.peers.clone();
            let cmds = st.cmds.clone();
            crate::camera_window::open(
                std::sync::Arc::new(move || {
                    peers
                        .lock()
                        .ok()
                        .and_then(|g| g.first().map(|(_, n)| n.clone()))
                }),
                Box::new(move |a| {
                    let _ = cmds.send(match a {
                        crate::camera_window::Action::Toggle => {
                            if crate::camera::is_running() || crate::camera::pending() {
                                Cmd::StopCamera
                            } else {
                                Cmd::StartCamera
                            }
                        }
                        crate::camera_window::Action::Lens => Cmd::SwitchCameraLens,
                        crate::camera_window::Action::Remove => Cmd::RemoveCamera,
                        crate::camera_window::Action::Repair => Cmd::RepairCamera,
                    });
                }),
            );
        }
        ID_CAMERA => {
            let _ = st
                .cmds
                .send(if crate::camera::is_running() || crate::camera::pending() {
                    Cmd::StopCamera
                } else {
                    Cmd::StartCamera
                });
        }
        ID_SCREEN => {
            if crate::screen_window::is_open() {
                crate::screen_window::close();
            } else if crate::core::screen_pending() {
                let _ = st.cmds.send(Cmd::CancelScreen);
            } else {
                let _ = st.cmds.send(Cmd::ShowScreen);
            }
        }
        ID_QUIT => {
            // Transfers running: ask first; "yes" comes back here confirmed.
            let running = crate::transfers::active_count();
            if running > 0 && !QUIT_CONFIRMED.swap(false, std::sync::atomic::Ordering::SeqCst) {
                let raw = hwnd.0 as isize;
                let (heading, body) = crate::transfers::quit_warning(running);
                ask_yes_no(
                    Dialog::question(heading, body, "Quit and stop", "Keep running"),
                    move |yes| {
                        if yes {
                            QUIT_CONFIRMED.store(true, std::sync::atomic::Ordering::SeqCst);
                            post_command(raw, ID_QUIT);
                        }
                    },
                );
                return;
            }
            // Running transfers end cleanly: the peers are told. Best effort,
            // the process exits right after.
            let ended = crate::transfers::cancel_all(crate::transfers::CancelBy::Quit) > 0;
            // Audio and the phone-screen window are stopped the same way, so the phone shows
            // "stopped" and not "lost the connection".
            let audio = crate::audio_state::Dir::ALL
                .iter()
                .any(|d| crate::audio::phase(*d) == crate::audio_state::Phase::Running);
            crate::audio::stop_all();
            crate::screen_window::close();
            if ended || audio {
                std::thread::sleep(std::time::Duration::from_millis(400));
            }
            crate::camera_window::close();
            // Nothing of the camera stays behind: Stop, Remove, Shutdown.
            crate::camera::stop();
            // SAFETY: our own window.
            let _ = unsafe { windows::Win32::UI::WindowsAndMessaging::DestroyWindow(hwnd) };
        }
        n if n >= ID_UNPAIR_BASE => {
            let peer = st
                .peers
                .lock()
                .ok()
                .and_then(|g| g.get(n - ID_UNPAIR_BASE).cloned());
            if let Some((pid, name)) = peer {
                let cmds = st.cmds.clone();
                ask_yes_no(
                    Dialog::question(
                        format!("Remove \"{name}\"?"),
                        "It will no longer be able to connect.".into(),
                        "Remove",
                        "Cancel",
                    ),
                    move |yes| {
                        if yes {
                            let _ = cmds.send(Cmd::Unpair(pid));
                        }
                    },
                );
            }
        }
        _ => {}
    }
}

extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let Some(st) = STATE.get() else {
        // SAFETY: default handling for messages before init.
        return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) };
    };
    let taskbar = TASKBAR_CREATED.load(std::sync::atomic::Ordering::Relaxed);
    if taskbar != 0 && msg == taskbar {
        // Explorer restarted: our icon is gone, add it again (same tooltip).
        add_icon(hwnd, &current_tip(st));
        return LRESULT(0);
    }
    match msg {
        WM_TRAY => {
            let ev = (lparam.0 & 0xffff) as u32;
            // Version 4: exactly one event per gesture, so a click opens one surface.
            if ev == NIN_SELECT || ev == NIN_KEYSELECT {
                open_flyout(hwnd, st);
            } else if ev == WM_CONTEXTMENU {
                crate::flyout::close();
                show_menu(hwnd);
            } else if ev == NIN_BALLOONUSERCLICK {
                if let Some(u) = LAST_LINK.lock().ok().and_then(|mut g| g.take()) {
                    open_link(&u);
                } else if let Some(p) = LAST_FILE.lock().ok().and_then(|g| g.clone()) {
                    reveal(&p);
                }
            }
            LRESULT(0)
        }
        WM_CORE_EVENT => {
            let drained: Vec<UiEvent> = EVENTS
                .lock()
                .map(|mut q| q.drain(..).collect())
                .unwrap_or_default();
            for e in drained {
                handle_event(hwnd, st, e);
            }
            if core_error().is_some() {
                set_tip(hwnd, &current_tip(st));
            }
            if crate::main_window::is_open() {
                crate::main_window::refresh();
            }
            refresh_flyout(st);
            LRESULT(0)
        }
        WM_TOAST => {
            let drained: Vec<ToastEvent> = TOAST_EVENTS
                .lock()
                .map(|mut q| q.drain(..).collect())
                .unwrap_or_default();
            for ev in drained {
                on_toast(hwnd, st, ev);
            }
            if crate::main_window::is_open() {
                crate::main_window::refresh();
            }
            LRESULT(0)
        }
        WM_WTSSESSION_CHANGE => {
            match wparam.0 {
                WTS_SESSION_LOCK => notify_ui().locked = true,
                WTS_SESSION_UNLOCK => {
                    notify_ui().locked = false;
                    refresh_hidden(hwnd, st);
                }
                _ => {}
            }
            // Home lists notifications only while their content may show.
            if crate::main_window::is_open() {
                crate::main_window::refresh();
            }
            LRESULT(0)
        }
        windows::Win32::UI::WindowsAndMessaging::WM_TIMER if wparam.0 == TIMER_RELEASE => {
            kill_timer(hwnd, TIMER_RELEASE);
            // Busy: a window opened meanwhile; its closing arms this again.
            if crate::d2d::release_if_unused() == crate::d2d::Release::Released {
                trim_working_set();
                st.log.line("UI device released");
            }
            LRESULT(0)
        }
        WM_TIMELINE_WAKE => {
            arm_timeline(hwnd);
            LRESULT(0)
        }
        windows::Win32::UI::WindowsAndMessaging::WM_TIMER if wparam.0 == TIMER_TIMELINE => {
            use std::sync::atomic::Ordering;
            let mut wants_flush = false;
            crate::timeline::with(|t| {
                t.flush_if_due(std::time::Instant::now());
                wants_flush = t.wants_flush();
            });
            // Catches what changes without a message (a session or
            // connection ending, timeline adds from the core, "5 min ago");
            // the window repaints only if its model changed.
            let main_open = crate::main_window::is_open();
            if main_open {
                crate::main_window::refresh();
            }
            refresh_flyout(st);
            let flyout_open = crate::flyout::is_open();
            if !timeline_timer_needed(wants_flush, main_open, flyout_open) {
                // Idle: no more wake-ups until an add or a window opens.
                kill_timer(hwnd, TIMER_TIMELINE);
                TIMELINE_ARMED.store(false, Ordering::SeqCst);
                // An add that saw the timer still armed posted no wake.
                let mut late = false;
                crate::timeline::with(|t| late = t.wants_flush());
                if late {
                    arm_timeline(hwnd);
                }
            }
            LRESULT(0)
        }
        windows::Win32::UI::WindowsAndMessaging::WM_TIMER if wparam.0 == TIMER_NOTIFY => {
            check_cleared(hwnd, st);
            LRESULT(0)
        }
        windows::Win32::UI::WindowsAndMessaging::WM_TIMER if wparam.0 == TIMER_OTP => {
            kill_timer(hwnd, TIMER_OTP);
            if let Some((seq, code)) = notify_ui().otp_pending.take() {
                let now = match clipboard::get_text_marked(Some(hwnd)) {
                    Ok((ClipText::Text(t), _)) => Some(t),
                    _ => None,
                };
                if crate::notify::otp_should_clear(seq, clipboard_seq(), now.as_deref(), &code)
                    && clipboard::clear(Some(hwnd)).is_ok()
                {
                    OWN_SEQ.store(clipboard_seq(), std::sync::atomic::Ordering::Relaxed);
                }
            }
            LRESULT(0)
        }
        WM_CLIPBOARDUPDATE => {
            // Ignore our own writes; debounce bursts from apps that set the
            // clipboard several times in a row.
            if clipboard_seq() != OWN_SEQ.load(std::sync::atomic::Ordering::Relaxed) {
                // SAFETY: timer on our own window.
                unsafe {
                    windows::Win32::UI::WindowsAndMessaging::SetTimer(
                        Some(hwnd),
                        TIMER_CLIP,
                        300,
                        None,
                    )
                };
            }
            LRESULT(0)
        }
        windows::Win32::UI::WindowsAndMessaging::WM_TIMER if wparam.0 == TIMER_CLIP => {
            // SAFETY: timer created above on this window.
            let _ = unsafe {
                windows::Win32::UI::WindowsAndMessaging::KillTimer(Some(hwnd), TIMER_CLIP)
            };
            let (on, images) = st
                .settings
                .lock()
                .map(|s| (s.universal_clipboard, s.share_images))
                .unwrap_or((false, false));
            if on {
                // Private (password manager) content reads as Excluded and is skipped.
                // Passing it along is the user's choice ("Send passwords and codes").
                let skip = st.settings.lock().map(|s| s.skip_private).unwrap_or(true);
                if let Ok((ClipText::Text(text), sensitive)) =
                    clipboard::get_text_marked(Some(hwnd))
                {
                    if !(sensitive && skip) {
                        let _ = st.cmds.send(Cmd::ClipboardChanged { text, sensitive });
                    }
                } else if crate::clip_image::enabled(on, images) {
                    read_clipboard_png(hwnd, st.cmds.clone());
                }
            }
            LRESULT(0)
        }
        WM_COMMAND => {
            command(hwnd, st, wparam.0 & 0xffff);
            if crate::main_window::is_open() {
                crate::main_window::refresh();
            }
            // The main window opens through a command: refresh it every second.
            if crate::main_window::is_open() {
                arm_timeline(hwnd);
            }
            LRESULT(0)
        }
        windows::Win32::UI::WindowsAndMessaging::WM_HOTKEY if wparam.0 as i32 == HOTKEY_CONTROL => {
            toggle_control(hwnd, st);
            LRESULT(0)
        }
        crate::control::WM_CONTROL_EXIT => {
            end_control(hwnd, "Back to this PC");
            LRESULT(0)
        }
        windows::Win32::UI::WindowsAndMessaging::WM_TIMER if wparam.0 == TIMER_CONTROL => {
            if crate::control::session_closed() {
                end_control(hwnd, "The phone ended control");
            }
            LRESULT(0)
        }
        WM_PAGE_FILE => {
            let drained: Vec<_> = PAGE_FILES
                .lock()
                .map(|mut q| q.drain(..).collect())
                .unwrap_or_default();
            for t in drained {
                balloon(hwnd, "Relay", &t);
            }
            // A file found missing (or back) shows so on its row now.
            crate::main_window::repaint_page();
            LRESULT(0)
        }
        WM_SETTINGS_ACTION => {
            let drained: Vec<_> = SETTINGS_ACTIONS
                .lock()
                .map(|mut q| q.drain(..).collect())
                .unwrap_or_default();
            for a in drained {
                apply_settings_action(hwnd, st, a);
            }
            if crate::main_window::is_open() {
                crate::main_window::refresh();
            }
            LRESULT(0)
        }
        // Windows switched light/dark (or accent): repaint the themed windows.
        WM_SETTINGCHANGE => {
            if lparam.0 != 0 {
                // SAFETY: for WM_SETTINGCHANGE a non-null lParam is a
                // NUL-terminated string naming the changed area.
                let area = unsafe { PCWSTR(lparam.0 as *const u16).to_string() };
                if area.is_ok_and(|a| a == "ImmersiveColorSet") {
                    crate::fluent::on_theme_change();
                }
            }
            // SAFETY: default handling.
            unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
        }
        // Windows sign-out or shutdown: Stop, Remove, Shutdown now (Session
        // lifetime would also remove the device, but nothing is left to chance).
        WM_ENDSESSION => {
            if wparam.0 != 0 {
                // The history first: stopping the camera can take a while
                // and Windows may end the process at any moment now.
                crate::timeline::with(|t| t.flush());
                crate::camera::stop();
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            crate::camera::stop();
            crate::timeline::with(|t| t.flush());
            let nid = icon_data(hwnd);
            // SAFETY: removes our icon; then ends the message loop.
            unsafe {
                let _ = Shell_NotifyIconW(NIM_DELETE, &nid);
                PostQuitMessage(0);
            }
            LRESULT(0)
        }
        // SAFETY: default handling.
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

/// Create the hidden window and tray icon. Returns the window as an integer
/// so the core thread can post to it.
pub fn create(
    cmds: UnboundedSender<Cmd>,
    peers: Peers,
    auto_accept: bool,
    log: crate::core::Log,
    settings: crate::core::SharedSettings,
    settings_path: std::path::PathBuf,
) -> windows::core::Result<isize> {
    let close_cmds = cmds.clone();
    crate::qr_window::set_on_user_close(Box::new(move || {
        let _ = close_cmds.send(Cmd::CancelPair);
    }));
    crate::main_window::init(crate::main_window::Host {
        log: log.clone(),
        settings: settings.clone(),
        settings_path: settings_path.clone(),
        model: Box::new(|| STATE.get().map(page_model).unwrap_or_default()),
        act: Box::new(page_action),
        check_peers: Box::new(|| {
            if let Some(st) = STATE.get() {
                check_peers(&st.cmds);
            }
        }),
    });
    // A Timeline file check that changes a row repaints the window.
    crate::pages::timeline::files::set_notify(|| {
        let raw = TRAY_HWND.load(std::sync::atomic::Ordering::Relaxed);
        if raw != 0 {
            post_page_file(raw);
        }
    });
    let _ = STATE.set(State {
        cmds,
        peers,
        auto_accept,
        log,
        settings,
        settings_path,
    });
    // SAFETY: standard window-class registration and creation on this thread.
    unsafe {
        let hinstance = GetModuleHandleW(None)?;
        let wc = WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: hinstance.into(),
            lpszClassName: CLASS,
            ..Default::default()
        };
        RegisterClassW(&wc);
        // A hidden top-level window (never shown), not a message-only one:
        // only top-level windows get the TaskbarCreated broadcast we need to
        // put the icon back after Explorer restarts.
        let hwnd = CreateWindowExW(
            windows::Win32::UI::WindowsAndMessaging::WS_EX_TOOLWINDOW,
            CLASS,
            w!("Relay"),
            windows::Win32::UI::WindowsAndMessaging::WS_POPUP,
            0,
            0,
            0,
            0,
            None,
            None,
            Some(hinstance.into()),
            None,
        )?;
        TASKBAR_CREATED.store(
            windows::Win32::UI::WindowsAndMessaging::RegisterWindowMessageW(w!("TaskbarCreated")),
            std::sync::atomic::Ordering::Relaxed,
        );
        TRAY_HWND.store(hwnd.0 as isize, std::sync::atomic::Ordering::Relaxed);
        let paused = STATE
            .get()
            .and_then(|st| st.settings.lock().ok().map(|s| s.paused))
            .unwrap_or(false);
        add_icon(
            hwnd,
            &if paused {
                crate::pause::tooltip(true, "")
            } else {
                "Relay (starting)".to_owned()
            },
        );
        // Real toasts with Phone Bridge's identity; balloons if that fails.
        let raw = hwnd.0 as isize;
        let push = move |ev: ToastEvent| {
            if let Ok(mut q) = TOAST_EVENTS.lock() {
                q.push_back(ev);
            }
            // SAFETY: posting to our own window.
            let _ = PostMessageW(Some(HWND(raw as *mut _)), WM_TOAST, WPARAM(0), LPARAM(0));
        };
        let on_dismiss = push;
        let registered = crate::toast::register(
            Box::new(move |a, reply| push(ToastEvent::Activated(a, reply))),
            Box::new(move |tag| on_dismiss(ToastEvent::Dismissed(tag))),
        );
        if let Some(st) = STATE.get() {
            match &registered {
                Ok(()) => st.log.line("TOAST identity registered"),
                Err(e) => st
                    .log
                    .line(&format!("TOAST setup failed, using balloons: {e}")),
            }
        }
        notify_ui().toasts_ok = registered.is_ok();
        if registered.is_ok() {
            let (restored, total) = restore_from_history();
            if let Some(st) = STATE.get() {
                st.log.line(&format!(
                    "TOAST restored {restored} of {total} toasts from the notification centre"
                ));
            }
            arm_timer(hwnd, TIMER_NOTIFY, 5000);
        }
        // The last surface on this thread going schedules the device's
        // release (a window opening before then keeps it).
        let raw = hwnd.0 as isize;
        crate::d2d::set_on_idle(Box::new(move || {
            arm_timer(HWND(raw as *mut _), TIMER_RELEASE, RELEASE_AFTER_MS)
        }));
        // Stops itself after the first tick when nothing needs it. The
        // shared Direct2D device is made by the first window (about 0.2 s)
        // rather than ahead of time: kept warm it costs ~50 MB of private
        // bytes for as long as the tray runs.
        arm_timeline(hwnd);
        // Session lock and unlock (content hidden while locked).
        let _ = windows::Win32::System::RemoteDesktop::WTSRegisterSessionNotification(
            hwnd,
            windows::Win32::System::RemoteDesktop::NOTIFY_FOR_THIS_SESSION,
        );
        // Universal Clipboard: get WM_CLIPBOARDUPDATE for every clipboard change.
        let _ = windows::Win32::System::DataExchange::AddClipboardFormatListener(hwnd);
        // Ctrl+Alt+P: drive the phone with this PC's mouse and keyboard.
        let _ = windows::Win32::UI::Input::KeyboardAndMouse::RegisterHotKey(
            Some(hwnd),
            HOTKEY_CONTROL,
            windows::Win32::UI::Input::KeyboardAndMouse::MOD_CONTROL
                | windows::Win32::UI::Input::KeyboardAndMouse::MOD_ALT
                | windows::Win32::UI::Input::KeyboardAndMouse::MOD_NOREPEAT,
            0x50,
        );
        Ok(hwnd.0 as isize)
    }
}

/// Message Explorer broadcasts when the taskbar is (re)created.
static TASKBAR_CREATED: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

fn add_icon(hwnd: HWND, tip: &str) {
    // SAFETY: icon data refers to our window; resource 1 is the app icon
    // embedded by build.rs.
    unsafe {
        let mut nid = icon_data(hwnd);
        nid.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP;
        nid.uCallbackMessage = WM_TRAY;
        nid.hIcon = GetModuleHandleW(None)
            .ok()
            .and_then(|h| {
                LoadIconW(
                    Some(h.into()),
                    PCWSTR(std::ptr::without_provenance::<u16>(1)), /* MAKEINTRESOURCE(1) */
                )
                .ok()
            })
            .or_else(|| LoadIconW(None, IDI_APPLICATION).ok())
            .unwrap_or_default();
        copy_into(&mut nid.szTip, tip);
        let _ = Shell_NotifyIconW(NIM_ADD, &nid);
        nid.Anonymous.uVersion = NOTIFYICON_VERSION_4;
        let _ = Shell_NotifyIconW(NIM_SETVERSION, &nid);
    }
}

/// Run the Win32 message loop until Quit.
pub fn message_loop() {
    let mut msg = MSG::default();
    // SAFETY: standard message loop on the thread that owns the window.
    unsafe {
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_withdrawn_pairing_prompt_is_closed_once() {
        let flag = sas_open(9_000_001);
        assert!(flag.load(std::sync::atomic::Ordering::SeqCst));
        assert!(sas_withdraw(9_000_001));
        assert!(!flag.load(std::sync::atomic::Ordering::SeqCst));
        assert!(!sas_withdraw(9_000_001), "second withdrawal finds nothing");
        // An answered prompt is not withdrawn later.
        let flag = sas_open(9_000_002);
        sas_closed(9_000_002);
        assert!(!sas_withdraw(9_000_002));
        assert!(flag.load(std::sync::atomic::Ordering::SeqCst));
    }

    #[test]
    fn a_failed_settings_save_is_announced_once_per_streak() {
        let f = std::sync::atomic::AtomicBool::new(false);
        assert!(!tell_save_failure(true, &f));
        assert!(tell_save_failure(false, &f));
        assert!(!tell_save_failure(false, &f));
        assert!(!tell_save_failure(true, &f));
        assert!(tell_save_failure(false, &f));
    }

    #[test]
    fn timeline_timer_runs_only_while_needed() {
        assert!(
            !timeline_timer_needed(false, false, false),
            "idle: no wake-ups"
        );
        assert!(timeline_timer_needed(true, false, false), "a write is owed");
        assert!(
            timeline_timer_needed(false, true, false),
            "the main window refreshes"
        );
        assert!(
            timeline_timer_needed(false, false, true),
            "the flyout refreshes"
        );
    }

    #[test]
    fn show_in_folder_quotes_the_path_and_refuses_a_quote() {
        assert_eq!(
            select_arg(r"C:\Users\Me\Downloads\a b,c.pdf").as_deref(),
            Some(r#"/select,"C:\Users\Me\Downloads\a b,c.pdf""#)
        );
        // A quote would end the quoting early: never passed on.
        assert_eq!(select_arg(r#"C:\x" /root,C:\Windows"#), None);
    }

    #[test]
    fn peer_checks_run_at_most_every_30_seconds() {
        let t0 = std::time::Instant::now();
        let s = std::time::Duration::from_secs;
        assert!(peer_check_due(None, t0));
        assert!(!peer_check_due(Some(t0), t0 + s(29)));
        assert!(peer_check_due(Some(t0), t0 + s(30)));
        // A last check "in the future" is not due yet.
        assert!(!peer_check_due(Some(t0 + s(5)), t0));
    }

    #[test]
    fn a_failed_open_names_the_file() {
        assert_eq!(
            failed_text(r"C:\Users\Me\Downloads\report.pdf", true),
            "Couldn't open report.pdf"
        );
        assert_eq!(
            failed_text(r"C:\Users\Me\Downloads\report.pdf", false),
            "Couldn't show report.pdf in its folder"
        );
    }

    #[test]
    fn local_offset_is_a_real_time_zone() {
        let tz = local_offset_s();
        assert!((-14 * 3600..=14 * 3600).contains(&tz), "{tz}");
        assert_eq!(tz % 900, 0, "zones are whole quarter hours: {tz}");
    }
}
