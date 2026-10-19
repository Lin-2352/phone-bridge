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
mod control;
#[cfg(windows)]
mod core;
#[cfg(windows)]
mod fluent;
#[cfg(windows)]
mod flyout;
#[cfg(windows)]
mod ipc;
#[cfg(windows)]
mod menu;
#[cfg_attr(not(windows), allow(dead_code))]
mod qr;
#[cfg(windows)]
mod qr_window;
#[cfg(windows)]
mod settings;
#[cfg(windows)]
mod settings_window;
#[cfg(windows)]
mod share;
#[cfg(windows)]
mod status;
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
    let mut ui_request: Option<ipc::Request> = None;
    let mut send_files: Vec<PathBuf> = Vec::new();
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
            "--settings" => ui_request = Some(ipc::Request::Settings),
            "--open" => ui_request = Some(ipc::Request::Open),
            "--menu" => ui_request = Some(ipc::Request::Menu),
            // Everything after --send is a file (Explorer "Send to" passes paths).
            "--send" => send_files.extend(args.by_ref().map(PathBuf::from)),
            _ => {}
        }
    }
    // Crisp text and correct sizes on scaled displays.
    #[allow(unsafe_code)]
    // SAFETY: called once at startup before any window exists.
    unsafe {
        let _ = windows::Win32::UI::HiDpi::SetProcessDpiAwarenessContext(
            windows::Win32::UI::HiDpi::DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
        );
    }
    // Started by the Windows share sheet: hand the item to the running tray
    // (starting it if needed), tell Windows we are done, and exit.
    if let Some(share) = share::take() {
        let req = match &share.item {
            share::Shared::Files(f) if f.is_empty() => {
                share.failed("Only files on this PC can be sent");
                std::process::exit(1);
            }
            share::Shared::Files(f) => ipc::Request::Send(f.clone()),
            share::Shared::Text(t) => ipc::Request::Text {
                link: false,
                text: t.clone(),
            },
            share::Shared::Link(u) => ipc::Request::Text {
                link: true,
                text: u.clone(),
            },
        };
        let mut ok = ipc::send_request(&req);
        if !ok {
            if let Ok(exe) = std::env::current_exe() {
                let _ = std::process::Command::new(exe).spawn();
            }
            for _ in 0..50 {
                std::thread::sleep(std::time::Duration::from_millis(100));
                if ipc::send_request(&req) {
                    ok = true;
                    break;
                }
            }
        }
        if ok {
            share.done();
        } else {
            share.failed("Phone Bridge is not running");
        }
        std::process::exit(if ok { 0 } else { 1 });
    }
    if !single_instance() {
        // Hand the request to the running tray over its local pipe.
        let mut ok = true;
        if opts.pair_on_start {
            ok &= ipc::send_request(&ipc::Request::Pair);
        }
        if send_clipboard {
            ok &= ipc::send_request(&ipc::Request::SendClipboard);
        }
        if let Some(r) = &ui_request {
            ok &= ipc::send_request(r);
        }
        if !send_files.is_empty() {
            ok &= ipc::send_request(&ipc::Request::Send(send_files));
        }
        std::process::exit(if ok { 0 } else { 1 });
    }
    let _ = std::fs::create_dir_all(&opts.dir);
    let log = core::Log::open(opts.dir.join("tray.log"));
    // Idle almost always: run on efficient cores at low clock.
    if let Err(e) = pb_platform_win::power::set_eco_qos(true) {
        log.line(&format!("EcoQoS unavailable: {e}"));
    }

    let (cmd_tx, cmd_rx) = tokio::sync::mpsc::unbounded_channel();
    let ipc_tx = cmd_tx.clone();
    let peers: core::Peers = Arc::new(Mutex::new(Vec::new()));
    let settings_path = opts.dir.join("settings.txt");
    let settings: core::SharedSettings =
        Arc::new(Mutex::new(settings::Settings::load(&settings_path)));
    fluent::set_theme(settings.lock().map(|s| s.theme).unwrap_or(0));
    let hwnd = match ui::create(
        cmd_tx.clone(),
        peers.clone(),
        auto_accept,
        log.clone(),
        settings.clone(),
        settings_path,
    ) {
        Ok(h) => h,
        Err(e) => {
            log.line(&format!("could not create tray icon: {e}"));
            return;
        }
    };

    // Started by Explorer "Send to" (or with --send) while no tray ran:
    // queue the files; the core sends them once it is up.
    if !send_files.is_empty() {
        let _ = cmd_tx.send(core::Cmd::SendFiles(send_files));
    }

    let core_log = log.clone();
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("tokio runtime");
        rt.spawn(ipc::serve(ipc_tx, move |r| {
            ui::post_command(
                hwnd,
                match r {
                    ipc::Request::Settings => ui::ID_SETTINGS,
                    ipc::Request::Open => ui::ID_OPEN_FLYOUT,
                    ipc::Request::Menu => ui::ID_MENU,
                    _ => ui::ID_SEND_CLIPBOARD,
                },
            )
        }));
        let result = rt.block_on(core::run(
            opts,
            cmd_rx,
            peers,
            settings,
            core_log.clone(),
            move |e| ui::post_event(hwnd, e),
        ));
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
