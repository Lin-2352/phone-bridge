//! Windows clipboard text with the privacy formats Windows defines:
//! - `ExcludeClipboardContentFromMonitorProcessing`: set by password managers;
//!   we never send such content, and set it ourselves for sensitive text.
//! - `CanIncludeInClipboardHistory` / `CanUploadToCloudClipboard` = 0 keeps
//!   sensitive text out of Win+V history and cloud sync.
#![allow(unsafe_code)]

use std::io;

use windows::{
    core::{w, PCWSTR},
    Win32::{
        Foundation::{GlobalFree, HANDLE, HGLOBAL, HWND},
        System::{
            DataExchange::{
                CloseClipboard, EmptyClipboard, GetClipboardData, IsClipboardFormatAvailable,
                OpenClipboard, RegisterClipboardFormatW, SetClipboardData,
            },
            Memory::{GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock, GMEM_MOVEABLE},
            Ole::CF_UNICODETEXT,
        },
    },
};

/// Largest clipboard text we read (characters); larger content is refused.
const MAX_CHARS: usize = 1 << 20;

/// What we found on the clipboard.
#[derive(Debug, PartialEq, Eq)]
pub enum ClipText {
    Text(String),
    /// Another app marked the content private; it must not be sent.
    Excluded,
    Empty,
}

fn format(name: PCWSTR) -> u32 {
    // SAFETY: `name` is a static NUL-terminated wide string.
    unsafe { RegisterClipboardFormatW(name) }
}

struct Open;

impl Open {
    fn new(owner: Option<HWND>) -> io::Result<Self> {
        // Another process may hold the clipboard briefly; retry a few times.
        for _ in 0..10 {
            // SAFETY: plain Win32 call; paired with CloseClipboard in Drop.
            if unsafe { OpenClipboard(owner) }.is_ok() {
                return Ok(Open);
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        Err(io::Error::other("clipboard is busy"))
    }
}

impl Drop for Open {
    fn drop(&mut self) {
        // SAFETY: we opened the clipboard in `new`.
        let _ = unsafe { CloseClipboard() };
    }
}

/// Copy bytes into a movable global block the clipboard can own.
fn global_bytes(bytes: &[u8]) -> io::Result<HGLOBAL> {
    // SAFETY: allocate, lock, copy exactly `bytes.len()` bytes, unlock.
    unsafe {
        let h = GlobalAlloc(GMEM_MOVEABLE, bytes.len().max(1)).map_err(io::Error::other)?;
        let p = GlobalLock(h) as *mut u8;
        if p.is_null() {
            let _ = GlobalFree(Some(h));
            return Err(io::Error::other("GlobalLock failed"));
        }
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), p, bytes.len());
        let _ = GlobalUnlock(h);
        Ok(h)
    }
}

fn set(fmt: u32, bytes: &[u8]) -> io::Result<()> {
    let h = global_bytes(bytes)?;
    // SAFETY: on success the clipboard owns `h`; on failure we free it.
    unsafe {
        if SetClipboardData(fmt, Some(HANDLE(h.0))).is_err() {
            let _ = GlobalFree(Some(h));
            return Err(io::Error::other("SetClipboardData failed"));
        }
    }
    Ok(())
}

/// Put text on the clipboard. Sensitive text is excluded from clipboard
/// history, cloud clipboard and clipboard monitors.
pub fn set_text(text: &str, sensitive: bool, owner: Option<HWND>) -> io::Result<()> {
    let mut wide: Vec<u16> = text.encode_utf16().collect();
    wide.push(0);
    let bytes: Vec<u8> = wide.iter().flat_map(|c| c.to_le_bytes()).collect();
    let _open = Open::new(owner)?;
    // SAFETY: the clipboard is open (guard above).
    unsafe { EmptyClipboard() }.map_err(io::Error::other)?;
    set(u32::from(CF_UNICODETEXT.0), &bytes)?;
    if sensitive {
        let zero = 0u32.to_le_bytes();
        let marked = set(
            format(w!("ExcludeClipboardContentFromMonitorProcessing")),
            &zero,
        )
        .and_then(|_| set(format(w!("CanIncludeInClipboardHistory")), &zero))
        .and_then(|_| set(format(w!("CanUploadToCloudClipboard")), &zero));
        if let Err(e) = marked {
            // Never leave a password on the clipboard without its markers
            // (it would reach Win+V history and the cloud clipboard).
            // SAFETY: the clipboard is still open (guard above).
            let _ = unsafe { EmptyClipboard() };
            return Err(e);
        }
    }
    Ok(())
}

/// Empty the clipboard (a copied one-time code that is no longer needed).
pub fn clear(owner: Option<HWND>) -> io::Result<()> {
    let _open = Open::new(owner)?;
    // SAFETY: the clipboard is open (guard above).
    unsafe { EmptyClipboard() }.map_err(io::Error::other)
}

/// Read clipboard text, refusing content other apps marked as private.
pub fn get_text(owner: Option<HWND>) -> io::Result<ClipText> {
    Ok(match get_text_marked(owner)? {
        (ClipText::Text(_), true) => ClipText::Excluded,
        (t, _) => t,
    })
}

/// Read clipboard text and whether the copying app marked it private
/// (password managers). Only for callers whose user opted in to syncing it.
pub fn get_text_marked(owner: Option<HWND>) -> io::Result<(ClipText, bool)> {
    let _open = Open::new(owner)?;
    // SAFETY: clipboard is open; the handle is only read while locked.
    unsafe {
        let private =
            IsClipboardFormatAvailable(format(w!("ExcludeClipboardContentFromMonitorProcessing")))
                .is_ok();
        let text = read_unicode()?;
        Ok((text, private))
    }
}

/// CF_UNICODETEXT of the open clipboard.
///
/// # Safety
/// The clipboard must be open on this thread.
unsafe fn read_unicode() -> io::Result<ClipText> {
    unsafe {
        let Ok(h) = GetClipboardData(u32::from(CF_UNICODETEXT.0)) else {
            return Ok(ClipText::Empty);
        };
        let hg = HGLOBAL(h.0);
        let p = GlobalLock(hg) as *const u16;
        if p.is_null() {
            return Ok(ClipText::Empty);
        }
        let max = (GlobalSize(hg) / 2).min(MAX_CHARS + 1);
        let slice = std::slice::from_raw_parts(p, max);
        let len = slice.iter().position(|&c| c == 0).unwrap_or(max);
        let text = String::from_utf16_lossy(&slice[..len]);
        let _ = GlobalUnlock(hg);
        if len > MAX_CHARS {
            return Err(io::Error::other("clipboard text is too large"));
        }
        Ok(if text.is_empty() {
            ClipText::Empty
        } else {
            ClipText::Text(text)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // One test touches the real clipboard so steps cannot interleave.
    #[test]
    fn round_trip_and_sensitive_content_is_excluded() {
        set_text("héllo from test", false, None).unwrap();
        assert_eq!(
            get_text(None).unwrap(),
            ClipText::Text("héllo from test".into())
        );
        set_text("hunter2", true, None).unwrap();
        assert_eq!(get_text(None).unwrap(), ClipText::Excluded);
        assert_eq!(
            get_text_marked(None).unwrap(),
            (ClipText::Text("hunter2".into()), true)
        );
        set_text("", false, None).unwrap();
        assert_eq!(get_text(None).unwrap(), ClipText::Empty);
        set_text("482913", true, None).unwrap();
        clear(None).unwrap();
        assert_eq!(get_text_marked(None).unwrap(), (ClipText::Empty, false));
    }
}
