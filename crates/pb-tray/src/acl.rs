//! Whether a folder can be changed by accounts other than this user and the
//! machine's own (SYSTEM, Administrators, TrustedInstaller). Relay must not
//! run anything elevated from such a folder: another account could have
//! swapped Relay.exe or the camera DLL beside it.

/// One entry of a folder's access list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ace {
    pub sid: String,
    pub allow: bool,
    pub mask: u32,
}

/// SYSTEM, Administrators, TrustedInstaller and CREATOR OWNER (which stands
/// for whoever creates a file inside).
const TRUSTED: &[&str] = &[
    "S-1-5-18",
    "S-1-5-32-544",
    "S-1-5-80-956008885-3418522649-1831038044-1853292631-2271478464",
    "S-1-3-0",
];

/// Rights that let an account put, replace or delete a file, or take over
/// the folder: add file or subfolder, delete child, delete, write DACL,
/// write owner, generic write or all.
const WRITE_MASK: u32 = 0x0000_0002
    | 0x0000_0004
    | 0x0000_0040
    | 0x0001_0000
    | 0x0004_0000
    | 0x0008_0000
    | 0x1000_0000
    | 0x4000_0000;

/// Whether an allow entry gives write rights to anyone but `me` and the
/// machine. Deny entries only take rights away; inheritable entries count
/// too, because existing files carry them.
pub fn others_can_write(aces: &[Ace], me: &str) -> bool {
    aces.iter().any(|a| {
        a.allow && a.mask & WRITE_MASK != 0 && a.sid != me && !TRUSTED.contains(&a.sid.as_str())
    })
}

/// The folder's access entries; None when Windows does not say.
#[cfg(windows)]
#[allow(unsafe_code)]
pub fn folder_aces(path: &std::path::Path) -> Option<Vec<Ace>> {
    use windows::{
        core::{HSTRING, PWSTR},
        Win32::{
            Foundation::{LocalFree, HLOCAL, WIN32_ERROR},
            Security::{
                AclSizeInformation,
                Authorization::{ConvertSidToStringSidW, GetNamedSecurityInfoW, SE_FILE_OBJECT},
                GetAce, GetAclInformation, ACCESS_ALLOWED_ACE, ACE_HEADER, ACL,
                ACL_SIZE_INFORMATION, DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, PSID,
            },
        },
    };
    // SAFETY: Windows allocates the descriptor (freed below); the DACL and
    // the ACEs point into it and are read only while it lives. ACCESS_ALLOWED
    // and ACCESS_DENIED entries share one layout.
    unsafe {
        let mut dacl: *mut ACL = std::ptr::null_mut();
        let mut sd = PSECURITY_DESCRIPTOR::default();
        let r = GetNamedSecurityInfoW(
            &HSTRING::from(path.as_os_str()),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION,
            None,
            None,
            Some(&mut dacl),
            None,
            &mut sd,
        );
        if r != WIN32_ERROR(0) {
            return None;
        }
        let out = if dacl.is_null() {
            // No DACL at all: everyone has full access.
            Some(vec![Ace {
                sid: "S-1-1-0".into(),
                allow: true,
                mask: 0x1000_0000,
            }])
        } else {
            let mut info = ACL_SIZE_INFORMATION::default();
            if GetAclInformation(
                dacl,
                (&mut info as *mut ACL_SIZE_INFORMATION).cast(),
                std::mem::size_of::<ACL_SIZE_INFORMATION>() as u32,
                AclSizeInformation,
            )
            .is_err()
            {
                None
            } else {
                let mut aces = Vec::new();
                for i in 0..info.AceCount {
                    let mut p: *mut core::ffi::c_void = std::ptr::null_mut();
                    if GetAce(dacl, i, &mut p).is_err() || p.is_null() {
                        continue;
                    }
                    let kind = (*(p as *const ACE_HEADER)).AceType;
                    // 0 allowed, 1 denied; other kinds (object, callback) don't occur on files.
                    if kind > 1 {
                        continue;
                    }
                    let a = &*(p as *const ACCESS_ALLOWED_ACE);
                    let sid = PSID(std::ptr::addr_of!(a.SidStart) as *mut _);
                    let mut s = PWSTR::null();
                    if ConvertSidToStringSidW(sid, &mut s).is_ok() {
                        aces.push(Ace {
                            sid: s.to_string().unwrap_or_default(),
                            allow: kind == 0,
                            mask: a.Mask,
                        });
                        let _ = LocalFree(Some(HLOCAL(s.0.cast())));
                    }
                }
                Some(aces)
            }
        };
        let _ = LocalFree(Some(HLOCAL(sd.0)));
        out
    }
}

/// Only `me` and the machine's accounts can change `path`. Unknown counts as unsafe.
#[cfg(windows)]
pub fn folder_is_safe(path: &std::path::Path, me: &str) -> bool {
    folder_aces(path).is_some_and(|aces| !others_can_write(&aces, me))
}

#[cfg(test)]
mod tests {
    use super::*;

    const ME: &str = "S-1-5-21-1-2-3-1001";
    const FULL: u32 = 0x001f_01ff;
    const MODIFY: u32 = 0x0013_01bf;
    const READ_EXECUTE: u32 = 0x0012_00a9;

    fn ace(sid: &str, allow: bool, mask: u32) -> Ace {
        Ace {
            sid: sid.into(),
            allow,
            mask,
        }
    }

    #[test]
    fn everyone_write_is_unsafe() {
        assert!(others_can_write(
            &[ace(ME, true, FULL), ace("S-1-1-0", true, MODIFY)],
            ME
        ));
    }

    #[test]
    fn authenticated_users_modify_is_unsafe() {
        // What a folder made directly under C:\ inherits.
        assert!(others_can_write(
            &[ace(ME, true, FULL), ace("S-1-5-11", true, MODIFY)],
            ME
        ));
    }

    #[test]
    fn owner_system_admins_only_is_safe() {
        let aces = [
            ace(ME, true, FULL),
            ace("S-1-5-18", true, FULL),
            ace("S-1-5-32-544", true, FULL),
            ace("S-1-3-0", true, FULL),
            ace("S-1-5-32-545", true, READ_EXECUTE),
        ];
        assert!(!others_can_write(&aces, ME));
    }

    #[test]
    fn deny_aces_do_not_make_it_unsafe() {
        assert!(!others_can_write(
            &[ace(ME, true, FULL), ace("S-1-1-0", false, FULL)],
            ME
        ));
    }

    #[test]
    fn another_user_with_write_is_unsafe() {
        assert!(others_can_write(
            &[ace(ME, true, FULL), ace("S-1-5-21-1-2-3-1002", true, 0x2)],
            ME
        ));
    }
}
