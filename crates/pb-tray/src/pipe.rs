//! The single-instance pipe's name, one per Windows account.

/// A user SID in string form: `S-1-` and digits and dashes, nothing that
/// could change the meaning of the SDDL (or pipe name) it is put into.
pub fn is_valid_user_sid(sid: &str) -> bool {
    sid.starts_with("S-1-")
        && sid.len() <= 80
        && sid.len() > 4
        && sid[2..].bytes().all(|b| b.is_ascii_digit() || b == b'-')
}

/// `\\.\pipe\PhoneBridgeTray-…` named after the account's SID, so every account has its own.
/// Without a usable SID: the user name's ASCII letters, digits, `-` and `_`, or, when that leaves
/// nothing (a name in another script), the name's UTF-16 in hex, so such names never collide.
pub fn pipe_name_for(sid: Option<&str>, user: &str) -> String {
    let id = match sid.filter(|s| is_valid_user_sid(s)) {
        Some(s) => s.to_string(),
        None => {
            let ascii: String = user
                .chars()
                .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
                .collect();
            if ascii.is_empty() {
                user.encode_utf16().map(|u| format!("{u:04x}")).collect()
            } else {
                ascii
            }
        }
    };
    format!(r"\\.\pipe\PhoneBridgeTray-{id}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_sid_shape() {
        assert!(is_valid_user_sid("S-1-5-21-1-2-3-1001"));
        assert!(!is_valid_user_sid(""));
        assert!(!is_valid_user_sid("S-1-"));
        assert!(!is_valid_user_sid("S-1-5-21)(A;;GA;;;WD"), "SDDL injection");
        assert!(!is_valid_user_sid("Administrators"));
        assert!(!is_valid_user_sid(&format!("S-1-{}", "1".repeat(90))));
    }

    #[test]
    fn sid_names_the_pipe() {
        assert_eq!(
            pipe_name_for(Some("S-1-5-21-1-2-3-1001"), "张伟"),
            r"\\.\pipe\PhoneBridgeTray-S-1-5-21-1-2-3-1001"
        );
    }

    #[test]
    fn invalid_sid_falls_back_to_the_name() {
        assert_eq!(
            pipe_name_for(Some("S-1-5)(A;;GA;;;WD"), "alice"),
            r"\\.\pipe\PhoneBridgeTray-alice"
        );
    }

    #[test]
    fn ascii_name_fallback_unchanged() {
        assert_eq!(
            pipe_name_for(None, "Jo Smith.x"),
            r"\\.\pipe\PhoneBridgeTray-JoSmithx"
        );
    }

    #[test]
    fn cjk_names_without_sid_do_not_collide() {
        let a = pipe_name_for(None, "张伟");
        let b = pipe_name_for(None, "李娜");
        assert_ne!(a, b);
        assert_ne!(a, r"\\.\pipe\PhoneBridgeTray-");
    }
}
