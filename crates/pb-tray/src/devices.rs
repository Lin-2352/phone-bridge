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
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, self.to_text())?;
        std::fs::rename(tmp, path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: &str = "aa00000000000000000000000000000000000000000000000000000000000001";
    const B: &str = "bb00000000000000000000000000000000000000000000000000000000000002";

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
