//! Hardware video decoding for the phone-screen window: Media Foundation's
//! synchronous decoder MFTs on the window's D3D11 device (DXVA), in
//! low-latency mode (one frame in, one frame out). Frames stay on the GPU.
#![allow(unsafe_code)]

use std::sync::OnceLock;

use pb_video::{
    record::{Config, Record, FLAG_CONFIG, FLAG_KEY},
    Codec,
};
use windows::{
    core::{Interface, GUID},
    Win32::{
        Graphics::Direct3D11::{ID3D11Device, ID3D11Multithread, ID3D11Texture2D},
        Media::MediaFoundation::{
            IMFActivate, IMFDXGIBuffer, IMFDXGIDeviceManager, IMFMediaType, IMFSample,
            IMFTransform, MFCreateDXGIDeviceManager, MFCreateMediaType, MFCreateMemoryBuffer,
            MFCreateSample, MFMediaType_Video, MFStartup, MFTEnumEx, MFVideoFormat_H264,
            MFVideoFormat_HEVC, MFVideoFormat_NV12, MFVideoFormat_P010,
            MFVideoInterlace_Progressive, MFSTARTUP_NOSOCKET, MFT_CATEGORY_VIDEO_DECODER,
            MFT_ENUM_FLAG_SORTANDFILTER, MFT_ENUM_FLAG_SYNCMFT, MFT_MESSAGE_NOTIFY_BEGIN_STREAMING,
            MFT_MESSAGE_NOTIFY_START_OF_STREAM, MFT_MESSAGE_SET_D3D_MANAGER,
            MFT_OUTPUT_DATA_BUFFER, MFT_OUTPUT_STREAM_PROVIDES_SAMPLES, MFT_REGISTER_TYPE_INFO,
            MF_E_NOTACCEPTING, MF_E_TRANSFORM_NEED_MORE_INPUT, MF_E_TRANSFORM_STREAM_CHANGE,
            MF_LOW_LATENCY, MF_MT_FRAME_SIZE, MF_MT_INTERLACE_MODE, MF_MT_MAJOR_TYPE,
            MF_MT_SUBTYPE, MF_SA_D3D11_AWARE, MF_VERSION,
        },
        System::Com::CoTaskMemFree,
    },
};

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// Media Foundation, started once for the process.
fn startup() -> Result<(), String> {
    static STARTED: OnceLock<Result<(), String>> = OnceLock::new();
    STARTED
        // SAFETY: plain library start-up; never shut down (process lifetime).
        .get_or_init(|| unsafe { MFStartup(MF_VERSION, MFSTARTUP_NOSOCKET) }.map_err(err))
        .clone()
}

fn subtype(c: Codec) -> GUID {
    match c {
        Codec::H264 => MFVideoFormat_H264,
        Codec::H265 | Codec::H265Main10 => MFVideoFormat_HEVC,
    }
}

/// Synchronous decoder MFTs for this input type, best first.
fn decoders(input: GUID) -> Vec<IMFActivate> {
    let info = MFT_REGISTER_TYPE_INFO {
        guidMajorType: MFMediaType_Video,
        guidSubtype: input,
    };
    let mut list: *mut Option<IMFActivate> = std::ptr::null_mut();
    let mut n = 0u32;
    // SAFETY: MFTEnumEx fills a CoTaskMem array of `n` activates; each is
    // taken out once and the array freed.
    unsafe {
        if MFTEnumEx(
            MFT_CATEGORY_VIDEO_DECODER,
            MFT_ENUM_FLAG_SYNCMFT | MFT_ENUM_FLAG_SORTANDFILTER,
            Some(&info),
            None,
            &mut list,
            &mut n,
        )
        .is_err()
            || list.is_null()
        {
            return Vec::new();
        }
        let out = (0..n as usize)
            .filter_map(|i| (*list.add(i)).take())
            .collect();
        CoTaskMemFree(Some(list as *const _));
        out
    }
}

/// What this PC can decode, best first (sent to the phone in `ScreenStart`).
pub fn supported_codecs() -> Vec<Codec> {
    if startup().is_err() {
        return vec![Codec::H264];
    }
    let mut v = Vec::new();
    if !decoders(MFVideoFormat_HEVC).is_empty() {
        // The HEVC decoder handles Main10 where the GPU does (RTX 40 / Intel Xe).
        v.extend([Codec::H265Main10, Codec::H265]);
    }
    if !decoders(MFVideoFormat_H264).is_empty() {
        v.push(Codec::H264);
    }
    v
}

/// A decoded frame, on the GPU.
pub struct Frame {
    pub texture: ID3D11Texture2D,
    /// Array slice of `texture` (decoders output into texture arrays).
    pub index: u32,
    pub pts_us: i64,
}

pub struct Decoder {
    device: ID3D11Device,
    codec: Codec,
    mft: Option<IMFTransform>,
    _manager: Option<IMFDXGIDeviceManager>,
    config: Option<Config>,
    provides_samples: bool,
}

impl Decoder {
    pub fn new(device: &ID3D11Device, codec: Codec) -> Result<Self, String> {
        startup()?;
        // The decoder and the renderer share the device from two threads.
        // SAFETY: plain COM calls on a valid device.
        unsafe {
            let _ = device
                .cast::<ID3D11Multithread>()
                .map_err(err)?
                .SetMultithreadProtected(true);
        }
        Ok(Self {
            device: device.clone(),
            codec,
            mft: None,
            _manager: None,
            config: None,
            provides_samples: false,
        })
    }

    /// One record in; a frame out when the decoder has one.
    pub fn push(&mut self, r: &Record) -> Result<Option<Frame>, String> {
        if r.flags & FLAG_CONFIG != 0 {
            let cfg = Config::decode(&r.payload).ok_or("bad config record")?;
            let resized = self
                .config
                .as_ref()
                .is_none_or(|c| (c.width, c.height) != (cfg.width, cfg.height));
            if resized {
                // A new size or rotation: a fresh decoder at the next key frame.
                self.mft = None;
            }
            self.config = Some(cfg);
            return Ok(None);
        }
        let key = r.flags & FLAG_KEY != 0;
        if self.mft.is_none() {
            if !key || self.config.is_none() {
                return Ok(None);
            }
            self.open()?;
        }
        let mut data = Vec::new();
        if key {
            if let Some(c) = &self.config {
                data.extend_from_slice(&c.headers);
            }
        }
        data.extend_from_slice(&r.payload);
        let sample = make_sample(&data, r.pts_us)?;
        let mft = self.mft.clone().ok_or("no decoder")?;
        // SAFETY: valid MFT and sample; output drained when it is full.
        unsafe {
            if let Err(e) = mft.ProcessInput(0, &sample, 0) {
                if e.code() != MF_E_NOTACCEPTING {
                    return Err(err(e));
                }
                let frame = self.drain()?;
                mft.ProcessInput(0, &sample, 0).map_err(err)?;
                if frame.is_some() {
                    return Ok(frame);
                }
            }
        }
        self.drain()
    }

    fn open(&mut self) -> Result<(), String> {
        let cfg = self.config.clone().ok_or("no config")?;
        let act = decoders(subtype(self.codec))
            .into_iter()
            .next()
            .ok_or("no decoder for this codec")?;
        // SAFETY: standard MFT set-up on valid objects.
        unsafe {
            let mft: IMFTransform = act.ActivateObject().map_err(err)?;
            let attrs = mft.GetAttributes().map_err(err)?;
            let _ = attrs.SetUINT32(&MF_LOW_LATENCY, 1);
            let mut manager = None;
            if attrs.GetUINT32(&MF_SA_D3D11_AWARE).unwrap_or(0) != 0 {
                let mut token = 0u32;
                let mut m: Option<IMFDXGIDeviceManager> = None;
                MFCreateDXGIDeviceManager(&mut token, &mut m).map_err(err)?;
                let m = m.ok_or("no device manager")?;
                m.ResetDevice(&self.device, token).map_err(err)?;
                mft.ProcessMessage(MFT_MESSAGE_SET_D3D_MANAGER, m.as_raw() as usize)
                    .map_err(err)?;
                manager = Some(m);
            }
            let input: IMFMediaType = MFCreateMediaType().map_err(err)?;
            input
                .SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)
                .map_err(err)?;
            input
                .SetGUID(&MF_MT_SUBTYPE, &subtype(self.codec))
                .map_err(err)?;
            input
                .SetUINT64(
                    &MF_MT_FRAME_SIZE,
                    (u64::from(cfg.width) << 32) | u64::from(cfg.height),
                )
                .map_err(err)?;
            input
                .SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)
                .map_err(err)?;
            mft.SetInputType(0, &input, 0).map_err(err)?;
            self.mft = Some(mft.clone());
            self._manager = manager;
            self.set_output()?;
            mft.ProcessMessage(MFT_MESSAGE_NOTIFY_BEGIN_STREAMING, 0)
                .map_err(err)?;
            mft.ProcessMessage(MFT_MESSAGE_NOTIFY_START_OF_STREAM, 0)
                .map_err(err)?;
        }
        Ok(())
    }

    /// NV12, or P010 for 10-bit streams.
    fn set_output(&mut self) -> Result<(), String> {
        let mft = self.mft.clone().ok_or("no decoder")?;
        let want = if self.codec == Codec::H265Main10 {
            [MFVideoFormat_P010, MFVideoFormat_NV12]
        } else {
            [MFVideoFormat_NV12, MFVideoFormat_P010]
        };
        // SAFETY: enumerating and setting output types on a valid MFT.
        unsafe {
            for w in want {
                let mut i = 0;
                while let Ok(t) = mft.GetOutputAvailableType(0, i) {
                    if t.GetGUID(&MF_MT_SUBTYPE).ok() == Some(w) {
                        mft.SetOutputType(0, &t, 0).map_err(err)?;
                        let info = mft.GetOutputStreamInfo(0).map_err(err)?;
                        self.provides_samples =
                            info.dwFlags & MFT_OUTPUT_STREAM_PROVIDES_SAMPLES.0 as u32 != 0;
                        return Ok(());
                    }
                    i += 1;
                }
            }
        }
        Err("the decoder offers no NV12/P010 output".into())
    }

    /// The next decoded frame, if any.
    fn drain(&mut self) -> Result<Option<Frame>, String> {
        let mft = self.mft.clone().ok_or("no decoder")?;
        loop {
            // SAFETY: one output buffer; the sample it returns is released
            // below (ManuallyDrop).
            let r = unsafe {
                let mut buf = MFT_OUTPUT_DATA_BUFFER {
                    dwStreamID: 0,
                    ..Default::default()
                };
                if !self.provides_samples {
                    let info = mft.GetOutputStreamInfo(0).map_err(err)?;
                    let s = MFCreateSample().map_err(err)?;
                    s.AddBuffer(&MFCreateMemoryBuffer(info.cbSize.max(1)).map_err(err)?)
                        .map_err(err)?;
                    buf.pSample = std::mem::ManuallyDrop::new(Some(s));
                }
                let mut status = 0u32;
                let res = mft.ProcessOutput(0, std::slice::from_mut(&mut buf), &mut status);
                let sample = std::mem::ManuallyDrop::take(&mut buf.pSample);
                drop(std::mem::ManuallyDrop::take(&mut buf.pEvents));
                res.map(|_| sample)
            };
            match r {
                Ok(Some(sample)) => return frame_of(&sample).map(Some),
                Ok(None) => return Ok(None),
                Err(e) if e.code() == MF_E_TRANSFORM_NEED_MORE_INPUT => return Ok(None),
                Err(e) if e.code() == MF_E_TRANSFORM_STREAM_CHANGE => self.set_output()?,
                Err(e) => return Err(err(e)),
            }
        }
    }
}

fn make_sample(data: &[u8], pts_us: i64) -> Result<IMFSample, String> {
    // SAFETY: the buffer is locked for exactly `data.len()` bytes it owns.
    unsafe {
        let buf = MFCreateMemoryBuffer(data.len() as u32).map_err(err)?;
        let mut ptr = std::ptr::null_mut();
        buf.Lock(&mut ptr, None, None).map_err(err)?;
        std::ptr::copy_nonoverlapping(data.as_ptr(), ptr, data.len());
        buf.Unlock().map_err(err)?;
        buf.SetCurrentLength(data.len() as u32).map_err(err)?;
        let s = MFCreateSample().map_err(err)?;
        s.AddBuffer(&buf).map_err(err)?;
        // 100 ns units.
        s.SetSampleTime(pts_us * 10).map_err(err)?;
        Ok(s)
    }
}

fn frame_of(sample: &IMFSample) -> Result<Frame, String> {
    // SAFETY: the sample's first buffer is a DXGI buffer when the decoder
    // runs on our device; its resource is a texture (array).
    unsafe {
        let pts_us = sample.GetSampleTime().unwrap_or(0) / 10;
        let buf = sample.GetBufferByIndex(0).map_err(err)?;
        let dxgi: IMFDXGIBuffer = buf
            .cast()
            .map_err(|_| "the decoder is not using the GPU".to_string())?;
        let mut tex: *mut core::ffi::c_void = std::ptr::null_mut();
        dxgi.GetResource(&ID3D11Texture2D::IID, &mut tex)
            .map_err(err)?;
        let texture = ID3D11Texture2D::from_raw(tex);
        let index = dxgi.GetSubresourceIndex().map_err(err)?;
        Ok(Frame {
            texture,
            index,
            pts_us,
        })
    }
}

#[cfg(test)]
mod tests {
    use windows::Win32::{
        Graphics::{
            Direct3D::{D3D_DRIVER_TYPE_HARDWARE, D3D_FEATURE_LEVEL_11_0},
            Direct3D11::{
                D3D11CreateDevice, D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                D3D11_CREATE_DEVICE_VIDEO_SUPPORT, D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC,
            },
        },
        Media::MediaFoundation::{
            MFT_CATEGORY_VIDEO_ENCODER, MFT_MESSAGE_COMMAND_DRAIN,
            MFT_MESSAGE_NOTIFY_END_OF_STREAM, MF_MT_AVG_BITRATE, MF_MT_FRAME_RATE,
        },
    };

    use super::*;

    fn hardware_device() -> Option<ID3D11Device> {
        let mut dev = None;
        // SAFETY: standard device creation; None on machines without a GPU.
        unsafe {
            D3D11CreateDevice(
                None,
                D3D_DRIVER_TYPE_HARDWARE,
                Default::default(),
                D3D11_CREATE_DEVICE_VIDEO_SUPPORT | D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                Some(&[D3D_FEATURE_LEVEL_11_0]),
                D3D11_SDK_VERSION,
                Some(&mut dev),
                None,
                None,
            )
            .ok()?;
        }
        dev
    }

    /// `n` NV12 gradient frames encoded with the in-box H.264 encoder.
    fn encode_h264(w: u32, h: u32, n: usize) -> Vec<Vec<u8>> {
        // SAFETY: test-only MF encoder set-up with owned buffers.
        unsafe {
            startup().unwrap();
            let out = MFT_REGISTER_TYPE_INFO {
                guidMajorType: MFMediaType_Video,
                guidSubtype: MFVideoFormat_H264,
            };
            let mut list: *mut Option<IMFActivate> = std::ptr::null_mut();
            let mut count = 0u32;
            MFTEnumEx(
                MFT_CATEGORY_VIDEO_ENCODER,
                MFT_ENUM_FLAG_SYNCMFT | MFT_ENUM_FLAG_SORTANDFILTER,
                None,
                Some(&out),
                &mut list,
                &mut count,
            )
            .unwrap();
            assert!(count > 0, "no H.264 encoder");
            let enc: IMFTransform = (*list).take().unwrap().ActivateObject().unwrap();
            CoTaskMemFree(Some(list as *const _));
            let size = (u64::from(w) << 32) | u64::from(h);
            let ot = MFCreateMediaType().unwrap();
            ot.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video).unwrap();
            ot.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_H264).unwrap();
            ot.SetUINT64(&MF_MT_FRAME_SIZE, size).unwrap();
            ot.SetUINT64(&MF_MT_FRAME_RATE, (30u64 << 32) | 1).unwrap();
            ot.SetUINT32(&MF_MT_AVG_BITRATE, 1_000_000).unwrap();
            ot.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)
                .unwrap();
            enc.SetOutputType(0, &ot, 0).unwrap();
            let it = MFCreateMediaType().unwrap();
            it.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video).unwrap();
            it.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_NV12).unwrap();
            it.SetUINT64(&MF_MT_FRAME_SIZE, size).unwrap();
            it.SetUINT64(&MF_MT_FRAME_RATE, (30u64 << 32) | 1).unwrap();
            it.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)
                .unwrap();
            enc.SetInputType(0, &it, 0).unwrap();
            enc.ProcessMessage(MFT_MESSAGE_NOTIFY_BEGIN_STREAMING, 0)
                .unwrap();
            let mut units = Vec::new();
            let pull = |units: &mut Vec<Vec<u8>>| loop {
                let info = enc.GetOutputStreamInfo(0).unwrap();
                let os = MFCreateSample().unwrap();
                os.AddBuffer(&MFCreateMemoryBuffer(info.cbSize.max(1 << 20)).unwrap())
                    .unwrap();
                let mut buf = MFT_OUTPUT_DATA_BUFFER {
                    pSample: std::mem::ManuallyDrop::new(Some(os)),
                    ..Default::default()
                };
                let mut status = 0;
                let r = enc.ProcessOutput(0, std::slice::from_mut(&mut buf), &mut status);
                let s = std::mem::ManuallyDrop::take(&mut buf.pSample);
                drop(std::mem::ManuallyDrop::take(&mut buf.pEvents));
                if r.is_err() {
                    break;
                }
                let b = s.unwrap().ConvertToContiguousBuffer().unwrap();
                let mut p = std::ptr::null_mut();
                let mut len = 0u32;
                b.Lock(&mut p, None, Some(&mut len)).unwrap();
                units.push(std::slice::from_raw_parts(p, len as usize).to_vec());
                b.Unlock().unwrap();
            };
            let frame_len = (w * h * 3 / 2) as usize;
            for f in 0..n {
                let pixels: Vec<u8> = (0..frame_len).map(|i| ((i + f * 7) % 251) as u8).collect();
                let s = make_sample(&pixels, f as i64 * 33_333).unwrap();
                s.SetSampleDuration(333_333).unwrap();
                enc.ProcessInput(0, &s, 0).unwrap();
                pull(&mut units);
            }
            // The encoder holds a few frames back: drain them.
            enc.ProcessMessage(MFT_MESSAGE_NOTIFY_END_OF_STREAM, 0)
                .unwrap();
            enc.ProcessMessage(MFT_MESSAGE_COMMAND_DRAIN, 0).unwrap();
            pull(&mut units);
            units
        }
    }

    /// Run by hand: `cargo test -p pb-tray prints_supported_codecs -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn prints_supported_codecs() {
        println!("supported codecs: {:?}", supported_codecs());
    }

    #[test]
    fn decodes_an_h264_stream_from_the_mf_encoder() {
        let Some(device) = hardware_device() else {
            eprintln!("no hardware D3D11 device here (CI): skipped");
            return;
        };
        let units = encode_h264(320, 240, 30);
        assert!(!units.is_empty());
        let mut dec = Decoder::new(&device, Codec::H264).unwrap();
        let cfg = Config {
            rotation: 0,
            width: 320,
            height: 240,
            headers: Vec::new(), // the encoder puts SPS/PPS in the stream
        };
        dec.push(&Record {
            pts_us: 0,
            flags: FLAG_CONFIG,
            payload: cfg.encode(),
        })
        .unwrap();
        let mut frames = Vec::new();
        for (i, u) in units.iter().enumerate() {
            let r = Record {
                pts_us: i as i64 * 33_333,
                flags: if i == 0 { FLAG_KEY } else { 0 },
                payload: u.clone(),
            };
            if let Some(f) = dec.push(&r).unwrap() {
                frames.push(f);
            }
        }
        assert!(
            frames.len() >= 8,
            "decoded {} of {}",
            frames.len(),
            units.len()
        );
        let mut desc = D3D11_TEXTURE2D_DESC::default();
        // SAFETY: reading the description of a valid texture.
        unsafe { frames[0].texture.GetDesc(&mut desc) };
        assert!(
            desc.Width >= 320 && desc.Height >= 240,
            "{}x{}",
            desc.Width,
            desc.Height
        );
    }
}
