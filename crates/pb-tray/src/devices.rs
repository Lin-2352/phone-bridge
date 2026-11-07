//! What this PC remembers per paired device, in `devices.txt` next to the settings. Kept apart
//! from `peers.txt`: an older Relay drops lines with extra columns there, which would lose the pairing.
//!
//! Format: `version=1`, then `<peer id hex>\t<key>=<value>` lines.

use std::collections::BTreeMap;
use std::path::Path;

pub const VERSION: u32 = 1;
const MAX_DEVICES: usize = 64;
const MAX_KEYS: usize = 32;
const MAX_VALUE: usize = 256;
const ID_LEN: usize = 64;
/// The longest nickname kept.
pub const MAX_NICKNAME: usize = 64;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct DeviceStore {
    devices: BTreeMap<String, BTreeMap<String, String>>,
}

fn valid_id(id: &str) -> bool {
    id.len() == ID_LEN && id.bytes().all(|b| b.is_ascii_hexdigit())
}

fn valid_key(k: &str) -> bool {
    !k.is_empty()
        && k.len() <= 32
        && k.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

fn clean(v: &str) -> String {
    v.chars()
        .filter(|c| !c.is_control())
        .take(MAX_VALUE)
        .collect()
}

/// A nickname is a local label: control characters dropped, trimmed, cut to [`MAX_NICKNAME`] characters.
pub fn clean_nickname(raw: &str) -> Option<String> {
    let s: String = raw.chars().filter(|c| !c.is_control()).collect();
    let s: String = s.trim().chars().take(MAX_NICKNAME).collect();
    (!s.is_empty()).then_some(s)
}

/// The default device after one was unpaired: the current default stays unless it is the one removed, then the
/// first remaining device by name ignoring case, or empty when none is left. `remaining` is (id, name).
#[cfg_attr(not(test), allow(dead_code))] // used by the unpair path
pub fn default_after_removal(
    remaining: &[(String, String)],
    removed: &str,
    current: &str,
) -> String {
    if !current.is_empty() && current != removed && remaining.iter().any(|(id, _)| id == current) {
        return current.to_owned();
    }
    remaining
        .iter()
        .min_by_key(|(_, name)| name.to_lowercase())
        .map(|(id, _)| id.clone())
        .unwrap_or_default()
}

/// Position of the default device among `ids`: the saved one while it is still there, otherwise the first.
pub fn default_position(ids: &[String], saved: &str) -> Option<usize> {
    ids.iter()
        .position(|i| i == saved)
        .or(if ids.is_empty() { None } else { Some(0) })
}

/// Where a send goes: straight to one phone, to a choice among several, or nowhere because none is paired.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SendTarget {
    One(String),
    /// Several phones: ids ordered default first, then by name ignoring case.
    Ask(Vec<String>),
    None,
}

/// Picks the target of a send from the paired phones (id, name). An explicit id that is still paired wins; one
/// phone needs no question; several ask, with the default first; an explicit id that is no longer paired is ignored.
#[cfg_attr(not(test), allow(dead_code))] // used by the send entry points
pub fn send_target(
    peers: &[(String, String)],
    explicit: Option<&str>,
    default: &str,
) -> SendTarget {
    if let Some(e) = explicit {
        if peers.iter().any(|(id, _)| id == e) {
            return SendTarget::One(e.to_owned());
        }
    }
    match peers {
        [] => SendTarget::None,
        [(id, _)] => SendTarget::One(id.clone()),
        _ => {
            let mut rows: Vec<&(String, String)> = peers.iter().collect();
            rows.sort_by_key(|(id, name)| (id != default, name.to_lowercase()));
            SendTarget::Ask(rows.into_iter().map(|(id, _)| id.clone()).collect())
        }
    }
}

impl DeviceStore {
    /// Reads the text; bad lines and anything over the limits are skipped.
    pub fn parse(text: &str) -> Self {
        let mut s = Self::default();
        for line in text.lines() {
            let Some((id, kv)) = line.split_once('\t') else {
                continue;
            };
            let Some((k, v)) = kv.split_once('=') else {
                continue;
            };
            s.set(&id.to_ascii_lowercase(), k, v);
        }
        s
    }

    pub fn to_text(&self) -> String {
        let mut t = format!("version={VERSION}\n");
        for (id, kv) in &self.devices {
            for (k, v) in kv {
                t += &format!("{id}\t{k}={v}\n");
            }
        }
        t
    }

    #[cfg_attr(not(test), allow(dead_code))] // read by the device list from M8
    pub fn get(&self, id: &str, key: &str) -> Option<&str> {
        self.devices.get(id)?.get(key).map(String::as_str)
    }

    /// True when the stored value changed. Out-of-limit input changes nothing.
    pub fn set(&mut self, id: &str, key: &str, value: &str) -> bool {
        if !valid_id(id) || !valid_key(key) {
            return false;
        }
        let value = clean(value);
        if !self.devices.contains_key(id) && self.devices.len() >= MAX_DEVICES {
            return false;
        }
        let kv = self.devices.entry(id.to_owned()).or_default();
        if !kv.contains_key(key) && kv.len() >= MAX_KEYS {
            return false;
        }
        kv.insert(key.to_owned(), value.clone()).as_deref() != Some(value.as_str())
    }

    /// The local nickname of a device, if it has one. Never sent to the device.
    #[cfg_attr(not(test), allow(dead_code))] // read by the device list
    pub fn nickname(&self, id: &str) -> Option<String> {
        clean_nickname(self.get(id, "nickname")?)
    }

    /// Sets (or, when blank, clears) the nickname. True when the stored value changed.
    #[cfg_attr(not(test), allow(dead_code))] // called by the rename action
    pub fn set_nickname(&mut self, id: &str, raw: &str) -> bool {
        match clean_nickname(raw) {
            Some(n) => self.set(id, "nickname", &n),
            None => self
                .devices
                .get_mut(id)
                .is_some_and(|kv| kv.remove("nickname").is_some()),
        }
    }

    /// Forgets a device (it was unpaired).
    pub fn forget(&mut self, id: &str) -> bool {
        self.devices.remove(id).is_some()
    }

    pub fn load(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .map(|t| Self::parse(&t))
            .unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        use std::io::Write;
        let tmp = path.with_extension("tmp");
        // Flush to disk before the rename, so a power cut leaves the old file or the new one, never half.
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(self.to_text().as_bytes())?;
        f.sync_all()?;
        drop(f);
        std::fs::rename(tmp, path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: &str = "aa00000000000000000000000000000000000000000000000000000000000001";
    const B: &str = "bb00000000000000000000000000000000000000000000000000000000000002";
    const C: &str = "cc00000000000000000000000000000000000000000000000000000000000003";

    #[test]
    fn values_round_trip() {
        let mut s = DeviceStore::default();
        assert!(s.set(A, "app_version", "0.3.0"));
        assert!(!s.set(A, "app_version", "0.3.0"), "same value: no change");
        assert!(s.set(B, "app_version", "0.2.0"));
        let back = DeviceStore::parse(&s.to_text());
        assert_eq!(back, s);
        assert_eq!(back.get(A, "app_version"), Some("0.3.0"));
    }

    #[test]
    fn bad_lines_and_ids_are_skipped() {
        let t = format!("version=1\njunk\nshort\tk=v\n{A}\tBAD KEY=v\n{A}\tok=1\n{A}\tno equals\n");
        let s = DeviceStore::parse(&t);
        assert_eq!(s.get(A, "ok"), Some("1"));
        assert_eq!(s.to_text().lines().count(), 2); // version + one value
    }

    #[test]
    fn limits_hold() {
        let mut s = DeviceStore::default();
        for i in 0..70u32 {
            s.set(&format!("{i:064x}"), "k", "v");
        }
        assert_eq!(s.devices.len(), MAX_DEVICES);
        let mut s = DeviceStore::default();
        for i in 0..40 {
            s.set(A, &format!("key{i}"), "v");
        }
        assert_eq!(s.devices[A].len(), MAX_KEYS);
        assert!(s.set(A, "key0", &"x".repeat(1000)));
        assert_eq!(s.get(A, "key0").unwrap().len(), MAX_VALUE);
    }

    #[test]
    fn unpairing_forgets_the_device() {
        let mut s = DeviceStore::default();
        s.set(A, "app_version", "0.3.0");
        assert!(s.forget(A));
        assert!(!s.forget(A));
        assert_eq!(s.get(A, "app_version"), None);
    }

    #[test]
    fn nickname_is_cleaned_and_capped() {
        let mut s = DeviceStore::default();
        assert!(s.set_nickname(A, "  Desk\u{7} PC  "));
        assert_eq!(s.nickname(A).as_deref(), Some("Desk PC"));
        assert!(s.set_nickname(A, &"x".repeat(200)));
        assert_eq!(s.nickname(A).unwrap().chars().count(), MAX_NICKNAME);
        assert!(s.set_nickname(A, "   "), "a blank nickname clears it");
        assert_eq!(s.nickname(A), None);
        assert!(!s.set_nickname(A, ""), "already clear: no change");
    }

    #[test]
    fn save_survives_a_crash_between_write_and_rename() {
        let dir = std::env::temp_dir().join(format!("pb-dev-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("devices.txt");
        let mut s = DeviceStore::default();
        s.set(A, "app_version", "0.3.0");
        s.save(&path).unwrap();
        // A crash after the temp file was written but before the rename leaves a stray temp file.
        std::fs::write(path.with_extension("tmp"), "version=1\ngarbage").unwrap();
        assert_eq!(DeviceStore::load(&path), s);
        s.set(A, "app_version", "0.4.0");
        s.save(&path).unwrap();
        assert_eq!(
            DeviceStore::load(&path).get(A, "app_version"),
            Some("0.4.0")
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn default_is_dropped_when_its_device_is_unpaired() {
        let rest = vec![
            (B.to_owned(), "bravo".to_owned()),
            (C.to_owned(), "Alpha".to_owned()),
        ];
        assert_eq!(
            default_after_removal(&rest, A, A),
            C,
            "first by name, ignoring case"
        );
        assert_eq!(
            default_after_removal(&rest, A, B),
            B,
            "another device was removed: keep"
        );
        assert_eq!(
            default_after_removal(&[], A, A),
            "",
            "no device left: no default"
        );
    }

    fn two() -> Vec<(String, String)> {
        vec![
            (B.to_owned(), "bravo".to_owned()),
            (C.to_owned(), "Alpha".to_owned()),
        ]
    }

    #[test]
    fn one_phone_needs_no_picker() {
        let one = vec![(B.to_owned(), "bravo".to_owned())];
        assert_eq!(send_target(&one, None, ""), SendTarget::One(B.to_owned()));
    }

    #[test]
    fn several_phones_ask_with_the_default_first() {
        assert_eq!(
            send_target(&two(), None, B),
            SendTarget::Ask(vec![B.to_owned(), C.to_owned()])
        );
        assert_eq!(
            send_target(&two(), None, ""),
            SendTarget::Ask(vec![C.to_owned(), B.to_owned()]),
            "no default: by name"
        );
    }

    #[test]
    fn an_explicit_phone_skips_the_picker() {
        assert_eq!(
            send_target(&two(), Some(C), B),
            SendTarget::One(C.to_owned())
        );
    }

    #[test]
    fn an_unpaired_explicit_phone_falls_back_to_asking() {
        assert_eq!(
            send_target(&two(), Some(A), ""),
            SendTarget::Ask(vec![C.to_owned(), B.to_owned()])
        );
    }

    #[test]
    fn default_position_follows_the_saved_id_or_the_first() {
        let ids = vec![B.to_owned(), C.to_owned()];
        assert_eq!(default_position(&ids, C), Some(1));
        assert_eq!(
            default_position(&ids, A),
            Some(0),
            "saved device is gone: the first"
        );
        assert_eq!(default_position(&ids, ""), Some(0));
        assert_eq!(default_position(&[], C), None);
    }

    #[test]
    fn no_phone_means_pair_first() {
        assert_eq!(send_target(&[], Some(A), A), SendTarget::None);
    }

    #[test]
    fn random_text_never_panics() {
        let alphabet: Vec<char> = "ab0\t=\n k_\r\u{0}\u{e9}".chars().collect();
        let mut x: u64 = 0x9e37_79b9_7f4a_7c15;
        for _ in 0..3000 {
            let mut t = String::new();
            for _ in 0..80 {
                x = x
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                t.push(alphabet[(x >> 33) as usize % alphabet.len()]);
            }
            let s = DeviceStore::parse(&t);
            assert_eq!(DeviceStore::parse(&s.to_text()), s);
        }
    }
}
