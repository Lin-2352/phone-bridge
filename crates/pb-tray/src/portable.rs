//! A portable copy keeps everything in a `data` folder beside the exe, however
//! it is started: double-click, Start menu, a toast, or Windows at sign-in.

use std::path::{Path, PathBuf};

/// The `data` folder beside the exe, if there is one.
pub fn data_dir(exe_dir: &Path) -> Option<PathBuf> {
    let dir = exe_dir.join("data");
    dir.is_dir().then_some(dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_data_folder_beside_the_exe_makes_it_portable() {
        let root = std::env::temp_dir().join(format!("relay portable {}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        assert_eq!(data_dir(&root), None);
        std::fs::create_dir_all(root.join("data")).unwrap();
        assert_eq!(data_dir(&root), Some(root.join("data")));
        let _ = std::fs::remove_dir_all(root);
    }
}
