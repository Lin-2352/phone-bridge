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
            Ole::{CF_DIB, CF_DIBV5, CF_UNICODETEXT},
        },
    },
};

/// Largest clipboard text we read (characters); larger content is refused.
const MAX_CHARS: usize = 1 << 20;

/// Largest image block we read from the clipboard (bytes); a 32 bpp DIB at
/// the conversion limit is 128 MiB, anything bigger is refused unread.
const MAX_IMAGE_BYTES: usize = 160 << 20;

/// An image found on the clipboard, as the copying app put it.
#[derive(Debug, PartialEq, Eq)]
pub enum ClipImage {
    /// The registered "PNG" format (keeps transparency).
    Png(Vec<u8>),
    /// `CF_DIBV5` or `CF_DIB` bytes, for `dib::dib_to_png`.
    Dib(Vec<u8>),
}

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
        if let Err(e) = mark_private() {
            // Never leave a password on the clipboard without its markers
            // (it would reach Win+V history and the cloud clipboard).
            // SAFETY: the clipboard is still open (guard above).
            let _ = unsafe { EmptyClipboard() };
            return Err(e);
        }
    }
    Ok(())
}

fn mark_private() -> io::Result<()> {
    let zero = 0u32.to_le_bytes();
    set(
        format(w!("ExcludeClipboardContentFromMonitorProcessing")),
        &zero,
    )
    .and_then(|_| set(format(w!("CanIncludeInClipboardHistory")), &zero))
    .and_then(|_| set(format(w!("CanUploadToCloudClipboard")), &zero))
}

/// Put an image on the clipboard as `CF_DIBV5` (Windows adds `CF_DIB` and
/// `CF_BITMAP` itself) plus the registered "PNG" format. A private image is
/// kept out of history, the cloud clipboard and monitors, like private text.
pub fn set_image(png: &[u8], dibv5: &[u8], sensitive: bool, owner: Option<HWND>) -> io::Result<()> {
    let _open = Open::new(owner)?;
    // SAFETY: the clipboard is open (guard above).
    unsafe { EmptyClipboard() }.map_err(io::Error::other)?;
    let written = set(u32::from(CF_DIBV5.0), dibv5)
        .and_then(|_| set(format(w!("PNG")), png))
        .and_then(|_| if sensitive { mark_private() } else { Ok(()) });
    if let Err(e) = written {
        // Never leave half an image, or a private one without its markers.
        // SAFETY: the clipboard is still open (guard above).
        let _ = unsafe { EmptyClipboard() };
        return Err(e);
    }
    Ok(())
}

/// Cut a clipboard "PNG" block (rounded up by `GlobalSize`) after its IEND
/// chunk. Without an IEND the block is not a whole PNG.
fn trim_png(block: &[u8]) -> Option<&[u8]> {
    let at = block.windows(4).rposition(|w| w == b"IEND")?;
    block.get(..at + 8) // "IEND" + its 4-byte CRC
}

/// Read an image from the clipboard and whether the copying app marked it
/// private. Text on the clipboard means no image: a copy that carries both
/// (a spreadsheet range) is text.
pub fn get_image(owner: Option<HWND>) -> io::Result<(Option<ClipImage>, bool)> {
    let _open = Open::new(owner)?;
    // SAFETY: clipboard is open; handles are only read while locked.
    unsafe {
        if IsClipboardFormatAvailable(u32::from(CF_UNICODETEXT.0)).is_ok() {
            return Ok((None, false));
        }
        let private =
            IsClipboardFormatAvailable(format(w!("ExcludeClipboardContentFromMonitorProcessing")))
                .is_ok();
        let png = format(w!("PNG"));
        if IsClipboardFormatAvailable(png).is_ok() {
            if let Some(b) = read_block(png)? {
                if let Some(t) = trim_png(&b) {
                    return Ok((Some(ClipImage::Png(t.to_vec())), private));
                }
            }
        }
        for fmt in [CF_DIBV5, CF_DIB] {
            if IsClipboardFormatAvailable(u32::from(fmt.0)).is_ok() {
                if let Some(b) = read_block(u32::from(fmt.0))? {
                    return Ok((Some(ClipImage::Dib(b)), private));
                }
            }
        }
        Ok((None, private))
    }
}

/// The bytes of clipboard format `fmt`, or None if absent or empty.
///
/// # Safety
/// The clipboard must be open on this thread.
unsafe fn read_block(fmt: u32) -> io::Result<Option<Vec<u8>>> {
    unsafe {
        let Ok(h) = GetClipboardData(fmt) else {
            return Ok(None);
        };
        let hg = HGLOBAL(h.0);
        let n = GlobalSize(hg);
        if n > MAX_IMAGE_BYTES {
            return Err(io::Error::other("clipboard image is too large"));
        }
        let p = GlobalLock(hg) as *const u8;
        if p.is_null() || n == 0 {
            return Ok(None);
        }
        let bytes = std::slice::from_raw_parts(p, n).to_vec();
        let _ = GlobalUnlock(hg);
        Ok(Some(bytes))
    }
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

    /// Tests share the one real clipboard, so they take turns.
    static CLIPBOARD: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn tiny_png() -> Vec<u8> {
        let mut out = Vec::new();
        let mut e = png::Encoder::new(std::io::Cursor::new(&mut out), 2, 1);
        e.set_color(png::ColorType::Rgba);
        e.set_depth(png::BitDepth::Eight);
        e.write_header()
            .unwrap()
            .write_image_data(&[255, 0, 0, 255, 0, 0, 255, 128])
            .unwrap();
        out
    }

    #[test]
    fn an_image_round_trips_and_private_images_are_marked() {
        let _turn = CLIPBOARD.lock().unwrap_or_else(|e| e.into_inner());
        let png = tiny_png();
        let dib = crate::dib::png_to_dibv5(&png).unwrap();
        set_image(&png, &dib, false, None).unwrap();
        // The PNG we wrote comes back byte for byte, unmarked.
        assert_eq!(
            get_image(None).unwrap(),
            (Some(ClipImage::Png(png.clone())), false)
        );
        // Windows also offers it as a plain DIB for apps that only know that.
        assert!(unsafe { IsClipboardFormatAvailable(u32::from(CF_DIB.0)) }.is_ok());
        // Text on the clipboard means no image.
        set_text("just text", false, None).unwrap();
        assert_eq!(get_image(None).unwrap(), (None, false));
        // A private image is reported as private.
        set_image(&png, &dib, true, None).unwrap();
        assert!(get_image(None).unwrap().1);
        clear(None).unwrap();
        assert_eq!(get_image(None).unwrap(), (None, false));
    }

    #[test]
    fn a_padded_png_block_is_cut_after_iend_and_junk_is_refused() {
        let png = tiny_png();
        let mut padded = png.clone();
        padded.extend([0u8; 13]);
        assert_eq!(trim_png(&padded), Some(&png[..]));
        assert_eq!(trim_png(b"no end chunk here"), None);
        assert_eq!(trim_png(b""), None);
    }

    // One test touches the real clipboard so steps cannot interleave.
    #[test]
    fn round_trip_and_sensitive_content_is_excluded() {
        let _turn = CLIPBOARD.lock().unwrap_or_else(|e| e.into_inner());
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
