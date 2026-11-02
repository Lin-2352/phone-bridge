//! Direct2D surfaces: one process-wide Direct2D and DirectWrite factory, and
//! per window a device context drawing into a flip-model swap chain. Mica
//! windows present a premultiplied composition swap chain through
//! DirectComposition so the backdrop shows through; every other window (and
//! Mica when it is unavailable) uses an opaque HWND swap chain.
#![allow(unsafe_code)]

use std::{
    cell::{Cell, RefCell},
    sync::OnceLock,
};

use windows::{
    core::{Interface, HRESULT},
    Win32::{
        Foundation::{D2DERR_RECREATE_TARGET, HMODULE, HWND, RECT},
        Graphics::{
            Direct2D::{
                Common::{
                    D2D1_ALPHA_MODE, D2D1_ALPHA_MODE_IGNORE, D2D1_ALPHA_MODE_PREMULTIPLIED,
                    D2D1_COLOR_F, D2D1_PIXEL_FORMAT,
                },
                D2D1CreateFactory, ID2D1Bitmap1, ID2D1Device, ID2D1DeviceContext, ID2D1Factory1,
                D2D1_BITMAP_OPTIONS_CANNOT_DRAW, D2D1_BITMAP_OPTIONS_TARGET,
                D2D1_BITMAP_PROPERTIES1, D2D1_DEVICE_CONTEXT_OPTIONS_NONE,
                D2D1_FACTORY_TYPE_MULTI_THREADED,
            },
            Direct3D::{
                D3D_DRIVER_TYPE, D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_UNKNOWN,
                D3D_DRIVER_TYPE_WARP,
            },
            Direct3D11::{
                D3D11CreateDevice, ID3D11Device, D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                D3D11_SDK_VERSION,
            },
            DirectComposition::{
                DCompositionCreateDevice, IDCompositionDevice, IDCompositionTarget,
                IDCompositionVisual,
            },
            DirectWrite::{DWriteCreateFactory, IDWriteFactory, DWRITE_FACTORY_TYPE_SHARED},
            Dwm::{DwmExtendFrameIntoClientArea, DWMSBT_AUTO, DWMSBT_MAINWINDOW},
            Dxgi::{
                Common::{
                    DXGI_ALPHA_MODE_IGNORE, DXGI_ALPHA_MODE_PREMULTIPLIED,
                    DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_FORMAT_UNKNOWN, DXGI_SAMPLE_DESC,
                },
                IDXGIAdapter, IDXGIDevice, IDXGIFactory2, IDXGISurface, IDXGISwapChain1,
                DXGI_ERROR_DEVICE_REMOVED, DXGI_ERROR_DEVICE_RESET, DXGI_PRESENT,
                DXGI_SWAP_CHAIN_DESC1, DXGI_SWAP_CHAIN_FLAG, DXGI_SWAP_EFFECT_FLIP_DISCARD,
                DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL, DXGI_USAGE_RENDER_TARGET_OUTPUT,
            },
            Gdi::InvalidateRect,
        },
        UI::{
            Controls::MARGINS,
            WindowsAndMessaging::{
                DestroyWindow, GetClientRect, WINDOW_EX_STYLE, WS_EX_NOREDIRECTIONBITMAP,
            },
        },
    },
};

use crate::{fluent, screen_render};

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// What shows behind the window's content.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backdrop {
    /// DWM Mica behind a premultiplied composition swap chain.
    Mica,
    /// An opaque HWND swap chain painted from the tokens.
    Opaque,
}

/// Mica only when it is wanted, DWM accepted the backdrop and the
/// DirectComposition swap chain was built; otherwise opaque.
pub fn choose(want: Backdrop, mica_supported: bool, dcomp_ok: bool) -> Backdrop {
    match want {
        Backdrop::Mica if mica_supported && dcomp_ok => Backdrop::Mica,
        _ => Backdrop::Opaque,
    }
}

/// A 0xRRGGBB token colour with straight alpha 0..=1.
pub fn color(rgb: u32, alpha: f32) -> D2D1_COLOR_F {
    let ch = |shift: u32| ((rgb >> shift) & 0xFF) as f32 / 255.0;
    D2D1_COLOR_F {
        r: ch(16),
        g: ch(8),
        b: ch(0),
        a: alpha,
    }
}

type Factories = (ID2D1Factory1, IDWriteFactory);

static FACTORIES: OnceLock<Result<Factories, String>> = OnceLock::new();

/// The factories, or why they cannot be made (Direct2D missing entirely).
fn try_factories() -> Result<&'static Factories, String> {
    FACTORIES
        .get_or_init(|| {
            // SAFETY: plain factory creation. Multi-threaded and shared,
            // because windows on different threads use the same factories.
            unsafe {
                let d2d: ID2D1Factory1 =
                    D2D1CreateFactory(D2D1_FACTORY_TYPE_MULTI_THREADED, None).map_err(err)?;
                let dwrite: IDWriteFactory =
                    DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED).map_err(err)?;
                Ok((d2d, dwrite))
            }
        })
        .as_ref()
        .map_err(Clone::clone)
}

/// The process-wide Direct2D and DirectWrite factories. Call only once a
/// `Surface` exists, which proves they could be made.
pub fn factories() -> &'static Factories {
    try_factories().expect("Direct2D factories")
}

/// A BGRA D3D11 device: the window's own GPU, else the default GPU, else
/// WARP (software; also what a GPU-less CI runner gets).
fn device(hwnd: HWND) -> Result<ID3D11Device, String> {
    let mut tries: Vec<(Option<IDXGIAdapter>, D3D_DRIVER_TYPE)> = Vec::new();
    if let Some(adapter) = screen_render::adapter_for_window(hwnd) {
        // An explicit adapter needs the UNKNOWN driver type.
        tries.push((Some(adapter), D3D_DRIVER_TYPE_UNKNOWN));
    }
    tries.push((None, D3D_DRIVER_TYPE_HARDWARE));
    tries.push((None, D3D_DRIVER_TYPE_WARP));
    let mut last = String::from("no D3D11 device");
    for (adapter, driver) in tries {
        let mut device = None;
        // SAFETY: device creation with owned out-params.
        let r = unsafe {
            D3D11CreateDevice(
                adapter.as_ref(),
                driver,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                None,
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                None,
            )
        };
        match (r, device) {
            (Ok(()), Some(d)) => return Ok(d),
            (Err(e), _) => last = err(e),
            (Ok(()), None) => {}
        }
    }
    Err(last)
}

/// DirectComposition objects that keep the composition swap chain on screen.
struct Composition {
    _device: IDCompositionDevice,
    _target: IDCompositionTarget,
    _visual: IDCompositionVisual,
}

/// Everything that dies with the GPU device.
struct Gfx {
    dc: ID2D1DeviceContext,
    /// The back buffer as a D2D target; made lazily, dropped before resizing.
    target: Option<ID2D1Bitmap1>,
    swap: IDXGISwapChain1,
    _comp: Option<Composition>,
    alpha: D2D1_ALPHA_MODE,
}

fn swap_desc(size: (u32, u32), mica: bool) -> DXGI_SWAP_CHAIN_DESC1 {
    DXGI_SWAP_CHAIN_DESC1 {
        Width: size.0,
        Height: size.1,
        Format: DXGI_FORMAT_B8G8R8A8_UNORM,
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
        BufferCount: 2,
        SwapEffect: if mica {
            DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL
        } else {
            DXGI_SWAP_EFFECT_FLIP_DISCARD
        },
        AlphaMode: if mica {
            DXGI_ALPHA_MODE_PREMULTIPLIED
        } else {
            DXGI_ALPHA_MODE_IGNORE
        },
        ..Default::default()
    }
}

/// A composition swap chain shown through one DirectComposition visual.
fn composition(
    hwnd: HWND,
    d3d: &ID3D11Device,
    dxgi: &IDXGIDevice,
    factory: &IDXGIFactory2,
    size: (u32, u32),
) -> Result<(IDXGISwapChain1, Composition), String> {
    // SAFETY: COM calls on objects owned here; the target is our own window.
    unsafe {
        let swap = factory
            .CreateSwapChainForComposition(d3d, &swap_desc(size, true), None)
            .map_err(err)?;
        let device: IDCompositionDevice = DCompositionCreateDevice(dxgi).map_err(err)?;
        // Not topmost: child controls (the search EDIT) draw above it.
        let target = device.CreateTargetForHwnd(hwnd, false).map_err(err)?;
        let visual = device.CreateVisual().map_err(err)?;
        visual.SetContent(&swap).map_err(err)?;
        target.SetRoot(&visual).map_err(err)?;
        device.Commit().map_err(err)?;
        let comp = Composition {
            _device: device,
            _target: target,
            _visual: visual,
        };
        Ok((swap, comp))
    }
}

/// The GPU objects every window on a thread shares: making a D3D11 and a
/// Direct2D device costs far more than a swap chain, so only the first
/// window pays for it.
#[derive(Clone)]
struct Shared {
    d3d: ID3D11Device,
    dxgi: IDXGIDevice,
    d2d: ID2D1Device,
    factory: IDXGIFactory2,
}

thread_local! {
    static SHARED: RefCell<Option<Shared>> = const { RefCell::new(None) };
    /// Surfaces alive on this thread (they hold the shared device).
    static LIVE: Cell<usize> = const { Cell::new(0) };
    /// Runs when the last surface on this thread goes (set on the UI thread).
    static ON_IDLE: RefCell<Option<Box<dyn Fn()>>> = const { RefCell::new(None) };
}

/// What `release_if_unused` did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Release {
    Released,
    /// A surface still draws with the device.
    Busy,
    /// There was no device.
    Nothing,
}

/// `f` runs on this thread each time its last surface is dropped (the tray
/// schedules `release_if_unused` from it). It must not make or drop
/// surfaces.
pub fn set_on_idle(f: Box<dyn Fn()>) {
    ON_IDLE.with(|c| *c.borrow_mut() = Some(f));
}

/// The thread's shared device, made on first use (on `hwnd`'s GPU).
fn shared(hwnd: HWND) -> Result<Shared, String> {
    if let Some(s) = SHARED.with(|c| c.borrow().clone()) {
        return Ok(s);
    }
    let (d2d_factory, _) = try_factories()?;
    let d3d = device(hwnd)?;
    // SAFETY: COM calls on objects owned here.
    let s = unsafe {
        let dxgi: IDXGIDevice = d3d.cast().map_err(err)?;
        let d2d = d2d_factory.CreateDevice(&dxgi).map_err(err)?;
        let factory: IDXGIFactory2 = dxgi.GetAdapter().map_err(err)?.GetParent().map_err(err)?;
        Shared {
            d3d,
            dxgi,
            d2d,
            factory,
        }
    };
    SHARED.with(|c| *c.borrow_mut() = Some(s.clone()));
    Ok(s)
}

/// Drops the thread's shared device once no surface uses it, so an idle
/// tray does not keep the GPU driver's memory (tens of MB of private
/// bytes); the next window makes a new one.
pub fn release_if_unused() -> Release {
    if LIVE.with(Cell::get) > 0 {
        return Release::Busy;
    }
    match SHARED.with(|c| c.borrow_mut().take()) {
        Some(_) => Release::Released,
        None => Release::Nothing,
    }
}

/// Forgets the shared device if the GPU removed or reset it, so the next
/// surface (re)build makes a new one; other windows follow on their own
/// next device-loss error.
fn drop_shared_if_lost() {
    SHARED.with(|c| {
        let mut g = c.borrow_mut();
        // SAFETY: plain query on a live device.
        if g.as_ref()
            .is_some_and(|s| unsafe { s.d3d.GetDeviceRemovedReason() }.is_err())
        {
            *g = None;
        }
    });
}

/// Builds the context and the swap chain for `want` on the shared device.
fn build(hwnd: HWND, want: Backdrop, size: (u32, u32)) -> Result<(Gfx, Backdrop), String> {
    let Shared {
        d3d,
        dxgi,
        d2d,
        factory,
    } = shared(hwnd)?;
    // SAFETY: COM calls on objects owned here, for our own window.
    unsafe {
        let dc = d2d
            .CreateDeviceContext(D2D1_DEVICE_CONTEXT_OPTIONS_NONE)
            .map_err(err)?;
        let mut comp = None;
        if want == Backdrop::Mica {
            let mica_supported =
                fluent::transparency_on() && fluent::set_backdrop(hwnd, DWMSBT_MAINWINDOW);
            let built = if mica_supported {
                composition(hwnd, &d3d, &dxgi, &factory, size).ok()
            } else {
                None
            };
            if choose(want, mica_supported, built.is_some()) == Backdrop::Mica {
                comp = built;
            } else if mica_supported {
                // Mica would only show in the title bar over an opaque client.
                fluent::set_backdrop(hwnd, DWMSBT_AUTO);
            }
        }
        let (swap, comp, backdrop, alpha) = match comp {
            Some((swap, comp)) => (
                swap,
                Some(comp),
                Backdrop::Mica,
                D2D1_ALPHA_MODE_PREMULTIPLIED,
            ),
            None => {
                let swap = factory
                    .CreateSwapChainForHwnd(&d3d, hwnd, &swap_desc(size, false), None, None)
                    .map_err(err)?;
                (swap, None, Backdrop::Opaque, D2D1_ALPHA_MODE_IGNORE)
            }
        };
        let gfx = Gfx {
            dc,
            target: None,
            swap,
            _comp: comp,
            alpha,
        };
        Ok((gfx, backdrop))
    }
}

fn client_size(hwnd: HWND) -> (u32, u32) {
    let mut r = RECT::default();
    // SAFETY: reads our own window's client rect.
    let _ = unsafe { GetClientRect(hwnd, &mut r) };
    (
        (r.right - r.left).max(1) as u32,
        (r.bottom - r.top).max(1) as u32,
    )
}

/// The device-loss errors after which the whole device is rebuilt.
fn lost(code: HRESULT) -> bool {
    code == D2DERR_RECREATE_TARGET
        || code == DXGI_ERROR_DEVICE_REMOVED
        || code == DXGI_ERROR_DEVICE_RESET
}

/// One window's Direct2D drawing surface. Use it from the window's thread.
pub struct Surface {
    hwnd: HWND,
    want: Backdrop,
    backdrop: Backdrop,
    size: (u32, u32),
    /// None after device loss or a failed resize; rebuilt by the next `draw`.
    gfx: Option<Gfx>,
}

impl Surface {
    /// A surface for `hwnd`: Mica when wanted and available, else opaque.
    /// Err only when nothing can draw at all (no Direct2D, not even WARP).
    pub fn new(hwnd: HWND, want: Backdrop) -> Result<Surface, String> {
        let size = client_size(hwnd);
        let (gfx, backdrop) = build(hwnd, want, size)?;
        LIVE.with(|n| n.set(n.get() + 1));
        Ok(Surface {
            hwnd,
            want,
            backdrop,
            size,
            gfx: Some(gfx),
        })
    }

    /// What the surface actually got (Mica can fall back to opaque).
    pub fn backdrop(&self) -> Backdrop {
        self.backdrop
    }

    /// Resizes the back buffers (call on WM_SIZE, then draw).
    pub fn resize(&mut self, w: u32, h: u32) {
        let size = (w.max(1), h.max(1));
        if size == self.size {
            return;
        }
        self.size = size;
        let Some(gfx) = self.gfx.as_mut() else {
            return;
        };
        // SAFETY: the D2D target is released before ResizeBuffers, as DXGI
        // requires (no outstanding back-buffer references).
        let r = unsafe {
            gfx.dc.SetTarget(None);
            gfx.target = None;
            gfx.swap.ResizeBuffers(
                0,
                size.0,
                size.1,
                DXGI_FORMAT_UNKNOWN,
                DXGI_SWAP_CHAIN_FLAG(0),
            )
        };
        if r.is_err() {
            // Rebuilt at the new size by the next draw.
            self.gfx = None;
        }
    }

    /// Draws one frame with `f` and presents it. After device loss the
    /// device is rebuilt and the window invalidated, and Ok is returned so
    /// the caller simply repaints.
    pub fn draw(&mut self, f: impl FnOnce(&ID2D1DeviceContext)) -> Result<(), String> {
        if self.gfx.is_none() {
            // Lost earlier (e.g. a failed resize): rebuild and still draw
            // this frame, so WM_SIZE paints in the same message.
            self.rebuild()?;
        }
        match self.frame(f) {
            Ok(()) => Ok(()),
            Err(e) if lost(e.code()) => self.rebuild(),
            Err(e) => Err(err(e)),
        }
    }

    fn frame(&mut self, f: impl FnOnce(&ID2D1DeviceContext)) -> windows::core::Result<()> {
        let Some(gfx) = self.gfx.as_mut() else {
            return Ok(());
        };
        // SAFETY: COM calls on this window's own device objects.
        unsafe {
            if gfx.target.is_none() {
                let surface: IDXGISurface = gfx.swap.GetBuffer(0)?;
                // 96 DPI: DIPs are pixels; the caller scales for the monitor.
                let props = D2D1_BITMAP_PROPERTIES1 {
                    pixelFormat: D2D1_PIXEL_FORMAT {
                        format: DXGI_FORMAT_B8G8R8A8_UNORM,
                        alphaMode: gfx.alpha,
                    },
                    dpiX: 96.0,
                    dpiY: 96.0,
                    bitmapOptions: D2D1_BITMAP_OPTIONS_TARGET | D2D1_BITMAP_OPTIONS_CANNOT_DRAW,
                    ..Default::default()
                };
                gfx.target = Some(gfx.dc.CreateBitmapFromDxgiSurface(&surface, Some(&props))?);
            }
            if let Some(target) = &gfx.target {
                gfx.dc.SetTarget(target);
            }
            gfx.dc.BeginDraw();
            f(&gfx.dc);
            gfx.dc.EndDraw(None, None)?;
            // DXGI_STATUS_OCCLUDED (hidden window) is a success code.
            gfx.swap.Present(1, DXGI_PRESENT(0)).ok()
        }
    }

    fn rebuild(&mut self) -> Result<(), String> {
        // Release the old swap chain and DirectComposition target first: a
        // window can hold only one composition target.
        self.gfx = None;
        drop_shared_if_lost();
        let (gfx, backdrop) = build(self.hwnd, self.want, self.size)?;
        self.gfx = Some(gfx);
        self.backdrop = backdrop;
        // SAFETY: invalidates our own window so it repaints on the new device.
        unsafe {
            let _ = InvalidateRect(Some(self.hwnd), None, false);
        }
        Ok(())
    }
}

impl Drop for Surface {
    fn drop(&mut self) {
        // The swap chain and context go first; then the count.
        self.gfx = None;
        let left = LIVE.with(|n| {
            n.set(n.get().saturating_sub(1));
            n.get()
        });
        if left == 0 {
            ON_IDLE.with(|c| {
                if let Some(f) = c.borrow().as_ref() {
                    f();
                }
            });
        }
    }
}

/// A window and its surface from `make_window`.
pub struct Made {
    pub hwnd: HWND,
    pub surface: Surface,
    /// How long the (final) surface took to build.
    pub surface_ms: u128,
}

/// Creates a window with `create` (given the extra extended style it must
/// add) and then its surface. A Mica window has no redirection bitmap, but
/// an HWND swap chain needs one, so when Mica is unavailable (transparency
/// effects off, or it fell through while building) the window is destroyed
/// and remade plain with an opaque surface. On Mica the whole window
/// becomes DWM frame so the backdrop shows wherever the swap chain is
/// transparent. `create` must leave the window hidden and unregistered: a
/// discarded first attempt is simply destroyed.
pub fn make_window(
    want: Backdrop,
    create: impl Fn(WINDOW_EX_STYLE) -> Result<HWND, String>,
) -> Result<Made, String> {
    let mica = want == Backdrop::Mica && fluent::transparency_on();
    let first = if mica {
        Backdrop::Mica
    } else {
        Backdrop::Opaque
    };
    let ex = if mica {
        WS_EX_NOREDIRECTIONBITMAP
    } else {
        WINDOW_EX_STYLE::default()
    };
    let hwnd = create(ex)?;
    let t = std::time::Instant::now();
    match Surface::new(hwnd, first) {
        Ok(surface) if surface.backdrop() == first => {
            if first == Backdrop::Mica {
                let m = MARGINS {
                    cxLeftWidth: -1,
                    cxRightWidth: -1,
                    cyTopHeight: -1,
                    cyBottomHeight: -1,
                };
                // SAFETY: attribute call on the window made above.
                let _ = unsafe { DwmExtendFrameIntoClientArea(hwnd, &m) };
            }
            Ok(Made {
                hwnd,
                surface,
                surface_ms: t.elapsed().as_millis(),
            })
        }
        other => {
            drop(other);
            // SAFETY: the window made above, not yet shown or registered.
            let _ = unsafe { DestroyWindow(hwnd) };
            if first == Backdrop::Mica {
                make_window(Backdrop::Opaque, create)
            } else {
                Err("no Direct2D surface".into())
            }
        }
    }
}

#[cfg(test)]
use windows::{
    core::w,
    Win32::{
        Foundation::{LPARAM, LRESULT, WPARAM},
        System::LibraryLoader::GetModuleHandleW,
        UI::WindowsAndMessaging::{
            CreateWindowExW, DefWindowProcW, RegisterClassW, WNDCLASSW, WS_POPUP,
        },
    },
};

#[cfg(test)]
unsafe extern "system" fn test_proc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    // SAFETY: default handling for the test window's messages.
    unsafe { DefWindowProcW(hwnd, msg, w, l) }
}

/// A hidden 200x100 popup (client area = window size) of a test class.
#[cfg(test)]
pub(crate) fn test_window() -> HWND {
    // SAFETY: registers a throwaway class (a second registration just
    // fails harmlessly) and creates a hidden window on this thread.
    unsafe {
        let instance = GetModuleHandleW(None).expect("module").into();
        let class = WNDCLASSW {
            lpfnWndProc: Some(test_proc),
            hInstance: instance,
            lpszClassName: w!("RelayD2dTest"),
            ..Default::default()
        };
        RegisterClassW(&class);
        CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            w!("RelayD2dTest"),
            w!("d2d test"),
            WS_POPUP,
            0,
            0,
            200,
            100,
            None,
            None,
            Some(instance),
            None,
        )
        .expect("test window")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::Graphics::Direct2D::Common::D2D_RECT_F;

    #[test]
    fn colour_converts_rgb_and_alpha() {
        let c = color(0x3651C5, 0.5);
        assert!(
            (c.r - 0x36 as f32 / 255.).abs() < 1e-6
                && (c.b - 0xC5 as f32 / 255.).abs() < 1e-6
                && c.a == 0.5
        );
    }

    #[test]
    fn mica_needs_both_dwm_and_dcomp() {
        assert!(matches!(choose(Backdrop::Mica, true, true), Backdrop::Mica));
        assert!(matches!(
            choose(Backdrop::Mica, false, true),
            Backdrop::Opaque
        ));
        assert!(matches!(
            choose(Backdrop::Mica, true, false),
            Backdrop::Opaque
        ));
        assert!(matches!(
            choose(Backdrop::Opaque, true, true),
            Backdrop::Opaque
        ));
    }

    #[cfg(windows)]
    #[test]
    fn windows_on_a_thread_share_one_device() {
        let device = || SHARED.with(|c| c.borrow().as_ref().map(|s| s.d3d.as_raw()));
        let (h1, h2) = (test_window(), test_window());
        let s1 = Surface::new(h1, Backdrop::Opaque).expect("surface");
        let first = device().expect("shared device made");
        let s2 = Surface::new(h2, Backdrop::Opaque).expect("surface");
        assert_eq!(device(), Some(first), "the second window reuses it");
        drop((s1, s2));
        // SAFETY: our own test windows, made above on this thread.
        unsafe {
            let _ = DestroyWindow(h1);
            let _ = DestroyWindow(h2);
        }
    }

    #[cfg(windows)]
    #[test]
    fn the_shared_device_is_released_only_after_the_last_surface() {
        use std::rc::Rc;
        let made = || SHARED.with(|c| c.borrow().is_some());
        let idle = Rc::new(Cell::new(0));
        let seen = idle.clone();
        set_on_idle(Box::new(move || seen.set(seen.get() + 1)));
        assert_eq!(release_if_unused(), Release::Nothing, "no device yet");
        let (hwnd, h2) = (test_window(), test_window());
        let s1 = Surface::new(hwnd, Backdrop::Opaque).expect("surface");
        let s2 = Surface::new(h2, Backdrop::Opaque).expect("surface");
        assert!(made());
        assert_eq!(release_if_unused(), Release::Busy, "surfaces still draw");
        drop(s1);
        assert_eq!(idle.get(), 0, "one surface is still alive");
        drop(s2);
        assert_eq!(idle.get(), 1, "the last surface going tells the tray");
        assert_eq!(release_if_unused(), Release::Released);
        assert!(!made(), "released");
        assert_eq!(release_if_unused(), Release::Nothing);
        // The next surface makes a new one.
        let s = Surface::new(hwnd, Backdrop::Opaque).expect("surface again");
        assert!(made());
        drop(s);
        assert_eq!(idle.get(), 2);
        assert_eq!(release_if_unused(), Release::Released);
        // SAFETY: our own test windows, made above on this thread.
        unsafe {
            let _ = DestroyWindow(hwnd);
            let _ = DestroyWindow(h2);
        }
    }

    #[cfg(windows)]
    #[test]
    fn draws_into_a_hidden_window_both_ways() {
        for want in [Backdrop::Opaque, Backdrop::Mica] {
            let hwnd = test_window();
            let mut s = Surface::new(hwnd, want).expect("surface");
            println!("wanted {want:?}, got {:?}", s.backdrop());
            if want == Backdrop::Opaque {
                assert_eq!(s.backdrop(), Backdrop::Opaque);
            }
            // SAFETY: drawing calls between the surface's BeginDraw/EndDraw.
            let fill = |dc: &ID2D1DeviceContext| unsafe {
                dc.Clear(Some(&color(0x202020, 0.0)));
                let brush = dc
                    .CreateSolidColorBrush(&color(0x3651C5, 1.0), None)
                    .expect("brush");
                let r = D2D_RECT_F {
                    left: 10.0,
                    top: 10.0,
                    right: 60.0,
                    bottom: 40.0,
                };
                dc.FillRectangle(&r, &brush);
            };
            s.draw(fill).expect("first draw");
            s.resize(300, 150);
            s.draw(fill).expect("draw after resize");
            drop(s);
            // SAFETY: our own test window, made above on this thread.
            unsafe {
                let _ = DestroyWindow(hwnd);
            }
        }
    }
}
