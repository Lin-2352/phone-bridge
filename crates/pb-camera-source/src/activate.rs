//! The class Windows' Frame Server activates. `MFCreateVirtualCamera` takes
//! the id of an **activator** (`IMFActivate`), not of the media source: the
//! activator's `ActivateObject` creates the source. `IMFActivate` is an
//! `IMFAttributes`, so this forwards the attribute store to a real one.
#![allow(unsafe_code)]
#![allow(clippy::not_unsafe_ptr_arg_deref)]

use std::ffi::c_void;

use windows::{
    core::{implement, Error, IUnknown, Interface, Ref, Result, BOOL, GUID, PCWSTR, PWSTR},
    Win32::{
        Foundation::E_POINTER, Media::MediaFoundation::*,
        System::Com::StructuredStorage::PROPVARIANT,
    },
};

use crate::com::PbCameraSource;

#[implement(IMFActivate)]
pub struct PbCameraActivate {
    attrs: IMFAttributes,
}

impl PbCameraActivate {
    pub fn create() -> Result<IMFActivate> {
        let mut a = None;
        // SAFETY: an empty attribute store.
        unsafe { MFCreateAttributes(&mut a, 0)? };
        let attrs = a.ok_or_else(|| Error::from(E_POINTER))?;
        Ok(PbCameraActivate { attrs }.into())
    }
}

impl IMFActivate_Impl for PbCameraActivate_Impl {
    fn ActivateObject(&self, riid: *const GUID, ppv: *mut *mut c_void) -> Result<()> {
        if riid.is_null() || ppv.is_null() {
            return Err(E_POINTER.into());
        }
        crate::com::log("IMFActivate::ActivateObject");
        let source = PbCameraSource::create()?;
        // SAFETY: COM's valid out-pointers.
        unsafe { source.query(riid, ppv).ok() }
    }

    fn ShutdownObject(&self) -> Result<()> {
        crate::com::log("IMFActivate::ShutdownObject");
        Ok(())
    }

    fn DetachObject(&self) -> Result<()> {
        Ok(())
    }
}

// The attribute store: every call goes to the real one.
impl IMFAttributes_Impl for PbCameraActivate_Impl {
    fn GetItem(&self, k: *const GUID, v: *mut PROPVARIANT) -> Result<()> {
        unsafe {
            self.attrs
                .GetItem(k, if v.is_null() { None } else { Some(v) })
        }
    }
    fn GetItemType(&self, k: *const GUID) -> Result<MF_ATTRIBUTE_TYPE> {
        unsafe { self.attrs.GetItemType(k) }
    }
    fn CompareItem(&self, k: *const GUID, v: *const PROPVARIANT) -> Result<BOOL> {
        unsafe { self.attrs.CompareItem(k, v) }
    }
    fn Compare(&self, theirs: Ref<IMFAttributes>, t: MF_ATTRIBUTES_MATCH_TYPE) -> Result<BOOL> {
        unsafe { self.attrs.Compare(theirs.as_ref(), t) }
    }
    fn GetUINT32(&self, k: *const GUID) -> Result<u32> {
        unsafe { self.attrs.GetUINT32(k) }
    }
    fn GetUINT64(&self, k: *const GUID) -> Result<u64> {
        unsafe { self.attrs.GetUINT64(k) }
    }
    fn GetDouble(&self, k: *const GUID) -> Result<f64> {
        unsafe { self.attrs.GetDouble(k) }
    }
    fn GetGUID(&self, k: *const GUID) -> Result<GUID> {
        unsafe { self.attrs.GetGUID(k) }
    }
    fn GetStringLength(&self, k: *const GUID) -> Result<u32> {
        unsafe { self.attrs.GetStringLength(k) }
    }
    fn GetString(&self, k: *const GUID, buf: PWSTR, size: u32, len: *mut u32) -> Result<()> {
        // SAFETY: the caller's buffer of `size` UTF-16 units.
        unsafe {
            let slice = std::slice::from_raw_parts_mut(buf.0, size as usize);
            self.attrs
                .GetString(k, slice, if len.is_null() { None } else { Some(len) })
        }
    }
    fn GetAllocatedString(&self, k: *const GUID, s: *mut PWSTR, len: *mut u32) -> Result<()> {
        unsafe { self.attrs.GetAllocatedString(k, s, len) }
    }
    fn GetBlobSize(&self, k: *const GUID) -> Result<u32> {
        unsafe { self.attrs.GetBlobSize(k) }
    }
    fn GetBlob(&self, k: *const GUID, buf: *mut u8, size: u32, len: *mut u32) -> Result<()> {
        // SAFETY: the caller's buffer of `size` bytes.
        unsafe {
            let slice = std::slice::from_raw_parts_mut(buf, size as usize);
            self.attrs
                .GetBlob(k, slice, if len.is_null() { None } else { Some(len) })
        }
    }
    fn GetAllocatedBlob(&self, k: *const GUID, buf: *mut *mut u8, size: *mut u32) -> Result<()> {
        unsafe { self.attrs.GetAllocatedBlob(k, buf, size) }
    }
    fn GetUnknown(&self, k: *const GUID, riid: *const GUID, ppv: *mut *mut c_void) -> Result<()> {
        // SAFETY: the raw call keeps the caller's riid and out-pointer.
        unsafe {
            let vt = Interface::vtable(&self.attrs);
            (vt.GetUnknown)(Interface::as_raw(&self.attrs), k, riid, ppv).ok()
        }
    }
    fn SetItem(&self, k: *const GUID, v: *const PROPVARIANT) -> Result<()> {
        unsafe { self.attrs.SetItem(k, v) }
    }
    fn DeleteItem(&self, k: *const GUID) -> Result<()> {
        unsafe { self.attrs.DeleteItem(k) }
    }
    fn DeleteAllItems(&self) -> Result<()> {
        unsafe { self.attrs.DeleteAllItems() }
    }
    fn SetUINT32(&self, k: *const GUID, v: u32) -> Result<()> {
        unsafe { self.attrs.SetUINT32(k, v) }
    }
    fn SetUINT64(&self, k: *const GUID, v: u64) -> Result<()> {
        unsafe { self.attrs.SetUINT64(k, v) }
    }
    fn SetDouble(&self, k: *const GUID, v: f64) -> Result<()> {
        unsafe { self.attrs.SetDouble(k, v) }
    }
    fn SetGUID(&self, k: *const GUID, v: *const GUID) -> Result<()> {
        unsafe { self.attrs.SetGUID(k, v) }
    }
    fn SetString(&self, k: *const GUID, v: &PCWSTR) -> Result<()> {
        unsafe { self.attrs.SetString(k, *v) }
    }
    fn SetBlob(&self, k: *const GUID, buf: *const u8, size: u32) -> Result<()> {
        // SAFETY: the caller's buffer of `size` bytes.
        unsafe {
            self.attrs
                .SetBlob(k, std::slice::from_raw_parts(buf, size as usize))
        }
    }
    fn SetUnknown(&self, k: *const GUID, u: Ref<IUnknown>) -> Result<()> {
        unsafe { self.attrs.SetUnknown(k, u.as_ref()) }
    }
    fn LockStore(&self) -> Result<()> {
        unsafe { self.attrs.LockStore() }
    }
    fn UnlockStore(&self) -> Result<()> {
        unsafe { self.attrs.UnlockStore() }
    }
    fn GetCount(&self) -> Result<u32> {
        unsafe { self.attrs.GetCount() }
    }
    fn GetItemByIndex(&self, i: u32, k: *mut GUID, v: *mut PROPVARIANT) -> Result<()> {
        unsafe {
            self.attrs
                .GetItemByIndex(i, k, if v.is_null() { None } else { Some(v) })
        }
    }
    fn CopyAllItems(&self, dest: Ref<IMFAttributes>) -> Result<()> {
        unsafe { self.attrs.CopyAllItems(dest.as_ref()) }
    }
}

#[cfg(test)]
mod tests {
    use windows::Win32::Media::MediaFoundation::{
        IMFMediaSourceEx, MFStartup, MFSTARTUP_FULL, MF_VERSION,
    };

    use super::*;

    #[test]
    fn the_activator_creates_the_media_source() {
        // SAFETY: Media Foundation start-up for this test process.
        unsafe { MFStartup(MF_VERSION, MFSTARTUP_FULL).unwrap() };
        let act = PbCameraActivate::create().unwrap();
        // SAFETY: activation of our own class.
        unsafe {
            let src: IMFMediaSourceEx = act.ActivateObject().expect("ActivateObject");
            let pd = src.CreatePresentationDescriptor().unwrap();
            assert_eq!(pd.GetStreamDescriptorCount().unwrap(), 1);
            src.Shutdown().unwrap();
            act.ShutdownObject().unwrap();
        }
    }

    #[test]
    fn the_activator_forwards_attributes() {
        // SAFETY: Media Foundation start-up for this test process.
        unsafe { MFStartup(MF_VERSION, MFSTARTUP_FULL).unwrap() };
        let act = PbCameraActivate::create().unwrap();
        let key = GUID::from_u128(0x1234_5678_0000_0000_0000_0000_0000_0001);
        // SAFETY: plain attribute calls on our own object.
        unsafe {
            assert_eq!(act.GetCount().unwrap(), 0);
            act.SetUINT32(&key, 77).unwrap();
            assert_eq!(act.GetUINT32(&key).unwrap(), 77);
            assert_eq!(act.GetCount().unwrap(), 1);
            act.DeleteAllItems().unwrap();
            assert_eq!(act.GetCount().unwrap(), 0);
        }
    }
}
