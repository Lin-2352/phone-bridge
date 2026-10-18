//! Phone Bridge tray app for Windows.
//!
//! pb-tray [--dir DIR] [--port N] [--name NAME] [--pair] [--test-auto-accept]
//!
//! If the tray is already running, `--pair` and `--send-clipboard` are
//! forwarded to it (for keyboard shortcuts) and this process exits.
//! `--pair` opens a pairing code at start; `--test-auto-accept` accepts the
//! SAS without asking and exists only for automated tests.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

#[cfg(windows)]
mod core;
#[cfg(windows)]
mod ui;

#[cfg(not(windows))]
fn main() {
    eprintln!("pb-tray runs on Windows only");
}

#[cfg(windows)]
#[allow(unsafe_code)]
fn single_instance() -> bool {
    use windows::{
        core::w,
        Win32::{
            Foundation::{GetLastError, ERROR_ALREADY_EXISTS},
            System::Threading::CreateMutexW,
        },
    };
    // SAFETY: named mutex lives for the whole process (handle intentionally kept).
    unsafe {
        let ok = CreateMutexW(None, true, w!("Local\\PhoneBridgeTray")).is_ok();
        ok && GetLastError() != ERROR_ALREADY_EXISTS
    }
}

#[cfg(windows)]
fn main() {
    use std::{
        path::PathBuf,
        sync::{Arc, Mutex},
    };

    let mut opts = core::Options {
        dir: std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir)
            .join("PhoneBridge")
            .join("tray"),
        port: 47200,
        name: std::env::var("COMPUTERNAME").unwrap_or_else(|_| "PC".into()),
        pair_on_start: false,
    };
    let mut auto_accept = false;
    let mut send_clipboard = false;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--dir" => opts.dir = args.next().map(PathBuf::from).unwrap_or(opts.dir),
            "--port" => {
                opts.port = args
                    .next()
                    .and_then(|p| p.parse().ok())
                    .unwrap_or(opts.port)
            }
            "--name" => opts.name = args.next().unwrap_or(opts.name),
            "--pair" => opts.pair_on_start = true,
            "--test-auto-accept" => auto_accept = true,
            "--send-clipboard" => send_clipboard = true,
            _ => {}
        }
    }
    if !single_instance() {
        let mut fwd = Vec::new();
        if opts.pair_on_start {
            fwd.push(ui::ID_PAIR);
        }
        if send_clipboard {
            fwd.push(ui::ID_SEND_CLIPBOARD);
        }
        let ok = fwd.into_iter().all(ui::forward_command);
        std::process::exit(if ok { 0 } else { 1 });
    }
    let _ = std::fs::create_dir_all(&opts.dir);
    let log = core::Log::open(opts.dir.join("tray.log"));
    // Idle almost always: run on efficient cores at low clock.
    if let Err(e) = pb_platform_win::power::set_eco_qos(true) {
        log.line(&format!("EcoQoS unavailable: {e}"));
    }

    let (cmd_tx, cmd_rx) = tokio::sync::mpsc::unbounded_channel();
    let peers: core::Peers = Arc::new(Mutex::new(Vec::new()));
    let hwnd = match ui::create(cmd_tx, peers.clone(), auto_accept, log.clone()) {
        Ok(h) => h,
        Err(e) => {
            log.line(&format!("could not create tray icon: {e}"));
            return;
        }
    };

    let core_log = log.clone();
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("tokio runtime");
        let result = rt.block_on(core::run(opts, cmd_rx, peers, core_log.clone(), move |e| {
            ui::post_event(hwnd, e)
        }));
        if let Err(e) = result {
            core_log.line(&format!("FATAL {e}"));
            ui::post_event(
                hwnd,
                core::UiEvent::Info(format!("Phone Bridge stopped: {e}")),
            );
        }
    });
    log.line("tray started");
    ui::message_loop();
    log.line("tray exiting");
}
