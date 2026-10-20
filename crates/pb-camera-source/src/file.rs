//! Opening the frames file. Both the tray and the source open it with
//! `FILE_SHARE_READ | FILE_SHARE_WRITE` (so neither gets
//! `ERROR_SHARING_VIOLATION`) and map it without a name: named objects do
//! not cross from the tray's session into Frame Server's.
#![allow(unsafe_code)]

use std::path::Path;

use pb_camera_frames::FILE_BYTES;
use windows::{
    core::HSTRING,
    Win32::{
        Foundation::{CloseHandle, GENERIC_READ, GENERIC_WRITE, HANDLE},
        Storage::FileSystem::{
            CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
        },
        System::Memory::{
            CreateFileMappingW, MapViewOfFile, UnmapViewOfFile, FILE_MAP_READ, FILE_MAP_WRITE,
            MEMORY_MAPPED_VIEW_ADDRESS, PAGE_READONLY, PAGE_READWRITE,
        },
    },
};

/// The file mapped into this process. `words()` is valid while this lives.
pub struct Mapped {
    file: HANDLE,
    mapping: HANDLE,
    view: MEMORY_MAPPED_VIEW_ADDRESS,
}

// SAFETY: plain handles and an address; access goes through the Ring's
// atomics and seqlock.
unsafe impl Send for Mapped {}

impl Mapped {
    /// `write`: the tray (read-write). The source opens read-only.
    pub fn open(path: &Path, write: bool) -> Result<Self, String> {
        let access = if write {
            GENERIC_READ.0 | GENERIC_WRITE.0
        } else {
            GENERIC_READ.0
        };
        // SAFETY: plain Win32 calls with valid arguments; handles closed in Drop.
        unsafe {
            let file = CreateFileW(
                &HSTRING::from(path.as_os_str()),
                access,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                None,
                OPEN_EXISTING,
                FILE_ATTRIBUTE_NORMAL,
                None,
            )
            .map_err(|e| format!("open frames file: {e}"))?;
            let protect = if write { PAGE_READWRITE } else { PAGE_READONLY };
            let mapping = match CreateFileMappingW(file, None, protect, 0, FILE_BYTES as u32, None)
            {
                Ok(m) => m,
                Err(e) => {
                    let _ = CloseHandle(file);
                    return Err(format!("map frames file: {e}"));
                }
            };
            let view = MapViewOfFile(
                mapping,
                if write {
                    FILE_MAP_READ | FILE_MAP_WRITE
                } else {
                    FILE_MAP_READ
                },
                0,
                0,
                FILE_BYTES,
            );
            if view.Value.is_null() {
                let _ = CloseHandle(mapping);
                let _ = CloseHandle(file);
                return Err("map view of the frames file failed".into());
            }
            Ok(Self {
                file,
                mapping,
                view,
            })
        }
    }

    /// The mapped bytes as words (a mapping is page-aligned). The tray
    /// writes through it; the source only reads, so it never writes.
    ///
    /// # Safety note
    /// Both processes use the same atomics and seqlock (`Ring`), which is
    /// what makes the shared access sound in practice.
    pub fn words(&mut self) -> &mut [u64] {
        // SAFETY: the view is FILE_BYTES long, page-aligned, and stays
        // mapped while `self` lives.
        unsafe {
            std::slice::from_raw_parts_mut(self.view.Value as *mut u64, FILE_BYTES.div_ceil(8))
        }
    }
}

impl Drop for Mapped {
    fn drop(&mut self) {
        // SAFETY: unmapping and closing what `open` created.
        unsafe {
            let _ = UnmapViewOfFile(self.view);
            let _ = CloseHandle(self.mapping);
            let _ = CloseHandle(self.file);
        }
    }
}

#[cfg(test)]
mod tests {
    use pb_camera_frames::{Read, Ring};

    use super::*;

    fn temp_file() -> std::path::PathBuf {
        let p = std::env::temp_dir().join(format!("pbcf-test-{}.frames", std::process::id()));
        std::fs::write(&p, vec![0u8; FILE_BYTES]).unwrap();
        p
    }

    #[test]
    fn two_handles_open_the_frames_file_at_once() {
        let path = temp_file();
        let mut tray = Mapped::open(&path, true).expect("the tray's read-write open");
        let mut source =
            Mapped::open(&path, false).expect("the source's read open, no sharing violation");
        {
            let mut ring = Ring::init(tray.words());
            ring.write_frame(1280, 720, 7, |s| s[..1280 * 720 * 3 / 2].fill(42));
        }
        // The reader maps the same bytes: it sees what the writer wrote.
        let mut copy = source.words().to_vec();
        let ring = Ring::open(&mut copy).expect("magic through the other mapping");
        let mut out = Vec::new();
        assert!(matches!(
            ring.read_newest(0, (1280, 720), &mut out),
            Read::New { pts_us: 7, .. }
        ));
        assert!(out.iter().all(|b| *b == 42));
        drop(tray);
        drop(source);
        let _ = std::fs::remove_file(path);
    }
}
