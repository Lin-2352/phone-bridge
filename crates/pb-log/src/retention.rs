//! Pruning and compression of old day folders. Never touches today.

use crate::timefmt::{day_for, parse_day};
use crate::Caps;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// Size of every file under `p`.
pub fn dir_size(p: &Path) -> u64 {
    let Ok(rd) = fs::read_dir(p) else { return 0 };
    rd.flatten()
        .map(|e| match fs::metadata(e.path()) {
            Ok(m) if m.is_dir() => dir_size(&e.path()),
            Ok(m) => m.len(),
            Err(_) => 0,
        })
        .sum()
}

/// Day folders under `root`, oldest first, as (name, days since epoch).
pub fn day_folders(root: &Path) -> Vec<(String, i64)> {
    let mut v: Vec<(String, i64)> = fs::read_dir(root)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.metadata().map(|m| m.is_dir()).unwrap_or(false))
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            parse_day(&name).map(|d| (name, d))
        })
        .collect();
    v.sort_by_key(|(_, d)| *d);
    v
}

fn gzip_file(path: &Path) -> io::Result<()> {
    let mut gz = PathBuf::from(path);
    let mut name = gz.file_name().unwrap_or_default().to_os_string();
    name.push(".gz");
    gz.set_file_name(name);
    let tmp = gz.with_extension("gz.tmp");
    let data = fs::read(path)?;
    let mut enc =
        flate2::write::GzEncoder::new(fs::File::create(&tmp)?, flate2::Compression::default());
    enc.write_all(&data)?;
    enc.finish()?.sync_all()?;
    fs::rename(&tmp, &gz)?;
    fs::remove_file(path)
}

/// Apply retention at `now_ms`: delete days older than `keep_days`, gzip days
/// older than `compress_after_days`, then delete oldest days while the total
/// exceeds `total_bytes`. Errors are skipped (best effort).
pub fn prune(root: &Path, caps: &Caps, now_ms: u64, offset_secs: i32) {
    let today = parse_day(&day_for((now_ms / 1000) as i64, offset_secs)).unwrap_or(0);
    for (name, d) in day_folders(root) {
        let age = today - d;
        let dir = root.join(&name);
        if age > i64::from(caps.keep_days) {
            let _ = fs::remove_dir_all(&dir);
        } else if age >= i64::from(caps.compress_after_days) && age > 0 {
            for e in fs::read_dir(&dir).into_iter().flatten().flatten() {
                let p = e.path();
                if p.extension().is_none_or(|x| x != "gz" && x != "tmp") {
                    let _ = gzip_file(&p);
                }
            }
        }
    }
    let days = day_folders(root);
    let mut total = dir_size(root);
    for (name, d) in days {
        if total <= caps.total_bytes || d >= today {
            break;
        }
        let dir = root.join(&name);
        total = total.saturating_sub(dir_size(&dir));
        let _ = fs::remove_dir_all(&dir);
    }
}
