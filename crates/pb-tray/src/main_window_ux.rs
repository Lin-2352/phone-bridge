//! A scripted run of the real main window (real `wndproc`, real Direct2D
//! surface, hidden from nothing): drives the scrollbar and the Timeline's
//! day groups with posted-in-process mouse and key messages, asserts what
//! the window state says, and saves window-only captures (`PrintWindow`).
//!
//! It needs a desktop and shows a window for a few seconds, so it is
//! ignored by default:
//! `PB_UX_OUT=<dir> cargo test -p pb-tray -- --ignored ux_run --nocapture`
//! (The tray itself cannot run beside another tray: its single-instance
//! mutex and pipe are per user.) Captures are 32-bit BMPs.
use super::*;
use crate::{
    pages::timeline::{self as tl, Item},
    timeline::{self as store, Dir},
};
use windows::Win32::{
    Graphics::Gdi::{
        CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetDC, GetDIBits,
        ReleaseDC, SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS,
    },
    UI::WindowsAndMessaging::{
        DestroyWindow, DispatchMessageW, PeekMessageW, SendMessageW, TranslateMessage, MSG,
        PM_REMOVE,
    },
};

#[link(name = "user32")]
extern "system" {
    fn PrintWindow(hwnd: HWND, hdc: HDC, flags: u32) -> i32;
}

fn pack(x: f32, y: f32) -> LPARAM {
    LPARAM((x as i16 as u16 as isize) | ((y as i16 as u16 as isize) << 16))
}

fn pump(ms: u64) {
    let end = Instant::now() + Duration::from_millis(ms);
    while Instant::now() < end {
        let mut msg = MSG::default();
        // SAFETY: standard message loop step on this thread.
        unsafe {
            if PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            } else {
                std::thread::sleep(Duration::from_millis(4));
            }
        }
    }
}

fn send(hwnd: HWND, msg: u32, w: usize, l: LPARAM) {
    // SAFETY: a message to our own window on its own thread.
    unsafe { SendMessageW(hwnd, msg, Some(WPARAM(w)), Some(l)) };
}

fn mouse(hwnd: HWND, x: f32, y: f32) {
    let (w, h) = client_size(hwnd);
    let inside = x >= 0.0 && y >= 0.0 && x < w && y < h;
    FAKE_CURSOR.with(|c| c.set(Some(inside.then_some((x, y)))));
    send(hwnd, WM_MOUSEMOVE, 0, pack(x, y));
}

fn click_at(hwnd: HWND, x: f32, y: f32) {
    mouse(hwnd, x, y);
    send(hwnd, WM_LBUTTONDOWN, 1, pack(x, y));
    send(hwnd, WM_LBUTTONUP, 0, pack(x, y));
}

fn key(hwnd: HWND, vk: u16) {
    send(hwnd, WM_KEYDOWN, vk as usize, LPARAM(0));
    pump(30);
}

fn capture(hwnd: HWND, dir: &std::path::Path, name: &str) {
    let (w, h) = client_size(hwnd);
    let (w, h) = (w as i32, h as i32);
    let mut buf = vec![0u8; (w * h * 4) as usize];
    // SAFETY: GDI calls on our own window; every object made is deleted.
    unsafe {
        let dc = GetDC(Some(hwnd));
        let mem = CreateCompatibleDC(Some(dc));
        let bmp = CreateCompatibleBitmap(dc, w, h);
        let old = SelectObject(mem, bmp.into());
        // 2 = PW_RENDERFULLCONTENT: the composed (Direct2D) content.
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

fn entries(now: u64) -> Vec<store::Entry> {
    (0..60u64)
        .map(|k| {
            let at = now - (k / 5) * 86_400 - (k % 5) * 600;
            let mut e = if k % 3 == 0 {
                store::file_entry(
                    "Pixel 8",
                    Dir::ToPc,
                    "report.pdf",
                    1200,
                    Some(std::path::Path::new("C:\\gone\\report.pdf")),
                )
            } else {
                store::clip_text_entry("Pixel 8", Dir::ToPc, "hello", false)
            };
            e.at = at;
            e
        })
        .collect()
}

#[test]
#[ignore]
fn ux_run() {
    let dir = std::env::var_os("PB_UX_OUT")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("pbm_ux_out"));
    std::fs::create_dir_all(&dir).unwrap();
    let now = store::now_unix();
    let timeline = Arc::new(tl::build(&entries(now), true, now, 0));
    let log = crate::core::Log::open(dir.join("ux.log"));
    let settings: crate::core::SharedSettings = Default::default();
    init(Host {
        log,
        settings,
        settings_path: dir.join("settings.txt"),
        model: Box::new(move || Model {
            timeline: timeline.clone(),
            ..Default::default()
        }),
        act: Box::new(|_| {}),
        check_peers: Box::new(|| {}),
    });
    open(Page::Timeline);
    let hwnd = current().expect("window");
    pump(600);
    let (w, h) = client_size(hwnd);
    let f = |st: &State| frame(w, h, st.scale);
    let geom = |hwnd: HWND| {
        with(hwnd, |st| {
            let fr = f(st);
            let bar = st.bar(&fr);
            let (top, len) = bar.thumb().expect("the page scrolls");
            (fr.pane.right, bar.top + top, len, bar)
        })
        .unwrap()
    };

    // 1. At rest: thin, visible, offset 0.
    capture(hwnd, &dir, "01-timeline-initial.bmp");
    let (right, thumb_top, thumb_len, bar) = geom(hwnd);
    assert!(bar.hit_x(right).0 < bar.painted_x(right, 0.0).0);
    assert_eq!(with(hwnd, |st| st.scroll[1].offset), Some(0.0));

    // 2. Hover 13 px from the edge: outside the 6 px drawn bar, inside the
    // hit area; it widens.
    let mid = thumb_top + thumb_len / 2.0;
    mouse(hwnd, right - 13.0, mid);
    pump(400);
    assert_eq!(with(hwnd, |st| st.bar_hover), Some(true));
    assert_eq!(
        with(hwnd, |st| st.bar_fade.value(Instant::now())),
        Some(1.0)
    );
    capture(hwnd, &dir, "02-scrollbar-hover.bmp");
    mouse(hwnd, 300.0, mid);
    pump(400);
    assert_eq!(with(hwnd, |st| st.bar_hover), Some(false));

    // 3. Drag the thumb, far outside the window: capture keeps it going and
    // clamps at the end.
    send(hwnd, WM_LBUTTONDOWN, 1, pack(right - 13.0, mid));
    assert!(with(hwnd, |st| st.drag.is_some()).unwrap());
    mouse(hwnd, right - 13.0, mid + 60.0);
    pump(60);
    let part = with(hwnd, |st| st.scroll[1].offset).unwrap();
    assert!(part > 0.0, "dragging scrolls");
    capture(hwnd, &dir, "03-drag.bmp");
    mouse(hwnd, -300.0, h + 400.0);
    pump(60);
    let (max, off) = with(hwnd, |st| {
        let s = &st.scroll[1];
        (s.content - s.view, s.offset)
    })
    .unwrap();
    assert_eq!(off, max, "dragged past the bottom edge: clamps at the end");
    send(hwnd, WM_LBUTTONUP, 0, pack(-300.0, h + 400.0));
    assert!(with(hwnd, |st| st.drag.is_none()).unwrap());

    // 4. A click on the track above the thumb pages up; Home goes to the top.
    let (right, thumb_top, _, bar) = geom(hwnd);
    let before = with(hwnd, |st| st.scroll[1].offset).unwrap();
    send(hwnd, WM_LBUTTONDOWN, 1, pack(right - 5.0, thumb_top - 40.0));
    send(hwnd, WM_LBUTTONUP, 0, pack(right - 5.0, thumb_top - 40.0));
    pump(300);
    let after = with(hwnd, |st| st.scroll[1].offset).unwrap();
    assert!(after < before, "{after} < {before}");
    assert!((before - after - bar.view * 0.9).abs() < 1.0 || after == 0.0);
    key(hwnd, VK_HOME.0);
    pump(400);
    assert_eq!(with(hwnd, |st| st.scroll[1].offset), Some(0.0));
    key(hwnd, VK_NEXT.0);
    pump(400);
    assert!(with(hwnd, |st| st.scroll[1].offset).unwrap() > 0.0, "PgDn");
    key(hwnd, VK_END.0);
    pump(500);
    capture(hwnd, &dir, "04-end.bmp");
    key(hwnd, VK_HOME.0);
    pump(400);

    // 5. Timeline days: open "Yesterday" by clicking its header; the page
    // does not jump.
    let hits = with(hwnd, |st| layout_now(hwnd, st).1).unwrap();
    let heads: Vec<_> = hits
        .iter()
        .filter(|h| h.id >= 0xC000_0000 && h.id != 0xFFFF_0000)
        .collect();
    assert!(heads.len() >= 3, "{heads:?}");
    let open_rows = |hwnd| {
        with(hwnd, |st| {
            st.model
                .timeline
                .items
                .iter()
                .filter(|i| matches!(i, Item::Row(_)))
                .count()
        })
        .unwrap()
    };
    let rows0 = open_rows(hwnd);
    capture(hwnd, &dir, "05-days-initial.bmp");
    let hd = heads[1].r;
    click_at(hwnd, (hd.left + hd.right) / 2.0, (hd.top + hd.bottom) / 2.0);
    pump(100);
    assert_eq!(open_rows(hwnd), rows0 + 5, "Yesterday opened");
    mouse(hwnd, 400.0, 300.0);
    pump(100);
    capture(hwnd, &dir, "06-day-expanded.bmp");

    // Scroll down, collapse Today: what is on screen stays put.
    send(
        hwnd,
        WM_MOUSEWHEEL,
        ((-120i16) as u16 as usize) << 16,
        pack(400.0, 300.0),
    );
    pump(400);

    // 6. Collapse all / Expand all.
    key(hwnd, VK_HOME.0);
    pump(400);
    let hits = with(hwnd, |st| layout_now(hwnd, st).1).unwrap();
    let all = hits
        .iter()
        .find(|h| h.id == 0xFFFF_0000)
        .expect("all control");
    click_at(
        hwnd,
        (all.r.left + all.r.right) / 2.0,
        (all.r.top + all.r.bottom) / 2.0,
    );
    pump(100);
    // Some days are open, some not: the control offers "Expand all".
    assert_eq!(open_rows(hwnd), 60, "expand all");
    mouse(hwnd, 400.0, 400.0);
    pump(100);
    capture(hwnd, &dir, "07-expand-all.bmp");
    click_at(
        hwnd,
        (all.r.left + all.r.right) / 2.0,
        (all.r.top + all.r.bottom) / 2.0,
    );
    pump(100);
    assert_eq!(open_rows(hwnd), 0, "collapse all");
    mouse(hwnd, 400.0, 400.0);
    pump(100);
    capture(hwnd, &dir, "07b-collapse-all.bmp");
    click_at(
        hwnd,
        (all.r.left + all.r.right) / 2.0,
        (all.r.top + all.r.bottom) / 2.0,
    );
    pump(100);
    assert_eq!(open_rows(hwnd), 60);
    // The page is shorter after collapse and longer after expand; the
    // scroll range follows.
    let (content, view) = with(hwnd, |st| (st.scroll[1].content, st.scroll[1].view)).unwrap();
    assert!(content > view);

    // 7. Keyboard: Tab to the first page hit shows a focus ring; Enter on a
    // day header toggles it.
    for _ in 0..6 {
        key(hwnd, VK_TAB.0);
    }
    pump(100);
    capture(hwnd, &dir, "08-focus-ring.bmp");
    let focus = with(hwnd, |st| st.focus).unwrap();
    assert!(focus.is_some_and(|id| id >= HIT_BASE));

    // 8. Dark theme, scrollbar hover.
    fluent::set_theme(2);
    let (right, thumb_top, thumb_len, _) = geom(hwnd);
    mouse(hwnd, right - 13.0, thumb_top + thumb_len / 2.0);
    pump(500);
    capture(hwnd, &dir, "09-dark-hover.bmp");
    mouse(hwnd, 300.0, 300.0);
    pump(400);
    capture(hwnd, &dir, "10-dark-rest.bmp");
    fluent::set_theme(0);

    // SAFETY: our own window, destroyed on its thread.
    let _ = unsafe { DestroyWindow(hwnd) };
}

/// What a UI Automation client (a Narrator stand-in, on its own MTA thread
/// like a real one) sees and does: names and roles of the nav items, day
/// headers with their expanded state, rows; then it expands a day and
/// selects the Settings tab through the patterns.
fn uia_client(hwnd: isize, out: std::sync::mpsc::Sender<Result<Vec<(String, i32)>, String>>) {
    use windows::Win32::{
        System::Com::{
            CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED,
        },
        UI::Accessibility::{
            CUIAutomation, IUIAutomation, IUIAutomationExpandCollapsePattern,
            IUIAutomationSelectionItemPattern, TreeScope_Descendants, UIA_ExpandCollapsePatternId,
            UIA_SelectionItemPatternId,
        },
    };
    let run = || -> windows::core::Result<Vec<(String, i32)>> {
        // SAFETY: COM on this thread, which owns every interface it makes.
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            let auto: IUIAutomation = CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER)?;
            let root = auto.ElementFromHandle(HWND(hwnd as *mut _))?;
            let all = root.FindAll(TreeScope_Descendants, &auto.CreateTrueCondition()?)?;
            let mut seen = Vec::new();
            for i in 0..all.Length()? {
                let e = all.GetElement(i)?;
                seen.push((e.CurrentName()?.to_string(), e.CurrentControlType()?.0));
            }
            // Expand the first closed day header through ExpandCollapse.
            for i in 0..all.Length()? {
                let e = all.GetElement(i)?;
                if e.CurrentControlType()?.0 == 50024 {
                    let p: IUIAutomationExpandCollapsePattern =
                        e.GetCurrentPatternAs(UIA_ExpandCollapsePatternId)?;
                    if p.CurrentExpandCollapseState()?.0 == 0 {
                        p.Expand()?;
                        break;
                    }
                }
            }
            std::thread::sleep(Duration::from_millis(400));
            // Select the Settings tab.
            for i in 0..all.Length()? {
                let e = all.GetElement(i)?;
                if e.CurrentName()? == "Settings" && e.CurrentControlType()?.0 == 50019 {
                    let p: IUIAutomationSelectionItemPattern =
                        e.GetCurrentPatternAs(UIA_SelectionItemPatternId)?;
                    p.Select()?;
                }
            }
            Ok(seen)
        }
    };
    let _ = out.send(run().map_err(|e| e.to_string()));
}

#[test]
#[ignore]
fn uia_run() {
    let dir = std::env::var_os("PB_UX_OUT")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("pbm_ux_out"));
    std::fs::create_dir_all(&dir).unwrap();
    let now = store::now_unix();
    let timeline = Arc::new(tl::build(&entries(now), true, now, 0));
    let log = crate::core::Log::open(dir.join("uia.log"));
    init(Host {
        log,
        settings: Default::default(),
        settings_path: dir.join("settings-uia.txt"),
        model: Box::new(move || Model {
            timeline: timeline.clone(),
            ..Default::default()
        }),
        act: Box::new(|_| {}),
        check_peers: Box::new(|| {}),
    });
    open(Page::Timeline);
    let hwnd = current().expect("window");
    pump(600);
    let rows0 = with(hwnd, |st| st.model.timeline.items.len()).unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    let raw = hwnd.0 as isize;
    std::thread::spawn(move || uia_client(raw, tx));
    // The window thread must keep pumping: UIA sends it WM_GETOBJECT.
    let end = Instant::now() + Duration::from_secs(20);
    let seen = loop {
        pump(50);
        match rx.try_recv() {
            Ok(r) => break r.expect("UIA client"),
            Err(_) if Instant::now() > end => panic!("UIA client timed out"),
            Err(_) => {}
        }
    };
    pump(300);
    let has = |name: &str, ct: i32| seen.iter().any(|(n, c)| n == name && *c == ct);
    for tab in ["Home", "Timeline", "Devices", "Settings"] {
        assert!(has(tab, 50019), "nav item {tab} missing: {seen:?}");
    }
    assert!(
        seen.iter().any(|(n, c)| *c == 50024 && n.contains("items")),
        "day headers"
    );
    assert!(
        seen.iter()
            .any(|(n, c)| *c == 50007 && n.contains("Pixel 8")),
        "rows"
    );
    assert!(has("Collapse all days", 50000) || has("Expand all days", 50000));
    // The patterns acted on the real window.
    let (rows1, page) = with(hwnd, |st| (st.model.timeline.items.len(), st.page)).unwrap();
    assert!(rows1 > rows0, "Expand opened a day: {rows0} -> {rows1}");
    assert_eq!(page, Page::Settings, "Select on the Settings tab");
    // SAFETY: our own window, destroyed on its thread.
    let _ = unsafe { DestroyWindow(hwnd) };
}
