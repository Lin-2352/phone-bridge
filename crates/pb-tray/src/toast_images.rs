//! Image files for toasts: Windows toasts from a non-Store app only load
//! images from disk. Everything lives in one private folder, emptied at
//! start and kept under a size cap.

use std::{collections::HashMap, path::PathBuf, sync::Mutex};

/// Largest the folder may grow before the oldest files go.
pub const MAX_BYTES: u64 = 50 * 1024 * 1024;

pub struct ToastImages {
    dir: PathBuf,
    max: u64,
    /// icon hash -> file, for this run
    icons: Mutex<HashMap<Vec<u8>, PathBuf>>,
}

impl ToastImages {
    /// Use `dir`, emptied first (images left by a previous run).
    pub fn new(dir: PathBuf) -> Self {
        for e in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            let _ = std::fs::remove_file(e.path());
        }
        let _ = std::fs::create_dir_all(&dir);
        Self {
            dir,
            max: MAX_BYTES,
            icons: Mutex::default(),
        }
    }

    /// Store an app icon (PNG only); returns its file.
    pub fn icon(&self, hash: &[u8], png: &[u8]) -> Option<PathBuf> {
        if !png.starts_with(b"\x89PNG\r\n\x1a\n") || hash.is_empty() {
            return None;
        }
        let path = self
            .dir
            .join(format!("icon-{}.png", hex(&hash[..hash.len().min(8)])));
        if !path.exists() {
            std::fs::write(&path, png).ok()?;
        }
        self.icons
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(hash.to_vec(), path.clone());
        self.trim();
        Some(path)
    }

    pub fn icon_path(&self, hash: &[u8]) -> Option<PathBuf> {
        self.icons
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(hash)
            .cloned()
    }

    /// Store a photo or avatar (JPEG only) for the toast `tag`. The name
    /// includes a content hash: Windows caches toast images by path.
    pub fn picture(&self, tag: &str, jpeg: &[u8]) -> Option<PathBuf> {
        if !jpeg.starts_with(&[0xFF, 0xD8])
            || tag.is_empty()
            || !tag.bytes().all(|b| b.is_ascii_hexdigit())
        {
            return None;
        }
        let digest = pb_transport::notify::key_hash(&hex(jpeg));
        let path = self.dir.join(format!("{tag}-{}.jpg", &digest[..8]));
        std::fs::write(&path, jpeg).ok()?;
        self.trim();
        Some(path)
    }

    #[cfg(test)]
    pub fn dir(&self) -> &std::path::Path {
        &self.dir
    }

    /// Delete the oldest files until the folder is under the cap.
    fn trim(&self) {
        let mut files: Vec<(std::time::SystemTime, u64, PathBuf)> = std::fs::read_dir(&self.dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| {
                let m = e.metadata().ok()?;
                Some((m.modified().ok()?, m.len(), e.path()))
            })
            .collect();
        let mut total: u64 = files.iter().map(|f| f.1).sum();
        files.sort();
        for (_, len, path) in files {
            if total <= self.max {
                break;
            }
            if std::fs::remove_file(&path).is_ok() {
                total -= len;
                self.icons
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .retain(|_, p| *p != path);
            }
        }
    }
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const PNG: &[u8] = &[0x89, b'P', b'N', b'G', 13, 10, 26, 10, 1, 2, 3];
    const JPEG: &[u8] = &[0xFF, 0xD8, 0xFF, 0xE0, 1, 2, 3];

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("pb-toast-img-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn start_empties_the_folder() {
        let d = tmp("start");
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("old.jpg"), b"x").unwrap();
        let _imgs = ToastImages::new(d.clone());
        assert_eq!(std::fs::read_dir(&d).unwrap().count(), 0);
    }

    #[test]
    fn icons_are_written_once_and_found_by_hash() {
        let imgs = ToastImages::new(tmp("icons"));
        let p = imgs.icon(&[0xAB; 32], PNG).unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), PNG);
        assert_eq!(imgs.icon_path(&[0xAB; 32]), Some(p));
        assert_eq!(imgs.icon_path(&[0xCD; 32]), None);
    }

    #[test]
    fn images_reject_non_image_bytes() {
        let imgs = ToastImages::new(tmp("reject"));
        assert_eq!(imgs.icon(&[1; 32], b"MZ\x90\x00 not a png"), None);
        assert_eq!(imgs.picture("0123456789abcdef", b"<svg/>"), None);
        assert_eq!(imgs.picture("../../evil", JPEG), None, "tags are hex only");
    }

    #[test]
    fn a_changed_picture_gets_a_new_file_name() {
        let imgs = ToastImages::new(tmp("pics"));
        let a = imgs.picture("0123456789abcdef", JPEG).unwrap();
        let mut other = JPEG.to_vec();
        other.push(9);
        let b = imgs.picture("0123456789abcdef", &other).unwrap();
        assert_ne!(a, b, "Windows caches toast images by path");
        assert!(a.starts_with(imgs.dir()));
    }

    #[test]
    fn folder_is_capped() {
        let mut imgs = ToastImages::new(tmp("cap"));
        imgs.max = 3 * 1024;
        let mut big = JPEG.to_vec();
        big.resize(1024, 0);
        for i in 0..6u8 {
            big[10] = i;
            let tag = format!("{:016x}", i);
            imgs.picture(&tag, &big).unwrap();
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let total: u64 = std::fs::read_dir(imgs.dir())
            .unwrap()
            .flatten()
            .map(|e| e.metadata().unwrap().len())
            .sum();
        assert!(total <= 3 * 1024, "{total}");
        let newest = format!("{:016x}-", 5u8);
        assert!(
            std::fs::read_dir(imgs.dir())
                .unwrap()
                .flatten()
                .any(|e| e.file_name().to_string_lossy().starts_with(&newest)),
            "the newest stays"
        );
    }
}
