//! The plain update check: reads GitHub's release list and says whether a newer version exists.
//!
//! No signature is checked (the owner ruled on 2026-11-09 that Relay holds no update key), so a result
//! only means "GitHub says a newer release exists". It is shown as a link to the release page and
//! nothing is downloaded or installed.

use crate::{Version, RELEASE_URL_PREFIX};

/// The longest response read. Release lists carry notes, so this is larger than the manifest limit.
pub const MAX_RESPONSE: usize = 512 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Release {
    pub version: Version,
    pub tag: String,
    pub url: String,
    pub prerelease: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CheckError {
    /// The answer was longer than `MAX_RESPONSE`.
    TooLarge,
    /// The answer was not a JSON list of releases.
    Malformed,
}

/// The highest release newer than `installed`, or `None` when the installed version is the newest.
/// Drafts, tags that are not `major.minor.patch` and links outside this project's release pages are
/// ignored. Pre-releases count: every release so far is one.
pub fn newest_after(json: &[u8], installed: Version) -> Result<Option<Release>, CheckError> {
    if json.len() > MAX_RESPONSE {
        return Err(CheckError::TooLarge);
    }
    let list: serde_json::Value =
        serde_json::from_slice(json).map_err(|_| CheckError::Malformed)?;
    let items = list.as_array().ok_or(CheckError::Malformed)?;
    let mut best: Option<Release> = None;
    for item in items {
        if item["draft"].as_bool().unwrap_or(false) {
            continue;
        }
        let (Some(tag), Some(url)) = (item["tag_name"].as_str(), item["html_url"].as_str()) else {
            continue;
        };
        let Some(version) = Version::parse(tag.strip_prefix('v').unwrap_or(tag)) else {
            continue;
        };
        if version <= installed || !url.starts_with(RELEASE_URL_PREFIX) {
            continue;
        }
        if best.as_ref().is_none_or(|b| version > b.version) {
            best = Some(Release {
                version,
                tag: tag.to_string(),
                url: url.to_string(),
                prerelease: item["prerelease"].as_bool().unwrap_or(false),
            });
        }
    }
    Ok(best)
}

#[cfg(test)]
mod tests {
    use super::*;

    const V3: Version = Version(0, 3, 0);

    fn rel(tag: &str, pre: bool, draft: bool) -> String {
        format!(
            r#"{{"tag_name":"{tag}","prerelease":{pre},"draft":{draft},"html_url":"{RELEASE_URL_PREFIX}tag/{tag}"}}"#
        )
    }
    fn list(items: &[String]) -> Vec<u8> {
        format!("[{}]", items.join(",")).into_bytes()
    }

    #[test]
    fn offers_a_newer_release() {
        let j = list(&[rel("v0.4.0", false, false), rel("v0.2.0", true, false)]);
        let r = newest_after(&j, V3).unwrap().unwrap();
        assert_eq!(r.version, Version(0, 4, 0));
        assert_eq!(r.url, format!("{RELEASE_URL_PREFIX}tag/v0.4.0"));
    }

    #[test]
    fn never_offers_the_installed_version_or_older() {
        let j = list(&[rel("v0.3.0", false, false), rel("v0.2.0", true, false)]);
        assert_eq!(newest_after(&j, V3).unwrap(), None);
    }

    #[test]
    fn prereleases_count_while_every_release_is_one() {
        let j = list(&[rel("v0.3.1", true, false)]);
        assert!(newest_after(&j, V3).unwrap().unwrap().prerelease);
    }

    #[test]
    fn the_highest_wins_whatever_the_order() {
        let j = list(&[
            rel("v0.3.5", false, false),
            rel("v0.10.0", false, false),
            rel("v0.4.0", false, false),
        ]);
        assert_eq!(
            newest_after(&j, V3).unwrap().unwrap().version,
            Version(0, 10, 0)
        );
    }

    #[test]
    fn drafts_and_odd_tags_are_ignored() {
        let j = list(&[
            rel("v0.9.0", false, true),
            rel("v0.8.0-rc1", true, false),
            rel("nightly", true, false),
        ]);
        assert_eq!(newest_after(&j, V3).unwrap(), None);
    }

    #[test]
    fn a_link_to_another_site_is_ignored() {
        let j = br#"[{"tag_name":"v0.9.0","prerelease":false,"draft":false,"html_url":"https://evil.example/x"}]"#;
        assert_eq!(newest_after(j, V3).unwrap(), None);
    }

    #[test]
    fn garbage_is_an_error_not_up_to_date() {
        assert_eq!(
            newest_after(b"<html>rate limited</html>", V3),
            Err(CheckError::Malformed)
        );
        assert_eq!(
            newest_after(br#"{"message":"API rate limit exceeded"}"#, V3),
            Err(CheckError::Malformed)
        );
        assert_eq!(newest_after(b"", V3), Err(CheckError::Malformed));
    }

    #[test]
    fn an_oversized_answer_is_refused() {
        let big = vec![b' '; MAX_RESPONSE + 1];
        assert_eq!(newest_after(&big, V3), Err(CheckError::TooLarge));
    }

    #[test]
    fn an_empty_list_is_up_to_date() {
        assert_eq!(newest_after(b"[]", V3).unwrap(), None);
    }
}
