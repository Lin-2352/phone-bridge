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
    /// Text or a link from the share sheet.
    Text {
        link: bool,
        text: String,
    },
    Send(Vec<PathBuf>),
}

fn encode(r: &Request) -> String {
    match r {
        Request::Pair => "pair\n".into(),
        Request::SendClipboard => "send-clipboard\n".into(),
        Request::Settings => "settings\n".into(),
        Request::Open => "open\n".into(),
        Request::Menu => "menu\n".into(),
        Request::Text { link, text } => format!("text\n{}\n{text}", u8::from(*link)),
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
        "text" => {
            let link = lines.next()? == "1";
            let text: Vec<&str> = lines.collect();
            Some(Request::Text {
                link,
                text: text.join("\n"),
            })
        }
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
pub async fn serve(cmds: UnboundedSender<Cmd>, on_ui: impl Fn(&Request) + Send + Sync + 'static) {
    let name = pipe_name();
    let on_ui = std::sync::Arc::new(on_ui);
    let create = |first: bool| {
        ServerOptions::new()
            .first_pipe_instance(first)
            .reject_remote_clients(true)
            .create(&name)
    };
    let Ok(mut server) = create(true) else {
        return; // another tray owns the pipe
    };
    loop {
        if server.connect().await.is_err() {
            match create(false) {
                Ok(s) => server = s,
                Err(_) => tokio::time::sleep(std::time::Duration::from_secs(1)).await,
            }
            continue;
        }
        // Next instance first, so another request never finds the pipe busy
        // while this client is being served.
        let client = server;
        server = loop {
            match create(false) {
                Ok(s) => break s,
                Err(_) => tokio::time::sleep(std::time::Duration::from_secs(1)).await,
            }
        };
        let (cmds, on_ui) = (cmds.clone(), on_ui.clone());
        tokio::spawn(async move {
            let mut client = client;
            let mut buf = Vec::new();
            // Bounded in size and time: a request is a handful of paths, and a
            // client that never finishes must not hold anything.
            let mut limited = (&mut client).take(64 * 1024);
            let read = limited.read_to_end(&mut buf);
            if tokio::time::timeout(std::time::Duration::from_secs(2), read)
                .await
                .is_err()
            {
                return;
            }
            match decode(&String::from_utf8_lossy(&buf)) {
                Some(Request::Pair) => {
                    let _ = cmds.send(Cmd::Pair);
                }
                Some(Request::Send(files)) => {
                    let files: Vec<_> = files.into_iter().filter(|f| f.is_file()).collect();
                    if !files.is_empty() {
                        let _ = cmds.send(Cmd::SendFiles(files));
                    }
                }
                Some(Request::Text { link, text }) => {
                    let _ = cmds.send(Cmd::SendText { text, link });
                }
                Some(
                    r
                    @ (Request::SendClipboard | Request::Settings | Request::Open | Request::Menu),
                ) => on_ui(&r),
                None => {}
            }
        });
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
