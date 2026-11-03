//! A scripted run of the real transfer window (real wndproc, real Direct2D
//! surface): starts a send, a multi-file batch and a receive in the
//! registry, checks the window appears on start, shows progress, cancel
//! and retry states, and saves window-only captures (`PrintWindow`).
//!
//! It needs a desktop and shows a window for a few seconds, so it is
//! ignored by default:
//! `PB_UX_OUT=<dir> cargo test -p pb-tray -- --ignored transfer_ux --nocapture`
//! Captures are 32-bit BMPs of the window only, never the screen.
use std::{
    sync::atomic::Ordering,
    time::{Duration, Instant},
};

use windows::Win32::{
    Foundation::{HWND, RECT},
    Graphics::Gdi::{
        CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetDC, GetDIBits,
        ReleaseDC, SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, HDC,
    },
    UI::WindowsAndMessaging::{GetClientRect, IsWindowVisible},
};

use crate::{transfer_window::HWND_RAW, transfers as tr};

#[link(name = "user32")]
extern "system" {
    fn PrintWindow(hwnd: HWND, hdc: HDC, flags: u32) -> i32;
}

fn capture(hwnd: HWND, dir: &std::path::Path, name: &str) {
    let mut r = RECT::default();
    // SAFETY: plain query on a window of this process.
    unsafe {
        let _ = GetClientRect(hwnd, &mut r);
    }
    let (w, h) = (r.right.max(1), r.bottom.max(1));
    let mut buf = vec![0u8; (w * h * 4) as usize];
    // SAFETY: GDI calls on a window of this process; every object made is deleted.
    unsafe {
        let dc = GetDC(Some(hwnd));
        let mem = CreateCompatibleDC(Some(dc));
        let bmp = CreateCompatibleBitmap(dc, w, h);
        let old = SelectObject(mem, bmp.into());
        PrintWindow(hwnd, mem, 2);
        let mut bi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                biHeight: h,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        GetDIBits(
            mem,
            bmp,
            0,
            h as u32,
            Some(buf.as_mut_ptr().cast()),
            &mut bi,
            DIB_RGB_COLORS,
        );
        SelectObject(mem, old);
        let _ = DeleteObject(bmp.into());
        let _ = DeleteDC(mem);
        ReleaseDC(Some(hwnd), dc);
    }
    let mut out = Vec::with_capacity(buf.len() + 54);
    out.extend_from_slice(b"BM");
    out.extend_from_slice(&(54 + buf.len() as u32).to_le_bytes());
    out.extend_from_slice(&[0; 4]);
    out.extend_from_slice(&54u32.to_le_bytes());
    out.extend_from_slice(&40u32.to_le_bytes());
    out.extend_from_slice(&w.to_le_bytes());
    out.extend_from_slice(&h.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&32u16.to_le_bytes());
    out.extend_from_slice(&[0; 24]);
    out.extend_from_slice(&buf);
    std::fs::write(dir.join(name), out).expect("write capture");
}

fn wait_for(what: &str, mut f: impl FnMut() -> bool) {
    let end = Instant::now() + Duration::from_secs(5);
    while Instant::now() < end {
        if f() {
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!("timed out waiting for {what}");
}

fn window() -> HWND {
    HWND(HWND_RAW.load(Ordering::SeqCst) as *mut _)
}

fn send_row(name: &str, batch: Option<tr::Batch>) -> (u64, pb_transport::files::Cancel) {
    tr::start(tr::NewTransfer {
        dir: tr::Dir::Send,
        peer: "Pixel 8".into(),
        name: name.into(),
        size: 400 << 20,
        path: Some(std::env::temp_dir().join(name)),
        batch,
    })
}

#[test]
#[ignore]
fn transfer_ux_run() {
    let out = std::env::var("PB_UX_OUT")
        .map(std::path::PathBuf::from)
        .ok();
    if let Some(d) = &out {
        std::fs::create_dir_all(d).unwrap();
    }
    assert_eq!(
        HWND_RAW.load(Ordering::SeqCst),
        0,
        "no window before a transfer starts"
    );

    // A transfer starts: the window appears on its own (never silent).
    let batch = tr::new_batch_id();
    let (a, _) = send_row(
        "holiday-video.mp4",
        Some(tr::Batch {
            id: batch,
            index: 2,
            total: 5,
        }),
    );
    wait_for("the window to appear", || {
        let h = window();
        !h.0.is_null() && unsafe { IsWindowVisible(h) }.as_bool()
    });
    let rows = tr::rows();
    assert!(rows[0].status_line().starts_with("Waiting for Pixel 8"));
    if let Some(d) = &out {
        std::thread::sleep(Duration::from_millis(400));
        capture(window(), d, "1-waiting.bmp");
    }

    // Progress: bytes, speed, ETA after the warm-up.
    let cb = tr::progress_cb(a);
    let t0 = Instant::now();
    while t0.elapsed() < Duration::from_millis(2600) {
        let done = (t0.elapsed().as_secs_f64() * 20e6) as u64;
        cb(done, 400 << 20);
        std::thread::sleep(Duration::from_millis(50));
    }
    let line = tr::rows()[0].status_line();
    assert!(line.contains("/s") && line.contains("left"), "{line}");
    let (b, _) = tr::start(tr::NewTransfer {
        dir: tr::Dir::Receive,
        peer: "Pixel 8".into(),
        name: "IMG_2041.jpg".into(),
        size: 3 << 20,
        path: None,
        batch: None,
    });
    tr::progress_cb(b)(1 << 20, 3 << 20);
    std::thread::sleep(Duration::from_millis(500));
    if let Some(d) = &out {
        capture(window(), d, "2-progress.bmp");
    }

    // Cancel stops the send; the row offers Retry; the receive finishes.
    assert!(tr::cancel(a, tr::CancelBy::User));
    tr::finish(b, Some(std::env::temp_dir().join("IMG_2041.jpg")));
    std::thread::sleep(Duration::from_millis(500));
    let rows = tr::rows();
    assert!(rows[0].can_retry && !rows[0].can_cancel);
    assert!(rows[1].status_line().starts_with("Done"));
    if let Some(d) = &out {
        capture(window(), d, "3-cancelled-and-done.bmp");
    }

    // A failure reads as an action, not a code.
    let (c, _) = send_row("report.pdf", None);
    tr::fail(
        c,
        tr::describe(
            tr::Dir::Send,
            &pb_transport::Error::Timeout,
            &tr::Ctx {
                peer: "Pixel 8",
                file: "report.pdf",
                folder: "",
                refusal: None,
            },
        ),
    );
    std::thread::sleep(Duration::from_millis(500));
    if let Some(d) = &out {
        capture(window(), d, "4-failed.bmp");
    }

    // Dismissing everything closes the window.
    for r in tr::rows() {
        tr::dismiss(r.id);
    }
    wait_for("the window to close", || {
        HWND_RAW.load(Ordering::SeqCst) == 0
    });
}
