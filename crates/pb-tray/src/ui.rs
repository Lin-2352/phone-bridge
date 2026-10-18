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
                MB_SETFOREGROUND, MB_YESNO, MF_GRAYED, MF_POPUP, MF_SEPARATOR, MF_STRING, MSG,
                TPM_BOTTOMALIGN, TPM_RIGHTBUTTON, WINDOW_EX_STYLE, WINDOW_STYLE, WM_APP,
                WM_COMMAND, WM_CONTEXTMENU, WM_DESTROY, WM_LBUTTONUP, WM_RBUTTONUP, WNDCLASSW,
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
        } => match clipboard::set_text(&text, sensitive, Some(hwnd)) {
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
        UiEvent::Info(msg) => balloon(hwnd, "Phone Bridge", &msg),
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
            if ev == WM_RBUTTONUP || ev == WM_CONTEXTMENU || ev == WM_LBUTTONUP {
                show_menu(hwnd, st);
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
        nid.hIcon = LoadIconW(None, IDI_APPLICATION)?;
        copy_into(&mut nid.szTip, "Phone Bridge (starting)");
        let _ = Shell_NotifyIconW(NIM_ADD, &nid);
        Ok(hwnd.0 as isize)
    }
}

/// Send a menu command to an already running tray (second-instance path).
pub fn forward_command(id: usize) -> bool {
    use windows::Win32::UI::WindowsAndMessaging::FindWindowExW;
    // SAFETY: lookup of a message-only window by our class, then a post.
    unsafe {
        match FindWindowExW(Some(HWND_MESSAGE), None, CLASS, PCWSTR::null()) {
            Ok(h) if !h.is_invalid() => {
                PostMessageW(Some(h), WM_COMMAND, WPARAM(id), LPARAM(0)).is_ok()
            }
            _ => false,
        }
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
