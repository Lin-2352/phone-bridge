//! Windows share sheet target. When the user shares to Phone Bridge, Windows
//! starts `pb-tray.exe` inside its (sparse) package with share activation;
//! this reads what was shared so `main` can hand it to the running tray.
//! Outside the package `take` returns `None` at once.

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
}

pub fn take() -> Option<Share> {
    let args = AppInstance::GetActivatedEventArgs().ok()?;
    if args.Kind().ok()? != ActivationKind::ShareTarget {
        return None;
    }
    let st: ShareTargetActivatedEventArgs = args.cast().ok()?;
    let op = st.ShareOperation().ok()?;
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
    let _ = op.ReportStarted();
    Some(Share { op, item })
}
