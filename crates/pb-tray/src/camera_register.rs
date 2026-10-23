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
    core::{HSTRING, PCWSTR, PWSTR},
    Win32::{
        Foundation::{CloseHandle, LocalFree, HLOCAL, WAIT_OBJECT_0, WIN32_ERROR},
        Security::{
            Authorization::{
                ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
                GetNamedSecurityInfoW, SetNamedSecurityInfoW, SDDL_REVISION_1, SE_FILE_OBJECT,
            },
            GetSecurityDescriptorDacl, ACL, DACL_SECURITY_INFORMATION, OWNER_SECURITY_INFORMATION,
            PROTECTED_DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, PSID,
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
        UI::Shell::{
            FOLDERID_ProgramData, IsUserAnAdmin, SHGetKnownFolderPath, ShellExecuteExW,
            KF_FLAG_DEFAULT, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW,
        },
    },
};

/// Exit codes of the elevated helper.
pub const EXIT_OK: i32 = 0;
pub const EXIT_NOT_ELEVATED: i32 = 2;
pub const EXIT_COPY_FAILED: i32 = 3;
pub const EXIT_REGISTRY_FAILED: i32 = 4;
pub const EXIT_ACL_FAILED: i32 = 5;
/// The install folder is not ours and could not be replaced.
pub const EXIT_UNSAFE_FOLDER: i32 = 6;
pub const EXIT_BAD_SID: i32 = 7;

#[derive(Debug, PartialEq, Eq)]
pub enum RegisterError {
    /// The user said no to the UAC prompt.
    Declined,
    /// The elevated helper failed with this exit code (or could not start).
    Failed(i32),
}

const DLL_SOURCE_NAME: &str = "pb_camera_source.dll";

/// `C:\ProgramData` from Windows itself: the `ProgramData` environment
/// variable belongs to the user, and the elevated installer must not follow it.
fn program_data() -> PathBuf {
    // SAFETY: the returned string is copied and freed.
    unsafe {
        match SHGetKnownFolderPath(&FOLDERID_ProgramData, KF_FLAG_DEFAULT, None) {
            Ok(p) => {
                let s = p.to_string().unwrap_or_default();
                windows::Win32::System::Com::CoTaskMemFree(Some(p.0.cast()));
                if s.is_empty() {
                    PathBuf::from("C:\\ProgramData")
                } else {
                    PathBuf::from(s)
                }
            }
            Err(_) => PathBuf::from("C:\\ProgramData"),
        }
    }
}

/// `C:\ProgramData\PhoneBridge`.
fn base_dir() -> PathBuf {
    program_data().join("PhoneBridge")
}

/// `C:\ProgramData\PhoneBridge\camera`.
pub fn dll_dir() -> PathBuf {
    base_dir().join("camera")
}

/// Owners an install folder may have: SYSTEM, Administrators, a service
/// account such as TrustedInstaller. A folder any other account created
/// (the default for new folders in ProgramData) is not trusted.
pub fn is_trusted_owner(sid: &str) -> bool {
    matches!(sid, "S-1-5-18" | "S-1-5-32-544") || sid.starts_with("S-1-5-80-")
}

/// A user SID in string form: `S-1-` and digits and dashes, nothing that
/// could change the meaning of the SDDL it is put into.
pub fn is_valid_user_sid(sid: &str) -> bool {
    sid.starts_with("S-1-")
        && sid.len() <= 80
        && sid.len() > 4
        && sid[2..].bytes().all(|b| b.is_ascii_digit() || b == b'-')
}

fn owner_sid_string(path: &std::path::Path) -> Option<String> {
    // SAFETY: a security descriptor query; both allocations are freed.
    unsafe {
        let mut owner = PSID::default();
        let mut sd = PSECURITY_DESCRIPTOR::default();
        let r = GetNamedSecurityInfoW(
            &HSTRING::from(path.as_os_str()),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION,
            Some(&mut owner),
            None,
            None,
            None,
            &mut sd,
        );
        if r != WIN32_ERROR(0) {
            return None;
        }
        let mut s = PWSTR::null();
        let out = ConvertSidToStringSidW(owner, &mut s)
            .ok()
            .and_then(|()| s.to_string().ok());
        let _ = LocalFree(Some(HLOCAL(s.0.cast())));
        let _ = LocalFree(Some(HLOCAL(sd.0)));
        out
    }
}

fn is_reparse_point(path: &std::path::Path) -> bool {
    use std::os::windows::fs::MetadataExt;
    std::fs::symlink_metadata(path).is_ok_and(|m| m.file_attributes() & 0x400 != 0)
}

/// A folder only a trusted owner controls: created fresh, or an existing
/// one whose owner is trusted. A junction or symbolic link is refused, and
/// a folder some other account made is deleted and recreated (the
/// installer is elevated; whatever was planted there goes).
fn ensure_trusted_dir(path: &std::path::Path) -> Result<(), ()> {
    if std::fs::symlink_metadata(path).is_ok() {
        if is_reparse_point(path) {
            return Err(());
        }
        if owner_sid_string(path).is_some_and(|s| is_trusted_owner(&s)) {
            return Ok(());
        }
        std::fs::remove_dir_all(path).map_err(|_| ())?;
    }
    std::fs::create_dir(path).map_err(|_| ())
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

/// Windows' "Camera access" switches: the machine-wide one (HKLM) and the
/// user's (HKCU). Either `Deny` blocks every camera, this one included.
pub fn combine_access(hklm: Option<&str>, hkcu: Option<&str>) -> crate::camera_status::Access {
    use crate::camera_status::Access;
    let is = |v: Option<&str>, w: &str| v.is_some_and(|x| x.eq_ignore_ascii_case(w));
    if is(hklm, "Deny") || is(hkcu, "Deny") {
        Access::Deny
    } else if is(hklm, "Allow") || is(hkcu, "Allow") {
        Access::Allow
    } else {
        Access::Unknown
    }
}

fn read_string(root: HKEY, subkey: &str, name: &str) -> Option<String> {
    // SAFETY: a read-only registry query; the key is closed again.
    unsafe {
        let mut key = HKEY::default();
        RegOpenKeyExW(root, &HSTRING::from(subkey), None, KEY_READ, &mut key)
            .ok()
            .ok()?;
        let mut buf = vec![0u16; 64];
        let mut bytes = (buf.len() * 2) as u32;
        let r = RegQueryValueExW(
            key,
            &HSTRING::from(name),
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
        Some(String::from_utf16_lossy(&buf[..len]))
    }
}

/// Read the two switches now.
pub fn camera_access() -> crate::camera_status::Access {
    use windows::Win32::System::Registry::HKEY_CURRENT_USER;
    let key = "Software\\Microsoft\\Windows\\CurrentVersion\\CapabilityAccessManager\\ConsentStore\\webcam";
    combine_access(
        read_string(HKEY_LOCAL_MACHINE, key, "Value").as_deref(),
        read_string(HKEY_CURRENT_USER, key, "Value").as_deref(),
    )
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
    if !is_valid_user_sid(user_sid) {
        return EXIT_BAD_SID;
    }
    // Both folders must be ours before anything goes in: ProgramData lets
    // any user create folders, and DLLs here are loaded by Frame Server.
    let dir = dll_dir();
    if ensure_trusted_dir(&base_dir()).is_err() || ensure_trusted_dir(&dir).is_err() {
        return EXIT_UNSAFE_FOLDER;
    }
    if set_dacl(&base_dir(), &dir_sddl()).is_err() || set_dacl(&dir, &dir_sddl()).is_err() {
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
    // Never trust a file that is already there: write the shipped bytes.
    let same = std::fs::read(&installed).is_ok_and(|b| b == bytes);
    if !same {
        let _ = std::fs::remove_file(&installed);
        if std::fs::write(&installed, &bytes).is_err() {
            return EXIT_COPY_FAILED;
        }
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
    let _ = std::fs::remove_dir(base_dir());
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
    fn only_system_and_administrators_own_the_install_folder() {
        assert!(is_trusted_owner("S-1-5-18"));
        assert!(is_trusted_owner("S-1-5-32-544"));
        assert!(is_trusted_owner(
            "S-1-5-80-956008885-3418522649-1831038044-1853292631-2271478464"
        ));
        assert!(!is_trusted_owner("S-1-5-21-1-2-3-1001"), "an ordinary user");
        assert!(!is_trusted_owner("S-1-5-32-545"), "the Users group");
        assert!(!is_trusted_owner(""));
    }

    #[test]
    fn user_sids_cannot_carry_sddl() {
        assert!(is_valid_user_sid("S-1-5-21-1-2-3-1001"));
        assert!(!is_valid_user_sid(""));
        assert!(!is_valid_user_sid("S-1-"));
        assert!(!is_valid_user_sid("S-1-5-21)(A;;GA;;;WD"), "SDDL injection");
        assert!(!is_valid_user_sid("Administrators"));
        assert!(!is_valid_user_sid(&format!("S-1-{}", "1".repeat(90))));
    }

    #[test]
    fn a_planted_folder_is_replaced_and_a_junction_refused() {
        let root = std::env::temp_dir().join(format!("pbcam-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        // A folder made by this account (not SYSTEM or Administrators, unless
        // the test itself runs elevated): the installer replaces it.
        let planted = root.join("camera");
        std::fs::create_dir(&planted).unwrap();
        std::fs::write(planted.join("evil.dll"), b"x").unwrap();
        let owner = owner_sid_string(&planted).unwrap_or_default();
        assert!(ensure_trusted_dir(&planted).is_ok());
        if !is_trusted_owner(&owner) {
            assert!(!planted.join("evil.dll").exists(), "planted file gone");
        }
        // A junction is never followed.
        let target = root.join("elsewhere");
        std::fs::create_dir(&target).unwrap();
        let link = root.join("link");
        let made = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(&link)
            .arg(&target)
            .output()
            .is_ok_and(|o| o.status.success());
        if made {
            assert!(is_reparse_point(&link));
            assert!(ensure_trusted_dir(&link).is_err(), "a junction is refused");
            assert!(target.exists(), "the target is untouched");
        }
        let _ = std::process::Command::new("cmd")
            .args(["/C", "rmdir"])
            .arg(&link)
            .output();
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn either_camera_switch_off_blocks_every_camera() {
        use crate::camera_status::Access;
        assert_eq!(combine_access(Some("Allow"), Some("Allow")), Access::Allow);
        assert_eq!(combine_access(Some("Deny"), Some("Allow")), Access::Deny);
        assert_eq!(combine_access(Some("Allow"), Some("deny")), Access::Deny);
        assert_eq!(combine_access(None, Some("Allow")), Access::Allow);
        assert_eq!(combine_access(None, None), Access::Unknown);
        assert_eq!(combine_access(Some(""), Some("Prompt")), Access::Unknown);
    }

    #[test]
    fn the_real_registry_gives_an_answer() {
        // Any machine has these keys; the value is one of the three.
        let a = camera_access();
        assert!(matches!(
            a,
            crate::camera_status::Access::Allow
                | crate::camera_status::Access::Deny
                | crate::camera_status::Access::Unknown
        ));
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
