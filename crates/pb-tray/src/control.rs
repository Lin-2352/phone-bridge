//! "Control phone": while active, this PC's mouse and keyboard drive the
//! phone. Low-level hooks capture input, suppress it locally and stream it
//! (the cursor stays parked). Printable keys are translated with the PC's own
//! keyboard layout and sent as text; every key also goes as a scan code so
//! the phone can act on shortcuts (Ctrl+C, Esc, arrows). Ctrl+Alt+P returns.
//! The cursor is parked mid-screen for the session (a move is measured from
//! there, so a cursor left at a screen edge would lose movement that way)
//! and put back afterwards.
//!
//! The hooks live on their own thread with its own message loop, created for
//! a session and ended with it. Windows calls low-level hooks on the thread
//! that installed them; if that thread is busy (clipboard, disk), every key
//! and mouse move on the PC waits, so it must do nothing else.
#![allow(unsafe_code)]

use std::sync::Mutex;

use pb_protocol::InputEvent;
use pb_transport::input::InputSender;
use windows::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM},
    System::{LibraryLoader::GetModuleHandleW, Threading::GetCurrentThreadId},
    UI::{
        Input::KeyboardAndMouse::{GetKeyState, GetKeyboardLayout, ToUnicodeEx},
        WindowsAndMessaging::{
            CallNextHookEx, GetCursorPos, GetMessageW, GetSystemMetrics, PostMessageW,
            PostThreadMessageW, SetCursorPos, SetWindowsHookExW, UnhookWindowsHookEx,
            KBDLLHOOKSTRUCT, LLKHF_EXTENDED, LLKHF_INJECTED, LLKHF_UP, LLMHF_INJECTED, MSG,
            MSLLHOOKSTRUCT, SM_CXSCREEN, SM_CYSCREEN, WH_KEYBOARD_LL, WH_MOUSE_LL, WM_LBUTTONDOWN,
            WM_LBUTTONUP, WM_MBUTTONDOWN, WM_MBUTTONUP, WM_MOUSEHWHEEL, WM_MOUSEMOVE,
            WM_MOUSEWHEEL, WM_QUIT, WM_RBUTTONDOWN, WM_RBUTTONUP, WM_XBUTTONDOWN, WM_XBUTTONUP,
        },
    },
};

/// Posted to the tray window when the user asks to return (Ctrl+Alt+P).
pub const WM_CONTROL_EXIT: u32 = windows::Win32::UI::WindowsAndMessaging::WM_APP + 6;

struct Active {
    sender: InputSender,
    anchor: POINT,
    /// Where the cursor was before the session (restored at the end).
    home: POINT,
    owner: isize,
    /// Thread running the hooks; WM_QUIT to it ends the session.
    hook_thread: u32,
    /// Our own view of key state (the real one never sees suppressed keys).
    keys: [u8; 256],
    caps_on: bool,
}

static ACTIVE: Mutex<Option<Active>> = Mutex::new(None);

pub fn is_active() -> bool {
    ACTIVE.lock().map(|g| g.is_some()).unwrap_or(false)
}

/// The session ended on the other side (or the link dropped).
pub fn session_closed() -> bool {
    ACTIVE
        .lock()
        .ok()
        .and_then(|g| g.as_ref().map(|a| !a.sender.is_open()))
        .unwrap_or(false)
}

pub fn start(owner: HWND, sender: InputSender) -> bool {
    stop();
    let mut home = POINT::default();
    // SAFETY: plain queries, and moving the cursor before the hooks exist.
    let (anchor, caps_on) = unsafe {
        let _ = GetCursorPos(&mut home);
        let mid = POINT {
            x: GetSystemMetrics(SM_CXSCREEN) / 2,
            y: GetSystemMetrics(SM_CYSCREEN) / 2,
        };
        let anchor = if SetCursorPos(mid.x, mid.y).is_ok() {
            mid
        } else {
            home
        };
        (anchor, GetKeyState(VK_CAPITAL as i32) & 1 != 0)
    };
    if let Ok(mut g) = ACTIVE.lock() {
        *g = Some(Active {
            sender,
            anchor,
            home,
            owner: owner.0 as isize,
            hook_thread: 0,
            keys: [0; 256],
            caps_on,
        });
    }
    let (tx, rx) = std::sync::mpsc::channel::<Option<u32>>();
    let spawned = std::thread::Builder::new()
        .name("pb-hooks".into())
        .spawn(move || {
            // SAFETY: hooks installed and removed on this thread, which runs
            // the message loop Windows needs to call them.
            unsafe {
                // The process runs under EcoQoS; this thread must not.
                let _ = pb_platform_win::power::make_thread_responsive();
                let Ok(hinst) = GetModuleHandleW(None) else {
                    let _ = tx.send(None);
                    return;
                };
                let Ok(mouse) =
                    SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_proc), Some(hinst.into()), 0)
                else {
                    let _ = tx.send(None);
                    return;
                };
                let Ok(keyboard) =
                    SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_proc), Some(hinst.into()), 0)
                else {
                    let _ = UnhookWindowsHookEx(mouse);
                    let _ = tx.send(None);
                    return;
                };
                let _ = tx.send(Some(GetCurrentThreadId()));
                let mut msg = MSG::default();
                while GetMessageW(&mut msg, None, 0, 0).as_bool() {}
                let _ = UnhookWindowsHookEx(keyboard);
                let _ = UnhookWindowsHookEx(mouse);
            }
        });
    let tid = spawned
        .ok()
        .and_then(|_| rx.recv_timeout(std::time::Duration::from_secs(2)).ok())
        .flatten();
    match tid {
        Some(t) => {
            if let Ok(mut g) = ACTIVE.lock() {
                if let Some(a) = g.as_mut() {
                    a.hook_thread = t;
                }
            }
            true
        }
        None => {
            if let Ok(mut g) = ACTIVE.lock() {
                *g = None;
            }
            // SAFETY: put the cursor back where the user left it.
            let _ = unsafe { SetCursorPos(home.x, home.y) };
            false
        }
    }
}

pub fn stop() {
    let old = ACTIVE.lock().ok().and_then(|mut g| g.take());
    if let Some(a) = old {
        // Release modifiers and buttons the phone may think are held.
        for sc in [0x1Du16, 0x38, 0x2A, 0x36, 0xE01D, 0xE038] {
            a.sender.send(InputEvent::Key {
                scancode: sc,
                pressed: false,
            });
        }
        for button in 0..3 {
            a.sender.send(InputEvent::MouseButton {
                button,
                pressed: false,
            });
        }
        a.sender.stop();
        if a.hook_thread != 0 {
            // SAFETY: ends the hook thread's message loop; it unhooks itself.
            let _ = unsafe { PostThreadMessageW(a.hook_thread, WM_QUIT, WPARAM(0), LPARAM(0)) };
        }
        // SAFETY: the cursor goes back where the user left it.
        let _ = unsafe { SetCursorPos(a.home.x, a.home.y) };
    }
}

fn pass(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    // SAFETY: hand the event to the next hook / the system unchanged.
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

extern "system" fn mouse_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code < 0 {
        return pass(code, wparam, lparam);
    }
    // SAFETY: for WH_MOUSE_LL with code >= 0, lparam points to MSLLHOOKSTRUCT.
    let info = unsafe { &*(lparam.0 as *const MSLLHOOKSTRUCT) };
    if info.flags & LLMHF_INJECTED != 0 {
        return pass(code, wparam, lparam);
    }
    let hi = (info.mouseData >> 16) as u16 as i16;
    let Ok(g) = ACTIVE.lock() else {
        return pass(code, wparam, lparam);
    };
    // No session (or it is ending): never swallow the user's mouse.
    let Some(a) = g.as_ref() else {
        return pass(code, wparam, lparam);
    };
    let ev = match wparam.0 as u32 {
        WM_MOUSEMOVE => Some(InputEvent::MouseMove {
            dx: (info.pt.x - a.anchor.x).clamp(-32768, 32767) as i16,
            dy: (info.pt.y - a.anchor.y).clamp(-32768, 32767) as i16,
        }),
        WM_LBUTTONDOWN => Some(InputEvent::MouseButton {
            button: 0,
            pressed: true,
        }),
        WM_LBUTTONUP => Some(InputEvent::MouseButton {
            button: 0,
            pressed: false,
        }),
        WM_RBUTTONDOWN => Some(InputEvent::MouseButton {
            button: 1,
            pressed: true,
        }),
        WM_RBUTTONUP => Some(InputEvent::MouseButton {
            button: 1,
            pressed: false,
        }),
        WM_MBUTTONDOWN => Some(InputEvent::MouseButton {
            button: 2,
            pressed: true,
        }),
        WM_MBUTTONUP => Some(InputEvent::MouseButton {
            button: 2,
            pressed: false,
        }),
        WM_XBUTTONDOWN | WM_XBUTTONUP => Some(InputEvent::MouseButton {
            button: if hi == 1 { 3 } else { 4 },
            pressed: wparam.0 as u32 == WM_XBUTTONDOWN,
        }),
        WM_MOUSEWHEEL => Some(InputEvent::Wheel { dx: 0, dy: hi }),
        WM_MOUSEHWHEEL => Some(InputEvent::Wheel { dx: hi, dy: 0 }),
        _ => None,
    };
    if let Some(e) = ev {
        a.sender.send(e);
    }
    // Suppressed locally: the PC cursor stays where it was.
    LRESULT(1)
}

const VK_SHIFT: usize = 0x10;
const VK_CONTROL: usize = 0x11;
const VK_MENU: usize = 0x12;
const VK_CAPITAL: usize = 0x14;
const VK_P: u32 = 0x50;

extern "system" fn keyboard_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code < 0 {
        return pass(code, wparam, lparam);
    }
    // SAFETY: for WH_KEYBOARD_LL with code >= 0, lparam points to KBDLLHOOKSTRUCT.
    let k = unsafe { &*(lparam.0 as *const KBDLLHOOKSTRUCT) };
    if k.flags.0 & LLKHF_INJECTED.0 != 0 {
        return pass(code, wparam, lparam);
    }
    let up = k.flags.0 & LLKHF_UP.0 != 0;
    let vk = k.vkCode as usize & 0xff;
    let scancode = (k.scanCode as u16 & 0xff)
        | if k.flags.0 & LLKHF_EXTENDED.0 != 0 {
            0xE000
        } else {
            0
        };
    let mut exit_to = None;
    {
        let Ok(mut g) = ACTIVE.lock() else {
            return pass(code, wparam, lparam);
        };
        let Some(a) = g.as_mut() else {
            return pass(code, wparam, lparam);
        };
        // A key that was already down when control started (Ctrl+Alt of the
        // hotkey): let its release reach the PC, or it stays stuck down here.
        if up && a.keys[vk] & 0x80 == 0 {
            return pass(code, wparam, lparam);
        }
        // Track state, folding left/right modifiers into the generic ones.
        let generic = match vk {
            0xA0 | 0xA1 => Some(VK_SHIFT),
            0xA2 | 0xA3 => Some(VK_CONTROL),
            0xA4 | 0xA5 => Some(VK_MENU),
            _ => None,
        };
        let state = if up { 0 } else { 0x80 };
        a.keys[vk] = state;
        if let Some(gv) = generic {
            a.keys[gv] = state;
        }
        if vk == VK_CAPITAL && !up {
            a.caps_on = !a.caps_on;
        }
        a.keys[VK_CAPITAL] = (a.keys[VK_CAPITAL] & 0x80) | u8::from(a.caps_on);
        let ctrl = a.keys[VK_CONTROL] & 0x80 != 0;
        let alt = a.keys[VK_MENU] & 0x80 != 0;
        if !up && ctrl && alt && k.vkCode == VK_P {
            exit_to = Some(a.owner);
        } else {
            a.sender.send(InputEvent::Key {
                scancode,
                pressed: !up,
            });
            // Characters only for plain or AltGr typing, not shortcuts.
            if !up && (!ctrl || alt) {
                let mut buf = [0u16; 8];
                // SAFETY: valid key state and buffer; flag 0x4 leaves the
                // keyboard's dead-key state untouched (Windows 10 1607+).
                let n = unsafe {
                    ToUnicodeEx(
                        k.vkCode,
                        k.scanCode,
                        &a.keys,
                        &mut buf,
                        0x4,
                        Some(GetKeyboardLayout(0)),
                    )
                };
                if n > 0 {
                    for &unit in buf[..n as usize].iter().filter(|&&u| u >= 0x20) {
                        a.sender.send(InputEvent::Char { unit });
                    }
                }
            }
        }
    }
    if let Some(owner) = exit_to {
        // SAFETY: posting to the tray's own window; handled on its thread.
        let _ = unsafe {
            PostMessageW(
                Some(HWND(owner as *mut _)),
                WM_CONTROL_EXIT,
                WPARAM(0),
                LPARAM(0),
            )
        };
    }
    LRESULT(1)
}
