//! Inject remote pointer and keyboard events with `SendInput`.
//!
//! Relative moves go through the user's own pointer speed and acceleration,
//! so a phone touchpad feels like a laptop's. Keys use scan codes (layout
//! independent); typed text uses Unicode packets. Whatever is still held when
//! the session ends is released, so a dropped link never leaves a key down.
#![allow(unsafe_code)]

use std::collections::BTreeSet;

use pb_protocol::InputEvent;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, KEYBDINPUT, KEYBD_EVENT_FLAGS,
    KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP, KEYEVENTF_SCANCODE, KEYEVENTF_UNICODE,
    MOUSEEVENTF_HWHEEL, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, MOUSEEVENTF_MIDDLEDOWN,
    MOUSEEVENTF_MIDDLEUP, MOUSEEVENTF_MOVE, MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP,
    MOUSEEVENTF_WHEEL, MOUSEEVENTF_XDOWN, MOUSEEVENTF_XUP, MOUSEINPUT, MOUSE_EVENT_FLAGS,
    VIRTUAL_KEY,
};

/// One input session's injector.
#[derive(Default)]
pub struct Injector {
    buttons: BTreeSet<u8>,
    keys: BTreeSet<u16>,
}

fn mouse(flags: MOUSE_EVENT_FLAGS, dx: i32, dy: i32, data: i32) -> INPUT {
    INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dx,
                dy,
                mouseData: data as u32,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

fn key(scan: u16, flags: KEYBD_EVENT_FLAGS) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(0),
                wScan: scan,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

fn send(inputs: &[INPUT]) {
    // SAFETY: a slice of fully initialised INPUT structs and their size.
    unsafe { SendInput(inputs, std::mem::size_of::<INPUT>() as i32) };
}

fn button_flags(button: u8, pressed: bool) -> Option<(MOUSE_EVENT_FLAGS, i32)> {
    Some(match (button, pressed) {
        (0, true) => (MOUSEEVENTF_LEFTDOWN, 0),
        (0, false) => (MOUSEEVENTF_LEFTUP, 0),
        (1, true) => (MOUSEEVENTF_RIGHTDOWN, 0),
        (1, false) => (MOUSEEVENTF_RIGHTUP, 0),
        (2, true) => (MOUSEEVENTF_MIDDLEDOWN, 0),
        (2, false) => (MOUSEEVENTF_MIDDLEUP, 0),
        (3 | 4, true) => (MOUSEEVENTF_XDOWN, i32::from(button - 2)),
        (3 | 4, false) => (MOUSEEVENTF_XUP, i32::from(button - 2)),
        _ => return None,
    })
}

fn key_input(scancode: u16, pressed: bool) -> INPUT {
    let mut flags = KEYEVENTF_SCANCODE;
    if scancode >> 8 == 0xE0 {
        flags |= KEYEVENTF_EXTENDEDKEY;
    }
    if !pressed {
        flags |= KEYEVENTF_KEYUP;
    }
    key(scancode & 0xff, flags)
}

impl Injector {
    pub fn inject(&mut self, ev: InputEvent) {
        match ev {
            InputEvent::MouseMove { dx, dy } => {
                send(&[mouse(MOUSEEVENTF_MOVE, dx.into(), dy.into(), 0)]);
            }
            InputEvent::MouseButton { button, pressed } => {
                if let Some((f, data)) = button_flags(button, pressed) {
                    if pressed {
                        self.buttons.insert(button);
                    } else {
                        self.buttons.remove(&button);
                    }
                    send(&[mouse(f, 0, 0, data)]);
                }
            }
            InputEvent::Wheel { dx, dy } => {
                if dy != 0 {
                    send(&[mouse(MOUSEEVENTF_WHEEL, 0, 0, dy.into())]);
                }
                if dx != 0 {
                    send(&[mouse(MOUSEEVENTF_HWHEEL, 0, 0, dx.into())]);
                }
            }
            InputEvent::Key { scancode, pressed } => {
                if pressed {
                    self.keys.insert(scancode);
                } else {
                    self.keys.remove(&scancode);
                }
                send(&[key_input(scancode, pressed)]);
            }
            InputEvent::Char { unit } => {
                send(&[
                    key(unit, KEYEVENTF_UNICODE),
                    key(unit, KEYEVENTF_UNICODE | KEYEVENTF_KEYUP),
                ]);
            }
        }
    }
}

impl Drop for Injector {
    fn drop(&mut self) {
        for k in std::mem::take(&mut self.keys) {
            send(&[key_input(k, false)]);
        }
        for b in std::mem::take(&mut self.buttons) {
            if let Some((f, data)) = button_flags(b, false) {
                send(&[mouse(f, 0, 0, data)]);
            }
        }
    }
}
