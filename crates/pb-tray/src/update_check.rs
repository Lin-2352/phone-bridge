//! The "Check for updates" row: asks GitHub's release list once, when the button is pressed, and says
//! what it found. Nothing runs in the background and nothing is downloaded or installed; a newer
//! release is only a link to its page. No signature is checked (see `pb_update::releases`).

use pb_update::releases::{CheckError, Release};
use pb_update::Version;
use std::sync::Mutex;

pub const RELEASES_URL: &str =
    "https://api.github.com/repos/Lin-2352/phone-bridge/releases?per_page=10";

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub enum Phase {
    #[default]
    Idle,
    Checking,
    UpToDate,
    Found(String, String),
    Failed(&'static str),
}

static PHASE: Mutex<Phase> = Mutex::new(Phase::Idle);

pub fn phase() -> Phase {
    PHASE.lock().map(|p| p.clone()).unwrap_or_default()
}

fn set(p: Phase) {
    if let Ok(mut g) = PHASE.lock() {
        *g = p;
    }
}

/// The second line of the row and whether the button now opens the release page.
pub fn note(p: &Phase, installed: &str) -> (String, bool) {
    match p {
        Phase::Idle => (
            format!("Relay {installed}. Asks GitHub only when you press Check"),
            false,
        ),
        Phase::Checking => ("Checking\u{2026}".to_owned(), false),
        Phase::UpToDate => (
            format!("No newer release is listed on GitHub (this is {installed})"),
            false,
        ),
        Phase::Found(v, _) => (
            format!(
                "{v} is listed on GitHub. Not checked for authenticity: open the page and compare"
            ),
            true,
        ),
        Phase::Failed(why) => (format!("Couldn't check: {why}"), false),
    }
}

/// The phase a finished fetch leads to. A failure is never "up to date".
pub fn outcome(fetched: Result<Vec<u8>, &'static str>, installed: Version) -> Phase {
    match fetched {
        Err(why) => Phase::Failed(why),
        Ok(bytes) => match pb_update::releases::newest_after(&bytes, installed) {
            Ok(None) => Phase::UpToDate,
            Ok(Some(Release { tag, url, .. })) => Phase::Found(tag, url),
            Err(CheckError::TooLarge) => Phase::Failed("the answer was too large"),
            Err(CheckError::Malformed) => Phase::Failed("GitHub's answer was not a release list"),
        },
    }
}

/// The link to open, when a newer release was found.
pub fn open_url() -> Option<String> {
    match phase() {
        Phase::Found(_, url) => Some(url),
        _ => None,
    }
}

/// Starts a check on a worker thread; `done` runs when it finishes (to redraw). Does nothing while
/// one is already running.
pub fn start(done: impl FnOnce() + Send + 'static) {
    {
        let Ok(mut g) = PHASE.lock() else { return };
        if *g == Phase::Checking {
            return;
        }
        *g = Phase::Checking;
    }
    std::thread::spawn(move || {
        let installed = Version::parse(env!("CARGO_PKG_VERSION")).unwrap_or(Version(0, 0, 0));
        let fetched = crate::update_http::get(RELEASES_URL, pb_update::releases::MAX_RESPONSE + 1);
        set(outcome(fetched, installed));
        done();
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    const V3: Version = Version(0, 3, 0);

    #[test]
    fn a_failed_fetch_is_never_up_to_date() {
        let p = outcome(Err("no connection"), V3);
        assert_eq!(p, Phase::Failed("no connection"));
        assert!(note(&p, "0.3.0").0.starts_with("Couldn't check"));
    }

    #[test]
    fn an_html_page_is_a_failure() {
        assert!(matches!(
            outcome(Ok(b"<html>".to_vec()), V3),
            Phase::Failed(_)
        ));
    }

    #[test]
    fn an_empty_list_is_up_to_date() {
        assert_eq!(outcome(Ok(b"[]".to_vec()), V3), Phase::UpToDate);
    }

    #[test]
    fn a_newer_release_turns_the_button_into_open_page() {
        let j = br#"[{"tag_name":"v0.4.0","prerelease":true,"draft":false,"html_url":"https://github.com/Lin-2352/phone-bridge/releases/tag/v0.4.0"}]"#;
        let p = outcome(Ok(j.to_vec()), V3);
        let (text, open) = note(&p, "0.3.0");
        assert!(open);
        assert!(text.contains("v0.4.0"));
        let low = text.to_lowercase();
        assert!(!low.contains("verified") && !low.contains("safe"));
    }

    #[test]
    fn nothing_else_offers_the_page() {
        for p in [
            Phase::Idle,
            Phase::Checking,
            Phase::UpToDate,
            Phase::Failed("x"),
        ] {
            assert!(!note(&p, "0.3.0").1);
        }
    }
}
