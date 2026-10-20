//! Presents decoded phone frames: a flip-model swap chain with a waitable
//! frame-latency object (latency 1) on the window's own GPU; the D3D11 video
//! processor converts BT.709 limited YUV to full-range RGB and letterboxes.
#![allow(unsafe_code)]

use windows::{
    core::Interface,
    Win32::{
        Foundation::{HANDLE, HWND, RECT},
        Graphics::{
            Direct3D::{D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_UNKNOWN},
            Direct3D11::{
                D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D,
                ID3D11VideoContext1, ID3D11VideoDevice, ID3D11VideoProcessor,
                ID3D11VideoProcessorEnumerator, ID3D11VideoProcessorInputView,
                ID3D11VideoProcessorOutputView, D3D11_BIND_RENDER_TARGET, D3D11_CPU_ACCESS_READ,
                D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_CREATE_DEVICE_VIDEO_SUPPORT,
                D3D11_MAPPED_SUBRESOURCE, D3D11_MAP_READ, D3D11_SDK_VERSION, D3D11_TEX2D_VPIV,
                D3D11_TEX2D_VPOV, D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT, D3D11_USAGE_STAGING,
                D3D11_VIDEO_COLOR, D3D11_VIDEO_COLOR_0, D3D11_VIDEO_COLOR_RGBA,
                D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE, D3D11_VIDEO_PROCESSOR_CONTENT_DESC,
                D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC, D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC_0,
                D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC, D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC_0,
                D3D11_VIDEO_PROCESSOR_STREAM, D3D11_VIDEO_USAGE_PLAYBACK_NORMAL,
                D3D11_VPIV_DIMENSION_TEXTURE2D, D3D11_VPOV_DIMENSION_TEXTURE2D,
            },
            Dxgi::{
                Common::{
                    DXGI_COLOR_SPACE_RGB_FULL_G22_NONE_P709,
                    DXGI_COLOR_SPACE_YCBCR_STUDIO_G22_LEFT_P709, DXGI_FORMAT,
                    DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_FORMAT_R10G10B10A2_UNORM, DXGI_FORMAT_UNKNOWN,
                    DXGI_SAMPLE_DESC,
                },
                CreateDXGIFactory1, IDXGIAdapter, IDXGIAdapter1, IDXGIDevice, IDXGIFactory1,
                IDXGIFactory2, IDXGISwapChain1, IDXGISwapChain2, DXGI_PRESENT,
                DXGI_SWAP_CHAIN_DESC1, DXGI_SWAP_CHAIN_FLAG_FRAME_LATENCY_WAITABLE_OBJECT,
                DXGI_SWAP_EFFECT_FLIP_DISCARD, DXGI_USAGE_RENDER_TARGET_OUTPUT,
            },
            Gdi::{MonitorFromWindow, MONITOR_DEFAULTTONEAREST},
        },
        System::Threading::WaitForSingleObjectEx,
    },
};

use crate::screen_decode::Frame;

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// A frame this dark on average (0..255) counts as a secure (black) screen.
const BLACK_MEAN: u32 = 3;

/// Whether the 16×16 brightness samples are a fully black frame.
pub fn is_black(samples: &[u8]) -> bool {
    if samples.is_empty() {
        return false;
    }
    let sum: u32 = samples.iter().map(|&b| u32::from(b)).sum();
    sum / samples.len() as u32 <= BLACK_MEAN && samples.iter().all(|&b| b < 64)
}

/// A D3D11 device on the GPU that drives the window's monitor (on hybrid
/// laptops usually the iGPU): no copies between GPUs, the dGPU can sleep.
pub fn device_for_window(hwnd: HWND) -> Result<ID3D11Device, String> {
    // SAFETY: DXGI enumeration and device creation with owned out-params.
    unsafe {
        let factory: IDXGIFactory1 = CreateDXGIFactory1().map_err(err)?;
        let monitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
        let mut chosen: Option<IDXGIAdapter1> = None;
        let mut i = 0;
        'adapters: while let Ok(adapter) = factory.EnumAdapters1(i) {
            let mut j = 0;
            while let Ok(output) = adapter.EnumOutputs(j) {
                if output
                    .GetDesc()
                    .map(|d| d.Monitor == monitor)
                    .unwrap_or(false)
                {
                    chosen = Some(adapter);
                    break 'adapters;
                }
                j += 1;
            }
            i += 1;
        }
        let adapter: Option<IDXGIAdapter> = chosen.and_then(|a| a.cast().ok());
        // An explicit adapter needs the UNKNOWN driver type; none: the default GPU.
        let driver = if adapter.is_some() {
            D3D_DRIVER_TYPE_UNKNOWN
        } else {
            D3D_DRIVER_TYPE_HARDWARE
        };
        let mut device = None;
        D3D11CreateDevice(
            adapter.as_ref(),
            driver,
            Default::default(),
            D3D11_CREATE_DEVICE_VIDEO_SUPPORT | D3D11_CREATE_DEVICE_BGRA_SUPPORT,
            None,
            D3D11_SDK_VERSION,
            Some(&mut device),
            None,
            None,
        )
        .map_err(err)?;
        device.ok_or_else(|| "no D3D11 device".into())
    }
}

struct Processor {
    video: (u32, u32),
    out: (u32, u32),
    enumerator: ID3D11VideoProcessorEnumerator,
    processor: ID3D11VideoProcessor,
}

pub struct Renderer {
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    video_device: ID3D11VideoDevice,
    video_context: ID3D11VideoContext1,
    swap: IDXGISwapChain2,
    waitable: HANDLE,
    format: DXGI_FORMAT,
    size: (u32, u32),
    vp: Option<Processor>,
    last_sample: std::time::Instant,
}

impl Renderer {
    pub fn new(hwnd: HWND, device: ID3D11Device) -> Result<Self, String> {
        // SAFETY: standard swap chain creation for our own window.
        unsafe {
            let context = device.GetImmediateContext().map_err(err)?;
            let video_device: ID3D11VideoDevice = device.cast().map_err(err)?;
            let video_context: ID3D11VideoContext1 = context.cast().map_err(err)?;
            let dxgi: IDXGIDevice = device.cast().map_err(err)?;
            let factory: IDXGIFactory2 =
                dxgi.GetAdapter().map_err(err)?.GetParent().map_err(err)?;
            let mut last = None;
            for format in [DXGI_FORMAT_R10G10B10A2_UNORM, DXGI_FORMAT_B8G8R8A8_UNORM] {
                let desc = DXGI_SWAP_CHAIN_DESC1 {
                    Format: format,
                    SampleDesc: DXGI_SAMPLE_DESC {
                        Count: 1,
                        Quality: 0,
                    },
                    BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
                    BufferCount: 2,
                    SwapEffect: DXGI_SWAP_EFFECT_FLIP_DISCARD,
                    Flags: DXGI_SWAP_CHAIN_FLAG_FRAME_LATENCY_WAITABLE_OBJECT.0 as u32,
                    ..Default::default()
                };
                match factory.CreateSwapChainForHwnd(&device, hwnd, &desc, None, None) {
                    Ok(sc) => {
                        let swap: IDXGISwapChain2 = sc
                            .cast::<IDXGISwapChain1>()
                            .and_then(|s| s.cast())
                            .map_err(err)?;
                        swap.SetMaximumFrameLatency(1).map_err(err)?;
                        let waitable = swap.GetFrameLatencyWaitableObject();
                        let d = swap.GetDesc1().map_err(err)?;
                        return Ok(Self {
                            device: device.clone(),
                            context,
                            video_device,
                            video_context,
                            swap,
                            waitable,
                            format,
                            size: (d.Width, d.Height),
                            vp: None,
                            last_sample: std::time::Instant::now(),
                        });
                    }
                    Err(e) => last = Some(e),
                }
            }
            Err(last.map(err).unwrap_or_else(|| "no swap chain".into()))
        }
    }

    pub fn resize(&mut self, w: u32, h: u32) {
        if w == 0 || h == 0 || (w, h) == self.size {
            return;
        }
        self.vp = None;
        // SAFETY: no back-buffer views are held outside `present`.
        unsafe {
            let _ = self.swap.ResizeBuffers(
                0,
                w,
                h,
                DXGI_FORMAT_UNKNOWN,
                DXGI_SWAP_CHAIN_FLAG_FRAME_LATENCY_WAITABLE_OBJECT,
            );
        }
        self.size = (w, h);
    }

    fn processor(&mut self, video: (u32, u32), out: (u32, u32)) -> Result<&Processor, String> {
        if self
            .vp
            .as_ref()
            .is_none_or(|p| p.video != video || p.out != out)
        {
            let desc = D3D11_VIDEO_PROCESSOR_CONTENT_DESC {
                InputFrameFormat: D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE,
                InputWidth: video.0,
                InputHeight: video.1,
                OutputWidth: out.0,
                OutputHeight: out.1,
                Usage: D3D11_VIDEO_USAGE_PLAYBACK_NORMAL,
                ..Default::default()
            };
            // SAFETY: creating the processor for these sizes.
            let (enumerator, processor) = unsafe {
                let e = self
                    .video_device
                    .CreateVideoProcessorEnumerator(&desc)
                    .map_err(err)?;
                let p = self.video_device.CreateVideoProcessor(&e, 0).map_err(err)?;
                self.video_context.VideoProcessorSetStreamColorSpace1(
                    &p,
                    0,
                    DXGI_COLOR_SPACE_YCBCR_STUDIO_G22_LEFT_P709,
                );
                self.video_context.VideoProcessorSetOutputColorSpace1(
                    &p,
                    DXGI_COLOR_SPACE_RGB_FULL_G22_NONE_P709,
                );
                self.video_context.VideoProcessorSetStreamFrameFormat(
                    &p,
                    0,
                    D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE,
                );
                let black = D3D11_VIDEO_COLOR {
                    Anonymous: D3D11_VIDEO_COLOR_0 {
                        RGBA: D3D11_VIDEO_COLOR_RGBA {
                            R: 0.0,
                            G: 0.0,
                            B: 0.0,
                            A: 1.0,
                        },
                    },
                };
                self.video_context
                    .VideoProcessorSetOutputBackgroundColor(&p, false, &black);
                (e, p)
            };
            self.vp = Some(Processor {
                video,
                out,
                enumerator,
                processor,
            });
        }
        self.vp.as_ref().ok_or_else(|| "no video processor".into())
    }

    /// The frame, letterboxed into `dest` (client coordinates); `dimmed`
    /// darkens it (Reconnecting…).
    pub fn present(&mut self, f: &Frame, dest: RECT, dimmed: bool) -> Result<(), String> {
        // SAFETY: D3D11 video processing into our back buffer; views are
        // released before the next resize.
        unsafe {
            let _ = WaitForSingleObjectEx(self.waitable, 100, true);
            let mut td = D3D11_TEXTURE2D_DESC::default();
            f.texture.GetDesc(&mut td);
            let size = self.size;
            let (enumerator, processor) = {
                let p = self.processor((td.Width, td.Height), size)?;
                (p.enumerator.clone(), p.processor.clone())
            };
            let back: ID3D11Texture2D = self.swap.GetBuffer(0).map_err(err)?;
            let out = self.output_view(&back, &enumerator)?;
            let input = self.input_view(&f.texture, f.index, &enumerator)?;
            self.video_context
                .VideoProcessorSetStreamDestRect(&processor, 0, true, Some(&dest));
            self.video_context.VideoProcessorSetStreamAlpha(
                &processor,
                0,
                dimmed,
                if dimmed { 0.4 } else { 1.0 },
            );
            let stream = D3D11_VIDEO_PROCESSOR_STREAM {
                Enable: true.into(),
                pInputSurface: std::mem::ManuallyDrop::new(Some(input)),
                ..Default::default()
            };
            let mut streams = [stream];
            let r = self
                .video_context
                .VideoProcessorBlt(&processor, &out, 0, &streams);
            std::mem::ManuallyDrop::drop(&mut streams[0].pInputSurface);
            r.map_err(err)?;
            self.swap.Present(1, DXGI_PRESENT(0)).ok().map_err(err)?;
        }
        Ok(())
    }

    fn output_view(
        &self,
        tex: &ID3D11Texture2D,
        e: &ID3D11VideoProcessorEnumerator,
    ) -> Result<ID3D11VideoProcessorOutputView, String> {
        let desc = D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC {
            ViewDimension: D3D11_VPOV_DIMENSION_TEXTURE2D,
            Anonymous: D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC_0 {
                Texture2D: D3D11_TEX2D_VPOV { MipSlice: 0 },
            },
        };
        let mut v = None;
        // SAFETY: a view of our own texture.
        unsafe {
            self.video_device
                .CreateVideoProcessorOutputView(tex, e, &desc, Some(&mut v))
                .map_err(err)?;
        }
        v.ok_or_else(|| "no output view".into())
    }

    fn input_view(
        &self,
        tex: &ID3D11Texture2D,
        index: u32,
        e: &ID3D11VideoProcessorEnumerator,
    ) -> Result<ID3D11VideoProcessorInputView, String> {
        let desc = D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC {
            FourCC: 0,
            ViewDimension: D3D11_VPIV_DIMENSION_TEXTURE2D,
            Anonymous: D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC_0 {
                Texture2D: D3D11_TEX2D_VPIV {
                    MipSlice: 0,
                    ArraySlice: index,
                },
            },
        };
        let mut v = None;
        // SAFETY: a view of the decoder's texture slice.
        unsafe {
            self.video_device
                .CreateVideoProcessorInputView(tex, e, &desc, Some(&mut v))
                .map_err(err)?;
        }
        v.ok_or_else(|| "no input view".into())
    }

    /// 16×16 brightness of `f`, at most once a second (secure-screen note).
    pub fn mean_brightness(&mut self, f: &Frame) -> Option<bool> {
        if self.last_sample.elapsed() < std::time::Duration::from_secs(1) {
            return None;
        }
        self.last_sample = std::time::Instant::now();
        // SAFETY: a tiny render target and staging copy owned here.
        unsafe {
            let mut td = D3D11_TEXTURE2D_DESC::default();
            f.texture.GetDesc(&mut td);
            let small_desc = D3D11_TEXTURE2D_DESC {
                Width: 16,
                Height: 16,
                MipLevels: 1,
                ArraySize: 1,
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                SampleDesc: DXGI_SAMPLE_DESC {
                    Count: 1,
                    Quality: 0,
                },
                Usage: D3D11_USAGE_DEFAULT,
                BindFlags: D3D11_BIND_RENDER_TARGET.0 as u32,
                ..Default::default()
            };
            let mut small = None;
            self.device
                .CreateTexture2D(&small_desc, None, Some(&mut small))
                .ok()?;
            let small = small?;
            let desc = D3D11_VIDEO_PROCESSOR_CONTENT_DESC {
                InputFrameFormat: D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE,
                InputWidth: td.Width,
                InputHeight: td.Height,
                OutputWidth: 16,
                OutputHeight: 16,
                Usage: D3D11_VIDEO_USAGE_PLAYBACK_NORMAL,
                ..Default::default()
            };
            let e = self
                .video_device
                .CreateVideoProcessorEnumerator(&desc)
                .ok()?;
            let p = self.video_device.CreateVideoProcessor(&e, 0).ok()?;
            let out = self.output_view(&small, &e).ok()?;
            let input = self.input_view(&f.texture, f.index, &e).ok()?;
            let mut streams = [D3D11_VIDEO_PROCESSOR_STREAM {
                Enable: true.into(),
                pInputSurface: std::mem::ManuallyDrop::new(Some(input)),
                ..Default::default()
            }];
            let blt = self.video_context.VideoProcessorBlt(&p, &out, 0, &streams);
            std::mem::ManuallyDrop::drop(&mut streams[0].pInputSurface);
            blt.ok()?;
            let staging_desc = D3D11_TEXTURE2D_DESC {
                Usage: D3D11_USAGE_STAGING,
                BindFlags: 0,
                CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
                ..small_desc
            };
            let mut staging = None;
            self.device
                .CreateTexture2D(&staging_desc, None, Some(&mut staging))
                .ok()?;
            let staging = staging?;
            self.context.CopyResource(&staging, &small);
            let mut map = D3D11_MAPPED_SUBRESOURCE::default();
            self.context
                .Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut map))
                .ok()?;
            let mut samples = Vec::with_capacity(256);
            for row in 0..16usize {
                let line = std::slice::from_raw_parts(
                    (map.pData as *const u8).add(row * map.RowPitch as usize),
                    64,
                );
                // BGRA: brightness from G (close enough for a black test).
                samples.extend(line.chunks(4).map(|px| px[1]));
            }
            self.context.Unmap(&staging, 0);
            Some(is_black(&samples))
        }
    }

    pub fn format_is_10_bit(&self) -> bool {
        self.format == DXGI_FORMAT_R10G10B10A2_UNORM
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secure_screen_detection_threshold() {
        assert!(is_black(&[0; 256]));
        let mut one_bright = [0u8; 256];
        one_bright[17] = 255;
        assert!(!is_black(&one_bright));
        assert!(!is_black(&[40; 256]), "a dark but visible screen");
        assert!(!is_black(&[]));
    }
}
