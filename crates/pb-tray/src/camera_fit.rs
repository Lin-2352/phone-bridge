//! The camera's GPU step: a decoded frame is rotated, scaled and pillarboxed
//! into a fixed 16:9 NV12 texture by the D3D11 video processor, read back
//! through two alternating staging textures, and copied row by row (the
//! GPU's `RowPitch` padding stripped) into a tightly packed NV12 buffer.
#![allow(unsafe_code)]

use pb_camera_frames::{copy_nv12, fit};
use windows::{
    core::Interface,
    Win32::{
        Foundation::RECT,
        Graphics::{
            Direct3D11::{
                D3D11_VIDEO_COLOR_YCbCrA, ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D,
                ID3D11VideoContext1, ID3D11VideoDevice, ID3D11VideoProcessor,
                ID3D11VideoProcessorEnumerator, ID3D11VideoProcessorInputView,
                ID3D11VideoProcessorOutputView, D3D11_BIND_RENDER_TARGET, D3D11_CPU_ACCESS_READ,
                D3D11_MAPPED_SUBRESOURCE, D3D11_MAP_READ, D3D11_TEX2D_VPIV, D3D11_TEX2D_VPOV,
                D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT, D3D11_USAGE_STAGING, D3D11_VIDEO_COLOR,
                D3D11_VIDEO_COLOR_0, D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE,
                D3D11_VIDEO_PROCESSOR_CONTENT_DESC, D3D11_VIDEO_PROCESSOR_FORMAT_SUPPORT_OUTPUT,
                D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC, D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC_0,
                D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC, D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC_0,
                D3D11_VIDEO_PROCESSOR_ROTATION, D3D11_VIDEO_PROCESSOR_ROTATION_180,
                D3D11_VIDEO_PROCESSOR_ROTATION_270, D3D11_VIDEO_PROCESSOR_ROTATION_90,
                D3D11_VIDEO_PROCESSOR_ROTATION_IDENTITY, D3D11_VIDEO_PROCESSOR_STREAM,
                D3D11_VIDEO_USAGE_PLAYBACK_NORMAL, D3D11_VPIV_DIMENSION_TEXTURE2D,
                D3D11_VPOV_DIMENSION_TEXTURE2D,
            },
            Dxgi::Common::{
                DXGI_COLOR_SPACE_YCBCR_STUDIO_G22_LEFT_P709, DXGI_FORMAT_NV12, DXGI_SAMPLE_DESC,
            },
        },
    },
};

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

struct Resources {
    video: (u32, u32),
    out: (u32, u32),
    enumerator: ID3D11VideoProcessorEnumerator,
    processor: ID3D11VideoProcessor,
    target: ID3D11Texture2D,
    staging: [ID3D11Texture2D; 2],
}

pub struct Fitter {
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    video_device: ID3D11VideoDevice,
    video_context: ID3D11VideoContext1,
    res: Option<Resources>,
    /// The staging texture the next frame goes into.
    next: usize,
    /// A frame is waiting in the other staging texture.
    primed: bool,
}

fn nv12_desc(w: u32, h: u32) -> D3D11_TEXTURE2D_DESC {
    D3D11_TEXTURE2D_DESC {
        Width: w,
        Height: h,
        MipLevels: 1,
        ArraySize: 1,
        Format: DXGI_FORMAT_NV12,
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        ..Default::default()
    }
}

fn rotation(deg: u32) -> D3D11_VIDEO_PROCESSOR_ROTATION {
    match deg % 360 {
        90 => D3D11_VIDEO_PROCESSOR_ROTATION_90,
        180 => D3D11_VIDEO_PROCESSOR_ROTATION_180,
        270 => D3D11_VIDEO_PROCESSOR_ROTATION_270,
        _ => D3D11_VIDEO_PROCESSOR_ROTATION_IDENTITY,
    }
}

impl Fitter {
    pub fn new(device: &ID3D11Device) -> Result<Self, String> {
        // SAFETY: plain COM queries on a valid device.
        unsafe {
            let context = device.GetImmediateContext().map_err(err)?;
            Ok(Self {
                device: device.clone(),
                video_device: device.cast().map_err(err)?,
                video_context: context.cast().map_err(err)?,
                context,
                res: None,
                next: 0,
                primed: false,
            })
        }
    }

    fn resources(&mut self, video: (u32, u32), out: (u32, u32)) -> Result<&Resources, String> {
        if self
            .res
            .as_ref()
            .is_none_or(|r| r.video != video || r.out != out)
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
            // SAFETY: creating the processor and textures for these sizes.
            let res = unsafe {
                let enumerator = self
                    .video_device
                    .CreateVideoProcessorEnumerator(&desc)
                    .map_err(err)?;
                let support = enumerator
                    .CheckVideoProcessorFormat(DXGI_FORMAT_NV12)
                    .map_err(err)?;
                if support & (D3D11_VIDEO_PROCESSOR_FORMAT_SUPPORT_OUTPUT.0 as u32) == 0 {
                    return Err("this GPU cannot write NV12 from the video processor".into());
                }
                let processor = self
                    .video_device
                    .CreateVideoProcessor(&enumerator, 0)
                    .map_err(err)?;
                self.video_context.VideoProcessorSetStreamColorSpace1(
                    &processor,
                    0,
                    DXGI_COLOR_SPACE_YCBCR_STUDIO_G22_LEFT_P709,
                );
                self.video_context.VideoProcessorSetOutputColorSpace1(
                    &processor,
                    DXGI_COLOR_SPACE_YCBCR_STUDIO_G22_LEFT_P709,
                );
                self.video_context.VideoProcessorSetStreamFrameFormat(
                    &processor,
                    0,
                    D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE,
                );
                // Black in studio-range YCbCr (Y 16, Cb = Cr = 128): the bars
                // are real black, not below-black.
                let black = D3D11_VIDEO_COLOR {
                    Anonymous: D3D11_VIDEO_COLOR_0 {
                        YCbCr: D3D11_VIDEO_COLOR_YCbCrA {
                            Y: 16.0 / 255.0,
                            Cb: 128.0 / 255.0,
                            Cr: 128.0 / 255.0,
                            A: 1.0,
                        },
                    },
                };
                self.video_context
                    .VideoProcessorSetOutputBackgroundColor(&processor, true, &black);
                let mut tdesc = nv12_desc(out.0, out.1);
                tdesc.Usage = D3D11_USAGE_DEFAULT;
                tdesc.BindFlags = D3D11_BIND_RENDER_TARGET.0 as u32;
                let mut target = None;
                self.device
                    .CreateTexture2D(&tdesc, None, Some(&mut target))
                    .map_err(|e| format!("NV12 render target: {e}"))?;
                let mut sdesc = nv12_desc(out.0, out.1);
                sdesc.Usage = D3D11_USAGE_STAGING;
                sdesc.CPUAccessFlags = D3D11_CPU_ACCESS_READ.0 as u32;
                let mut staging = [None, None];
                for s in &mut staging {
                    self.device
                        .CreateTexture2D(&sdesc, None, Some(s))
                        .map_err(|e| format!("NV12 staging texture: {e}"))?;
                }
                Resources {
                    video,
                    out,
                    enumerator,
                    processor,
                    target: target.ok_or("no target texture")?,
                    staging: [
                        staging[0].take().ok_or("no staging texture")?,
                        staging[1].take().ok_or("no staging texture")?,
                    ],
                }
            };
            self.res = Some(res);
            self.primed = false;
            self.next = 0;
        }
        self.res.as_ref().ok_or_else(|| "no resources".into())
    }

    /// `tex` (array slice `index`; its video area is `video`, decoders pad
    /// the texture) rotated by `rotation`, fitted into `out`, as tightly
    /// packed NV12 in `dst` (`out.0 * out.1 * 3 / 2` bytes). With the
    /// double buffering the bytes are those of the previous call after the
    /// first (one frame of latency, no GPU stall).
    pub fn fit_into(
        &mut self,
        tex: &ID3D11Texture2D,
        index: u32,
        video: (u32, u32),
        rot: u32,
        out: (u32, u32),
        dst: &mut [u8],
    ) -> Result<(), String> {
        assert!(dst.len() >= (out.0 * out.1 * 3 / 2) as usize);
        let mut td = D3D11_TEXTURE2D_DESC::default();
        // SAFETY: a plain query.
        unsafe { tex.GetDesc(&mut td) };
        self.resources((td.Width, td.Height), out)?;
        let r = fit(video, rot, out);
        let next = self.next;
        let primed = self.primed;
        let res = self.res.as_ref().ok_or("no resources")?;
        // SAFETY: video processing into our own target, then a copy into the
        // staging texture we map afterwards; views live for the call.
        unsafe {
            let src_rect = RECT {
                left: 0,
                top: 0,
                right: video.0.min(td.Width) as i32,
                bottom: video.1.min(td.Height) as i32,
            };
            let dest = RECT {
                left: r.x as i32,
                top: r.y as i32,
                right: (r.x + r.w) as i32,
                bottom: (r.y + r.h) as i32,
            };
            self.video_context.VideoProcessorSetStreamSourceRect(
                &res.processor,
                0,
                true,
                Some(&src_rect),
            );
            self.video_context.VideoProcessorSetStreamDestRect(
                &res.processor,
                0,
                true,
                Some(&dest),
            );
            self.video_context.VideoProcessorSetStreamRotation(
                &res.processor,
                0,
                !rot.is_multiple_of(360),
                rotation(rot),
            );
            let in_desc = D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC {
                FourCC: 0,
                ViewDimension: D3D11_VPIV_DIMENSION_TEXTURE2D,
                Anonymous: D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC_0 {
                    Texture2D: D3D11_TEX2D_VPIV {
                        MipSlice: 0,
                        ArraySlice: index,
                    },
                },
            };
            let mut input: Option<ID3D11VideoProcessorInputView> = None;
            self.video_device
                .CreateVideoProcessorInputView(tex, &res.enumerator, &in_desc, Some(&mut input))
                .map_err(|e| format!("input view: {e}"))?;
            let out_desc = D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC {
                ViewDimension: D3D11_VPOV_DIMENSION_TEXTURE2D,
                Anonymous: D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC_0 {
                    Texture2D: D3D11_TEX2D_VPOV { MipSlice: 0 },
                },
            };
            let mut output: Option<ID3D11VideoProcessorOutputView> = None;
            self.video_device
                .CreateVideoProcessorOutputView(
                    &res.target,
                    &res.enumerator,
                    &out_desc,
                    Some(&mut output),
                )
                .map_err(|e| format!("output view: {e}"))?;
            let stream = D3D11_VIDEO_PROCESSOR_STREAM {
                Enable: true.into(),
                pInputSurface: std::mem::ManuallyDrop::new(input),
                ..Default::default()
            };
            let mut streams = [stream];
            let blt = self.video_context.VideoProcessorBlt(
                &res.processor,
                output.as_ref().ok_or("no output view")?,
                0,
                &streams,
            );
            std::mem::ManuallyDrop::drop(&mut streams[0].pInputSurface);
            blt.map_err(|e| format!("blt: {e}"))?;
            self.context.CopyResource(&res.staging[next], &res.target);
            // The first frame is read at once; later ones one frame behind.
            let read = if primed { 1 - next } else { next };
            let mut m = D3D11_MAPPED_SUBRESOURCE::default();
            self.context
                .Map(&res.staging[read], 0, D3D11_MAP_READ, 0, Some(&mut m))
                .map_err(err)?;
            let pitch = m.RowPitch as usize;
            let (w, h) = (out.0 as usize, out.1 as usize);
            let base = m.pData as *const u8;
            let y = std::slice::from_raw_parts(base, pitch * h);
            let uv = std::slice::from_raw_parts(base.add(pitch * h), pitch * (h / 2));
            copy_nv12(y, uv, pitch, w, h, &mut dst[..w * h * 3 / 2]);
            self.context.Unmap(&res.staging[read], 0);
        }
        self.next = 1 - next;
        self.primed = true;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use windows::Win32::Graphics::Direct3D11::{
        D3D11_BIND_SHADER_RESOURCE, D3D11_SUBRESOURCE_DATA,
    };

    use super::*;
    use crate::screen_render::device_for_window;

    /// A gray-ish NV12 texture, Y = 200, Cb = 90, Cr = 200, for the GPU test.
    fn source(device: &ID3D11Device, w: u32, h: u32) -> Option<ID3D11Texture2D> {
        let mut data = vec![200u8; (w * h) as usize];
        for _ in 0..(w * h / 4) {
            data.push(90);
            data.push(200);
        }
        let mut desc = nv12_desc(w, h);
        desc.Usage = D3D11_USAGE_DEFAULT;
        desc.BindFlags = (D3D11_BIND_SHADER_RESOURCE.0 | D3D11_BIND_RENDER_TARGET.0) as u32;
        let init = D3D11_SUBRESOURCE_DATA {
            pSysMem: data.as_ptr().cast(),
            SysMemPitch: w,
            SysMemSlicePitch: 0,
        };
        let mut tex = None;
        // SAFETY: creating a texture from our buffer, copied by the driver.
        unsafe { device.CreateTexture2D(&desc, Some(&init), Some(&mut tex)) }.ok()?;
        tex
    }

    #[test]
    fn gpu_fit_writes_tight_pillarboxed_nv12() {
        let Ok(device) = device_for_window(Default::default()) else {
            eprintln!("no D3D11 device here (CI): skipped");
            return;
        };
        let Some(tex) = source(&device, 540, 960) else {
            eprintln!("this GPU cannot make an NV12 texture with data: skipped");
            return;
        };
        let mut fitter = Fitter::new(&device).unwrap();
        let (w, h) = (1280usize, 720usize);
        let mut dst = vec![0u8; w * h * 3 / 2];
        // Twice: the second call returns the (identical) previous frame.
        for _ in 0..2 {
            match fitter.fit_into(&tex, 0, (540, 960), 0, (1280, 720), &mut dst) {
                Ok(()) => {}
                Err(e) if std::env::var_os("CI").is_some() => {
                    eprintln!("GPU cannot do the NV12 fit on CI ({e}): skipped");
                    return;
                }
                Err(e) => panic!("fit failed: {e}"),
            }
        }
        let y = |row: usize, col: usize| dst[row * w + col];
        let uv = |row: usize, col: usize| {
            let o = w * h + (row / 2) * w + (col & !1);
            (dst[o], dst[o + 1])
        };
        // Portrait 540x960 in 1280x720: a 406-wide picture centred at x 436.
        for col in [0usize, 100, 400] {
            assert!(
                (i32::from(y(360, col)) - 16).abs() <= 6,
                "pillar Y at {col}"
            );
            let (cb, cr) = uv(360, col);
            assert!(
                (i32::from(cb) - 128).abs() <= 6 && (i32::from(cr) - 128).abs() <= 6,
                "pillar UV at {col}: {cb},{cr}"
            );
        }
        assert!((i32::from(y(360, 640)) - 200).abs() <= 8, "picture Y");
        let (cb, cr) = uv(360, 640);
        assert!(
            (i32::from(cb) - 90).abs() <= 8 && (i32::from(cr) - 200).abs() <= 8,
            "picture UV (a wrong plane offset shows here): {cb},{cr}"
        );
    }
}
