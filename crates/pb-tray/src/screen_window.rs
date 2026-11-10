//! The phone-screen window: the phone's screen, live, with a top bar (name,
//! status, Pin, Dim) and a bottom bar (Back, Home, Recents). Mouse and keys
//! on the video drive the phone. Decoding and presenting run on their own
//! thread; the window thread only handles input and layout.
//!
//! The video and each bar are child windows with their own swap chains, so
//! the D3D11 video and the Direct2D bars never share one window. The bars
//! are see-through to the mouse: clicks land on the main window, which
//! hit-tests the bar buttons as before.
#![allow(unsafe_code)]

use std::{
    cell::RefCell,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc, Mutex,
    },
    time::{Duration, Instant},
};

use pb_protocol::{InputEvent, TouchPhase};
use pb_transport::screen::{ScreenRemote, ScreenView};
use pb_video::{
    record::{Config, Record, FLAG_CONFIG},
    Codec,
};
use windows::{
    core::{w, PCWSTR},
    Win32::{
        Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
        Graphics::Gdi::{InvalidateRect, ScreenToClient, ValidateRect},
        System::LibraryLoader::GetModuleHandleW,
        UI::{
            Input::KeyboardAndMouse::{
                GetKeyState, ReleaseCapture, SetCapture, SetFocus, VK_CONTROL, VK_ESCAPE, VK_MENU,
            },
            Shell::{DragAcceptFiles, DragFinish, DragQueryFileW, HDROP},
            WindowsAndMessaging::{
                AdjustWindowRectEx, CreateWindowExW, DefWindowProcW, DestroyWindow,
                DispatchMessageW, GetClientRect, GetMessageW, GetSystemMetrics, GetWindowRect,
                LoadCursorW, MoveWindow, PostMessageW, PostQuitMessage, RegisterClassW,
                SetWindowPos, ShowWindow, TranslateMessage, CS_HREDRAW, CS_VREDRAW, HWND_NOTOPMOST,
                HWND_TOPMOST, IDC_ARROW, MSG, SIZE_MINIMIZED, SM_CYSCREEN, SWP_NOMOVE, SWP_NOSIZE,
                SWP_NOZORDER, SW_SHOW, WA_INACTIVE, WINDOW_EX_STYLE, WM_ACTIVATE, WM_APP, WM_CHAR,
                WM_CLOSE, WM_DESTROY, WM_DROPFILES, WM_ERASEBKGND, WM_KEYDOWN, WM_KEYUP,
                WM_KILLFOCUS, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MBUTTONUP, WM_MOUSEMOVE,
                WM_MOUSEWHEEL, WM_NCHITTEST, WM_PAINT, WM_RBUTTONUP, WM_SIZE, WM_SIZING, WNDCLASSW,
                WS_CHILD, WS_CLIPCHILDREN, WS_OVERLAPPEDWINDOW, WS_VISIBLE,
            },
        },
    },
};

use crate::{
    d2d::{Backdrop, Surface},
    fluent::{self, from_rect, px, rectf, Align, Canvas, Font},
    screen_decode::{Decoder, Frame},
    screen_geom::{
        fit_window, map_point, on_decode_error, state_changed, video_rect, Edge, HeldKeys, Next,
    },
    screen_render::{device_for_window, Renderer},
};

const CLASS: PCWSTR = w!("PhoneBridgeScreen");
const VIDEO_CLASS: PCWSTR = w!("PhoneBridgeScreenVideo");
const BAR_CLASS: PCWSTR = w!("PhoneBridgeScreenBar");
/// WM_NCHITTEST: let the mouse through to the window underneath.
const HTTRANSPARENT: isize = -1;
const WM_REDRAW: u32 = WM_APP + 1;
const WM_ENDED: u32 = WM_APP + 2;
const WM_GIVE_UP: u32 = WM_APP + 3;
const WM_VIDEO_SIZE: u32 = WM_APP + 4;
const TOP: i32 = 44;
const BOTTOM: i32 = 52;
const FEEDBACK: Duration = Duration::from_millis(100);

/// What the tray gives the window.
pub struct WindowOpts {
    /// Timing lines for tray.log (no content).
    pub log: Box<dyn Fn(&str) + Send + Sync>,
    pub name: String,
    pub pinned: bool,
    pub dim: bool,
    /// The PC's codecs, best first (for fallback).
    pub codecs: Vec<Codec>,
    pub on_pin: Box<dyn Fn(bool) + Send + Sync>,
    /// Dim was switched in the toolbar: remembered for the next window.
    pub on_dim: Box<dyn Fn(bool) + Send + Sync>,
    pub on_files: Box<dyn Fn(Vec<PathBuf>) + Send + Sync>,
    /// The session ended; the text says why (for the status pill).
    pub on_ended: Box<dyn Fn(String) + Send + Sync>,
    /// The user closed the window while the picture was live.
    pub on_closed: Box<dyn Fn() + Send + Sync>,
}

enum Msg {
    Record(Record),
    Resize(u32, u32),
    Ended,
    Stop,
}

#[derive(Clone, Copy, PartialEq)]
enum Target {
    Touch,
    Pin,
    Dim,
    Back,
    Home,
    Recents,
}

struct Shared {
    remote: Arc<ScreenRemote>,
    msgs: mpsc::Sender<Msg>,
    opts: Arc<WindowOpts>,
    hwnd: isize,
    video_hwnd: isize,
    /// The top and bottom bar windows.
    bars: [isize; 2],
    /// Current video size (changes with rotation).
    video: (u32, u32),
    pinned: bool,
    dim: bool,
    /// Touch on: the mouse and keys on the video drive the phone. Off: view only.
    touch: bool,
    status: String,
    dragging: bool,
    last_touch: (u16, u16),
    held: HeldKeys,
    /// The last (active, dim) sent to the phone.
    sent: Option<(bool, bool)>,
    active: bool,
    /// Window size minus client size (for WM_SIZING).
    frame: (i32, i32),
}

static SHARED: Mutex<Option<Shared>> = Mutex::new(None);
static FOCUSED: AtomicBool = AtomicBool::new(false);
/// Frames arrive: set on each decoded frame, cleared when the phone's
/// stream ends ("Reconnecting…") and when a new window opens.
static STREAMING: AtomicBool = AtomicBool::new(false);

thread_local! {
    /// The bars' Direct2D surfaces, by bar window (on the window thread).
    static BAR_SURFACES: RefCell<Vec<(isize, Surface)>> = const { RefCell::new(Vec::new()) };
}

/// The window is open and the phone's picture is live (not reconnecting).
pub fn is_streaming() -> bool {
    is_open() && STREAMING.load(Ordering::Relaxed)
}

/// The last notice of the current session (its end reason, when the phone gave one).
static LAST_NOTICE: std::sync::Mutex<String> = std::sync::Mutex::new(String::new());
fn with<R>(f: impl FnOnce(&mut Shared) -> R) -> Option<R> {
    SHARED.lock().ok()?.as_mut().map(f)
}

/// The window has the keyboard focus (notifications stay quiet then).
pub fn is_focused() -> bool {
    FOCUSED.load(Ordering::Relaxed)
}

/// Close the window if one is open (the tray's toggle).
pub fn close() {
    if let Some(h) = with(|s| s.hwnd) {
        // SAFETY: posting to our own window.
        let _ = unsafe { PostMessageW(Some(HWND(h as *mut _)), WM_CLOSE, WPARAM(0), LPARAM(0)) };
    }
}

pub fn is_open() -> bool {
    with(|_| ()).is_some()
}

fn set_status(text: &str) {
    if let Some(h) = with(|s| {
        s.status = text.to_owned();
        s.hwnd
    }) {
        // SAFETY: posting to our own window.
        let _ = unsafe { PostMessageW(Some(HWND(h as *mut _)), WM_REDRAW, WPARAM(0), LPARAM(0)) };
    }
}

/// Open the window for a running session (replaces an open one).
pub fn open(view: ScreenView, opts: WindowOpts, rt: &tokio::runtime::Handle) {
    close();
    STREAMING.store(false, Ordering::Relaxed);
    let (remote, mut records, mut state) = view.into_parts();
    let remote = Arc::new(remote);
    let (tx, rx) = mpsc::channel::<Msg>();
    let fwd = tx.clone();
    if let Ok(mut n) = LAST_NOTICE.lock() {
        n.clear();
    }
    let (state_done, state_done_rx) = tokio::sync::oneshot::channel::<()>();
    rt.spawn(async move {
        while let Some(r) = records.recv().await {
            if fwd.send(Msg::Record(r)).is_err() {
                return;
            }
        }
        // The phone's last notice says why it ended (a lock): let it land.
        let _ = tokio::time::timeout(std::time::Duration::from_secs(1), state_done_rx).await;
        let _ = fwd.send(Msg::Ended);
    });
    rt.spawn(async move {
        while let Some(st) = state.recv().await {
            if let Ok(mut n) = LAST_NOTICE.lock() {
                n.clone_from(&st.notice);
            }
            // An empty notice clears it (back to full quality).
            set_status(&st.notice);
        }
        let _ = state_done.send(());
    });
    std::thread::spawn(move || ui_thread(remote, tx, rx, Arc::new(opts)));
}

fn ui_thread(
    remote: Arc<ScreenRemote>,
    tx: mpsc::Sender<Msg>,
    rx: mpsc::Receiver<Msg>,
    opts: Arc<WindowOpts>,
) {
    let video = (remote.accept.width, remote.accept.height);
    let codec = Codec::from_u32(remote.accept.codec).unwrap_or(Codec::H264);
    // SAFETY: window creation and the message loop on this thread.
    unsafe {
        let Ok(module) = GetModuleHandleW(None) else {
            return;
        };
        let cursor = LoadCursorW(None, IDC_ARROW).unwrap_or_default();
        RegisterClassW(&WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(wndproc),
            hInstance: module.into(),
            lpszClassName: CLASS,
            hCursor: cursor,
            ..Default::default()
        });
        RegisterClassW(&WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(video_proc),
            hInstance: module.into(),
            lpszClassName: VIDEO_CLASS,
            hCursor: cursor,
            ..Default::default()
        });
        RegisterClassW(&WNDCLASSW {
            lpfnWndProc: Some(bar_proc),
            hInstance: module.into(),
            lpszClassName: BAR_CLASS,
            hCursor: cursor,
            ..Default::default()
        });
        // Video about 70% of the screen height, plus the bars.
        let vh = (GetSystemMetrics(SM_CYSCREEN) as f32 * 0.7) as i32;
        let vw = (vh as f32 * video.0 as f32 / video.1.max(1) as f32) as i32;
        let mut r = RECT {
            left: 0,
            top: 0,
            right: vw,
            bottom: vh + TOP + BOTTOM,
        };
        let _ = AdjustWindowRectEx(&mut r, WS_OVERLAPPEDWINDOW, false, WINDOW_EX_STYLE(0));
        let title: Vec<u16> = opts.name.encode_utf16().chain(Some(0)).collect();
        let Ok(hwnd) = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            CLASS,
            PCWSTR(title.as_ptr()),
            WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN,
            120,
            60,
            r.right - r.left,
            r.bottom - r.top,
            None,
            None,
            Some(module.into()),
            None,
        ) else {
            return;
        };
        let Ok(video_hwnd) = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            VIDEO_CLASS,
            w!(""),
            WS_CHILD | WS_VISIBLE,
            0,
            TOP,
            vw,
            vh,
            Some(hwnd),
            None,
            Some(module.into()),
            None,
        ) else {
            let _ = DestroyWindow(hwnd);
            return;
        };
        // Sized and placed by `layout`.
        let bar = || {
            CreateWindowExW(
                WINDOW_EX_STYLE(0),
                BAR_CLASS,
                w!(""),
                WS_CHILD | WS_VISIBLE,
                0,
                0,
                vw,
                TOP,
                Some(hwnd),
                None,
                Some(module.into()),
                None,
            )
        };
        let (Ok(top_bar), Ok(bottom_bar)) = (bar(), bar()) else {
            let _ = DestroyWindow(hwnd);
            return;
        };
        let mut wr = RECT::default();
        let mut cr = RECT::default();
        let _ = GetWindowRect(hwnd, &mut wr);
        let _ = GetClientRect(hwnd, &mut cr);
        *SHARED.lock().unwrap_or_else(|p| p.into_inner()) = Some(Shared {
            remote: remote.clone(),
            msgs: tx,
            opts: opts.clone(),
            hwnd: hwnd.0 as isize,
            video_hwnd: video_hwnd.0 as isize,
            bars: [top_bar.0 as isize, bottom_bar.0 as isize],
            video,
            pinned: opts.pinned,
            touch: true,
            dim: opts.dim,
            status: String::new(),
            dragging: false,
            last_touch: (0, 0),
            held: HeldKeys::default(),
            sent: None,
            active: true,
            frame: (
                (wr.right - wr.left) - cr.right,
                (wr.bottom - wr.top) - cr.bottom,
            ),
        });
        fluent::frame(hwnd, false);
        for h in [hwnd, top_bar, bottom_bar] {
            fluent::register_window(h);
        }
        DragAcceptFiles(hwnd, true);
        if opts.pinned {
            let _ = SetWindowPos(
                hwnd,
                Some(HWND_TOPMOST),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE,
            );
        }
        let _ = ShowWindow(hwnd, SW_SHOW);
        layout(hwnd);
        let (top, vid) = (hwnd.0 as isize, video_hwnd.0 as isize);
        let remote2 = remote.clone();
        let opts2 = opts.clone();
        std::thread::spawn(move || decode_thread(top, vid, rx, remote2, codec, video, opts2));
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    *SHARED.lock().unwrap_or_else(|p| p.into_inner()) = None;
    FOCUSED.store(false, Ordering::Relaxed);
}

fn decode_thread(
    top: isize,
    video_hwnd: isize,
    rx: mpsc::Receiver<Msg>,
    remote: Arc<ScreenRemote>,
    mut codec: Codec,
    mut video: (u32, u32),
    opts: Arc<WindowOpts>,
) {
    let opened = Instant::now();
    let mut first_record = true;
    let mut first_frame = true;
    // Per-second: frames presented, decode+present time.
    let mut stat_since = Instant::now();
    let mut stat_frames = 0u32;
    let mut stat_records = 0u32;
    let mut stat_skipped = 0u32;
    let mut stat_ms: Vec<f64> = Vec::new();
    // Messages taken early to see whether newer frames wait (latest wins).
    let mut backlog = std::collections::VecDeque::<Msg>::new();
    // The newest decoded frame was not presented yet.
    let mut unshown = false;
    let post = |m: u32| {
        // SAFETY: posting to our own window.
        let _ = unsafe { PostMessageW(Some(HWND(top as *mut _)), m, WPARAM(0), LPARAM(0)) };
    };
    let device = match device_for_window(HWND(top as *mut _)) {
        Ok(d) => d,
        Err(_) => return post(WM_GIVE_UP),
    };
    let Ok(mut renderer) = Renderer::new(HWND(video_hwnd as *mut _), device.clone()) else {
        return post(WM_GIVE_UP);
    };
    let Ok(mut decoder) = Decoder::new(&device, codec) else {
        return post(WM_GIVE_UP);
    };
    let mut size = (0u32, 0u32);
    let mut last: Option<Frame> = None;
    let mut last_pts = 0i64;
    let mut last_feedback = Instant::now();
    let mut dimmed = false;
    let mut secure = false;
    let present =
        |renderer: &mut Renderer, f: &Frame, size: (u32, u32), video: (u32, u32), dimmed: bool| {
            let (x, y, w, h) = video_rect(size, video);
            let dest = RECT {
                left: x,
                top: y,
                right: x + w as i32,
                bottom: y + h as i32,
            };
            let _ = renderer.present(f, video, dest, dimmed);
        };
    loop {
        let next = match backlog.pop_front() {
            Some(m) => Ok(m),
            None => rx.recv_timeout(FEEDBACK),
        };
        match next {
            Ok(Msg::Record(r)) => {
                if first_record {
                    first_record = false;
                    (opts.log)(&format!(
                        "SCREEN first record after {} ms",
                        opened.elapsed().as_millis()
                    ));
                }
                let t = Instant::now();
                stat_records += 1;
                if r.flags & FLAG_CONFIG != 0 {
                    if let Some(c) = Config::decode(&r.payload) {
                        let v = (u32::from(c.width), u32::from(c.height));
                        if v != video {
                            video = v;
                            with(|s| s.video = v);
                            post(WM_VIDEO_SIZE);
                        }
                    }
                }
                match decoder.push(&r) {
                    Ok(Some(f)) => {
                        STREAMING.store(true, Ordering::Relaxed);
                        // Every frame is decoded (later ones refer to it),
                        // but a present waits for the monitor's refresh: with
                        // newer frames already waiting only the newest is
                        // shown, so a backlog never builds up.
                        while let Ok(m) = rx.try_recv() {
                            backlog.push_back(m);
                        }
                        if backlog.iter().any(|m| matches!(m, Msg::Record(_))) {
                            unshown = true;
                            stat_skipped += 1;
                        } else {
                            present(&mut renderer, &f, size, video, dimmed);
                            unshown = false;
                            stat_frames += 1;
                        }
                        last_pts = f.pts_us;
                        // Confirm every frame at once: the phone's pacer
                        // counts unconfirmed frames as lag.
                        remote.feedback(last_pts);
                        last_feedback = Instant::now();
                        if first_frame {
                            first_frame = false;
                            (opts.log)(&format!(
                                "SCREEN first frame shown after {} ms",
                                opened.elapsed().as_millis()
                            ));
                        }
                        stat_ms.push(t.elapsed().as_secs_f64() * 1000.0);
                        if stat_since.elapsed() >= Duration::from_secs(1) {
                            stat_ms.sort_by(|a, b| a.total_cmp(b));
                            let p = |q: f64| {
                                stat_ms
                                    .get(((stat_ms.len() as f64 - 1.0) * q) as usize)
                                    .copied()
                                    .unwrap_or(0.0)
                            };
                            (opts.log)(&format!(
                                "SCREEN stats received={}/s shown={}/s skipped={}/s decode+present p50={:.1} p95={:.1} ms",
                                stat_records,
                                stat_frames,
                                stat_skipped,
                                p(0.5),
                                p(0.95)
                            ));
                            stat_frames = 0;
                            stat_records = 0;
                            stat_skipped = 0;
                            stat_ms.clear();
                            stat_since = Instant::now();
                        }
                        if let Some(black) = renderer.mean_brightness(&f) {
                            if black != secure {
                                secure = black;
                                set_status(if black {
                                    "Hidden by the app (secure screen)"
                                } else {
                                    ""
                                });
                            }
                        }
                        last = Some(f);
                    }
                    Ok(None) => {}
                    Err(_) => match on_decode_error(codec, &opts.codecs) {
                        Next::Retry(c) => {
                            remote.codec_failed();
                            codec = c;
                            match Decoder::new(&device, c) {
                                Ok(d) => decoder = d,
                                Err(_) => return post(WM_GIVE_UP),
                            }
                        }
                        Next::GiveUp => return post(WM_GIVE_UP),
                    },
                }
            }
            Ok(Msg::Resize(w, h)) => {
                renderer.resize(w, h);
                size = (w, h);
                if let Some(f) = &last {
                    present(&mut renderer, f, size, video, dimmed);
                }
            }
            Ok(Msg::Ended) => {
                STREAMING.store(false, Ordering::Relaxed);
                dimmed = true;
                if let Some(f) = &last {
                    present(&mut renderer, f, size, video, true);
                }
                post(WM_ENDED);
            }
            Ok(Msg::Stop) | Err(mpsc::RecvTimeoutError::Disconnected) => return,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
        // The frames that were waiting gave no picture (a config record):
        // the newest decoded one still has to be seen.
        if unshown && !backlog.iter().any(|m| matches!(m, Msg::Record(_))) {
            if let Some(f) = &last {
                present(&mut renderer, f, size, video, dimmed);
                stat_frames += 1;
            }
            unshown = false;
        }
        if last_pts > 0 && last_feedback.elapsed() >= FEEDBACK {
            remote.feedback(last_pts);
            last_feedback = Instant::now();
        }
    }
}

/// Bars and their buttons, for painting and clicks alike.
fn targets(c: RECT, s: f32) -> Vec<(RECT, Target)> {
    let (top, bottom) = (px(s, TOP), px(s, BOTTOM));
    let pad = px(s, 6);
    // Up to 72 px each, narrower in a narrow window (the name gives way first).
    let bw = px(s, 72).min(((c.right - px(s, 80)) / 3 - pad).max(px(s, 36)));
    let mut v = vec![
        (
            RECT {
                left: c.right - 3 * bw - 3 * pad,
                top: pad,
                right: c.right - 2 * bw - 3 * pad,
                bottom: top - pad,
            },
            Target::Touch,
        ),
        (
            RECT {
                left: c.right - 2 * bw - 2 * pad,
                top: pad,
                right: c.right - bw - 2 * pad,
                bottom: top - pad,
            },
            Target::Pin,
        ),
        (
            RECT {
                left: c.right - bw - pad,
                top: pad,
                right: c.right - pad,
                bottom: top - pad,
            },
            Target::Dim,
        ),
    ];
    let third = (c.right - 4 * pad) / 3;
    let y0 = c.bottom - bottom + pad;
    for (i, t) in [Target::Back, Target::Home, Target::Recents]
        .into_iter()
        .enumerate()
    {
        let left = pad + i as i32 * (third + pad);
        v.push((
            RECT {
                left,
                top: y0,
                right: left + third,
                bottom: c.bottom - pad,
            },
            t,
        ));
    }
    v
}

fn layout(hwnd: HWND) {
    let Some((video_hwnd, bars, msgs)) = with(|s| (s.video_hwnd, s.bars, s.msgs.clone())) else {
        return;
    };
    let s = fluent::scale(hwnd);
    let mut c = RECT::default();
    // SAFETY: sizing our own child windows.
    unsafe {
        let _ = GetClientRect(hwnd, &mut c);
        let (top, bottom) = (px(s, TOP), px(s, BOTTOM));
        let h = (c.bottom - top - bottom).max(1);
        let _ = MoveWindow(HWND(video_hwnd as *mut _), 0, top, c.right, h, true);
        let _ = MoveWindow(HWND(bars[0] as *mut _), 0, 0, c.right, top, true);
        let _ = MoveWindow(
            HWND(bars[1] as *mut _),
            0,
            c.bottom - bottom,
            c.right,
            bottom,
            true,
        );
        let _ = msgs.send(Msg::Resize(c.right.max(1) as u32, h as u32));
    }
}

/// Repaint both bars (what they show changed).
fn redraw_bars() {
    for h in with(|s| s.bars).unwrap_or_default() {
        // SAFETY: invalidating our own child window (any thread may).
        let _ = unsafe { InvalidateRect(Some(HWND(h as *mut _)), None, false) };
    }
}

/// One of the two toggles in the top bar. Its glyph (Segoe Fluent Icons) is what a narrow bar shows
/// alone; a wide bar writes `label` beside it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BarItem {
    pub glyph: char,
    pub label: &'static str,
    /// Stays at every width (nothing is dropped from the bar, it only loses its words).
    pub essential: bool,
}

/// Pin (E718) and Brightness (E706) from Segoe Fluent Icons.
const PIN_GLYPH: char = '\u{E718}';
const DIM_GLYPH: char = '\u{E706}';

/// The top bar's toggles at a bar `width` in pixels, each with the words it can afford. Below 60 px a
/// button shows only its icon (the old emoji labels are gone).
pub fn bar_labels(button_width: f32, pinned: bool, dimmed: bool) -> Vec<BarItem> {
    let narrow = button_width < 60.0;
    vec![
        BarItem {
            glyph: PIN_GLYPH,
            label: if narrow {
                ""
            } else if pinned {
                "Pinned"
            } else {
                "Pin"
            },
            essential: true,
        },
        BarItem {
            glyph: DIM_GLYPH,
            label: if narrow {
                ""
            } else if dimmed {
                "Dimmed"
            } else {
                "Dim"
            },
            essential: true,
        },
    ]
}

/// Touch (a pointer glyph from Segoe Fluent Icons) beside Pin and Dim.
const TOUCH_GLYPH: char = '\u{E7C9}';

/// The Touch toggle: "Touch" while the mouse and keys drive the phone, "View only" when they do not.
pub fn touch_item(button_width: f32, on: bool) -> BarItem {
    let narrow = button_width < 60.0;
    BarItem {
        glyph: TOUCH_GLYPH,
        label: if narrow {
            ""
        } else if on {
            "Touch"
        } else {
            "View only"
        },
        essential: true,
    }
}

/// With Touch off the video ignores the mouse, the wheel and typing (Ctrl shortcuts such as Ctrl+W still work).
fn blocks_input(view_only: bool, msg: u32, ctrl_down: bool) -> bool {
    if !view_only {
        return false;
    }
    match msg {
        WM_LBUTTONDOWN | WM_MOUSEMOVE | WM_LBUTTONUP | WM_RBUTTONUP | WM_MBUTTONUP
        | WM_MOUSEWHEEL | WM_CHAR => true,
        WM_KEYDOWN | WM_KEYUP => !ctrl_down,
        _ => false,
    }
}

/// What the bars show.
#[derive(Default)]
struct BarView {
    name: String,
    status: String,
    pinned: bool,
    dim: bool,
    touch: bool,
}

/// Both bars as laid out in the main window's client area `c`; each bar
/// window draws this shifted to its own place, and shows its part.
fn draw_bars(cv: &mut Canvas, c: RECT, v: &BarView) {
    let s = cv.s;
    let p = cv.p;
    let line = if v.status.is_empty() {
        v.name.clone()
    } else {
        format!("{}  \u{00b7}  {}", v.name, v.status)
    };
    let right = targets(c, s).first().map_or(c.right, |(r, _)| r.left) - px(s, 8);
    let tr = rectf(px(s, 12) as f32, 0.0, right as f32, px(s, TOP) as f32);
    fluent::text(cv, Font::Body, &line, tr, p.text, Align::Left);
    for (r, t) in targets(c, s) {
        let width = (r.right - r.left) as f32 / s;
        let items = bar_labels(width, v.pinned, v.dim);
        let (item, on) = match t {
            Target::Touch => (Some(touch_item(width, v.touch)), v.touch),
            Target::Pin => (Some(items[0]), v.pinned),
            Target::Dim => (Some(items[1]), v.dim),
            Target::Back => (None, false),
            Target::Home => (None, false),
            Target::Recents => (None, false),
        };
        let rect = from_rect(r);
        match item {
            // Icon and words when there is room, the icon alone when there is not.
            Some(item) if item.label.is_empty() => {
                fluent::button(cv, rect, "", on);
                fluent::icon(cv, item.glyph, rect, if on { p.on_primary } else { p.text });
            }
            Some(item) => fluent::button(cv, rect, item.label, on),
            None => fluent::button(
                cv,
                rect,
                match t {
                    Target::Back => "Back",
                    Target::Home => "Home",
                    _ => "Recents",
                },
                false,
            ),
        }
    }
}

fn paint_bar(bar: HWND) {
    let Some((top, view)) = with(|sh| {
        let view = BarView {
            name: sh.opts.name.clone(),
            status: sh.status.clone(),
            pinned: sh.pinned,
            dim: sh.dim,
            touch: sh.touch,
        };
        (HWND(sh.hwnd as *mut _), view)
    }) else {
        // SAFETY: our own window; nothing to show yet.
        let _ = unsafe { ValidateRect(Some(bar), None) };
        return;
    };
    let mut c = RECT::default();
    let mut br = RECT::default();
    // SAFETY: plain queries on our own windows.
    unsafe {
        let _ = GetClientRect(top, &mut c);
        let _ = GetWindowRect(bar, &mut br);
    }
    let mut origin = POINT {
        x: br.left,
        y: br.top,
    };
    // SAFETY: plain conversion on our own window.
    let _ = unsafe { ScreenToClient(top, &mut origin) };
    BAR_SURFACES.with(|cell| {
        let mut list = cell.borrow_mut();
        if !list.iter().any(|(h, _)| *h == bar.0 as isize) {
            if let Ok(surface) = Surface::new(bar, Backdrop::Opaque) {
                list.push((bar.0 as isize, surface));
            }
        }
        match list.iter_mut().find(|(h, _)| *h == bar.0 as isize) {
            Some((_, surface)) => fluent::paint_with(surface, bar, |cv, _| {
                // SAFETY: a state call inside the surface's BeginDraw/EndDraw.
                unsafe {
                    cv.dc.SetTransform(&windows_numerics::Matrix3x2 {
                        M11: 1.0,
                        M12: 0.0,
                        M21: 0.0,
                        M22: 1.0,
                        M31: -origin.x as f32,
                        M32: -origin.y as f32,
                    });
                }
                draw_bars(cv, c, &view);
            }),
            // Nothing can draw (no Direct2D at all): validate so WM_PAINT
            // does not repeat forever.
            // SAFETY: our own window.
            None => unsafe {
                let _ = ValidateRect(Some(bar), None);
            },
        }
    });
}

/// A bar: drawn with Direct2D, see-through to the mouse.
extern "system" fn bar_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_NCHITTEST => LRESULT(HTTRANSPARENT),
        WM_ERASEBKGND => LRESULT(1),
        WM_PAINT => {
            paint_bar(hwnd);
            LRESULT(0)
        }
        WM_DESTROY => {
            fluent::unregister_window(hwnd);
            let gone = BAR_SURFACES.with(|cell| {
                let mut list = cell.borrow_mut();
                list.iter()
                    .position(|(h, _)| *h == hwnd.0 as isize)
                    .map(|i| list.remove(i))
            });
            drop(gone);
            LRESULT(0)
        }
        // SAFETY: default handling.
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

/// Tell the phone the window's state, only when it changed.
fn push_state() {
    let change = with(|s| {
        let next = (s.active, s.dim);
        if state_changed(s.sent, next) {
            s.sent = Some(next);
            Some((s.remote.clone(), next))
        } else {
            None
        }
    })
    .flatten();
    if let Some((r, (active, dim))) = change {
        r.set_state(active, dim);
    }
}

fn send(e: InputEvent) {
    if let Some(r) = with(|s| s.remote.clone()) {
        r.input.send(e);
    }
}

/// Focus lost or closing: lift every key and the finger still down on the
/// phone (their key-ups and button-up would go to another window).
fn release_input() {
    let (keys, finger) = with(|s| {
        let finger = std::mem::take(&mut s.dragging).then_some(s.last_touch);
        (s.held.drain(), finger)
    })
    .unwrap_or_default();
    for scancode in keys {
        send(InputEvent::Key {
            scancode,
            pressed: false,
        });
    }
    if let Some((x, y)) = finger {
        // SAFETY: releasing our own capture.
        let _ = unsafe { ReleaseCapture() };
        send(InputEvent::Touch {
            phase: TouchPhase::Up,
            x,
            y,
        });
    }
}

fn click_button(b: u8) {
    send(InputEvent::MouseButton {
        button: b,
        pressed: true,
    });
    send(InputEvent::MouseButton {
        button: b,
        pressed: false,
    });
}

extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    // SAFETY: standard window procedure for our own window.
    unsafe {
        match msg {
            WM_SIZE => {
                let active = wparam.0 as u32 != SIZE_MINIMIZED;
                // Minimised: the phone pauses its capture. Resizes send nothing.
                with(|s| s.active = active);
                push_state();
                if active {
                    layout(hwnd);
                    redraw_bars();
                }
                LRESULT(0)
            }
            WM_SIZING => {
                let r = &mut *(lparam.0 as *mut RECT);
                let s = fluent::scale(hwnd);
                if let Some((video, frame)) = with(|sh| (sh.video, sh.frame)) {
                    let bars = px(s, TOP) + px(s, BOTTOM);
                    let proposed = (
                        (r.right - r.left - frame.0).max(1) as u32,
                        (r.bottom - r.top - frame.1 - bars).max(1) as u32,
                    );
                    let edge = match wparam.0 {
                        1 => Edge::Left,
                        2 => Edge::Right,
                        3 => Edge::Top,
                        4 => Edge::TopLeft,
                        5 => Edge::TopRight,
                        6 => Edge::Bottom,
                        7 => Edge::BottomLeft,
                        _ => Edge::BottomRight,
                    };
                    let (w, h) = fit_window(proposed, video, edge);
                    let (w, h) = (w as i32 + frame.0, h as i32 + frame.1 + bars);
                    match edge {
                        Edge::Left | Edge::TopLeft | Edge::BottomLeft => r.left = r.right - w,
                        _ => r.right = r.left + w,
                    }
                    match edge {
                        Edge::Top | Edge::TopLeft | Edge::TopRight => r.top = r.bottom - h,
                        _ => r.bottom = r.top + h,
                    }
                }
                LRESULT(1)
            }
            WM_VIDEO_SIZE => {
                // Rotated: the window turns with the phone, keeping its long side.
                if let Some(((vw, vh), frame)) = with(|s| (s.video, s.frame)) {
                    let s = fluent::scale(hwnd);
                    let mut c = RECT::default();
                    let _ = GetClientRect(hwnd, &mut c);
                    let bars = px(s, TOP) + px(s, BOTTOM);
                    let long = c.right.max(c.bottom - bars);
                    let (w, h) = if vw >= vh {
                        (long, (long as f32 * vh as f32 / vw as f32) as i32)
                    } else {
                        ((long as f32 * vw as f32 / vh as f32) as i32, long)
                    };
                    let _ = SetWindowPos(
                        hwnd,
                        None,
                        0,
                        0,
                        w + frame.0,
                        h + bars + frame.1,
                        SWP_NOMOVE | SWP_NOZORDER,
                    );
                }
                LRESULT(0)
            }
            WM_PAINT => {
                // Every pixel belongs to a child (video and bars).
                let _ = ValidateRect(Some(hwnd), None);
                LRESULT(0)
            }
            WM_REDRAW => {
                redraw_bars();
                LRESULT(0)
            }
            WM_LBUTTONUP => {
                let (x, y) = (
                    (lparam.0 & 0xffff) as i16 as i32,
                    ((lparam.0 >> 16) & 0xffff) as i16 as i32,
                );
                let s = fluent::scale(hwnd);
                let mut c = RECT::default();
                let _ = GetClientRect(hwnd, &mut c);
                if let Some((_, t)) = targets(c, s)
                    .into_iter()
                    .find(|(r, _)| fluent::inside(r, x, y))
                {
                    match t {
                        Target::Back => click_button(1),
                        Target::Home => click_button(2),
                        Target::Recents => click_button(4),
                        Target::Touch => {
                            let on = with(|sh| {
                                sh.touch = !sh.touch;
                                sh.touch
                            })
                            .unwrap_or(true);
                            if !on {
                                // Lift any finger or key still held when the mouse stops driving the phone.
                                release_input();
                            }
                        }
                        Target::Pin => {
                            if let Some((pinned, opts)) = with(|sh| {
                                sh.pinned = !sh.pinned;
                                (sh.pinned, sh.opts.clone())
                            }) {
                                let z = if pinned { HWND_TOPMOST } else { HWND_NOTOPMOST };
                                let _ = SetWindowPos(
                                    hwnd,
                                    Some(z),
                                    0,
                                    0,
                                    0,
                                    0,
                                    SWP_NOMOVE | SWP_NOSIZE,
                                );
                                (opts.on_pin)(pinned);
                            }
                        }
                        Target::Dim => {
                            let dim = with(|sh| {
                                sh.dim = !sh.dim;
                                (sh.dim, sh.opts.clone())
                            });
                            push_state();
                            if let Some((d, opts)) = dim {
                                (opts.on_dim)(d);
                            }
                        }
                    }
                    redraw_bars();
                }
                LRESULT(0)
            }
            WM_DROPFILES => {
                let drop = HDROP(wparam.0 as *mut _);
                let n = DragQueryFileW(drop, u32::MAX, None);
                let mut files = Vec::new();
                for i in 0..n {
                    let len = DragQueryFileW(drop, i, None) as usize;
                    let mut buf = vec![0u16; len + 1];
                    DragQueryFileW(drop, i, Some(&mut buf));
                    files.push(PathBuf::from(String::from_utf16_lossy(&buf[..len])));
                }
                DragFinish(drop);
                if let Some(opts) = with(|s| s.opts.clone()) {
                    (opts.on_files)(files);
                }
                LRESULT(0)
            }
            WM_ACTIVATE => {
                FOCUSED.store((wparam.0 & 0xffff) as u32 != WA_INACTIVE, Ordering::Relaxed);
                if let Some(v) = with(|s| s.video_hwnd) {
                    let _ = SetFocus(Some(HWND(v as *mut _)));
                }
                LRESULT(0)
            }
            WM_ENDED => {
                let last = LAST_NOTICE.lock().map(|n| n.clone()).unwrap_or_default();
                let why = crate::screen_end::reason(&last);
                set_status(if crate::screen_end::retries(why) {
                    "Reconnecting\u{2026}"
                } else {
                    "Phone locked"
                });
                if let Some(opts) = with(|s| s.opts.clone()) {
                    (opts.on_ended)(why.into());
                }
                LRESULT(0)
            }
            WM_GIVE_UP => {
                if let Some(opts) = with(|s| s.opts.clone()) {
                    (opts.on_ended)("Can't show this phone's screen on this PC".into());
                }
                let _ = PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
                LRESULT(0)
            }
            WM_CLOSE => {
                release_input();
                if STREAMING.swap(false, Ordering::Relaxed) {
                    if let Some(opts) = with(|s| s.opts.clone()) {
                        (opts.on_closed)();
                    }
                }
                if let Some((r, msgs)) = with(|s| (s.remote.clone(), s.msgs.clone())) {
                    r.close();
                    let _ = msgs.send(Msg::Stop);
                }
                let _ = DestroyWindow(hwnd);
                LRESULT(0)
            }
            WM_DESTROY => {
                fluent::unregister_window(hwnd);
                PostQuitMessage(0);
                LRESULT(0)
            }
            WM_ERASEBKGND => LRESULT(1),
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

fn point(lparam: LPARAM) -> (i32, i32) {
    (
        (lparam.0 & 0xffff) as i16 as i32,
        ((lparam.0 >> 16) & 0xffff) as i16 as i32,
    )
}

fn client_size(hwnd: HWND) -> (u32, u32) {
    let mut c = RECT::default();
    // SAFETY: plain query on our own window.
    let _ = unsafe { GetClientRect(hwnd, &mut c) };
    (c.right.max(0) as u32, c.bottom.max(0) as u32)
}

/// Mouse and keys on the video drive the phone.
extern "system" fn video_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let video = with(|s| s.video).unwrap_or((1, 1));
    let touch = |phase: TouchPhase, p: (i32, i32)| -> bool {
        let Some((x, y)) = map_point(p, client_size(hwnd), video) else {
            return false;
        };
        with(|s| s.last_touch = (x, y));
        send(InputEvent::Touch { phase, x, y });
        true
    };
    // SAFETY: standard child window procedure.
    unsafe {
        let view_only = !with(|s| s.touch).unwrap_or(true);
        if blocks_input(view_only, msg, GetKeyState(i32::from(VK_CONTROL.0)) < 0) {
            return LRESULT(0);
        }
        match msg {
            WM_LBUTTONDOWN => {
                let _ = SetFocus(Some(hwnd));
                if touch(TouchPhase::Down, point(lparam)) {
                    SetCapture(hwnd);
                    with(|s| s.dragging = true);
                }
                LRESULT(0)
            }
            WM_MOUSEMOVE => {
                if with(|s| s.dragging).unwrap_or(false) {
                    touch(TouchPhase::Move, point(lparam));
                }
                LRESULT(0)
            }
            WM_LBUTTONUP => {
                if with(|s| std::mem::take(&mut s.dragging)).unwrap_or(false) {
                    let _ = ReleaseCapture();
                    if !touch(TouchPhase::Up, point(lparam)) {
                        // Let go outside the video: lift where the finger was.
                        let (x, y) = with(|s| s.last_touch).unwrap_or_default();
                        send(InputEvent::Touch {
                            phase: TouchPhase::Up,
                            x,
                            y,
                        });
                    }
                }
                LRESULT(0)
            }
            WM_KILLFOCUS => {
                release_input();
                LRESULT(0)
            }
            WM_RBUTTONUP => {
                click_button(1);
                LRESULT(0)
            }
            WM_MBUTTONUP => {
                click_button(2);
                LRESULT(0)
            }
            WM_MOUSEWHEEL => {
                let delta = ((wparam.0 >> 16) & 0xffff) as i16;
                let mut pt = POINT {
                    x: point(lparam).0,
                    y: point(lparam).1,
                };
                let _ = ScreenToClient(hwnd, &mut pt);
                if let Some((x, y)) = map_point((pt.x, pt.y), client_size(hwnd), video) {
                    if wparam.0 & 0x0008 != 0 {
                        // Ctrl+Wheel: pinch out (zoom in) or in.
                        send(InputEvent::Pinch {
                            out: delta > 0,
                            x,
                            y,
                        });
                    } else {
                        send(InputEvent::Touch {
                            phase: TouchPhase::Hover,
                            x,
                            y,
                        });
                        send(InputEvent::Wheel { dx: 0, dy: delta });
                    }
                }
                LRESULT(0)
            }
            WM_KEYDOWN | WM_KEYUP => {
                let pressed = msg == WM_KEYDOWN;
                let ctrl = GetKeyState(i32::from(VK_CONTROL.0)) < 0;
                if wparam.0 == usize::from(VK_ESCAPE.0) {
                    if pressed {
                        click_button(1);
                    }
                } else if ctrl && wparam.0 == 'W' as usize {
                    if pressed {
                        close();
                    }
                } else {
                    let scan = ((lparam.0 >> 16) & 0xff) as u16;
                    let ext = if lparam.0 & (1 << 24) != 0 { 0xE000 } else { 0 };
                    let scancode = scan | ext;
                    with(|s| {
                        if pressed {
                            s.held.press(scancode);
                        } else {
                            s.held.release(scancode);
                        }
                    });
                    send(InputEvent::Key { scancode, pressed });
                }
                LRESULT(0)
            }
            WM_CHAR => {
                // Like Ctrl+Alt+P control: the key above, plus the character
                // for plain or AltGr typing (not shortcuts).
                let ctrl = GetKeyState(i32::from(VK_CONTROL.0)) < 0;
                let alt = GetKeyState(i32::from(VK_MENU.0)) < 0;
                let unit = wparam.0 as u16;
                if unit >= 0x20 && (!ctrl || alt) {
                    send(InputEvent::Char { unit });
                }
                LRESULT(0)
            }
            WM_ERASEBKGND => LRESULT(1),
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

#[cfg(test)]
mod bar_tests {
    use super::*;

    #[test]
    fn no_emoji_in_bar_labels() {
        for w in [20.0, 36.0, 59.0, 60.0, 72.0, 200.0] {
            for (pinned, dimmed) in [(false, false), (true, true)] {
                for item in bar_labels(w, pinned, dimmed) {
                    for ch in item.label.chars().chain([item.glyph]) {
                        assert!((ch as u32) < 0x1F000, "{ch:?} at width {w}");
                    }
                    // The glyph is a Segoe Fluent Icons code point (private use area).
                    assert!(('\u{E000}'..='\u{F8FF}').contains(&item.glyph));
                }
            }
        }
    }

    #[test]
    fn narrow_bars_keep_stop_visible() {
        // Stopping the share is the window's own close button in the system frame, which a bar width never
        // touches. What the bar itself must not do at 240 px is drop a control: both toggles stay, as icons.
        let items = bar_labels(240.0 / 6.0, false, false);
        assert_eq!(items.len(), 2);
        for item in items {
            assert!(item.essential);
            assert!(item.label.is_empty(), "icon only when narrow");
            assert_ne!(item.glyph, '\0');
        }
        // Wide: the words come back.
        assert_eq!(bar_labels(72.0, false, false)[0].label, "Pin");
        assert_eq!(bar_labels(72.0, true, true)[1].label, "Dimmed");
    }

    #[test]
    fn the_top_bar_keeps_touch_pin_and_dim_without_overlap_at_every_width() {
        let s = 1.0;
        for width in [240, 300, 360, 480, 800] {
            let c = RECT {
                left: 0,
                top: 0,
                right: width,
                bottom: 700,
            };
            let top: Vec<_> = targets(c, s)
                .into_iter()
                .filter(|(_, t)| matches!(t, Target::Touch | Target::Pin | Target::Dim))
                .collect();
            assert_eq!(top.len(), 3, "width {width}");
            assert!(
                matches!(top[0].1, Target::Touch),
                "Touch is first so the name is sized from it"
            );
            for w in top.windows(2) {
                assert!(
                    w[0].0.right <= w[1].0.left,
                    "buttons overlap at {width}: {:?} {:?}",
                    w[0].0,
                    w[1].0
                );
            }
            assert!(
                top.iter()
                    .all(|(r, _)| r.left >= 0 && r.right <= width && r.right > r.left),
                "inside the bar at {width}"
            );
        }
    }

    #[test]
    fn touch_says_what_it_does_and_keeps_its_icon_when_narrow() {
        assert_eq!(touch_item(72.0, true).label, "Touch");
        assert_eq!(touch_item(72.0, false).label, "View only");
        assert_eq!(touch_item(40.0, false).label, "");
        assert!(touch_item(40.0, true).essential);
    }

    #[test]
    fn view_only_blocks_mouse_wheel_and_typing_but_not_ctrl_shortcuts() {
        for m in [
            WM_LBUTTONDOWN,
            WM_MOUSEMOVE,
            WM_LBUTTONUP,
            WM_MOUSEWHEEL,
            WM_CHAR,
        ] {
            assert!(
                blocks_input(true, m, false),
                "message {m} must be blocked when view only"
            );
            assert!(
                !blocks_input(false, m, false),
                "message {m} passes when touch is on"
            );
        }
        assert!(blocks_input(true, WM_KEYDOWN, false));
        assert!(
            !blocks_input(true, WM_KEYDOWN, true),
            "Ctrl+W must still close the window"
        );
        assert!(
            !blocks_input(true, WM_KILLFOCUS, false),
            "focus loss must still release held input"
        );
    }
}
