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

fn balloon(hwnd: HWND, title: &str, text: &str) {
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
    if !matches!(e, UiEvent::FileReceived { .. } | UiEvent::Link { .. }) {
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
                balloon(
                    hwnd,
                    &format!("Controlling {name}"),
                    "Your mouse and keyboard now drive the phone. Press Ctrl+Alt+P to return.",
                );
            } else {
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
            balloon(hwnd, "Phone Bridge", &msg)
        }
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
    balloon(hwnd, "Phone Bridge", why);
}

fn show_menu(hwnd: HWND) {
    use crate::fluent::glyph;
    use crate::menu::Item;
    crate::menu::show(
        hwnd,
        vec![
            Item::new(glyph::PHONE, "Open Phone Bridge", ID_OPEN_FLYOUT),
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
