//! The "Phone Bridge Camera" device's life: it exists only while streaming.
//!
//! `CameraDevice` always ends with `stop`, `remove`, `shutdown`, each once,
//! on every path (menu stop, phone ended, tray exit, drop). The device is
//! created with Session lifetime, so Windows also removes it if the tray
//! dies. `sweep_leftovers` removes any that a crash still left.
//!
//! The camera is a pure software source: this module never calls
//! `AddDeviceSourceInfo`, so no physical webcam is touched.

/// What the device needs from a virtual camera (a fake in the tests).
pub trait VirtualCamera {
    fn start(&mut self) -> Result<(), String>;
    fn stop(&mut self);
    fn remove(&mut self);
    fn shutdown(&mut self);
}

/// A started virtual camera; ending it (or dropping it) tears it down.
pub struct CameraDevice<C: VirtualCamera> {
    cam: Option<C>,
}

impl<C: VirtualCamera> CameraDevice<C> {
    /// Start `c`. A failed start still removes and shuts the handle down
    /// (no `stop`: it never started), so nothing half-made stays.
    pub fn open(mut c: C) -> Result<Self, String> {
        match c.start() {
            Ok(()) => Ok(Self { cam: Some(c) }),
            Err(e) => {
                c.remove();
                c.shutdown();
                Err(e)
            }
        }
    }

    /// Tear down now (the tray drops the device instead).
    #[cfg(test)]
    pub fn close(mut self) {
        self.teardown();
    }

    fn teardown(&mut self) {
        if let Some(mut c) = self.cam.take() {
            c.stop();
            c.remove();
            c.shutdown();
        }
    }
}

impl<C: VirtualCamera> Drop for CameraDevice<C> {
    fn drop(&mut self) {
        self.teardown();
    }
}

#[cfg(windows)]
pub use real::{sweep_leftovers, MfVirtualCamera};

#[cfg(windows)]
mod real {
    #![allow(unsafe_code)]

    use std::sync::mpsc;

    use pb_camera_source::com::CLSID_PHONE_BRIDGE_CAMERA_SOURCE;
    use windows::{
        core::{HSTRING, PWSTR},
        Win32::{
            Media::{
                KernelStreaming::KSCATEGORY_VIDEO_CAMERA,
                MediaFoundation::{
                    IMFVirtualCamera, MFCreateAttributes, MFCreateVirtualCamera,
                    MFEnumDeviceSources, MFShutdown, MFStartup, MFVirtualCameraAccess_CurrentUser,
                    MFVirtualCameraLifetime_Session, MFVirtualCameraType_SoftwareCameraSource,
                    MFSTARTUP_FULL, MF_DEVSOURCE_ATTRIBUTE_FRIENDLY_NAME,
                    MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE,
                    MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_GUID, MF_VERSION,
                },
            },
            System::Com::{CoInitializeEx, CoTaskMemFree, CoUninitialize, COINIT_MULTITHREADED},
        },
    };

    use super::VirtualCamera;

    /// The name apps list.
    pub const FRIENDLY_NAME: &str = "Phone Bridge Camera";

    fn source_id() -> String {
        format!("{{{:?}}}", CLSID_PHONE_BRIDGE_CAMERA_SOURCE)
    }

    fn create() -> Result<IMFVirtualCamera, String> {
        // SAFETY: Media Foundation creation with valid strings.
        unsafe {
            MFCreateVirtualCamera(
                MFVirtualCameraType_SoftwareCameraSource,
                MFVirtualCameraLifetime_Session,
                MFVirtualCameraAccess_CurrentUser,
                &HSTRING::from(FRIENDLY_NAME),
                &HSTRING::from(source_id()),
                Some(&[KSCATEGORY_VIDEO_CAMERA]),
            )
            .map_err(|e| format!("MFCreateVirtualCamera: {e}"))
        }
    }

    enum Cmd {
        Start,
        Stop,
        Remove,
        Shutdown,
    }

    /// The real virtual camera. It lives on its own multithreaded-COM
    /// thread (never the UI thread); commands are answered in order.
    pub struct MfVirtualCamera {
        tx: mpsc::Sender<(Cmd, mpsc::Sender<Result<(), String>>)>,
        worker: Option<std::thread::JoinHandle<()>>,
    }

    impl MfVirtualCamera {
        pub fn spawn() -> Result<Self, String> {
            let (tx, rx) = mpsc::channel::<(Cmd, mpsc::Sender<Result<(), String>>)>();
            let (ready_tx, ready_rx) = mpsc::channel::<Result<(), String>>();
            let worker = std::thread::spawn(move || {
                // SAFETY: COM and Media Foundation start-up on this thread.
                unsafe {
                    let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
                    if let Err(e) = MFStartup(MF_VERSION, MFSTARTUP_FULL) {
                        let _ = ready_tx.send(Err(format!("MFStartup: {e}")));
                        CoUninitialize();
                        return;
                    }
                }
                let cam = match create() {
                    Ok(c) => c,
                    Err(e) => {
                        let _ = ready_tx.send(Err(e));
                        // SAFETY: undoing the start-up above.
                        unsafe {
                            let _ = MFShutdown();
                            CoUninitialize();
                        }
                        return;
                    }
                };
                let _ = ready_tx.send(Ok(()));
                while let Ok((cmd, reply)) = rx.recv() {
                    // SAFETY: calls on the camera this thread created.
                    let r = unsafe {
                        match cmd {
                            Cmd::Start => cam.Start(None),
                            Cmd::Stop => cam.Stop(),
                            Cmd::Remove => cam.Remove(),
                            Cmd::Shutdown => cam.Shutdown(),
                        }
                    }
                    .map_err(|e| e.to_string());
                    let _ = reply.send(r);
                }
                // SAFETY: undoing the start-up above.
                unsafe {
                    let _ = MFShutdown();
                    CoUninitialize();
                }
            });
            match ready_rx.recv() {
                Ok(Ok(())) => Ok(Self {
                    tx,
                    worker: Some(worker),
                }),
                Ok(Err(e)) => {
                    let _ = worker.join();
                    Err(e)
                }
                Err(_) => Err("the camera thread ended early".into()),
            }
        }

        fn call(&self, cmd: Cmd) -> Result<(), String> {
            let (reply_tx, reply_rx) = mpsc::channel();
            self.tx
                .send((cmd, reply_tx))
                .map_err(|_| "the camera thread is gone".to_string())?;
            reply_rx
                .recv()
                .map_err(|_| "the camera thread is gone".to_string())?
        }
    }

    impl VirtualCamera for MfVirtualCamera {
        fn start(&mut self) -> Result<(), String> {
            self.call(Cmd::Start)
        }
        fn stop(&mut self) {
            let _ = self.call(Cmd::Stop);
        }
        fn remove(&mut self) {
            let _ = self.call(Cmd::Remove);
        }
        fn shutdown(&mut self) {
            let _ = self.call(Cmd::Shutdown);
        }
    }

    impl Drop for MfVirtualCamera {
        fn drop(&mut self) {
            // Closing the channel ends the worker loop.
            let (dead_tx, _) = mpsc::channel();
            self.tx = dead_tx;
            if let Some(w) = self.worker.take() {
                let _ = w.join();
            }
        }
    }

    /// The video devices Media Foundation lists, by friendly name.
    pub fn list_camera_names() -> Vec<String> {
        // SAFETY: MF enumeration; the activate array and strings are freed.
        unsafe {
            let mut attrs = None;
            if MFCreateAttributes(&mut attrs, 1).is_err() {
                return Vec::new();
            }
            let Some(attrs) = attrs else {
                return Vec::new();
            };
            if attrs
                .SetGUID(
                    &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE,
                    &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_GUID,
                )
                .is_err()
            {
                return Vec::new();
            }
            let mut list = std::ptr::null_mut();
            let mut count = 0u32;
            if MFEnumDeviceSources(&attrs, &mut list, &mut count).is_err() || list.is_null() {
                return Vec::new();
            }
            let mut names = Vec::new();
            for i in 0..count as usize {
                if let Some(act) = (*list.add(i)).take() {
                    let mut s = PWSTR::null();
                    let mut len = 0u32;
                    if act
                        .GetAllocatedString(&MF_DEVSOURCE_ATTRIBUTE_FRIENDLY_NAME, &mut s, &mut len)
                        .is_ok()
                    {
                        names.push(s.to_string().unwrap_or_default());
                        CoTaskMemFree(Some(s.0.cast()));
                    }
                }
            }
            CoTaskMemFree(Some(list.cast()));
            names
        }
    }

    /// Remove any "Phone Bridge Camera" a crash left behind (call when this
    /// process owns none). Returns how many were removed.
    pub fn sweep_leftovers(log: &dyn Fn(&str)) -> usize {
        let Ok(mut cam) = MfVirtualCamera::spawn() else {
            return 0;
        };
        let leftover = list_camera_names()
            .iter()
            .filter(|n| n.as_str() == FRIENDLY_NAME)
            .count();
        if leftover == 0 {
            return 0;
        }
        // One handle removes the one registration by name; a second
        // Remove on a shut-down handle would only fail.
        cam.remove();
        cam.shutdown();
        log("CAMERA removed a leftover");
        1
    }
}

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, rc::Rc};

    #[cfg(windows)]
    use super::real::{list_camera_names, FRIENDLY_NAME};
    use super::*;

    struct Fake {
        calls: Rc<RefCell<Vec<&'static str>>>,
        fail_start: bool,
    }

    impl VirtualCamera for Fake {
        fn start(&mut self) -> Result<(), String> {
            self.calls.borrow_mut().push("start");
            if self.fail_start {
                Err("no".into())
            } else {
                Ok(())
            }
        }
        fn stop(&mut self) {
            self.calls.borrow_mut().push("stop");
        }
        fn remove(&mut self) {
            self.calls.borrow_mut().push("remove");
        }
        fn shutdown(&mut self) {
            self.calls.borrow_mut().push("shutdown");
        }
    }

    fn fake(fail_start: bool) -> (Fake, Rc<RefCell<Vec<&'static str>>>) {
        let calls = Rc::new(RefCell::new(Vec::new()));
        (
            Fake {
                calls: calls.clone(),
                fail_start,
            },
            calls,
        )
    }

    #[test]
    fn teardown_order_is_stop_remove_shutdown() {
        let (f, calls) = fake(false);
        let d = CameraDevice::open(f).unwrap();
        d.close();
        assert_eq!(*calls.borrow(), ["start", "stop", "remove", "shutdown"]);
    }

    #[test]
    fn drop_tears_down_once() {
        let (f, calls) = fake(false);
        let d = CameraDevice::open(f).unwrap();
        drop(d);
        assert_eq!(*calls.borrow(), ["start", "stop", "remove", "shutdown"]);
    }

    #[test]
    fn failed_start_still_removes_and_shuts_down() {
        let (f, calls) = fake(true);
        assert!(CameraDevice::open(f).is_err());
        assert_eq!(*calls.borrow(), ["start", "remove", "shutdown"]);
    }

    /// Needs the camera source registered on this PC. Activates the installed
    /// class through COM the way Frame Server does and asks for the
    /// interfaces it uses.
    #[cfg(windows)]
    #[test]
    #[ignore]
    #[allow(unsafe_code)]
    fn the_installed_source_activates_through_com() {
        use pb_camera_source::com::CLSID_PHONE_BRIDGE_CAMERA_SOURCE;
        use windows::{
            core::Interface,
            Win32::{
                Media::{
                    KernelStreaming::IKsControl,
                    MediaFoundation::{
                        IMFGetService, IMFMediaEventGenerator, IMFMediaSource, IMFMediaSourceEx,
                        MFStartup, MFSTARTUP_FULL, MF_VERSION,
                    },
                },
                System::Com::{
                    CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED,
                },
            },
        };
        // SAFETY: COM activation of our own installed class in this process.
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            MFStartup(MF_VERSION, MFSTARTUP_FULL).unwrap();
            let src: IMFMediaSourceEx = CoCreateInstance(
                &CLSID_PHONE_BRIDGE_CAMERA_SOURCE,
                None,
                CLSCTX_INPROC_SERVER,
            )
            .expect("CoCreateInstance(IMFMediaSourceEx)");
            src.cast::<IMFMediaSource>().expect("IMFMediaSource");
            src.cast::<IMFMediaEventGenerator>()
                .expect("IMFMediaEventGenerator");
            src.cast::<IMFGetService>().expect("IMFGetService");
            src.cast::<IKsControl>().expect("IKsControl");
            let pd = src
                .CreatePresentationDescriptor()
                .expect("presentation descriptor");
            assert_eq!(pd.GetStreamDescriptorCount().unwrap(), 1);
            src.Shutdown().unwrap();
        }
    }

    /// Needs the camera source registered (admin) on this PC: run by hand
    /// after `--register-camera`, with `cargo test -p pb-tray camera_device -- --ignored`.
    #[cfg(windows)]
    #[test]
    #[ignore]
    fn a_started_camera_is_listed_and_gone_after_the_stop_sequence() {
        let before = list_camera_names()
            .iter()
            .filter(|n| *n == FRIENDLY_NAME)
            .count();
        let cam = MfVirtualCamera::spawn().expect("create");
        let d = CameraDevice::open(cam).expect("start");
        std::thread::sleep(std::time::Duration::from_millis(500));
        let during = list_camera_names()
            .iter()
            .filter(|n| *n == FRIENDLY_NAME)
            .count();
        assert_eq!(during, before + 1, "listed while streaming");
        d.close();
        std::thread::sleep(std::time::Duration::from_millis(500));
        let after = list_camera_names()
            .iter()
            .filter(|n| *n == FRIENDLY_NAME)
            .count();
        assert_eq!(after, before, "nothing left after the stop sequence");
    }
}
