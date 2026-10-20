//! The COM side: `IMFMediaSourceEx` with one NV12 stream, driven by a 30 fps
//! timer thread that answers queued `RequestSample` tokens through `Pump`.
//! A pure software source: it never calls `AddDeviceSourceInfo` and never
//! opens another device.
#![allow(unsafe_code)]
// The COM trait signatures are fixed by the platform (raw pointers, `u32`
// event types), and the state is shared with the timer thread through
// `SendInner` (free-threaded Media Foundation objects behind mutexes).
#![allow(
    clippy::arc_with_non_send_sync,
    clippy::unnecessary_cast,
    clippy::not_unsafe_ptr_arg_deref
)]

use std::{
    ffi::c_void,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread::JoinHandle,
    time::Duration,
};

use pb_camera_frames::Ring;
use windows::{
    core::{
        implement, AgileReference, Error, IUnknown, Interface, Ref, Result, BOOL, GUID, HRESULT,
    },
    Win32::{
        Foundation::{E_NOINTERFACE, E_POINTER, S_FALSE, S_OK},
        Media::{
            KernelStreaming::{IKsControl, IKsControl_Impl, KSIDENTIFIER},
            MediaFoundation::*,
        },
        System::Com::{IClassFactory, IClassFactory_Impl, StructuredStorage::PROPVARIANT},
    },
};

use crate::{
    file::Mapped,
    pump::{Pump, TypeState, TYPES},
};

/// `{B8A7C1E2-5D3F-4A91-8E6B-2C4D9F0A7B13}`: the source's class id.
pub const CLSID_PHONE_BRIDGE_CAMERA_SOURCE: GUID =
    GUID::from_u128(0xb8a7c1e2_5d3f_4a91_8e6b_2c4d9f0a7b13);

/// `PINNAME_VIDCAP` ({fb6c4281-0353-11d1-905f-0000c0cc16ba}).
const PINNAME_VIDCAP: GUID = GUID::from_u128(0xfb6c4281_0353_11d1_905f_0000c0cc16ba);

const TICK: Duration = Duration::from_millis(33);

/// Where the tray's frames file lives.
pub fn frames_path() -> PathBuf {
    let base = std::env::var_os("ProgramData").unwrap_or_else(|| "C:\\ProgramData".into());
    PathBuf::from(base)
        .join("PhoneBridge")
        .join("camera")
        .join("frames")
}

/// Wraps a COM token so it can wait in the pump's queue across threads
/// (Media Foundation objects are free-threaded).
struct Token(Option<IUnknown>);
// SAFETY: the tokens are MF objects handed to RequestSample, which the
// platform allows to be answered from any thread.
unsafe impl Send for Token {}

/// The timer thread owns a clone of the source's state. Its COM objects are
/// free-threaded Media Foundation objects, safe to use from any thread.
struct SendInner(Arc<Inner>);
// SAFETY: see above; the state behind it is behind mutexes and atomics.
unsafe impl Send for SendInner {}

impl SendInner {
    // A method, so the closure captures the whole wrapper (not its field).
    fn into_inner(self) -> Arc<Inner> {
        self.0
    }
}

struct Shared {
    pump: Pump<Token>,
    mapped: Option<Mapped>,
    types: TypeState,
    started: bool,
    shutdown: bool,
}

struct Inner {
    shared: Mutex<Shared>,
    events: IMFMediaEventQueue,
    stream_events: IMFMediaEventQueue,
    stop: AtomicBool,
    timer: Mutex<Option<JoinHandle<()>>>,
    stream_desc: IMFStreamDescriptor,
    pres_desc: IMFPresentationDescriptor,
    /// The stream (set once created); cleared at shutdown to break the cycle.
    stream: Mutex<Option<IMFMediaStream2>>,
    source: Mutex<Option<AgileReference<IMFMediaSource>>>,
}

fn media_type(w: u32, h: u32) -> Result<IMFMediaType> {
    // SAFETY: plain Media Foundation calls on a type we own.
    unsafe {
        let t = MFCreateMediaType()?;
        t.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
        t.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_NV12)?;
        t.SetUINT64(&MF_MT_FRAME_SIZE, (u64::from(w) << 32) | u64::from(h))?;
        t.SetUINT64(&MF_MT_FRAME_RATE, (30u64 << 32) | 1)?;
        t.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, (1u64 << 32) | 1)?;
        t.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)?;
        t.SetUINT32(&MF_MT_ALL_SAMPLES_INDEPENDENT, 1)?;
        t.SetUINT32(&MF_MT_DEFAULT_STRIDE, w)?;
        Ok(t)
    }
}

fn size_of_type(t: &IMFMediaType) -> Result<(u32, u32)> {
    // SAFETY: reading an attribute of a live media type.
    let v = unsafe { t.GetUINT64(&MF_MT_FRAME_SIZE)? };
    Ok(((v >> 32) as u32, v as u32))
}

fn now_100ns() -> i64 {
    // SAFETY: a plain clock read.
    unsafe { MFGetSystemTime() }
}

fn now_ms() -> u64 {
    (now_100ns() / 10_000) as u64
}

impl Inner {
    fn new_with(frames: &std::path::Path) -> Result<Arc<Self>> {
        // SAFETY: Media Foundation object creation; the descriptors are ours.
        unsafe {
            let mut sizes = Vec::new();
            for (w, h) in TYPES {
                sizes.push(Some(media_type(w, h)?));
            }
            let sd = MFCreateStreamDescriptor(0, &sizes)?;
            sd.GetMediaTypeHandler()?
                .SetCurrentMediaType(sizes[0].as_ref().expect("a type"))?;
            sd.SetGUID(&MF_DEVICESTREAM_STREAM_CATEGORY, &PINNAME_VIDCAP)?;
            sd.SetUINT32(&MF_DEVICESTREAM_STREAM_ID, 0)?;
            sd.SetUINT32(&MF_DEVICESTREAM_FRAMESERVER_SHARED, 1)?;
            let pd = MFCreatePresentationDescriptor(Some(&[Some(sd.clone())]))?;
            pd.SelectStream(0)?;
            let (w, h) = TYPES[0];
            // Read-write: the source tells the tray which size the app chose.
            // Without the right (or the file) every frame is black.
            let mapped = Mapped::open(frames, true)
                .or_else(|_| Mapped::open(frames, false))
                .ok();
            let mut types = TypeState::default();
            let _ = types.set_current(w, h);
            Ok(Arc::new(Self {
                shared: Mutex::new(Shared {
                    pump: Pump::new((w, h)),
                    mapped,
                    types,
                    started: false,
                    shutdown: false,
                }),
                events: MFCreateEventQueue()?,
                stream_events: MFCreateEventQueue()?,
                stop: AtomicBool::new(false),
                timer: Mutex::new(None),
                stream_desc: sd,
                pres_desc: pd,
                stream: Mutex::new(None),
                source: Mutex::new(None),
            }))
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Shared> {
        self.shared.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// One 33 ms tick: answer at most one queued request.
    fn tick(&self) {
        let sample = {
            let mut g = self.lock();
            if g.shutdown {
                return;
            }
            let Shared { pump, mapped, .. } = &mut *g;
            match mapped.as_mut().and_then(|m| Ring::open(m.words())) {
                Some(ring) => {
                    let alive = ring.tray_alive(now_ms());
                    pump.tick(&ring, alive, now_100ns())
                }
                // No frames file or no ring yet: black, never a stall.
                None => pump.black_sample(now_100ns()),
            }
        };
        let Some(s) = sample else {
            return;
        };
        let _ = self.deliver(s.token.0, &s.data, s.pts_100ns, s.duration_100ns);
    }

    fn deliver(&self, token: Option<IUnknown>, data: &[u8], pts: i64, duration: i64) -> Result<()> {
        // SAFETY: a memory buffer we fill, wrapped in a sample.
        unsafe {
            let buf = MFCreateMemoryBuffer(data.len() as u32)?;
            let mut ptr = std::ptr::null_mut();
            buf.Lock(&mut ptr, None, None)?;
            std::ptr::copy_nonoverlapping(data.as_ptr(), ptr, data.len());
            buf.Unlock()?;
            buf.SetCurrentLength(data.len() as u32)?;
            let sample = MFCreateSample()?;
            sample.AddBuffer(&buf)?;
            sample.SetSampleTime(pts)?;
            sample.SetSampleDuration(duration)?;
            if let Some(t) = token {
                sample.SetUnknown(&MFSampleExtension_Token, &t)?;
            }
            self.stream_events.QueueEventParamUnk(
                MEMediaSample.0 as u32,
                &GUID::zeroed(),
                S_OK,
                &sample,
            )
        }
    }

    fn start_timer(self: &Arc<Self>) {
        let mut t = self.timer.lock().unwrap_or_else(|p| p.into_inner());
        if t.is_some() {
            return;
        }
        self.stop.store(false, Ordering::Release);
        let me = SendInner(Arc::clone(self));
        *t = Some(std::thread::spawn(move || {
            let me = me.into_inner();
            // SAFETY: a worker thread that calls Media Foundation.
            let _ = unsafe {
                windows::Win32::System::Com::CoInitializeEx(
                    None,
                    windows::Win32::System::Com::COINIT_MULTITHREADED,
                )
            };
            while !me.stop.load(Ordering::Acquire) {
                std::thread::sleep(TICK);
                me.tick();
            }
        }));
    }

    fn stop_timer(&self) {
        self.stop.store(true, Ordering::Release);
        if let Some(h) = self.timer.lock().unwrap_or_else(|p| p.into_inner()).take() {
            let _ = h.join();
        }
    }
}

#[implement(IMFMediaSourceEx, IMFGetService, IKsControl)]
pub struct PbCameraSource {
    inner: Arc<Inner>,
}

#[implement(IMFMediaStream2)]
pub struct PbCameraStream {
    inner: Arc<Inner>,
}

impl PbCameraSource {
    pub fn create() -> Result<IMFMediaSourceEx> {
        Self::create_with(&frames_path())
    }

    /// With the frames file at `frames` (tests use a temp file).
    pub fn create_with(frames: &std::path::Path) -> Result<IMFMediaSourceEx> {
        let inner = Inner::new_with(frames)?;
        let stream: IMFMediaStream2 = PbCameraStream {
            inner: inner.clone(),
        }
        .into();
        *inner.stream.lock().unwrap_or_else(|p| p.into_inner()) = Some(stream);
        let source: IMFMediaSourceEx = PbCameraSource {
            inner: inner.clone(),
        }
        .into();
        let as_source: IMFMediaSource = source.cast()?;
        *inner.source.lock().unwrap_or_else(|p| p.into_inner()) =
            Some(AgileReference::new(&as_source)?);
        Ok(source)
    }
}

impl IMFMediaEventGenerator_Impl for PbCameraSource_Impl {
    fn GetEvent(&self, flags: MEDIA_EVENT_GENERATOR_GET_EVENT_FLAGS) -> Result<IMFMediaEvent> {
        // SAFETY: the queue lives as long as the source.
        unsafe { self.inner.events.GetEvent(flags.0 as u32) }
    }
    fn BeginGetEvent(&self, callback: Ref<IMFAsyncCallback>, state: Ref<IUnknown>) -> Result<()> {
        // SAFETY: as above.
        unsafe {
            self.inner
                .events
                .BeginGetEvent(callback.as_ref(), state.as_ref())
        }
    }
    fn EndGetEvent(&self, result: Ref<IMFAsyncResult>) -> Result<IMFMediaEvent> {
        // SAFETY: as above.
        unsafe { self.inner.events.EndGetEvent(result.as_ref()) }
    }
    fn QueueEvent(
        &self,
        met: u32,
        ext: *const GUID,
        status: HRESULT,
        value: *const PROPVARIANT,
    ) -> Result<()> {
        // SAFETY: as above.
        unsafe {
            self.inner
                .events
                .QueueEventParamVar(met, ext, status, value)
        }
    }
}

impl IMFMediaSource_Impl for PbCameraSource_Impl {
    fn GetCharacteristics(&self) -> Result<u32> {
        Ok(MFMEDIASOURCE_IS_LIVE.0 as u32)
    }

    fn CreatePresentationDescriptor(&self) -> Result<IMFPresentationDescriptor> {
        if self.inner.lock().shutdown {
            return Err(MF_E_SHUTDOWN.into());
        }
        // SAFETY: cloning our own descriptor.
        unsafe { self.inner.pres_desc.Clone() }
    }

    fn Start(
        &self,
        pd: Ref<IMFPresentationDescriptor>,
        _time_format: *const GUID,
        _start: *const PROPVARIANT,
    ) -> Result<()> {
        let pd = pd.ok()?;
        // The type the app picked: it stays for the whole call.
        // SAFETY: reading the app's presentation descriptor.
        let (w, h) = unsafe {
            let sd = {
                let mut selected = BOOL(0);
                let mut sd = None;
                pd.GetStreamDescriptorByIndex(0, &mut selected, &mut sd)?;
                sd.ok_or_else(|| Error::from(E_POINTER))?
            };
            size_of_type(&sd.GetMediaTypeHandler()?.GetCurrentMediaType()?)?
        };
        {
            let mut g = self.inner.lock();
            if g.shutdown {
                return Err(MF_E_SHUTDOWN.into());
            }
            g.types
                .set_current(w, h)
                .map_err(|_| Error::from(MF_E_INVALIDREQUEST))?;
            g.types.start();
            g.pump = Pump::new((w, h));
            g.started = true;
            let Shared { mapped, .. } = &mut *g;
            if let Some(m) = mapped {
                if let Some(mut ring) = Ring::open(m.words()) {
                    ring.set_wanted(w, h);
                }
            }
        }
        let stream = self
            .inner
            .stream
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
            .ok_or_else(|| Error::from(MF_E_SHUTDOWN))?;
        // SAFETY: queueing events on our own queues.
        unsafe {
            self.inner.events.QueueEventParamUnk(
                MENewStream.0 as u32,
                &GUID::zeroed(),
                S_OK,
                &stream,
            )?;
            self.inner.stream_events.QueueEventParamVar(
                MEStreamStarted.0 as u32,
                &GUID::zeroed(),
                S_OK,
                &PROPVARIANT::default(),
            )?;
            self.inner.events.QueueEventParamVar(
                MESourceStarted.0 as u32,
                &GUID::zeroed(),
                S_OK,
                &PROPVARIANT::default(),
            )?;
        }
        // The Arc<Inner> behind a COM object: the timer needs its own clone.
        self.inner.start_timer();
        Ok(())
    }

    fn Stop(&self) -> Result<()> {
        self.inner.stop_timer();
        self.inner.lock().started = false;
        // SAFETY: queueing events on our own queues.
        unsafe {
            self.inner.stream_events.QueueEventParamVar(
                MEStreamStopped.0 as u32,
                &GUID::zeroed(),
                S_OK,
                &PROPVARIANT::default(),
            )?;
            self.inner.events.QueueEventParamVar(
                MESourceStopped.0 as u32,
                &GUID::zeroed(),
                S_OK,
                &PROPVARIANT::default(),
            )
        }
    }

    fn Pause(&self) -> Result<()> {
        // A live camera cannot pause.
        Err(MF_E_INVALID_STATE_TRANSITION.into())
    }

    fn Shutdown(&self) -> Result<()> {
        self.inner.stop_timer();
        {
            let mut g = self.inner.lock();
            g.shutdown = true;
            // Nothing of the picture stays in this process.
            g.mapped = None;
        }
        // SAFETY: shutting our own queues down.
        unsafe {
            let _ = self.inner.events.Shutdown();
            let _ = self.inner.stream_events.Shutdown();
        }
        // Break the source <-> stream cycle.
        *self.inner.stream.lock().unwrap_or_else(|p| p.into_inner()) = None;
        *self.inner.source.lock().unwrap_or_else(|p| p.into_inner()) = None;
        Ok(())
    }
}

impl IMFMediaSourceEx_Impl for PbCameraSource_Impl {
    fn GetSourceAttributes(&self) -> Result<IMFAttributes> {
        let mut a = None;
        // SAFETY: creating an empty attribute store.
        unsafe { MFCreateAttributes(&mut a, 0)? };
        a.ok_or_else(|| Error::from(E_POINTER))
    }

    fn GetStreamAttributes(&self, id: u32) -> Result<IMFAttributes> {
        if id != 0 {
            return Err(MF_E_INVALIDREQUEST.into());
        }
        self.inner.stream_desc.cast()
    }

    fn SetD3DManager(&self, _manager: Ref<IUnknown>) -> Result<()> {
        // Software frames only.
        Ok(())
    }
}

impl IMFGetService_Impl for PbCameraSource_Impl {
    fn GetService(
        &self,
        _service: *const GUID,
        _riid: *const GUID,
        _object: *mut *mut c_void,
    ) -> Result<()> {
        Err(Error::from(E_NOINTERFACE))
    }
}

impl IKsControl_Impl for PbCameraSource_Impl {
    // No device to control: every property, method and event is unsupported
    // (HRESULT_FROM_WIN32(ERROR_SET_NOT_FOUND)).
    fn KsProperty(
        &self,
        _p: *const KSIDENTIFIER,
        _l: u32,
        _d: *mut c_void,
        _dl: u32,
        _r: *mut u32,
    ) -> Result<()> {
        Err(Error::from(HRESULT(0x8007_0492u32 as i32)))
    }
    fn KsMethod(
        &self,
        _p: *const KSIDENTIFIER,
        _l: u32,
        _d: *mut c_void,
        _dl: u32,
        _r: *mut u32,
    ) -> Result<()> {
        Err(Error::from(HRESULT(0x8007_0492u32 as i32)))
    }
    fn KsEvent(
        &self,
        _p: *const KSIDENTIFIER,
        _l: u32,
        _d: *mut c_void,
        _dl: u32,
        _r: *mut u32,
    ) -> Result<()> {
        Err(Error::from(HRESULT(0x8007_0492u32 as i32)))
    }
}

impl IMFMediaEventGenerator_Impl for PbCameraStream_Impl {
    fn GetEvent(&self, flags: MEDIA_EVENT_GENERATOR_GET_EVENT_FLAGS) -> Result<IMFMediaEvent> {
        // SAFETY: the queue lives as long as the stream.
        unsafe { self.inner.stream_events.GetEvent(flags.0 as u32) }
    }
    fn BeginGetEvent(&self, callback: Ref<IMFAsyncCallback>, state: Ref<IUnknown>) -> Result<()> {
        // SAFETY: as above.
        unsafe {
            self.inner
                .stream_events
                .BeginGetEvent(callback.as_ref(), state.as_ref())
        }
    }
    fn EndGetEvent(&self, result: Ref<IMFAsyncResult>) -> Result<IMFMediaEvent> {
        // SAFETY: as above.
        unsafe { self.inner.stream_events.EndGetEvent(result.as_ref()) }
    }
    fn QueueEvent(
        &self,
        met: u32,
        ext: *const GUID,
        status: HRESULT,
        value: *const PROPVARIANT,
    ) -> Result<()> {
        // SAFETY: as above.
        unsafe {
            self.inner
                .stream_events
                .QueueEventParamVar(met, ext, status, value)
        }
    }
}

impl IMFMediaStream_Impl for PbCameraStream_Impl {
    fn GetMediaSource(&self) -> Result<IMFMediaSource> {
        self.inner
            .source
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .as_ref()
            .ok_or_else(|| Error::from(MF_E_SHUTDOWN))?
            .resolve()
    }

    fn GetStreamDescriptor(&self) -> Result<IMFStreamDescriptor> {
        Ok(self.inner.stream_desc.clone())
    }

    fn RequestSample(&self, token: Ref<IUnknown>) -> Result<()> {
        let mut g = self.inner.lock();
        if g.shutdown {
            return Err(MF_E_SHUTDOWN.into());
        }
        if !g.started {
            return Err(MF_E_INVALIDREQUEST.into());
        }
        // Bursts queue up; each 33 ms tick answers one.
        g.pump.request(Token(token.as_ref().cloned()));
        Ok(())
    }
}

impl IMFMediaStream2_Impl for PbCameraStream_Impl {
    fn SetStreamState(&self, _value: MF_STREAM_STATE) -> Result<()> {
        Ok(())
    }
    fn GetStreamState(&self) -> Result<MF_STREAM_STATE> {
        Ok(MF_STREAM_STATE_RUNNING)
    }
}

#[implement(IClassFactory)]
struct Factory;

impl IClassFactory_Impl for Factory_Impl {
    fn CreateInstance(
        &self,
        outer: Ref<IUnknown>,
        riid: *const GUID,
        object: *mut *mut c_void,
    ) -> Result<()> {
        if outer.is_some() {
            return Err(Error::from(
                windows::Win32::Foundation::CLASS_E_NOAGGREGATION,
            ));
        }
        if object.is_null() {
            return Err(E_POINTER.into());
        }
        let source = PbCameraSource::create()?;
        // SAFETY: `object` and `riid` are the caller's valid out-pointers.
        unsafe { source.query(riid, object).ok() }
    }

    fn LockServer(&self, _lock: BOOL) -> Result<()> {
        Ok(())
    }
}

/// COM entry point: Frame Server asks for our class factory.
///
/// # Safety
/// Called by COM with valid pointers.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn DllGetClassObject(
    rclsid: *const GUID,
    riid: *const GUID,
    ppv: *mut *mut c_void,
) -> HRESULT {
    if rclsid.is_null() || riid.is_null() || ppv.is_null() {
        return E_POINTER;
    }
    // SAFETY: the pointers were checked above.
    unsafe {
        if *rclsid != CLSID_PHONE_BRIDGE_CAMERA_SOURCE {
            return windows::Win32::Foundation::CLASS_E_CLASSNOTAVAILABLE;
        }
        let factory: IClassFactory = Factory.into();
        factory.query(riid, ppv)
    }
}

/// The DLL can unload only when nothing of it is alive; Frame Server owns
/// the lifetime, so it never claims to be unloadable while objects exist.
#[unsafe(no_mangle)]
pub extern "system" fn DllCanUnloadNow() -> HRESULT {
    S_FALSE
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use pb_camera_frames::FILE_BYTES;

    use super::*;

    fn temp_frames(name: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("pbcf-{name}-{}.frames", std::process::id()));
        std::fs::write(&p, vec![0u8; FILE_BYTES]).unwrap();
        let mut m = Mapped::open(&p, true).unwrap();
        let mut ring = Ring::init(m.words());
        ring.beat(now_ms());
        p
    }

    /// Poll a stream for `MEMediaSample` events without ever blocking.
    fn count_samples(stream: &IMFMediaStream2, want: usize, within: Duration) -> Vec<Instant> {
        let mut got = Vec::new();
        let t0 = Instant::now();
        while got.len() < want && t0.elapsed() < within {
            // SAFETY: a non-blocking read of our own event queue.
            match unsafe { stream.GetEvent(MF_EVENT_FLAG_NO_WAIT) } {
                Ok(ev) => {
                    if unsafe { ev.GetType() }.unwrap() == MEMediaSample.0 as u32 {
                        got.push(Instant::now());
                    }
                }
                Err(_) => std::thread::sleep(Duration::from_millis(2)),
            }
        }
        got
    }

    #[test]
    fn burst_of_requests_is_answered_one_per_tick_through_com() {
        // SAFETY: Media Foundation start-up for this test process.
        unsafe { MFStartup(MF_VERSION, MFSTARTUP_FULL).unwrap() };
        let path = temp_frames("burst");
        let inner = Inner::new_with(&path).unwrap();
        let stream: IMFMediaStream2 = PbCameraStream {
            inner: inner.clone(),
        }
        .into();
        inner.lock().started = true;
        inner.start_timer();
        for _ in 0..5 {
            // SAFETY: requesting samples with no token.
            unsafe { stream.RequestSample(None).unwrap() };
        }
        let got = count_samples(&stream, 5, Duration::from_secs(2));
        inner.stop_timer();
        assert_eq!(got.len(), 5, "all five requests answered");
        let span = got[4].duration_since(got[0]);
        assert!(
            span >= Duration::from_millis(100),
            "one per 33 ms tick, not all at once: {span:?}"
        );
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn requests_are_refused_before_start_and_after_shutdown() {
        // SAFETY: as above.
        unsafe { MFStartup(MF_VERSION, MFSTARTUP_FULL).unwrap() };
        let path = temp_frames("state");
        let inner = Inner::new_with(&path).unwrap();
        let stream: IMFMediaStream2 = PbCameraStream {
            inner: inner.clone(),
        }
        .into();
        // SAFETY: requests on our own stream.
        assert!(
            unsafe { stream.RequestSample(None) }.is_err(),
            "not started"
        );
        inner.lock().started = true;
        assert!(unsafe { stream.RequestSample(None) }.is_ok());
        inner.lock().shutdown = true;
        assert!(unsafe { stream.RequestSample(None) }.is_err(), "shut down");
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn the_source_creates_and_advertises_one_nv12_stream() {
        // SAFETY: as above.
        unsafe { MFStartup(MF_VERSION, MFSTARTUP_FULL).unwrap() };
        let path = temp_frames("create");
        let src = PbCameraSource::create_with(&path).unwrap();
        // SAFETY: reading the source's own descriptors.
        unsafe {
            let pd = src.CreatePresentationDescriptor().unwrap();
            assert_eq!(pd.GetStreamDescriptorCount().unwrap(), 1);
            let mut selected = BOOL(0);
            let mut sd = None;
            pd.GetStreamDescriptorByIndex(0, &mut selected, &mut sd)
                .unwrap();
            let h = sd.unwrap().GetMediaTypeHandler().unwrap();
            assert_eq!(h.GetMediaTypeCount().unwrap(), 2, "1080p and 720p");
            let t = h.GetMediaTypeByIndex(1).unwrap();
            assert_eq!(size_of_type(&t).unwrap(), (1280, 720));
            assert_eq!(t.GetGUID(&MF_MT_SUBTYPE).unwrap(), MFVideoFormat_NV12);
            assert_eq!(
                src.GetCharacteristics().unwrap(),
                MFMEDIASOURCE_IS_LIVE.0 as u32
            );
            src.Shutdown().unwrap();
        }
        let _ = std::fs::remove_file(path);
    }
}
