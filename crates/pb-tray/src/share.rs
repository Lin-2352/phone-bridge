//! Windows share sheet target. When the user shares to Phone Bridge, Windows
//! starts `pb-tray.exe` inside its (sparse) package with share activation;
//! this reads what was shared so `main` can hand it to the running tray.
//! Outside the package `take` returns `None` at once.
#![allow(unsafe_code)]

use std::path::PathBuf;

use windows::{
    core::Interface,
    ApplicationModel::{
        Activation::{ActivationKind, ShareTargetActivatedEventArgs},
        AppInstance,
        DataTransfer::{ShareTarget::ShareOperation, StandardDataFormats},
    },
};

pub enum Shared {
    Files(Vec<PathBuf>),
    Text(String),
    Link(String),
}

pub struct Share {
    op: ShareOperation,
    pub item: Shared,
}

impl Share {
    /// Tell Windows we are done so the share UI closes.
    pub fn done(&self) {
        let _ = self.op.ReportCompleted();
    }

    /// Delivery failed: Windows shows the error instead of a fake success.
    pub fn failed(&self, why: &str) {
        let _ = self.op.ReportError(&windows::core::HSTRING::from(why));
    }
}

/// True only when Windows started us inside our package (the share target).
/// Checked first so a normal launch never loads the WinRT activation runtime
/// (and its thread pool) just to find out there is nothing to do.
fn packaged() -> bool {
    let mut len = 0u32;
    // SAFETY: size query; APPMODEL_ERROR_NO_PACKAGE (15700) means unpackaged.
    let r = unsafe {
        windows::Win32::Storage::Packaging::Appx::GetCurrentPackageFullName(&mut len, None)
    };
    r.0 != 15700
}

pub fn take() -> Option<Share> {
    if !packaged() {
        return None;
    }
    let args = AppInstance::GetActivatedEventArgs().ok()?;
    if args.Kind().ok()? != ActivationKind::ShareTarget {
        return None;
    }
    let st: ShareTargetActivatedEventArgs = args.cast().ok()?;
    let op = st.ShareOperation().ok()?;
    // From here on Windows waits for an answer: every failure is reported,
    // or its share UI would spin until it gives up.
    match read(&op) {
        Some(item) => {
            let _ = op.ReportStarted();
            Some(Share { op, item })
        }
        None => {
            let _ = op.ReportError(&windows::core::HSTRING::from(
                "Relay could not read what was shared",
            ));
            None
        }
    }
}

fn read(op: &ShareOperation) -> Option<Shared> {
    let data = op.Data().ok()?;
    let has = |f: windows::core::Result<windows::core::HSTRING>| {
        f.and_then(|f| data.Contains(&f)).unwrap_or(false)
    };
    let item = if has(StandardDataFormats::StorageItems()) {
        let items = data.GetStorageItemsAsync().ok()?.join().ok()?;
        // Only items that are real files on disk (not virtual browser items).
        let files: Vec<PathBuf> = items
            .into_iter()
            .filter_map(|i| i.Path().ok())
            .map(|p| p.to_string())
            .filter(|p| !p.is_empty())
            .map(PathBuf::from)
            .collect();
        Shared::Files(files)
    } else if has(StandardDataFormats::WebLink()) {
        let uri = data.GetWebLinkAsync().ok()?.join().ok()?;
        Shared::Link(uri.AbsoluteUri().ok()?.to_string())
    } else if has(StandardDataFormats::Text()) {
        Shared::Text(data.GetTextAsync().ok()?.join().ok()?.to_string())
    } else {
        return None;
    };
    Some(item)
}
