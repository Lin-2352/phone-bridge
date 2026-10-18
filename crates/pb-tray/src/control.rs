//! "Control phone": while active, this PC's mouse and keyboard drive the
//! phone. Low-level hooks capture input, suppress it locally and stream it
//! (the cursor stays parked). Printable keys are translated with the PC's own
//! keyboard layout and sent as text; every key also goes as a scan code so
//! the phone can act on shortcuts (Ctrl+C, Esc, arrows). Ctrl+Alt+P returns.
//! Hooks exist only during a session, so they cost nothing otherwise.
#![allow(unsafe_code)]

use std::sync::Mutex;

use pb_protocol::InputEvent;
use pb_transport::input::InputSender;
use windows::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM},
    System::LibraryLoader::GetModuleHandleW,
    UI::{
        Input::KeyboardAndMouse::{GetKeyboardLayout, ToUnicodeEx},
        WindowsAndMessaging::{
            CallNextHookEx, GetCursorPos, PostMessageW, SetWindowsHookExW, UnhookWindowsHookEx,
            HHOOK, KBDLLHOOKSTRUCT, LLKHF_EXTENDED, LLKHF_INJECTED, LLKHF_UP, LLMHF_INJECTED,
            MSLLHOOKSTRUCT, WH_KEYBOARD_LL, WH_MOUSE_LL, WM_LBUTTONDOWN, WM_LBUTTONUP,
            WM_MBUTTONDOWN, WM_MBUTTONUP, WM_MOUSEHWHEEL, WM_MOUSEMOVE, WM_MOUSEWHEEL,
            WM_RBUTTONDOWN, WM_RBUTTONUP, WM_XBUTTONDOWN, WM_XBUTTONUP,
        },
    },
};

/// Posted to the tray window when the user asks to return (Ctrl+Alt+P).
pub const WM_CONTROL_EXIT: u32 = windows::Win32::UI::WindowsAndMessaging::WM_APP + 6;

struct Active {
    sender: InputSender,
    anchor: POINT,
    mouse: isize,
    keyboard: isize,
    owner: isize,
    /// Our own view of key state (the real one never sees suppressed keys).
    keys: [u8; 256],
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
    // SAFETY: hooks installed from the UI thread, which runs a message loop;
    // both are removed in `stop`.
    unsafe {
        let Ok(hinst) = GetModuleHandleW(None) else {
            return false;
        };
        let mut anchor = POINT::default();
        let _ = GetCursorPos(&mut anchor);
        let Ok(mouse) = SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_proc), Some(hinst.into()), 0)
        else {
            return false;
        };
        let Ok(keyboard) =
            SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_proc), Some(hinst.into()), 0)
        else {
            let _ = UnhookWindowsHookEx(mouse);
            return false;
        };
        if let Ok(mut g) = ACTIVE.lock() {
            *g = Some(Active {
                sender,
                anchor,
                mouse: mouse.0 as isize,
                keyboard: keyboard.0 as isize,
                owner: owner.0 as isize,
                keys: [0; 256],
            });
        }
    }
    true
}

pub fn stop() {
    let old = ACTIVE.lock().ok().and_then(|mut g| g.take());
    if let Some(a) = old {
        // Release modifiers the phone may think are held.
        for sc in [0x1Du16, 0x38, 0x2A, 0x36, 0xE01D, 0xE038] {
            a.sender.send(InputEvent::Key {
                scancode: sc,
                pressed: false,
            });
        }
        a.sender.stop();
        // SAFETY: hooks we installed in `start`.
        unsafe {
            let _ = UnhookWindowsHookEx(HHOOK(a.mouse as *mut _));
            let _ = UnhookWindowsHookEx(HHOOK(a.keyboard as *mut _));
        }
    }
}

fn send(e: InputEvent) {
    if let Ok(g) = ACTIVE.lock() {
        if let Some(a) = g.as_ref() {
            a.sender.send(e);
        }
    }
}

extern "system" fn mouse_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    // SAFETY: for WH_MOUSE_LL with code >= 0, lparam points to MSLLHOOKSTRUCT.
    let info = unsafe { &*(lparam.0 as *const MSLLHOOKSTRUCT) };
    if code < 0 || info.flags & LLMHF_INJECTED != 0 {
        // SAFETY: pass along what we do not handle.
        return unsafe { CallNextHookEx(None, code, wparam, lparam) };
    }
    let hi = (info.mouseData >> 16) as u16 as i16;
    let ev = match wparam.0 as u32 {
        WM_MOUSEMOVE => {
            let anchor = ACTIVE
                .lock()
                .ok()
                .and_then(|g| g.as_ref().map(|a| a.anchor));
            anchor.map(|p| InputEvent::MouseMove {
                dx: (info.pt.x - p.x).clamp(-32768, 32767) as i16,
                dy: (info.pt.y - p.y).clamp(-32768, 32767) as i16,
            })
        }
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
        send(e);
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
    // SAFETY: for WH_KEYBOARD_LL with code >= 0, lparam points to KBDLLHOOKSTRUCT.
    let k = unsafe { &*(lparam.0 as *const KBDLLHOOKSTRUCT) };
    if code < 0 || k.flags.0 & LLKHF_INJECTED.0 != 0 {
        // SAFETY: pass along what we do not handle.
        return unsafe { CallNextHookEx(None, code, wparam, lparam) };
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
    let mut text = Vec::new();
    if let Ok(mut g) = ACTIVE.lock() {
        if let Some(a) = g.as_mut() {
            // Track state, folding left/right modifiers into the generic ones.
            let generic = match vk {
                0xA0 | 0xA1 => Some(VK_SHIFT),
                0xA2 | 0xA3 => Some(VK_CONTROL),
                0xA4 | 0xA5 => Some(VK_MENU),
                _ => None,
            };
            let set = |keys: &mut [u8; 256], i: usize| keys[i] = if up { 0 } else { 0x80 };
            set(&mut a.keys, vk);
            if let Some(gv) = generic {
                set(&mut a.keys, gv);
            }
            if vk == VK_CAPITAL && !up {
                a.keys[VK_CAPITAL] ^= 1;
            }
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
                        text.extend(buf[..n as usize].iter().copied().filter(|&u| u >= 0x20));
                    }
                }
                for unit in text.drain(..) {
                    a.sender.send(InputEvent::Char { unit });
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
