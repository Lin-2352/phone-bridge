//! Search box: a native `EDIT` child (IME, screen readers, clipboard and undo
//! keep working) themed through `WM_CTLCOLOREDIT`, inside a frame that
//! Direct2D draws. The parent forwards `EN_CHANGE` as an ordinary `WM_COMMAND`.
#![allow(unsafe_code)]

use std::cell::Cell;

use windows::{
    core::{w, PCWSTR},
    Win32::{
        Foundation::{COLORREF, HWND, LPARAM, LRESULT, WPARAM},
        Graphics::Gdi::{
            CreateFontW, CreateSolidBrush, DeleteObject, InvalidateRect, SetBkColor, SetTextColor,
            FONT_QUALITY, HBRUSH, HDC, HFONT, HGDIOBJ,
        },
        UI::{
            Input::KeyboardAndMouse::{GetFocus, SetFocus, VK_ESCAPE, VK_RETURN, VK_TAB},
            Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass},
            WindowsAndMessaging::{
                CreateWindowExW, DestroyWindow, GetParent, GetWindowLongW, GetWindowTextLengthW,
                GetWindowTextW, MoveWindow, SendMessageW, SetWindowLongW, SetWindowTextW,
                ShowWindow, ES_AUTOHSCROLL, GWL_STYLE, HMENU, SW_HIDE, SW_SHOWNA, WINDOW_EX_STYLE,
                WINDOW_STYLE, WM_CHAR, WM_KEYDOWN, WM_NCDESTROY, WM_SETFONT, WS_CHILD,
                WS_CLIPCHILDREN, WS_TABSTOP, WS_VISIBLE,
            },
        },
    },
};

use crate::fluent::{self, icon, rectf, round_rect, Canvas, Palette, RectF};

/// Logical px: inset before the glyph, glyph area, right inset, edit height.
const LEFT_INSET: f32 = 12.0;
const GLYPH_AREA: f32 = 20.0;
const RIGHT_INSET: f32 = 8.0;
const EDIT_HEIGHT: f32 = 20.0;
/// Edit font height in logical px.
const FONT_PX: f32 = 14.0;

pub struct SearchBox {
    edit: HWND,
    brush: Cell<HBRUSH>,
    brush_rgb: Cell<u32>,
    font: Cell<HFONT>,
    font_scale: Cell<f32>,
}

fn make_font(s: f32) -> HFONT {
    // SAFETY: plain font creation; deleted when replaced and in `Drop`.
    unsafe {
        CreateFontW(
            -((FONT_PX * s).round() as i32),
            0,
            0,
            0,
            400,
            0,
            0,
            0,
            Default::default(),
            Default::default(),
            Default::default(),
            // CLEARTYPE_QUALITY
            FONT_QUALITY(5),
            0,
            w!("Segoe UI Variable Text"),
        )
    }
}

const SUBCLASS_ID: usize = 1;

/// The edit's keys outside a dialog: Tab and Shift+Tab go to the parent
/// (which moves its focus), Esc clears the text (`EN_CHANGE` follows) and
/// Enter does nothing; none of them beep.
unsafe extern "system" fn subclass(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    _data: usize,
) -> LRESULT {
    // SAFETY (whole body): messages on the edit this procedure subclasses.
    unsafe {
        match msg {
            WM_KEYDOWN if wparam.0 == VK_TAB.0 as usize => {
                if let Ok(parent) = GetParent(hwnd) {
                    SendMessageW(parent, WM_KEYDOWN, Some(wparam), Some(lparam));
                }
                LRESULT(0)
            }
            WM_KEYDOWN if wparam.0 == VK_ESCAPE.0 as usize => {
                if GetWindowTextLengthW(hwnd) > 0 {
                    let _ = SetWindowTextW(hwnd, w!(""));
                }
                LRESULT(0)
            }
            WM_KEYDOWN if wparam.0 == VK_RETURN.0 as usize => LRESULT(0),
            // Tab, Enter and Esc as characters: the edit would beep.
            WM_CHAR if matches!(wparam.0, 0x09 | 0x0d | 0x1b) => LRESULT(0),
            WM_NCDESTROY => {
                let _ = RemoveWindowSubclass(hwnd, Some(subclass), SUBCLASS_ID);
                DefSubclassProc(hwnd, msg, wparam, lparam)
            }
            _ => DefSubclassProc(hwnd, msg, wparam, lparam),
        }
    }
}

fn set_font(edit: HWND, font: HFONT) {
    // SAFETY: our edit; the font outlives its use (deleted in `place`/`Drop`).
    unsafe {
        SendMessageW(
            edit,
            WM_SETFONT,
            Some(WPARAM(font.0 as usize)),
            Some(LPARAM(1)),
        );
    }
}

impl SearchBox {
    /// Creates the `EDIT` under `parent` (which gets `WS_CLIPCHILDREN`, so
    /// the composed surface never paints over the control) with control id `id`.
    pub fn new(parent: HWND, id: u16) -> Result<Self, String> {
        // SAFETY: style change and child creation on a window of this thread.
        let edit = unsafe {
            let style = GetWindowLongW(parent, GWL_STYLE) as u32;
            if style & WS_CLIPCHILDREN.0 == 0 {
                SetWindowLongW(parent, GWL_STYLE, (style | WS_CLIPCHILDREN.0) as i32);
            }
            CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("EDIT"),
                PCWSTR::null(),
                WS_CHILD | WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE(ES_AUTOHSCROLL as u32),
                0,
                0,
                0,
                0,
                Some(parent),
                Some(HMENU(id as usize as *mut _)),
                None,
                None,
            )
        }
        .map_err(|e| format!("CreateWindowExW(EDIT): {e}"))?;
        // SAFETY: subclassing our own edit; removed in `subclass` on
        // WM_NCDESTROY.
        unsafe {
            let _ = SetWindowSubclass(edit, Some(subclass), SUBCLASS_ID, 0);
        }
        let font = make_font(1.0);
        set_font(edit, font);
        Ok(Self {
            edit,
            brush: Cell::new(HBRUSH::default()),
            brush_rgb: Cell::new(0),
            font: Cell::new(font),
            font_scale: Cell::new(1.0),
        })
    }

    /// Moves the edit inside `outer` (client pixels) at scale `s`.
    pub fn place(&self, outer: RectF, s: f32) {
        if (self.font_scale.get() - s).abs() > f32::EPSILON {
            let font = make_font(s);
            set_font(self.edit, font);
            // SAFETY: the control switched to the new font, so the old one is free.
            let _ = unsafe { DeleteObject(HGDIOBJ(self.font.get().0)) };
            self.font.set(font);
            self.font_scale.set(s);
        }
        let r = edit_rect(outer, s);
        // SAFETY: moves our own child window.
        let _ = unsafe {
            MoveWindow(
                self.edit,
                r.left.round() as i32,
                r.top.round() as i32,
                (r.right - r.left).round() as i32,
                (r.bottom - r.top).round() as i32,
                true,
            )
        };
    }

    pub fn text(&self) -> String {
        // SAFETY: reads our edit's text into a buffer of the reported length.
        unsafe {
            let n = GetWindowTextLengthW(self.edit).max(0) as usize;
            let mut buf = vec![0u16; n + 1];
            let got = GetWindowTextW(self.edit, &mut buf).max(0) as usize;
            String::from_utf16_lossy(&buf[..got])
        }
    }

    pub fn set_text(&self, t: &str) {
        let wide: Vec<u16> = t.encode_utf16().chain(std::iter::once(0)).collect();
        // SAFETY: `wide` is NUL-terminated and outlives the call.
        let _ = unsafe { SetWindowTextW(self.edit, PCWSTR(wide.as_ptr())) };
    }

    /// The grey hint shown while the box is empty (also what a screen reader
    /// reads as its purpose).
    pub fn set_cue(&self, t: &str) {
        let wide: Vec<u16> = t.encode_utf16().chain(std::iter::once(0)).collect();
        // SAFETY: EM_SETCUEBANNER (0x1501) copies the NUL-terminated text.
        unsafe {
            SendMessageW(
                self.edit,
                0x1501,
                Some(WPARAM(0)),
                Some(LPARAM(wide.as_ptr() as isize)),
            )
        };
    }

    pub fn focus(&self) {
        // SAFETY: focuses our own child.
        let _ = unsafe { SetFocus(Some(self.edit)) };
    }

    /// Typing (and pasting) stops at `units` UTF-16 units.
    pub fn limit(&self, units: usize) {
        // SAFETY: a plain message to our own edit.
        unsafe {
            SendMessageW(
                self.edit,
                windows::Win32::UI::Controls::EM_SETLIMITTEXT,
                Some(WPARAM(units)),
                None,
            );
        }
    }

    /// Shows the edit (without activating it) or hides it; a hidden edit
    /// hands the keyboard focus back to its parent.
    pub fn show(&self, on: bool) {
        // SAFETY: focus and visibility of our own child; may send focus
        // notifications to the parent, so the caller holds no borrow of it.
        unsafe {
            if !on && GetFocus() == self.edit {
                if let Ok(parent) = GetParent(self.edit) {
                    let _ = SetFocus(Some(parent));
                }
            }
            let _ = ShowWindow(self.edit, if on { SW_SHOWNA } else { SW_HIDE });
        }
    }

    /// Whether the edit has the keyboard focus.
    pub fn focused(&self) -> bool {
        // SAFETY: plain query.
        unsafe { GetFocus() == self.edit }
    }

    /// Repaints the edit (its colours come from `on_ctl_color`, so after a
    /// theme change).
    pub fn redraw(&self) {
        // SAFETY: repaint request on our own child.
        let _ = unsafe { InvalidateRect(Some(self.edit), None, true) };
    }

    /// The parent's answer to `WM_CTLCOLOREDIT`: text and background colours
    /// of the palette, and the brush that paints the edit's background
    /// (recreated when the palette's surface colour changes).
    pub fn on_ctl_color(&self, hdc: HDC, p: &Palette) -> LRESULT {
        // SAFETY: GDI calls on the DC the system handed to the parent; the
        // brush is ours and lives until it is replaced or dropped.
        unsafe {
            if self.brush.get().is_invalid() || self.brush_rgb.get() != p.card {
                if !self.brush.get().is_invalid() {
                    let _ = DeleteObject(HGDIOBJ(self.brush.get().0));
                }
                self.brush
                    .set(CreateSolidBrush(COLORREF(fluent::colorref(p.card))));
                self.brush_rgb.set(p.card);
            }
            SetTextColor(hdc, COLORREF(fluent::colorref(p.text)));
            SetBkColor(hdc, COLORREF(fluent::colorref(p.card)));
        }
        LRESULT(self.brush.get().0 as isize)
    }
}

impl Drop for SearchBox {
    fn drop(&mut self) {
        // SAFETY: handles we created; the edit goes first so nothing uses
        // the font or brush any more.
        unsafe {
            let _ = DestroyWindow(self.edit);
            let _ = DeleteObject(HGDIOBJ(self.font.get().0));
            if !self.brush.get().is_invalid() {
                let _ = DeleteObject(HGDIOBJ(self.brush.get().0));
            }
        }
    }
}

/// The edit's rectangle inside the frame `outer` (pixels) at scale `s`:
/// after the glyph area, 8 logical px from the right, 20 px high, centred.
pub fn edit_rect(outer: RectF, s: f32) -> RectF {
    let h = EDIT_HEIGHT * s;
    let top = outer.top + ((outer.bottom - outer.top) - h) / 2.0;
    rectf(
        outer.left + (LEFT_INSET + GLYPH_AREA) * s,
        top,
        outer.right - RIGHT_INSET * s,
        top + h,
    )
}

/// The frame around the edit: surface fill, a border (accent while focused)
/// and the search glyph. `c.s` is the scale.
pub fn draw_frame(c: &mut Canvas, outer: RectF, focused: bool) {
    let p = c.p;
    let border = if focused { p.primary } else { p.stroke };
    round_rect(
        c,
        &outer,
        Some((p.card, 1.0)),
        Some(border),
        c.px(4.0),
        false,
    );
    let left = outer.left + LEFT_INSET * c.s;
    let glyph = rectf(left, outer.top, left + GLYPH_AREA * c.s, outer.bottom);
    icon(c, fluent::glyph::SEARCH, glyph, p.text2);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fluent::rectf;

    #[test]
    fn edit_sits_inside_the_frame_after_the_glyph() {
        let r = edit_rect(rectf(0., 0., 300., 36.), 1.0);
        assert!(r.left >= 32.0 && r.right <= 292.0 && (r.top - 8.0).abs() < 0.5);
    }

    #[test]
    fn esc_clears_and_enter_and_tab_type_nothing() {
        let parent = crate::d2d::test_window();
        let sb = SearchBox::new(parent, 101).expect("search box");
        sb.set_text("cam");
        // SAFETY: messages to our own edit, as the keyboard would send them.
        unsafe {
            for ch in [0x0d, 0x09, 0x1b] {
                SendMessageW(sb.edit, WM_CHAR, Some(WPARAM(ch)), None);
            }
            SendMessageW(
                sb.edit,
                WM_KEYDOWN,
                Some(WPARAM(VK_RETURN.0 as usize)),
                None,
            );
        }
        assert_eq!(sb.text(), "cam");
        // SAFETY: as above.
        unsafe {
            SendMessageW(
                sb.edit,
                WM_KEYDOWN,
                Some(WPARAM(VK_ESCAPE.0 as usize)),
                None,
            );
        }
        assert_eq!(sb.text(), "");
        drop(sb);
        // SAFETY: our own test window.
        unsafe {
            let _ = windows::Win32::UI::WindowsAndMessaging::DestroyWindow(parent);
        }
    }

    #[test]
    fn typed_text_round_trips_and_parent_clips_children() {
        use windows::Win32::UI::WindowsAndMessaging::{
            DestroyWindow, GetWindowLongW, GWL_STYLE, WS_CLIPCHILDREN,
        };
        let parent = crate::d2d::test_window();
        let sb = SearchBox::new(parent, 101).expect("search box");
        sb.set_text("héllo 📄");
        assert_eq!(sb.text(), "héllo 📄");
        // SAFETY: our own test window.
        unsafe {
            let style = GetWindowLongW(parent, GWL_STYLE) as u32;
            assert!(style & WS_CLIPCHILDREN.0 != 0);
        }
        drop(sb);
        // SAFETY: our own test window.
        unsafe {
            let _ = DestroyWindow(parent);
        }
    }
}
