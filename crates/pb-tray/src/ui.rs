//! Win32 tray UI: notification-area icon, context menu, balloons and modal
//! prompts. All `unsafe` of the tray lives here.
#![allow(unsafe_code)]

use std::{
    collections::VecDeque,
    sync::{Mutex, OnceLock},
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
                MessageBoxW, PostMessageW, PostQuitMessage, RegisterClassW, TranslateMessage,
                IDI_APPLICATION, IDYES, MB_ICONQUESTION, MB_OK, MB_SETFOREGROUND, MB_YESNO, MSG,
                WM_APP, WM_COMMAND, WM_CONTEXTMENU, WM_DESTROY, WNDCLASSW,
            },
        },
    },
};

use crate::core::{Cmd, Peers, UiEvent};

/// Window class name; test tooling finds the window by it (hidden top-level window).
pub const CLASS: PCWSTR = w!("PhoneBridgeTray");
const WM_TRAY: u32 = WM_APP + 1;
pub const WM_CORE_EVENT: u32 = WM_APP + 2;

pub const ID_PAIR: usize = 1;
pub const ID_SEND_CLIPBOARD: usize = 2;
pub const ID_QUIT: usize = 3;
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
const ID_TOGGLE_CLIP: usize = 5;
const ID_TOGGLE_AUTO: usize = 6;
const ID_OPEN_FOLDER: usize = 7;
const ID_SENDTO: usize = 8;
const ID_TOGGLE_PRIVATE: usize = 9;
pub const ID_OPEN_FLYOUT: usize = 10;
pub const ID_SETTINGS: usize = 11;
pub const ID_MENU: usize = 12;
const ID_CONTROL: usize = 13;
const ID_SCREEN: usize = 14;
const TIMER_CONTROL: usize = 8;
const HOTKEY_CONTROL: i32 = 1;
const WM_SETTINGS_ACTION: u32 = WM_APP + 5;
static SETTINGS_ACTIONS: Mutex<VecDeque<crate::settings_window::Action>> =
    Mutex::new(VecDeque::new());
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

fn sendto_link() -> Option<std::path::PathBuf> {
    std::env::var_os("APPDATA").map(|a| {
        std::path::PathBuf::from(a)
            .join("Microsoft")
            .join("Windows")
            .join("SendTo")
            .join("Phone Bridge.lnk")
    })
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
    if link.exists() {
        std::fs::remove_file(&link).map_err(|e| e.to_string())?;
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
        sl.SetArguments(&HSTRING::from("--send"))
            .map_err(|e| e.to_string())?;
        sl.SetDescription(&HSTRING::from("Send to your phone with Phone Bridge"))
            .map_err(|e| e.to_string())?;
        sl.SetIconLocation(&HSTRING::from(exe.as_os_str()), 0)
            .map_err(|e| e.to_string())?;
        let pf: IPersistFile = sl.cast().map_err(|e| e.to_string())?;
        pf.Save(&HSTRING::from(link.as_os_str()), true)
            .map_err(|e| e.to_string())?;
    }
    Ok(true)
}

/// Last thing that happened, shown at the bottom of the flyout.
static LAST_ACTIVITY: Mutex<String> = Mutex::new(String::new());

fn note_activity(s: &str) {
    if let Ok(mut l) = LAST_ACTIVITY.lock() {
        *l = s.to_owned();
    }
}

fn open_flyout(hwnd: HWND, st: &State) {
    let model = crate::flyout::Model {
        peer: st
            .peers
            .lock()
            .ok()
            .and_then(|g| g.first().map(|(_, n)| n.clone())),
        clipboard_on: st
            .settings
            .lock()
            .map(|s| s.universal_clipboard)
            .unwrap_or(false),
        last: LAST_ACTIVITY.lock().map(|l| l.clone()).unwrap_or_default(),
    };
    let raw = hwnd.0 as isize;
    let cmds = st.cmds.clone();
    // Runs on the UI thread from the flyout's window procedure: only post
    // messages here, never call back into the flyout.
    crate::flyout::toggle(
        model,
        Box::new(move |a| match a {
            crate::flyout::Action::ToggleClipboard => post_command(raw, ID_TOGGLE_CLIP),
            crate::flyout::Action::SendFiles => post_command(raw, ID_SEND_FILES),
            crate::flyout::Action::Pair => post_command(raw, ID_PAIR),
            crate::flyout::Action::Settings => post_command(raw, ID_SETTINGS),
            crate::flyout::Action::ControlPhone => post_command(raw, ID_CONTROL),
            crate::flyout::Action::Dropped(files) => {
                let _ = cmds.send(Cmd::SendFiles(files));
            }
        }),
    );
}

fn save_settings(st: &State) {
    if let Ok(s) = st.settings.lock() {
        let _ = s.save(&st.settings_path);
    }
}

fn reveal(path: &std::path::Path) {
    let _ = std::process::Command::new("explorer")
        .arg(format!("/select,{}", path.display()))
        .spawn();
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
        let _ = dlg.SetTitle(w!("Send files with Phone Bridge"));
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

fn set_tip(hwnd: HWND, tip: &str) {
    let mut nid = icon_data(hwnd);
    nid.uFlags = NIF_TIP;
    copy_into(&mut nid.szTip, tip);
    // SAFETY: as in `balloon`.
    let _ = unsafe { Shell_NotifyIconW(NIM_MODIFY, &nid) };
}

/// Modal dialogs run on their own thread so the tray keeps working.
fn ask_yes_no(text: String, title: &'static str, done: impl FnOnce(bool) + Send + 'static) {
    std::thread::spawn(move || {
        let (t, c) = (wide(&text), wide(title));
        // SAFETY: both strings are NUL-terminated and outlive the call.
        let r = unsafe {
            MessageBoxW(
                None,
                PCWSTR(t.as_ptr()),
                PCWSTR(c.as_ptr()),
                MB_YESNO | MB_ICONQUESTION | MB_SETFOREGROUND,
            )
        };
        done(r == IDYES);
    });
}

fn show_info(text: String, title: &'static str) {
    std::thread::spawn(move || {
        let (t, c) = (wide(&text), wide(title));
        // SAFETY: as in `ask_yes_no`.
        unsafe {
            MessageBoxW(
                None,
                PCWSTR(t.as_ptr()),
                PCWSTR(c.as_ptr()),
                MB_OK | MB_SETFOREGROUND,
            )
        };
    });
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
            set_tip(hwnd, &format!("Phone Bridge ({})", &fingerprint[..9]));
            if let Ok(mut f) = MY_FINGERPRINT.lock() {
                *f = fingerprint[..9].to_owned();
            }
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
                show_info(
                    format!(
                        "Pairing code (valid 30 seconds):

{code}

({e})"
                    ),
                    "Pair a new device",
                );
            }
        }
        UiEvent::PairingClosed => {
            crate::qr_window::close();
            balloon(hwnd, "Phone Bridge", "Pairing window expired");
        }
        UiEvent::Sas { id, peer_name, sas } => {
            crate::qr_window::close();
            let cmds = st.cmds.clone();
            if st.auto_accept {
                st.log.line("SAS auto-accepted (test mode)");
                let _ = cmds.send(Cmd::Decide { id, accept: true });
                return;
            }
            ask_yes_no(
                format!(
                    "Pair with \"{peer_name}\"?\n\nDoes the phone show this code?\n\n        {sas}"
                ),
                "Phone Bridge pairing",
                move |yes| {
                    let _ = cmds.send(Cmd::Decide { id, accept: yes });
                },
            );
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
            Err(e) => balloon(
                hwnd,
                "Phone Bridge",
                &format!("Could not set clipboard: {e}"),
            ),
        },
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
                Err(e) => balloon(
                    hwnd,
                    "Phone Bridge",
                    &format!("Could not set clipboard: {e}"),
                ),
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
                    "Phone Bridge",
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
            let mb = size as f64 / 1_048_576.0;
            note_activity(&format!("Last: {name} received"));
            balloon(
                hwnd,
                &format!("{from} sent {name}"),
                &format!(
                    "{mb:.1} MB, saved to {}. Click to show.",
                    path.parent()
                        .map(|d| d.display().to_string())
                        .unwrap_or_default()
                ),
            );
        }
        UiEvent::AskControl { from, reply } => {
            ask_yes_no(
                format!("Allow {from} to control this PC's mouse and keyboard?\n\nYou can turn this off any time in Phone Bridge settings."),
                "Phone Bridge",
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
            reply,
        } => {
            let mb = size as f64 / 1_048_576.0;
            ask_yes_no(
                format!("{from} wants to send {name} ({mb:.1} MB).\n\nSave it to your Downloads\\Phone Bridge folder?"),
                "Phone Bridge",
                move |yes| {
                    let _ = reply.send(yes);
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
            balloon(hwnd, "Phone Bridge", &msg)
        }
        UiEvent::Notify {
            peer,
            n,
            logo,
            picture,
        } => show_phone_notification(hwnd, st, peer, *n, logo, picture, false),
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
        UiEvent::ScreenPinned(p) => {
            let _ = st.settings.lock().map(|mut s| s.screen_pinned = p);
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
            balloon(hwnd, "Phone Bridge", &text);
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

fn kill_timer(hwnd: HWND, id: usize) {
    // SAFETY: timer on our own window.
    let _ = unsafe { windows::Win32::UI::WindowsAndMessaging::KillTimer(Some(hwnd), id) };
}

/// Copy a one-time code, kept out of clipboard history; cleared after 60 s
/// if nothing else was copied meanwhile.
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

fn show_phone_notification(
    hwnd: HWND,
    st: &State,
    peer: pb_identity::PeerId,
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
    // Windows does not show it at all (turned off for Phone Bridge): do not
    // keep it in step, so its absence is never taken for a clear.
    let shown = crate::toast::enabled() && crate::toast::show(&spec).is_ok();
    if shown {
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
        show_phone_notification(hwnd, st, peer, *n, logo, picture, true);
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
                | ToastArgs::Mute { tag }
                | ToastArgs::Silence { tag } => tag.clone(),
            };
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
    crate::control::stop();
    // SAFETY: timer on our own window.
    let _ =
        unsafe { windows::Win32::UI::WindowsAndMessaging::KillTimer(Some(hwnd), TIMER_CONTROL) };
    crate::status::show(why, 4);
    balloon(hwnd, "Phone Bridge", why);
}

fn show_menu(hwnd: HWND) {
    use crate::fluent::glyph;
    use crate::menu::Item;
    crate::menu::show(
        hwnd,
        vec![
            Item::new(glyph::PHONE, "Open Phone Bridge", ID_OPEN_FLYOUT),
            Item::new(
                glyph::PHONE,
                if crate::screen_window::is_open() {
                    "Hide phone screen"
                } else {
                    "Show phone screen"
                },
                ID_SCREEN,
            ),
            Item::new(glyph::SEND, "Send files\u{2026}", ID_SEND_FILES),
            Item::new(glyph::FOLDER, "Open received files", ID_OPEN_FOLDER),
            Item::new(glyph::SETTINGS, "Settings", ID_SETTINGS),
            Item::separator(),
            Item::new(glyph::POWER, "Quit Phone Bridge", ID_QUIT),
        ],
    );
}

/// Snapshot of everything the settings window shows.
fn settings_model(st: &State) -> crate::settings_window::Model {
    let n = st.settings.lock().map(|s| s.clone()).unwrap_or_default();
    let (theme, share, passwords, keep, auto, save_dir, control) = st
        .settings
        .lock()
        .map(|s| {
            (
                s.theme,
                s.universal_clipboard,
                !s.skip_private,
                s.keep_in_history,
                s.auto_accept,
                s.save_dir.display().to_string(),
                s.allow_control,
            )
        })
        .unwrap_or_default();
    crate::settings_window::Model {
        theme,
        share,
        passwords,
        keep,
        auto,
        save_dir,
        sendto: sendto_link().is_some_and(|l| l.exists()),
        startup: startup_enabled(),
        control,
        devices: st
            .peers
            .lock()
            .map(|g| {
                g.iter()
                    .map(|(id, n)| (n.clone(), id.fingerprint()[..9].to_owned()))
                    .collect()
            })
            .unwrap_or_default(),
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
    }
}

fn open_settings(hwnd: HWND, st: &State) {
    let raw = hwnd.0 as isize;
    // Runs inside the settings window procedure: queue and post, never call
    // back into the window from here.
    crate::settings_window::open(
        settings_model(st),
        Box::new(move |a| {
            if let Ok(mut q) = SETTINGS_ACTIONS.lock() {
                q.push_back(a);
            }
            // SAFETY: posting to our own message window.
            let _ = unsafe {
                PostMessageW(
                    Some(HWND(raw as *mut _)),
                    WM_SETTINGS_ACTION,
                    WPARAM(0),
                    LPARAM(0),
                )
            };
        }),
    );
}

fn apply_settings_action(hwnd: HWND, st: &State, a: crate::settings_window::Action) {
    use crate::settings_window::{Action, Key};
    match a {
        Action::Theme(t) => {
            let _ = st.settings.lock().map(|mut s| s.theme = t);
            crate::fluent::set_theme(t);
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
                crate::settings_window::ChoiceKey::Content => s.notify_content = i.min(2),
                crate::settings_window::ChoiceKey::Sound => s.notify_sound = i.min(2),
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
        Action::Toggle(Key::Keep) => {
            let _ = st
                .settings
                .lock()
                .map(|mut s| s.keep_in_history = !s.keep_in_history);
            save_settings(st);
        }
        Action::Toggle(Key::Startup) => {
            if let Err(e) = set_startup(!startup_enabled()) {
                balloon(
                    hwnd,
                    "Phone Bridge",
                    &format!("Could not change startup: {e}"),
                );
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
                // SAFETY: posting to our own message window.
                let _ = unsafe {
                    PostMessageW(
                        Some(HWND(raw as *mut _)),
                        WM_SETTINGS_ACTION,
                        WPARAM(0),
                        LPARAM(0),
                    )
                };
            });
        }
        Action::OpenFolder => command(hwnd, st, ID_OPEN_FOLDER),
        Action::Remove(i) => command(hwnd, st, ID_UNPAIR_BASE + i),
        Action::Pair => command(hwnd, st, ID_PAIR),
    }
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
        let value = wide(&format!("\"{}\"", exe.display()));
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
                    "Phone Bridge",
                    "Not sent: the app that copied this marked it private",
                );
            }
            Ok(ClipText::Empty) => balloon(hwnd, "Phone Bridge", "Clipboard has no text"),
            Err(e) => balloon(
                hwnd,
                "Phone Bridge",
                &format!("Could not read clipboard: {e}"),
            ),
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
            Ok(true) => balloon(
                hwnd,
                "Phone Bridge",
                "Added: right-click files, Send to, Phone Bridge",
            ),
            Ok(false) => balloon(hwnd, "Phone Bridge", "Removed from the Send to menu"),
            Err(e) => balloon(
                hwnd,
                "Phone Bridge",
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
        ID_SETTINGS => open_settings(hwnd, st),
        ID_MENU => show_menu(hwnd),
        ID_CONTROL => toggle_control(hwnd, st),
        ID_SCREEN => {
            if crate::screen_window::is_open() {
                crate::screen_window::close();
            } else {
                let _ = st.cmds.send(Cmd::ShowScreen);
            }
        }
        ID_QUIT => {
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
                    format!("Remove \"{name}\"? It will no longer be able to connect."),
                    "Phone Bridge",
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
        let fp = MY_FINGERPRINT.lock().map(|f| f.clone()).unwrap_or_default();
        let tip = if fp.is_empty() {
            "Phone Bridge".to_owned()
        } else {
            format!("Phone Bridge ({fp})")
        };
        add_icon(hwnd, &tip);
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
            if crate::settings_window::is_open() {
                crate::settings_window::update(settings_model(st));
            }
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
            if crate::settings_window::is_open() {
                crate::settings_window::update(settings_model(st));
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
            if st
                .settings
                .lock()
                .map(|s| s.universal_clipboard)
                .unwrap_or(false)
            {
                // Private (password manager) content reads as Excluded and is skipped.
                // Passing it along is the user's choice ("Send passwords and codes").
                let skip = st.settings.lock().map(|s| s.skip_private).unwrap_or(true);
                if let Ok((ClipText::Text(text), sensitive)) =
                    clipboard::get_text_marked(Some(hwnd))
                {
                    if !(sensitive && skip) {
                        let _ = st.cmds.send(Cmd::ClipboardChanged { text, sensitive });
                    }
                }
            }
            LRESULT(0)
        }
        WM_COMMAND => {
            command(hwnd, st, wparam.0 & 0xffff);
            if crate::settings_window::is_open() {
                crate::settings_window::update(settings_model(st));
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
        WM_SETTINGS_ACTION => {
            let drained: Vec<_> = SETTINGS_ACTIONS
                .lock()
                .map(|mut q| q.drain(..).collect())
                .unwrap_or_default();
            for a in drained {
                apply_settings_action(hwnd, st, a);
            }
            crate::settings_window::update(settings_model(st));
            LRESULT(0)
        }
        WM_DESTROY => {
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
            w!("Phone Bridge"),
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
        add_icon(hwnd, "Phone Bridge (starting)");
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
