//! Real Windows toasts under Phone Bridge's own identity: a Start-menu
//! shortcut carrying the AppUserModelID and a toast-activator CLSID, and a
//! per-user COM registration, so buttons and replies on toasts that sit in
//! the notification centre reach the tray (or start it). No admin rights.
#![allow(unsafe_code)]

use std::{collections::HashSet, path::Path, sync::OnceLock};

use windows::{
    core::{implement, Interface, Ref, BOOL, GUID, HSTRING, PCWSTR},
    Data::Xml::Dom::XmlDocument,
    Foundation::TypedEventHandler,
    Win32::{
        Foundation::{CLASS_E_NOAGGREGATION, ERROR_SUCCESS, PROPERTYKEY},
        System::{
            Com::{
                CoCreateInstance, CoInitializeEx, CoRegisterClassObject, IClassFactory,
                IClassFactory_Impl, IPersistFile,
                StructuredStorage::{InitPropVariantFromCLSID, PropVariantClear, PROPVARIANT},
                CLSCTX_INPROC_SERVER, CLSCTX_LOCAL_SERVER, COINIT_APARTMENTTHREADED,
                REGCLS_MULTIPLEUSE,
            },
            Registry::{
                RegCloseKey, RegCreateKeyExW, RegSetValueExW, HKEY, HKEY_CURRENT_USER, KEY_WRITE,
                REG_OPTION_NON_VOLATILE, REG_SZ,
            },
            Variant::VT_LPWSTR,
        },
        UI::{
            Notifications::{
                INotificationActivationCallback, INotificationActivationCallback_Impl,
                NOTIFICATION_USER_INPUT_DATA,
            },
            Shell::{IShellLinkW, PropertiesSystem::IPropertyStore, SHStrDupW, ShellLink},
        },
    },
    UI::Notifications::{
        NotificationSetting, ToastDismissalReason, ToastDismissedEventArgs, ToastNotification,
        ToastNotificationManager,
    },
};

use crate::toast_xml::{toast_xml, ToastSpec};

pub const AUMID: &str = "PhoneBridge.Tray";
const ACTIVATOR: GUID = GUID::from_u128(0x72c4c623_ea97_46ea_8c4d_5de9e1f8582d);
const PKEY_APP_ID: PROPERTYKEY = PROPERTYKEY {
    fmtid: GUID::from_u128(0x9f4c2855_9f79_4b39_a8d0_e1d42de1d5f3),
    pid: 5,
};
const PKEY_TOAST_ACTIVATOR: PROPERTYKEY = PROPERTYKEY {
    fmtid: GUID::from_u128(0x9f4c2855_9f79_4b39_a8d0_e1d42de1d5f3),
    pid: 26,
};

/// Raw toast arguments (see `toast_xml::parse_args`) and the reply text.
type OnActivate = Box<dyn Fn(String, Option<String>) + Send + Sync>;
type OnDismiss = Box<dyn Fn(String) + Send + Sync>;
static CALLBACKS: OnceLock<(OnActivate, OnDismiss)> = OnceLock::new();

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// Set up the identity and start receiving activations. Call once, on the
/// UI thread (its STA message loop delivers the activations).
pub fn register(on_activate: OnActivate, on_dismiss: OnDismiss) -> Result<(), String> {
    let _ = CALLBACKS.set((on_activate, on_dismiss));
    let exe = std::env::current_exe().map_err(err)?;
    // SAFETY: COM init on this thread; repeated calls are harmless.
    let _ = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
    write_shortcut(&exe)?;
    write_com_server(&exe)?;
    let factory: IClassFactory = Factory.into();
    // SAFETY: the factory is a valid COM object; the registration lives as
    // long as the process (Windows drops it at exit).
    unsafe {
        CoRegisterClassObject(
            &ACTIVATOR,
            &factory,
            CLSCTX_LOCAL_SERVER,
            REGCLS_MULTIPLEUSE,
        )
        .map_err(err)?;
    }
    Ok(())
}

fn start_menu_link() -> Result<std::path::PathBuf, String> {
    let appdata = std::env::var_os("APPDATA").ok_or("no APPDATA folder")?;
    Ok(std::path::PathBuf::from(appdata)
        .join("Microsoft")
        .join("Windows")
        .join("Start Menu")
        .join("Programs")
        .join("Phone Bridge.lnk"))
}

/// A PROPVARIANT holding a CoTaskMem string (freed by PropVariantClear).
///
/// # Safety
/// The caller clears the result with `PropVariantClear`.
unsafe fn string_variant(s: &str) -> windows::core::Result<PROPVARIANT> {
    // SAFETY: SHStrDupW allocates with CoTaskMemAlloc, which PropVariantClear frees.
    let text = unsafe { SHStrDupW(&HSTRING::from(s))? };
    let mut pv = PROPVARIANT::default();
    // SAFETY: writing the active union member of a zeroed PROPVARIANT.
    unsafe {
        let inner = &mut *pv.Anonymous.Anonymous;
        inner.vt = VT_LPWSTR;
        inner.Anonymous.pwszVal = text;
    }
    Ok(pv)
}

fn write_shortcut(exe: &Path) -> Result<(), String> {
    let link = start_menu_link()?;
    // SAFETY: standard IShellLink / IPropertyStore usage on this thread's
    // STA; strings outlive each call; both PROPVARIANTs are cleared.
    unsafe {
        let sl: IShellLinkW =
            CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).map_err(err)?;
        sl.SetPath(&HSTRING::from(exe.as_os_str())).map_err(err)?;
        sl.SetDescription(&HSTRING::from("Phone Bridge"))
            .map_err(err)?;
        sl.SetIconLocation(&HSTRING::from(exe.as_os_str()), 0)
            .map_err(err)?;
        let store: IPropertyStore = sl.cast().map_err(err)?;
        let mut id = string_variant(AUMID).map_err(err)?;
        let mut clsid = InitPropVariantFromCLSID(&ACTIVATOR).map_err(err)?;
        let set = store
            .SetValue(&PKEY_APP_ID, &id)
            .and_then(|_| store.SetValue(&PKEY_TOAST_ACTIVATOR, &clsid))
            .and_then(|_| store.Commit());
        let _ = PropVariantClear(&mut id);
        let _ = PropVariantClear(&mut clsid);
        set.map_err(err)?;
        let pf: IPersistFile = sl.cast().map_err(err)?;
        pf.Save(&HSTRING::from(link.as_os_str()), true)
            .map_err(err)?;
    }
    Ok(())
}

/// `HKCU\Software\Classes\CLSID\{activator}\LocalServer32` = `"<exe>" -ToastActivated`
fn write_com_server(exe: &Path) -> Result<(), String> {
    let sub = format!("Software\\Classes\\CLSID\\{{{ACTIVATOR:?}}}\\LocalServer32");
    let value = format!("\"{}\" -ToastActivated", exe.display());
    let wide: Vec<u16> = value.encode_utf16().chain(std::iter::once(0)).collect();
    let bytes: Vec<u8> = wide.iter().flat_map(|c| c.to_le_bytes()).collect();
    let mut key = HKEY::default();
    // SAFETY: plain registry calls with owned buffers; the key is closed.
    unsafe {
        let r = RegCreateKeyExW(
            HKEY_CURRENT_USER,
            &HSTRING::from(sub),
            None,
            PCWSTR::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_WRITE,
            None,
            &mut key,
            None,
        );
        if r != ERROR_SUCCESS {
            return Err(format!("registry: {r:?}"));
        }
        let r = RegSetValueExW(key, PCWSTR::null(), None, REG_SZ, Some(&bytes));
        let _ = RegCloseKey(key);
        if r != ERROR_SUCCESS {
            return Err(format!("registry: {r:?}"));
        }
    }
    Ok(())
}

#[implement(IClassFactory)]
struct Factory;

impl IClassFactory_Impl for Factory_Impl {
    fn CreateInstance(
        &self,
        outer: Ref<windows::core::IUnknown>,
        iid: *const GUID,
        object: *mut *mut core::ffi::c_void,
    ) -> windows::core::Result<()> {
        if outer.is_some() {
            return Err(CLASS_E_NOAGGREGATION.into());
        }
        let activator: INotificationActivationCallback = Activator.into();
        // SAFETY: COM contract: `iid` and `object` come from the caller.
        unsafe { activator.query(iid, object).ok() }
    }

    fn LockServer(&self, _lock: BOOL) -> windows::core::Result<()> {
        Ok(())
    }
}

#[implement(INotificationActivationCallback)]
struct Activator;

impl INotificationActivationCallback_Impl for Activator_Impl {
    fn Activate(
        &self,
        _app: &PCWSTR,
        args: &PCWSTR,
        data: *const NOTIFICATION_USER_INPUT_DATA,
        count: u32,
    ) -> windows::core::Result<()> {
        // SAFETY: Windows passes valid strings and `count` input entries.
        let (args, reply) = unsafe {
            let args = args.to_string().unwrap_or_default();
            let mut reply = None;
            for i in 0..count as usize {
                let d = &*data.add(i);
                if d.Key.to_string().ok().as_deref() == Some("reply") {
                    reply = d.Value.to_string().ok();
                }
            }
            (args, reply)
        };
        if let Some((on_activate, _)) = CALLBACKS.get() {
            on_activate(args, reply);
        }
        Ok(())
    }
}

/// Show (or replace, by tag and group) a toast.
pub fn show(s: &ToastSpec) -> Result<(), String> {
    let doc = XmlDocument::new().map_err(err)?;
    doc.LoadXml(&HSTRING::from(toast_xml(s))).map_err(err)?;
    let toast = ToastNotification::CreateToastNotification(&doc).map_err(err)?;
    toast.SetTag(&HSTRING::from(&s.tag)).map_err(err)?;
    toast.SetGroup(&HSTRING::from(&s.group)).map_err(err)?;
    toast.SetSuppressPopup(!s.popup).map_err(err)?;
    let tag = s.tag.clone();
    toast
        .Dismissed(&TypedEventHandler::new(
            move |_: Ref<ToastNotification>, args: Ref<ToastDismissedEventArgs>| {
                if let Some(a) = args.as_ref() {
                    if a.Reason()? == ToastDismissalReason::UserCanceled {
                        if let Some((_, on_dismiss)) = CALLBACKS.get() {
                            on_dismiss(tag.clone());
                        }
                    }
                }
                Ok(())
            },
        ))
        .map_err(err)?;
    ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(AUMID))
        .and_then(|n| n.Show(&toast))
        .map_err(err)
}

/// Whether Windows will show Phone Bridge's toasts at all (the user may
/// have turned them off for the app, or for every app).
pub fn enabled() -> bool {
    ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(AUMID))
        .and_then(|n| n.Setting())
        .is_ok_and(|s| s == NotificationSetting::Enabled)
}

/// Phone Bridge's toasts in the notification centre: (tag, group, launch
/// arguments).
pub fn history() -> Vec<(String, String, String)> {
    let Ok(list) =
        ToastNotificationManager::History().and_then(|h| h.GetHistoryWithId(&HSTRING::from(AUMID)))
    else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for t in list {
        let launch = t
            .Content()
            .and_then(|c| c.DocumentElement())
            .and_then(|e| e.GetAttribute(&HSTRING::from("launch")))
            .map(|s| s.to_string())
            .unwrap_or_default();
        if let (Ok(tag), Ok(group)) = (t.Tag(), t.Group()) {
            out.push((tag.to_string(), group.to_string(), launch));
        }
    }
    out
}

pub fn remove(tag: &str, group: &str) {
    if let Ok(h) = ToastNotificationManager::History() {
        let _ = h.RemoveGroupedTagWithId(
            &HSTRING::from(tag),
            &HSTRING::from(group),
            &HSTRING::from(AUMID),
        );
    }
}

/// Tags of Phone Bridge's toasts still in the notification centre.
pub fn history_tags() -> Result<HashSet<String>, String> {
    let list = ToastNotificationManager::History()
        .and_then(|h| h.GetHistoryWithId(&HSTRING::from(AUMID)))
        .map_err(err)?;
    let mut tags = HashSet::new();
    for t in list {
        if let Ok(tag) = t.Tag() {
            tags.insert(tag.to_string());
        }
    }
    Ok(tags)
}
