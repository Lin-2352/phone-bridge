//! Installing the camera source for Windows' Frame Server (one UAC prompt):
//! `pb-tray --register-camera --for-sid <SID>` (elevated) copies the DLL to
//! `%ProgramData%\PhoneBridge\camera`, writes the HKLM class keys and
//! creates the frames file; `--unregister-camera` takes all of it away.
//!
//! The DLL must not load from the user's profile or a build folder:
//! Frame Server runs as LocalService, which cannot read there.
#![allow(unsafe_code)]

use std::path::PathBuf;

use pb_camera_frames::{Ring, FILE_BYTES};
use pb_camera_source::{com::CLSID_PHONE_BRIDGE_CAMERA_SOURCE, file::Mapped};
use windows::{
    core::{HSTRING, PCWSTR},
    Win32::{
        Foundation::{CloseHandle, WAIT_OBJECT_0, WIN32_ERROR},
        Security::{
            Authorization::{
                ConvertStringSecurityDescriptorToSecurityDescriptorW, SetNamedSecurityInfoW,
                SDDL_REVISION_1, SE_FILE_OBJECT,
            },
            GetSecurityDescriptorDacl, ACL, DACL_SECURITY_INFORMATION,
            PROTECTED_DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR,
        },
        Storage::FileSystem::{MoveFileExW, MOVEFILE_DELAY_UNTIL_REBOOT},
        System::{
            Registry::{
                RegCloseKey, RegCreateKeyExW, RegDeleteTreeW, RegOpenKeyExW, RegQueryValueExW,
                RegSetValueExW, HKEY, HKEY_LOCAL_MACHINE, KEY_READ, KEY_WRITE,
                REG_OPTION_NON_VOLATILE, REG_SZ,
            },
            Threading::{GetExitCodeProcess, WaitForSingleObject, INFINITE},
        },
        UI::Shell::{IsUserAnAdmin, ShellExecuteExW, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW},
    },
};

/// Exit codes of the elevated helper.
pub const EXIT_OK: i32 = 0;
pub const EXIT_NOT_ELEVATED: i32 = 2;
pub const EXIT_COPY_FAILED: i32 = 3;
pub const EXIT_REGISTRY_FAILED: i32 = 4;
pub const EXIT_ACL_FAILED: i32 = 5;

#[derive(Debug, PartialEq, Eq)]
pub enum RegisterError {
    /// The user said no to the UAC prompt.
    Declined,
    /// The elevated helper failed with this exit code (or could not start).
    Failed(i32),
}

const DLL_SOURCE_NAME: &str = "pb_camera_source.dll";

/// `%ProgramData%\PhoneBridge\camera`.
pub fn dll_dir() -> PathBuf {
    let base = std::env::var_os("ProgramData").unwrap_or_else(|| "C:\\ProgramData".into());
    PathBuf::from(base).join("PhoneBridge").join("camera")
}

/// The installed DLL's file name: the version and a hash of its bytes, so
/// an update installs next to a DLL that Frame Server still holds, and the
/// same bytes always land on the same name.
pub fn dll_file_name(bytes: &[u8]) -> String {
    // FNV-1a, 32 bit: a name, not a security check.
    let mut h: u32 = 0x811c_9dc5;
    for b in bytes {
        h ^= u32::from(*b);
        h = h.wrapping_mul(0x0100_0193);
    }
    format!("pb-camera-source-{}-{h:08x}.dll", env!("CARGO_PKG_VERSION"))
}

/// The DLL path the class key names, if the key exists.
fn registered_dll() -> Option<PathBuf> {
    // SAFETY: a read-only registry query; the key is closed again.
    unsafe {
        let mut key = HKEY::default();
        let sub = HSTRING::from(format!("{}\\InprocServer32", clsid_key()));
        RegOpenKeyExW(HKEY_LOCAL_MACHINE, &sub, None, KEY_READ, &mut key)
            .ok()
            .ok()?;
        let mut buf = vec![0u16; 520];
        let mut bytes = (buf.len() * 2) as u32;
        let r = RegQueryValueExW(
            key,
            PCWSTR::null(),
            None,
            None,
            Some(buf.as_mut_ptr().cast()),
            Some(&mut bytes),
        );
        let _ = RegCloseKey(key);
        if r != WIN32_ERROR(0) {
            return None;
        }
        let len = (bytes as usize / 2).saturating_sub(1).min(buf.len());
        Some(PathBuf::from(String::from_utf16_lossy(&buf[..len])))
    }
}

pub fn frames_path() -> PathBuf {
    dll_dir().join("frames")
}

fn clsid_key() -> String {
    format!(
        "Software\\Classes\\CLSID\\{{{:?}}}",
        CLSID_PHONE_BRIDGE_CAMERA_SOURCE
    )
}

/// The DLL folder: SYSTEM and Administrators full control; LocalService and
/// Users read and execute. Protected: nothing is inherited from ProgramData.
pub fn dir_sddl() -> String {
    "D:PAI(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)(A;OICI;FRFX;;;LS)(A;OICI;FRFX;;;BU)".to_owned()
}

/// The frames file: SYSTEM full; LocalService (Frame Server) and the
/// registering user read and write. No one else.
pub fn frames_sddl(user_sid: &str) -> String {
    format!("D:PAI(A;;FA;;;SY)(A;;FRFW;;;LS)(A;;FRFW;;;{user_sid})")
}

/// Installed, but not the DLL that ships with this tray (an update): the
/// next start of the camera installs the new one (one UAC prompt).
pub fn needs_install() -> bool {
    let shipped = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join(DLL_SOURCE_NAME)))
        .and_then(|p| std::fs::read(p).ok());
    let Some(bytes) = shipped else {
        return false; // nothing to install from
    };
    let want = dll_file_name(&bytes);
    !registered_dll()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
        .is_some_and(|n| n == want)
}

/// Whether the camera source is installed: the class key names a DLL that
/// exists.
pub fn is_registered() -> bool {
    registered_dll().is_some_and(|p| p.is_file()) && frames_path().is_file()
}

fn set_dacl(path: &std::path::Path, sddl: &str) -> Result<(), ()> {
    let mut sd = PSECURITY_DESCRIPTOR::default();
    // SAFETY: a valid SDDL string; the DACL pointer stays valid with `sd`,
    // which lives until the end of this function.
    unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            &HSTRING::from(sddl),
            SDDL_REVISION_1,
            &mut sd,
            None,
        )
        .map_err(|_| ())?;
        let (mut present, mut defaulted) = (Default::default(), Default::default());
        let mut dacl: *mut ACL = std::ptr::null_mut();
        GetSecurityDescriptorDacl(sd, &mut present, &mut dacl, &mut defaulted).map_err(|_| ())?;
        let r = SetNamedSecurityInfoW(
            &HSTRING::from(path.as_os_str()),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
            None,
            None,
            Some(dacl),
            None,
        );
        if r != WIN32_ERROR(0) {
            return Err(());
        }
    }
    Ok(())
}

fn set_reg_string(key: HKEY, name: Option<&str>, value: &str) -> bool {
    let data: Vec<u16> = value.encode_utf16().chain(std::iter::once(0)).collect();
    // SAFETY: the buffer outlives the call; REG_SZ wants UTF-16 with its NUL.
    let r = unsafe {
        let bytes = std::slice::from_raw_parts(data.as_ptr().cast::<u8>(), data.len() * 2);
        match name {
            Some(n) => RegSetValueExW(key, &HSTRING::from(n), None, REG_SZ, Some(bytes)),
            None => RegSetValueExW(key, PCWSTR::null(), None, REG_SZ, Some(bytes)),
        }
    };
    r == WIN32_ERROR(0)
}

/// The elevated install. `user_sid`: the account that will stream.
pub fn register_elevated(user_sid: &str) -> i32 {
    // SAFETY: a plain query.
    if !unsafe { IsUserAnAdmin() }.as_bool() {
        return EXIT_NOT_ELEVATED;
    }
    let dir = dll_dir();
    if std::fs::create_dir_all(&dir).is_err() {
        return EXIT_COPY_FAILED;
    }
    if set_dacl(&dir, &dir_sddl()).is_err() {
        return EXIT_ACL_FAILED;
    }
    let source = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join(DLL_SOURCE_NAME)));
    let Some(source) = source else {
        return EXIT_COPY_FAILED;
    };
    let Ok(bytes) = std::fs::read(&source) else {
        return EXIT_COPY_FAILED;
    };
    let installed = dir.join(dll_file_name(&bytes));
    if !installed.is_file() && std::fs::write(&installed, &bytes).is_err() {
        return EXIT_COPY_FAILED;
    }
    // Older copies go if Frame Server does not hold them (else they stay
    // until the next install or the removal).
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for e in entries.flatten() {
            let p = e.path();
            if p != installed && p.extension().is_some_and(|x| x == "dll") {
                let _ = std::fs::remove_file(&p);
            }
        }
    }
    // The frames file: full size, header formatted, then its own ACL.
    let frames = frames_path();
    if std::fs::write(&frames, vec![0u8; FILE_BYTES]).is_err() {
        return EXIT_COPY_FAILED;
    }
    if set_dacl(&frames, &frames_sddl(user_sid)).is_err() {
        return EXIT_ACL_FAILED;
    }
    match Mapped::open(&frames, true) {
        Ok(mut m) => {
            Ring::init(m.words());
        }
        Err(_) => return EXIT_COPY_FAILED,
    }
    // HKLM\Software\Classes\CLSID\{...}\InprocServer32 = the DLL, Both.
    // SAFETY: registry writes under our own class key; keys closed again.
    unsafe {
        let mut key = HKEY::default();
        let sub = HSTRING::from(format!("{}\\InprocServer32", clsid_key()));
        if RegCreateKeyExW(
            HKEY_LOCAL_MACHINE,
            &sub,
            None,
            PCWSTR::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_WRITE,
            None,
            &mut key,
            None,
        ) != WIN32_ERROR(0)
        {
            return EXIT_REGISTRY_FAILED;
        }
        let ok = set_reg_string(key, None, &installed.display().to_string())
            && set_reg_string(key, Some("ThreadingModel"), "Both");
        let _ = RegCloseKey(key);
        if !ok {
            return EXIT_REGISTRY_FAILED;
        }
    }
    EXIT_OK
}

/// The elevated removal: class keys, frames file and DLLs. A DLL Frame
/// Server still holds is deleted at the next restart.
pub fn unregister_elevated() -> i32 {
    // SAFETY: a plain query.
    if !unsafe { IsUserAnAdmin() }.as_bool() {
        return EXIT_NOT_ELEVATED;
    }
    // SAFETY: deleting our own class key.
    unsafe {
        let r = RegDeleteTreeW(HKEY_LOCAL_MACHINE, &HSTRING::from(clsid_key()));
        // 2 = the key was not there: already clean.
        if r != WIN32_ERROR(0) && r != WIN32_ERROR(2) {
            return EXIT_REGISTRY_FAILED;
        }
    }
    let _ = std::fs::remove_file(frames_path());
    if let Ok(entries) = std::fs::read_dir(dll_dir()) {
        for e in entries.flatten() {
            let p = e.path();
            if p.extension().is_some_and(|x| x == "dll") && std::fs::remove_file(&p).is_err() {
                // SAFETY: scheduling our own file for deletion at restart.
                let _ = unsafe {
                    MoveFileExW(
                        &HSTRING::from(p.as_os_str()),
                        PCWSTR::null(),
                        MOVEFILE_DELAY_UNTIL_REBOOT,
                    )
                };
            }
        }
    }
    let _ = std::fs::remove_dir(dll_dir());
    EXIT_OK
}

/// Run this exe elevated with `args` (one UAC prompt) and wait for it.
fn run_elevated(args: &str) -> Result<(), RegisterError> {
    let exe = std::env::current_exe().map_err(|_| RegisterError::Failed(-1))?;
    let file = HSTRING::from(exe.as_os_str());
    let params = HSTRING::from(args);
    let verb = HSTRING::from("runas");
    let mut info = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS,
        lpVerb: PCWSTR(verb.as_ptr()),
        lpFile: PCWSTR(file.as_ptr()),
        lpParameters: PCWSTR(params.as_ptr()),
        nShow: 0,
        ..Default::default()
    };
    // SAFETY: `info` and the strings it points to live through the calls;
    // the process handle is closed at the end.
    unsafe {
        if let Err(e) = ShellExecuteExW(&mut info) {
            // ERROR_CANCELLED: the user declined the prompt.
            return Err(if e.code().0 as u32 & 0xFFFF == 1223 {
                RegisterError::Declined
            } else {
                RegisterError::Failed(-2)
            });
        }
        let process = info.hProcess;
        if WaitForSingleObject(process, INFINITE) != WAIT_OBJECT_0 {
            let _ = CloseHandle(process);
            return Err(RegisterError::Failed(-3));
        }
        let mut code = 0u32;
        let _ = GetExitCodeProcess(process, &mut code);
        let _ = CloseHandle(process);
        if code as i32 == EXIT_OK {
            Ok(())
        } else {
            Err(RegisterError::Failed(code as i32))
        }
    }
}

/// Install (one UAC prompt). `user_sid`: this account.
pub fn request_register(user_sid: &str) -> Result<(), RegisterError> {
    run_elevated(&format!("--register-camera --for-sid {user_sid}"))
}

/// Remove (one UAC prompt).
pub fn request_unregister() -> Result<(), RegisterError> {
    run_elevated("--unregister-camera")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_are_under_programdata() {
        let d = dll_dir();
        assert!(d.ends_with("PhoneBridge\\camera"), "{d:?}");
        let name = dll_file_name(b"some dll bytes");
        assert!(name.contains(env!("CARGO_PKG_VERSION")), "{name}");
        assert!(name.starts_with("pb-camera-source-") && name.ends_with(".dll"));
        assert_eq!(
            name,
            dll_file_name(b"some dll bytes"),
            "same bytes, same name"
        );
        assert_ne!(
            name,
            dll_file_name(b"other dll bytes"),
            "new bytes, new name"
        );
        assert!(frames_path().starts_with(&d));
    }

    #[test]
    fn sddl_for_frames_grants_only_user_localservice_system() {
        let s = frames_sddl("S-1-5-21-1-2-3-1001");
        assert!(s.contains("(A;;FA;;;SY)"), "{s}");
        assert!(s.contains("(A;;FRFW;;;LS)"), "{s}");
        assert!(s.contains("(A;;FRFW;;;S-1-5-21-1-2-3-1001)"), "{s}");
        for everyone in [";;;WD)", ";;;BU)", ";;;AU)", ";;;BA)"] {
            assert!(!s.contains(everyone), "{s} must not grant {everyone}");
        }
    }

    #[test]
    fn sddl_for_the_dll_folder_lets_localservice_read_but_not_write() {
        let s = dir_sddl();
        assert!(s.contains("(A;OICI;FRFX;;;LS)"), "{s}");
        assert!(
            s.contains("(A;OICI;FA;;;BA)") && s.contains("(A;OICI;FA;;;SY)"),
            "{s}"
        );
        assert!(
            !s.contains("FRFW"),
            "no write for users or LocalService: {s}"
        );
        assert!(s.starts_with("D:P"), "protected: {s}");
    }

    #[test]
    fn the_class_key_names_our_clsid() {
        let k = clsid_key();
        assert!(k.starts_with("Software\\Classes\\CLSID\\{"), "{k}");
        assert!(
            k.to_ascii_lowercase()
                .contains("b8a7c1e2-5d3f-4a91-8e6b-2c4d9f0a7b13"),
            "{k}"
        );
    }

    #[test]
    fn not_elevated_means_nothing_is_touched() {
        // Tests run without elevation: the helpers refuse before any change.
        if unsafe { IsUserAnAdmin() }.as_bool() {
            return;
        }
        assert_eq!(register_elevated("S-1-5-21-1"), EXIT_NOT_ELEVATED);
        assert_eq!(unregister_elevated(), EXIT_NOT_ELEVATED);
    }
}
