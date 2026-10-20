//! The phone-screen window: the phone's screen, live, with a top bar (name,
//! status, Pin, Dim) and a bottom bar (Back, Home, Recents). Mouse and keys
//! on the video drive the phone. Decoding and presenting run on their own
//! thread; the window thread only handles input and layout.
#![allow(unsafe_code)]

use std::{
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
        Graphics::Gdi::{
            InvalidateRect, ScreenToClient, DT_END_ELLIPSIS, DT_LEFT, DT_SINGLELINE, DT_VCENTER,
        },
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
                SetWindowPos, ShowWindow, TranslateMessage, HWND_NOTOPMOST, HWND_TOPMOST,
                IDC_ARROW, MSG, SIZE_MINIMIZED, SM_CYSCREEN, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER,
                SW_SHOW, WA_INACTIVE, WINDOW_EX_STYLE, WM_ACTIVATE, WM_APP, WM_CHAR, WM_CLOSE,
                WM_DESTROY, WM_DROPFILES, WM_ERASEBKGND, WM_KEYDOWN, WM_KEYUP, WM_LBUTTONDOWN,
                WM_LBUTTONUP, WM_MBUTTONUP, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_PAINT, WM_RBUTTONUP,
                WM_SIZE, WM_SIZING, WNDCLASSW, WS_CHILD, WS_OVERLAPPEDWINDOW, WS_VISIBLE,
            },
        },
    },
};

use crate::{
    fluent::{self, px},
    screen_decode::{Decoder, Frame},
    screen_geom::{fit_window, map_point, on_decode_error, video_rect, Edge, Next},
    screen_render::{device_for_window, Renderer},
};

const CLASS: PCWSTR = w!("PhoneBridgeScreen");
const VIDEO_CLASS: PCWSTR = w!("PhoneBridgeScreenVideo");
const WM_REDRAW: u32 = WM_APP + 1;
const WM_ENDED: u32 = WM_APP + 2;
const WM_GIVE_UP: u32 = WM_APP + 3;
const WM_VIDEO_SIZE: u32 = WM_APP + 4;
const TOP: i32 = 44;
const BOTTOM: i32 = 52;
const FEEDBACK: Duration = Duration::from_millis(100);

/// What the tray gives the window.
pub struct WindowOpts {
    pub name: String,
    pub pinned: bool,
    pub dim: bool,
    /// The PC's codecs, best first (for fallback).
    pub codecs: Vec<Codec>,
    pub on_pin: Box<dyn Fn(bool) + Send + Sync>,
    pub on_files: Box<dyn Fn(Vec<PathBuf>) + Send + Sync>,
    /// The session ended; the text says why (for the status pill).
    pub on_ended: Box<dyn Fn(String) + Send + Sync>,
}

enum Msg {
    Record(Record),
    Resize(u32, u32),
    Ended,
    Stop,
}

#[derive(Clone, Copy, PartialEq)]
enum Target {
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
    /// Current video size (changes with rotation).
    video: (u32, u32),
    pinned: bool,
    dim: bool,
    status: String,
    dragging: bool,
    last_touch: (u16, u16),
    /// Window size minus client size (for WM_SIZING).
    frame: (i32, i32),
}

static SHARED: Mutex<Option<Shared>> = Mutex::new(None);
static FOCUSED: AtomicBool = AtomicBool::new(false);

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
    let (remote, mut records, mut state) = view.into_parts();
    let remote = Arc::new(remote);
    let (tx, rx) = mpsc::channel::<Msg>();
    let fwd = tx.clone();
    rt.spawn(async move {
        while let Some(r) = records.recv().await {
            if fwd.send(Msg::Record(r)).is_err() {
                return;
            }
        }
        let _ = fwd.send(Msg::Ended);
    });
    rt.spawn(async move {
        while let Some(st) = state.recv().await {
            if !st.notice.is_empty() {
                set_status(&st.notice);
            }
        }
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
            lpfnWndProc: Some(wndproc),
            hInstance: module.into(),
            lpszClassName: CLASS,
            hCursor: cursor,
            ..Default::default()
        });
        RegisterClassW(&WNDCLASSW {
            lpfnWndProc: Some(video_proc),
            hInstance: module.into(),
            lpszClassName: VIDEO_CLASS,
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
            WS_OVERLAPPEDWINDOW,
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
            video,
            pinned: opts.pinned,
            dim: opts.dim,
            status: String::new(),
            dragging: false,
            last_touch: (0, 0),
            frame: (
                (wr.right - wr.left) - cr.right,
                (wr.bottom - wr.top) - cr.bottom,
            ),
        });
        fluent::frame(hwnd, false);
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
        let codecs = opts.codecs.clone();
        let remote2 = remote.clone();
        std::thread::spawn(move || decode_thread(top, vid, rx, remote2, codecs, codec, video));
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
    codecs: Vec<Codec>,
    mut codec: Codec,
    mut video: (u32, u32),
) {
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
        match rx.recv_timeout(FEEDBACK) {
            Ok(Msg::Record(r)) => {
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
                        present(&mut renderer, &f, size, video, dimmed);
                        last_pts = f.pts_us;
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
                    Err(_) => match on_decode_error(codec, &codecs) {
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
                dimmed = true;
                if let Some(f) = &last {
                    present(&mut renderer, f, size, video, true);
                }
                post(WM_ENDED);
            }
            Ok(Msg::Stop) | Err(mpsc::RecvTimeoutError::Disconnected) => return,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
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
    let bw = px(s, 72);
    let pad = px(s, 6);
    let mut v = vec![
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
    let Some((video_hwnd, msgs)) = with(|s| (s.video_hwnd, s.msgs.clone())) else {
        return;
    };
    let s = fluent::scale(hwnd);
    let mut c = RECT::default();
    // SAFETY: sizing our own child window.
    unsafe {
        let _ = GetClientRect(hwnd, &mut c);
        let (top, bottom) = (px(s, TOP), px(s, BOTTOM));
        let h = (c.bottom - top - bottom).max(1);
        let _ = MoveWindow(HWND(video_hwnd as *mut _), 0, top, c.right, h, true);
        let _ = msgs.send(Msg::Resize(c.right.max(1) as u32, h as u32));
    }
}

fn send(e: InputEvent) {
    if let Some(r) = with(|s| s.remote.clone()) {
        r.input.send(e);
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
                if let Some((r, dim)) = with(|s| (s.remote.clone(), s.dim)) {
                    // Minimised: the phone pauses its capture.
                    r.set_state(active, dim);
                }
                if active {
                    layout(hwnd);
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
                let s = fluent::scale(hwnd);
                let (name, status, pinned, dim) =
                    with(|sh| (sh.opts.name.clone(), sh.status.clone(), sh.pinned, sh.dim))
                        .unwrap_or_default();
                fluent::paint(hwnd, |hdc, c| {
                    let p = fluent::palette();
                    let f = fluent::Fonts::new(s);
                    let line = if status.is_empty() {
                        name
                    } else {
                        format!("{name}  \u{00b7}  {status}")
                    };
                    let tr = RECT {
                        left: px(s, 12),
                        top: 0,
                        right: c.right - px(s, 160),
                        bottom: px(s, TOP),
                    };
                    fluent::text(
                        hdc,
                        f.body,
                        &line,
                        tr,
                        p.text,
                        DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS,
                    );
                    for (r, t) in targets(c, s) {
                        let (label, on) = match t {
                            Target::Pin => (if pinned { "Pinned" } else { "Pin" }, pinned),
                            Target::Dim => (if dim { "Dimmed" } else { "Dim" }, dim),
                            Target::Back => ("Back", false),
                            Target::Home => ("Home", false),
                            Target::Recents => ("Recents", false),
                        };
                        fluent::button(hdc, &f, &p, r, label, on, s);
                    }
                });
                LRESULT(0)
            }
            WM_REDRAW => {
                let _ = InvalidateRect(Some(hwnd), None, false);
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
                            if let Some((r, dim)) = with(|sh| {
                                sh.dim = !sh.dim;
                                (sh.remote.clone(), sh.dim)
                            }) {
                                r.set_state(true, dim);
                            }
                        }
                    }
                    let _ = InvalidateRect(Some(hwnd), None, false);
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
                set_status("Reconnecting\u{2026}");
                if let Some(opts) = with(|s| s.opts.clone()) {
                    (opts.on_ended)("Phone stopped sharing its screen".into());
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
                if let Some((r, msgs)) = with(|s| (s.remote.clone(), s.msgs.clone())) {
                    r.close();
                    let _ = msgs.send(Msg::Stop);
                }
                let _ = DestroyWindow(hwnd);
                LRESULT(0)
            }
            WM_DESTROY => {
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
                    send(InputEvent::Key {
                        scancode: scan | ext,
                        pressed,
                    });
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
