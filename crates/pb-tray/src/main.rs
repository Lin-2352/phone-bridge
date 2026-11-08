//! Phone Bridge tray app for Windows.
//!
//! pb-tray [--dir DIR] [--port N] [--name NAME] [--pair] [--test-auto-accept (test-hooks builds)]
//!
//! If the tray is already running, `--pair` and `--send-clipboard` are
//! forwarded to it (for keyboard shortcuts) and this process exits.
//! `--pair` opens a pairing code at start; `--test-auto-accept` accepts the
//! SAS without asking and exists only for automated tests.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

#[cfg(windows)]
mod a11y;
#[cfg_attr(not(windows), allow(dead_code))]
mod acl;
#[cfg_attr(not(windows), allow(dead_code))]
mod app_name;
#[cfg(windows)]
mod applog;
#[cfg(windows)]
mod applog_sys;
#[cfg(windows)]
mod audio;
#[cfg_attr(not(windows), allow(dead_code))]
mod audio_state;
#[cfg(windows)]
mod camera;
#[cfg(windows)]
mod camera_device;
#[cfg(windows)]
mod camera_fit;
#[cfg(windows)]
mod camera_preview;
#[cfg(windows)]
mod camera_register;
#[cfg_attr(not(windows), allow(dead_code))]
mod camera_status;
#[cfg(windows)]
mod camera_window;
mod clip_history;
#[cfg(windows)]
mod clip_image;
#[cfg(windows)]
mod control;
#[cfg(windows)]
mod core;
#[cfg(windows)]
mod d2d;
#[cfg_attr(not(windows), allow(dead_code))]
mod devices;
#[cfg(windows)]
mod dialog_window;
#[cfg_attr(not(windows), allow(dead_code))]
mod ease;
#[cfg(windows)]
mod fluent;
#[cfg(windows)]
mod flyout;
#[cfg(windows)]
mod flyout_tiles;
#[cfg_attr(not(windows), allow(dead_code))]
mod focus;
#[cfg(windows)]
mod ipc;
#[cfg(windows)]
mod main_window;
#[cfg(windows)]
mod menu;
#[cfg(windows)]
mod menu_model;
#[cfg_attr(not(windows), allow(dead_code))]
mod motw;
#[cfg_attr(not(windows), allow(dead_code))]
mod netpath;
#[cfg(windows)]
mod netsample;
#[cfg_attr(not(windows), allow(dead_code))]
mod notify;
#[cfg(windows)]
mod pages;
#[cfg(windows)]
mod pause;
#[cfg_attr(not(windows), allow(dead_code))]
mod pipe;
#[cfg_attr(not(windows), allow(dead_code))]
mod portable;
#[cfg_attr(not(windows), allow(dead_code))]
mod qr;
#[cfg(windows)]
mod qr_window;
mod queue;
#[cfg(windows)]
mod relaunch;
#[cfg(windows)]
mod screen_decode;
#[cfg_attr(not(windows), allow(dead_code))]
mod screen_end;
#[cfg_attr(not(windows), allow(dead_code))]
mod screen_geom;
#[cfg(windows)]
mod screen_render;
#[cfg(windows)]
mod screen_window;
#[cfg_attr(not(windows), allow(dead_code))]
mod settings;
#[cfg_attr(not(windows), allow(dead_code))]
mod settings_groups;
#[cfg_attr(not(windows), allow(dead_code))]
mod settings_rows;
#[cfg(windows)]
mod share;
#[cfg_attr(not(windows), allow(dead_code))]
mod shared_text;
#[cfg(windows)]
mod status;
#[cfg_attr(not(windows), allow(dead_code))]
mod strings;
#[cfg_attr(not(windows), allow(dead_code))]
mod timeline;
#[cfg(windows)]
#[allow(dead_code)] // used from Task 8 (the main window)
mod titlebar;
#[cfg(windows)]
mod toast;
#[cfg_attr(not(windows), allow(dead_code))]
mod toast_images;
#[cfg_attr(not(windows), allow(dead_code))]
mod toast_xml;
#[cfg(windows)]
mod tooltip;
#[cfg(windows)]
mod transfer_window;
mod transfers;
#[cfg(windows)]
mod uia;
// Generated; some tokens (spacing, other durations) are used by the widgets (pages use them from Task 8 on).
mod look;
#[cfg(windows)]
// Used from Task 11 (the settings page search).
#[allow(dead_code)]
mod search_box;
mod snapshot;
#[allow(dead_code)]
mod tokens;
mod tokens_rules;
#[cfg(windows)]
mod ui;
mod unlock;
#[cfg(windows)]
// Used from Task 8 on (the pages).
#[allow(dead_code)]
mod widgets;

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
    // DLLs Relay loads by name come from System32 only, never from its own
    // folder, where another account could have put one (M1.8).
    #[allow(unsafe_code)]
    // SAFETY: a process-wide setting, made before anything else runs.
    unsafe {
        let _ = windows::Win32::System::LibraryLoader::SetDefaultDllDirectories(
            windows::Win32::System::LibraryLoader::LOAD_LIBRARY_SEARCH_SYSTEM32,
        );
    }
    // `Relay.exe --version` exits at once: CI starts it next to planted DLLs
    // to prove none of them is loaded from Relay's folder.
    if std::env::args().nth(1).as_deref() == Some("--version") {
        return;
    }
    use std::{
        path::PathBuf,
        sync::{Arc, Mutex},
    };

    // The elevated camera helpers run on their own, before anything else.
    {
        let argv: Vec<String> = std::env::args().skip(1).collect();
        if argv.iter().any(|a| a == "--register-camera") {
            let sid = argv
                .iter()
                .position(|a| a == "--for-sid")
                .and_then(|i| argv.get(i + 1))
                .cloned()
                .unwrap_or_default();
            std::process::exit(camera_register::register_elevated(&sid));
        }
        if argv.iter().any(|a| a == "--unregister-camera") {
            std::process::exit(camera_register::unregister_elevated());
        }
    }
    let default_dir = portable::pick_default_dir(
        &std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir),
        |p| p.is_dir(),
    );
    let portable_dir = std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().and_then(portable::data_dir));
    let mut opts = core::Options {
        dir: portable_dir.unwrap_or_else(|| default_dir.clone()),
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
            a if is_test_auto_accept(a) => auto_accept = true,
            "--send-clipboard" => send_clipboard = true,
            "--settings" => ui_request = Some(ipc::Request::Settings),
            "--open" => ui_request = Some(ipc::Request::Open),
            "--menu" => ui_request = Some(ipc::Request::Menu),
            "--show-screen" => ui_request = Some(ipc::Request::ShowScreen),
            "--hide-screen" => ui_request = Some(ipc::Request::HideScreen),
            "--camera-window" => ui_request = Some(ipc::Request::CameraWindow),
            "--start-camera" => ui_request = Some(ipc::Request::StartCamera),
            "--stop-camera" => ui_request = Some(ipc::Request::StopCamera),
            "--audio-phone" => ui_request = Some(ipc::Request::AudioPhone),
            "--audio-pc" => ui_request = Some(ipc::Request::AudioPc),
            a if is_test_audio_synth(a) => {
                audio::TEST_SYNTH.store(true, std::sync::atomic::Ordering::Relaxed)
            }
            // Everything after --send is a file (Explorer "Send to" passes paths).
            "--send" => send_files.extend(args.by_ref().map(PathBuf::from)),
            _ => {}
        }
    }
    relaunch::set(relaunch::args_for(&opts.dir, &default_dir, &opts.name));
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
                let _ = std::process::Command::new(exe)
                    .args(relaunch::args())
                    .spawn();
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
            share.failed("Relay is not running");
        }
        std::process::exit(if ok { 0 } else { 1 });
    }
    if !single_instance() {
        // Hand the request to the running tray over its local pipe.
        let other = opts.pair_on_start || send_clipboard || !send_files.is_empty();
        let ui_request = ipc::second_launch_ui(ui_request, other);
        let mut ok = true;
        if opts.pair_on_start {
            ok &= ipc::send_request(&ipc::Request::Pair);
        }
        if send_clipboard {
            ok &= ipc::send_request(&ipc::Request::SendClipboard);
        }
        if let Some(r) = &ui_request {
            // The running tray raises a window for it: let it take the
            // foreground from this (foreground) process.
            #[allow(unsafe_code)]
            // SAFETY: plain call; failure only means the window may flash in
            // the taskbar instead of coming forward.
            unsafe {
                let _ = windows::Win32::UI::WindowsAndMessaging::AllowSetForegroundWindow(
                    windows::Win32::UI::WindowsAndMessaging::ASFW_ANY,
                );
            }
            ok &= ipc::send_request(r);
        }
        if !send_files.is_empty() {
            ok &= ipc::send_request(&ipc::Request::Send(send_files));
        }
        std::process::exit(if ok { 0 } else { 1 });
    }
    let _ = std::fs::create_dir_all(&opts.dir);
    // Only this user, SYSTEM and Administrators may read or change the data
    // (the identity key and the trusted devices), wherever it was unpacked.
    let protected = ipc::user_sid().is_some_and(|sid| {
        camera_register::set_dacl(&opts.dir, &portable::data_dir_sddl(&sid)).is_ok()
    });
    let log = core::Log::open(opts.dir.join("tray.log"));
    if !protected {
        log.line("DATA folder permissions could not be restricted");
    }
    // Idle almost always: run on efficient cores at low clock.
    if let Err(e) = pb_platform_win::power::set_eco_qos(true) {
        log.line(&format!("EcoQoS unavailable: {e}"));
    }

    let (cmd_tx, cmd_rx) = tokio::sync::mpsc::unbounded_channel();
    let ipc_tx = cmd_tx.clone();
    let peers: core::Peers = Arc::new(Mutex::new(Vec::new()));
    let settings_path = opts.dir.join("settings.txt");
    // Relay ran here before if it already has an identity or pairings: older settings files
    // (no version) are migrated once, so a new default can't move an existing install's folders.
    let migrate_cx = settings::MigrateCtx {
        existing_install: opts.dir.join("peers.txt").exists()
            || opts.dir.join("identity.sealed").exists()
            || opts.dir.join("identity.key").exists(),
        home: std::env::var_os("USERPROFILE")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(std::env::temp_dir),
    };
    let settings: core::SharedSettings = Arc::new(Mutex::new(
        settings::Settings::load_and_migrate(&settings_path, &migrate_cx),
    ));
    fluent::set_theme(settings.lock().map(|s| s.theme).unwrap_or(0));
    fluent::set_look(settings.lock().map(|s| s.look).unwrap_or(0));
    {
        let (want, keep) = settings
            .lock()
            .map(|s| (s.log_dir.clone(), s.keep_logs))
            .unwrap_or_default();
        match applog::init(&want, keep) {
            Some(dir) => {
                applog::install_panic_hook();
                if let Some(lg) = applog::handle() {
                    netsample::start(lg.clone());
                }
                log.line(&format!("LOGS folder {}", dir.display()));
            }
            None => log.line("LOGS could not be opened; structured logs are off"),
        }
    }
    {
        let keep = settings.lock().map(|s| s.keep_history).unwrap_or(true);
        let tl_log = log.clone();
        timeline::init(
            &opts.dir,
            Box::new(pb_platform_win::Dpapi),
            keep,
            move |m| tl_log.line(m),
        );
    }
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
    // Unpacked where other accounts can change it (say C:\Relay): say so once.
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(PathBuf::from));
    if let (Some(sid), Some(dir)) = (ipc::user_sid(), exe_dir) {
        if !acl::folder_is_safe(&dir, &sid) {
            log.line("FOLDER other accounts can change Relay's folder");
            ui::post_event(
                hwnd,
                core::UiEvent::Info(
                    "Other accounts on this PC can change Relay's folder. Move the Relay folder \
                     into your own user folder (for example Downloads)"
                        .into(),
                ),
            );
        }
    }

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
                    ipc::Request::Open => ui::ID_OPEN_MAIN,
                    ipc::Request::Menu => ui::ID_MENU,
                    ipc::Request::ShowScreen => ui::ID_SCREEN_SHOW,
                    ipc::Request::HideScreen => ui::ID_SCREEN_HIDE,
                    ipc::Request::CameraWindow => ui::ID_CAMERA_WINDOW,
                    ipc::Request::StartCamera => ui::ID_CAMERA_START,
                    ipc::Request::StopCamera => ui::ID_CAMERA_STOP,
                    ipc::Request::AudioPhone => menu_model::ID_AUDIO_PHONE,
                    ipc::Request::AudioPc => menu_model::ID_AUDIO_PC,
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
            ui::core_failed(hwnd, e.to_string());
        }
    });
    log.line("tray started");
    ui::message_loop();
    log.line("tray exiting");
    applog::flush();
}

#[cfg_attr(not(windows), allow(dead_code))]
/// `--test-auto-accept` confirms pairing by itself, for automated tests. It
/// exists only in builds with the `test-hooks` feature: a release Relay must
/// never skip that question.
#[cfg(feature = "test-hooks")]
fn is_test_auto_accept(arg: &str) -> bool {
    arg == "--test-auto-accept"
}

#[cfg_attr(not(windows), allow(dead_code))]
#[cfg(not(feature = "test-hooks"))]
fn is_test_auto_accept(_: &str) -> bool {
    false
}

/// `--test-audio-synth` plays test audio and skips the PC's audio question:
/// `test-hooks` builds only.
#[cfg_attr(not(windows), allow(dead_code))]
#[cfg(feature = "test-hooks")]
fn is_test_audio_synth(arg: &str) -> bool {
    arg == "--test-audio-synth"
}

#[cfg_attr(not(windows), allow(dead_code))]
#[cfg(not(feature = "test-hooks"))]
fn is_test_audio_synth(_: &str) -> bool {
    false
}

#[cfg(all(test, not(feature = "test-hooks")))]
mod test_hooks_absent {
    /// A release Relay must never confirm a pairing by itself.
    /// Test audio skips the PC's audio question: never in a release.
    #[test]
    fn test_audio_synth_flag_is_ignored_without_test_hooks() {
        assert!(!super::is_test_audio_synth("--test-audio-synth"));
    }

    #[test]
    fn test_auto_accept_flag_is_ignored_without_test_hooks() {
        assert!(!super::is_test_auto_accept("--test-auto-accept"));
    }
}
