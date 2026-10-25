//! How this tray was started (`--dir`, `--name`), so what starts it again
//! (Start with Windows, the share sheet, Send to) keeps the same data folder:
//! a portable copy must not fall back to `%LOCALAPPDATA%`.

use std::{path::Path, sync::OnceLock};

static ARGS: OnceLock<Vec<String>> = OnceLock::new();

/// The arguments that reproduce a non-default start: none for the default
/// folder, `--dir <dir> --name <name>` otherwise.
pub fn args_for(dir: &Path, default_dir: &Path, name: &str) -> Vec<String> {
    if dir == default_dir {
        return Vec::new();
    }
    vec![
        "--dir".into(),
        dir.display().to_string(),
        "--name".into(),
        name.into(),
    ]
}

pub fn set(args: Vec<String>) {
    let _ = ARGS.set(args);
}

pub fn args() -> &'static [String] {
    ARGS.get().map_or(&[], |v| v.as_slice())
}

/// The arguments, each in quotes, separated by spaces.
pub fn quoted(args: &[String]) -> String {
    args.iter()
        .map(|a| format!("\"{a}\""))
        .collect::<Vec<_>>()
        .join(" ")
}

/// A command line: the exe and each argument in quotes.
pub fn command_line(exe: &Path, args: &[String]) -> String {
    let exe = format!("\"{}\"", exe.display());
    if args.is_empty() {
        exe
    } else {
        format!("{exe} {}", quoted(args))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn the_default_folder_needs_no_arguments() {
        let d = PathBuf::from(r"C:\Users\a\AppData\Local\PhoneBridge\tray");
        assert!(args_for(&d, &d, "PC").is_empty());
    }

    #[test]
    fn a_portable_folder_is_passed_on() {
        let d = PathBuf::from(r"D:\My Apps\Relay\data");
        let def = PathBuf::from(r"C:\x\tray");
        assert_eq!(
            args_for(&d, &def, "DeskTray"),
            ["--dir", r"D:\My Apps\Relay\data", "--name", "DeskTray"]
        );
    }

    #[test]
    fn the_command_line_quotes_paths_with_spaces() {
        let exe = PathBuf::from(r"D:\My Apps\Relay\Relay.exe");
        let a = args_for(
            &PathBuf::from(r"D:\My Apps\Relay\data"),
            &PathBuf::from("x"),
            "PC",
        );
        assert_eq!(
            command_line(&exe, &a),
            r#""D:\My Apps\Relay\Relay.exe" "--dir" "D:\My Apps\Relay\data" "--name" "PC""#
        );
        assert_eq!(command_line(&exe, &[]), r#""D:\My Apps\Relay\Relay.exe""#);
        assert_eq!(quoted(&[]), "");
        assert_eq!(
            quoted(&a),
            r#""--dir" "D:\My Apps\Relay\data" "--name" "PC""#
        );
    }
}
