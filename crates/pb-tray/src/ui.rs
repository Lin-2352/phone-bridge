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
        Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM},
        System::LibraryLoader::GetModuleHandleW,
        UI::{
            Shell::{
                Shell_NotifyIconW, NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_TIP, NIIF_INFO, NIM_ADD,
                NIM_DELETE, NIM_MODIFY, NOTIFYICONDATAW,
            },
            WindowsAndMessaging::{
                AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu,
                DispatchMessageW, GetCursorPos, GetMessageW, LoadIconW, MessageBoxW, PostMessageW,
                PostQuitMessage, RegisterClassW, SetForegroundWindow, TrackPopupMenu,
                TranslateMessage, HWND_MESSAGE, IDI_APPLICATION, IDYES, MB_ICONQUESTION, MB_OK,
                MB_SETFOREGROUND, MB_YESNO, MF_CHECKED, MF_GRAYED, MF_POPUP, MF_SEPARATOR,
                MF_STRING, MSG, TPM_BOTTOMALIGN, TPM_RIGHTBUTTON, WINDOW_EX_STYLE, WINDOW_STYLE,
                WM_APP, WM_COMMAND, WM_CONTEXTMENU, WM_DESTROY, WM_LBUTTONUP, WM_RBUTTONUP,
                WNDCLASSW,
            },
        },
    },
};

use crate::core::{Cmd, Peers, UiEvent};

/// Window class name; test tooling finds the window by it (message-only window).
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
const WM_CLIPBOARDUPDATE: u32 = 0x031D;
const NIN_BALLOONUSERCLICK: u32 = 0x0405;
const TIMER_CLIP: usize = 7;

/// Clipboard sequence number right after our own write, so the listener
/// does not send back what we just received.
static OWN_SEQ: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
/// Last received file, opened when the user clicks the notification.
static LAST_FILE: Mutex<Option<std::path::PathBuf>> = Mutex::new(None);

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
    match e {
        UiEvent::Ready { fingerprint } => {
            set_tip(hwnd, &format!("Phone Bridge ({})", &fingerprint[..9]))
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
        UiEvent::RemoteClipboard { from, text } => {
            let keep = st
                .settings
                .lock()
                .map(|s| s.keep_in_history)
                .unwrap_or(false);
            match set_clipboard_own(hwnd, &text, !keep) {
                Ok(()) => balloon(hwnd, &format!("Copied on {from}"), "Ready to paste"),
                Err(e) => balloon(
                    hwnd,
                    "Phone Bridge",
                    &format!("Could not set clipboard: {e}"),
                ),
            }
        }
        UiEvent::FileReceived { from, path, size } => {
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            if let Ok(mut l) = LAST_FILE.lock() {
                *l = Some(path.clone());
            }
            let mb = size as f64 / 1_048_576.0;
            note_activity(&format!("Last: {name} received"));
            balloon(
                hwnd,
                &format!("{from} sent {name}"),
                &format!("{mb:.1} MB, saved to Downloads\\Phone Bridge. Click to show."),
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

fn show_menu(hwnd: HWND, st: &State) {
    // SAFETY: menu handles are created, used and destroyed on this thread.
    unsafe {
        let Ok(menu) = CreatePopupMenu() else { return };
        let peers = st.peers.lock().map(|g| g.clone()).unwrap_or_default();
        let _ = AppendMenuW(menu, MF_STRING, ID_PAIR, w!("Pair new device…"));
        let send_label = match peers.first() {
            Some((_, name)) => wide(&format!("Send clipboard to {name}")),
            None => wide("Send clipboard (no paired device)"),
        };
        let flags = if peers.is_empty() {
            MF_STRING | MF_GRAYED
        } else {
            MF_STRING
        };
        let _ = AppendMenuW(menu, flags, ID_SEND_CLIPBOARD, PCWSTR(send_label.as_ptr()));
        let _ = AppendMenuW(menu, flags, ID_SEND_FILES, w!("Send files\u{2026}"));
        let _ = AppendMenuW(menu, MF_STRING, ID_OPEN_FOLDER, w!("Open received files"));
        let sendto_on = sendto_link().is_some_and(|l| l.exists());
        let _ = AppendMenuW(
            menu,
            if sendto_on {
                MF_STRING | MF_CHECKED
            } else {
                MF_STRING
            },
            ID_SENDTO,
            w!("Show in Explorer \u{201c}Send to\u{201d} menu"),
        );
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
        let (clip_on, auto_on) = st
            .settings
            .lock()
            .map(|s| (s.universal_clipboard, s.auto_accept))
            .unwrap_or((false, true));
        let check = |on: bool| {
            if on {
                MF_STRING | MF_CHECKED
            } else {
                MF_STRING
            }
        };
        let _ = AppendMenuW(
            menu,
            check(clip_on),
            ID_TOGGLE_CLIP,
            w!("Universal Clipboard"),
        );
        let _ = AppendMenuW(
            menu,
            check(auto_on),
            ID_TOGGLE_AUTO,
            w!("Accept files automatically"),
        );
        if let Ok(sub) = CreatePopupMenu() {
            let labels: Vec<Vec<u16>> = peers
                .iter()
                .map(|(id, name)| wide(&format!("Remove {name} ({})", &id.fingerprint()[..9])))
                .collect();
            for (i, l) in labels.iter().enumerate() {
                let _ = AppendMenuW(sub, MF_STRING, ID_UNPAIR_BASE + i, PCWSTR(l.as_ptr()));
            }
            let f = if peers.is_empty() {
                MF_POPUP | MF_GRAYED
            } else {
                MF_POPUP
            };
            let _ = AppendMenuW(menu, f, sub.0 as usize, w!("Paired devices"));
        }
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
        let _ = AppendMenuW(menu, MF_STRING, ID_QUIT, w!("Quit"));
        let mut pt = POINT::default();
        let _ = GetCursorPos(&mut pt);
        // Required so the menu closes when the user clicks elsewhere.
        let _ = SetForegroundWindow(hwnd);
        let _ = TrackPopupMenu(
            menu,
            TPM_RIGHTBUTTON | TPM_BOTTOMALIGN,
            pt.x,
            pt.y,
            None,
            hwnd,
            None,
        );
        let _ = DestroyMenu(menu);
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
            if let Ok(s) = st.settings.lock() {
                let _ = std::fs::create_dir_all(&s.save_dir);
                let _ = std::process::Command::new("explorer")
                    .arg(&s.save_dir)
                    .spawn();
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
                "Universal Clipboard",
                if on {
                    "On: what you copy is shared with your devices"
                } else {
                    "Off"
                },
            );
        }
        ID_TOGGLE_AUTO => {
            let _ = st
                .settings
                .lock()
                .map(|mut s| s.auto_accept = !s.auto_accept);
            save_settings(st);
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
    match msg {
        WM_TRAY => {
            let ev = (lparam.0 & 0xffff) as u32;
            if ev == WM_LBUTTONUP {
                open_flyout(hwnd, st);
            } else if ev == WM_RBUTTONUP || ev == WM_CONTEXTMENU {
                show_menu(hwnd, st);
            } else if ev == NIN_BALLOONUSERCLICK {
                if let Some(p) = LAST_FILE.lock().ok().and_then(|g| g.clone()) {
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
                if let Ok(ClipText::Text(t)) = clipboard::get_text(Some(hwnd)) {
                    let _ = st.cmds.send(Cmd::ClipboardChanged(t));
                }
            }
            LRESULT(0)
        }
        WM_COMMAND => {
            command(hwnd, st, wparam.0 & 0xffff);
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
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            CLASS,
            w!("Phone Bridge"),
            WINDOW_STYLE::default(),
            0,
            0,
            0,
            0,
            Some(HWND_MESSAGE),
            None,
            Some(hinstance.into()),
            None,
        )?;
        let mut nid = icon_data(hwnd);
        nid.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP;
        nid.uCallbackMessage = WM_TRAY;
        // Resource 1 is the app icon embedded by build.rs.
        nid.hIcon = LoadIconW(
            Some(hinstance.into()),
            PCWSTR(std::ptr::without_provenance::<u16>(1)), /* MAKEINTRESOURCE(1) */
        )
        .or_else(|_| LoadIconW(None, IDI_APPLICATION))?;
        copy_into(&mut nid.szTip, "Phone Bridge (starting)");
        let _ = Shell_NotifyIconW(NIM_ADD, &nid);
        // Universal Clipboard: get WM_CLIPBOARDUPDATE for every clipboard change.
        let _ = windows::Win32::System::DataExchange::AddClipboardFormatListener(hwnd);
        Ok(hwnd.0 as isize)
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
