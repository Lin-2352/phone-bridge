//! Second-instance commands (Explorer "Send to", keyboard shortcuts) reach the
//! running tray over a local named pipe: `pb-tray --send <files>`,
//! `--pair`, `--send-clipboard`.
//!
//! The pipe name includes the user name; remote clients are rejected and the
//! default pipe ACL only lets the creating user (and admins/SYSTEM) write.
//! Messages are UTF-8 lines: a verb, then one file path per line for `send`.

use std::{io::Write, path::PathBuf};

use tokio::{
    io::AsyncReadExt, net::windows::named_pipe::ServerOptions, sync::mpsc::UnboundedSender,
};

use crate::core::Cmd;

fn pipe_name() -> String {
    let user: String = std::env::var("USERNAME")
        .unwrap_or_else(|_| "user".into())
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .collect();
    format!(r"\\.\pipe\PhoneBridgeTray-{user}")
}

/// What a second instance can ask for.
pub enum Request {
    Pair,
    SendClipboard,
    /// Open the settings window.
    Settings,
    /// Open the flyout.
    Open,
    /// Show the tray menu (for checks without clicking the icon).
    Menu,
    Send(Vec<PathBuf>),
}

fn encode(r: &Request) -> String {
    match r {
        Request::Pair => "pair\n".into(),
        Request::SendClipboard => "send-clipboard\n".into(),
        Request::Settings => "settings\n".into(),
        Request::Open => "open\n".into(),
        Request::Menu => "menu\n".into(),
        Request::Send(files) => {
            let mut s = "send\n".to_owned();
            for f in files {
                s.push_str(&f.display().to_string());
                s.push('\n');
            }
            s
        }
    }
}

/// Parse a request; unknown verbs and empty file lists are rejected.
pub fn decode(text: &str) -> Option<Request> {
    let mut lines = text.lines();
    match lines.next()? {
        "pair" => Some(Request::Pair),
        "send-clipboard" => Some(Request::SendClipboard),
        "settings" => Some(Request::Settings),
        "open" => Some(Request::Open),
        "menu" => Some(Request::Menu),
        "send" => {
            let files: Vec<PathBuf> = lines.filter(|l| !l.is_empty()).map(PathBuf::from).collect();
            (!files.is_empty()).then_some(Request::Send(files))
        }
        _ => None,
    }
}

/// Client side (second instance). Returns false if no tray is running.
pub fn send_request(r: &Request) -> bool {
    match std::fs::OpenOptions::new().write(true).open(pipe_name()) {
        Ok(mut pipe) => pipe.write_all(encode(r).as_bytes()).is_ok(),
        Err(_) => false,
    }
}

/// Server side: accept requests forever. Requests that need the UI thread
/// (clipboard, windows) go to `on_ui`.
pub async fn serve(cmds: UnboundedSender<Cmd>, on_ui: impl Fn(&Request) + Send + 'static) {
    let name = pipe_name();
    let mut first = true;
    loop {
        let server = match ServerOptions::new()
            .first_pipe_instance(first)
            .reject_remote_clients(true)
            .create(&name)
        {
            Ok(s) => s,
            Err(_) => return, // another tray owns the pipe
        };
        first = false;
        if server.connect().await.is_err() {
            continue;
        }
        let mut server = server;
        let mut buf = Vec::new();
        // Bounded read: a request is a handful of paths.
        let _ = (&mut server).take(64 * 1024).read_to_end(&mut buf).await;
        match decode(&String::from_utf8_lossy(&buf)) {
            Some(Request::Pair) => {
                let _ = cmds.send(Cmd::Pair);
            }
            Some(Request::Send(files)) => {
                let _ = cmds.send(Cmd::SendFiles(files));
            }
            Some(
                r @ (Request::SendClipboard | Request::Settings | Request::Open | Request::Menu),
            ) => on_ui(&r),
            None => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_round_trip_and_garbage_is_rejected() {
        let r = Request::Send(vec![
            PathBuf::from(r"C:\a b\c.jpg"),
            PathBuf::from(r"D:\x.txt"),
        ]);
        match decode(&encode(&r)) {
            Some(Request::Send(f)) => assert_eq!(f.len(), 2),
            _ => panic!(),
        }
        assert!(matches!(decode("pair\n"), Some(Request::Pair)));
        assert!(decode("send\n").is_none());
        assert!(decode("rm -rf /\n").is_none());
        assert!(decode("").is_none());
    }
}
