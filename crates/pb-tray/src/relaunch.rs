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

/// What `cmd` gets to start this exe again a moment after this one is gone
/// (the single-instance mutex is released then). Passed raw: `/S` makes cmd drop
/// only the outer quotes, and C-style escaped quotes would break it.
pub fn restart_line(exe: &Path, args: &[String]) -> String {
    format!(
        r#"/S /C "ping -n 3 127.0.0.1 >nul & start "" {}""#,
        command_line(exe, args)
    )
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

#[cfg(test)]
mod restart_tests {
    use super::*;

    #[test]
    fn the_restart_line_keeps_paths_with_spaces_whole() {
        let line = restart_line(
            Path::new(r"C:\A B\Relay.exe"),
            &["--dir".into(), r"C:\A B\data".into()],
        );
        assert_eq!(
            line,
            r#"/S /C "ping -n 3 127.0.0.1 >nul & start "" "C:\A B\Relay.exe" "--dir" "C:\A B\data"""#
        );
    }

    /// cmd reads this form itself: quotes escaped for C programs would break it.
    #[test]
    fn cmd_understands_that_form() {
        use std::os::windows::process::CommandExt;
        let dir = std::env::temp_dir().join(format!("relay restart {}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let out = dir.join("out file.txt");
        let line = format!(r#"/S /C "echo ok> "{}"""#, out.display());
        let status = std::process::Command::new("cmd")
            .raw_arg(&line)
            .status()
            .unwrap();
        assert!(status.success());
        assert!(std::fs::read_to_string(&out).unwrap().contains("ok"));
        let _ = std::fs::remove_dir_all(dir);
    }
}
