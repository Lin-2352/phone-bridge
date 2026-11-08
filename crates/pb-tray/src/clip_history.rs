//! Clipboard history on the PC: opt-in, encrypted at rest, seven days, searched in memory.
//!
//! The file `clip_history.bin` is a run of `[u32 length][sealed record]`. A record is sealed by
//! the [`Seal`] (DPAPI for the current user in the product; DPAPI authenticates the whole blob,
//! so the stored time cannot be edited to dodge the expiry). Plain data and rules only: the seal
//! is a trait, so the tests need no Windows. A clip marked sensitive or recognised as a one-time
//! code is never stored. A file that cannot be opened (the key is gone) reads as empty and is
//! deleted: never garbage, never a crash.
#![allow(dead_code)]

use std::{
    io,
    path::{Path, PathBuf},
};

/// How long a clip is kept.
pub const KEEP_SECS: u64 = 7 * 24 * 3600;
/// Largest clip kept (bytes of text); longer ones are skipped, not cut.
pub const MAX_CLIP: usize = 64 * 1024;
/// Most clips kept; the oldest go first.
pub const MAX_ENTRIES: usize = 200;

/// How far ahead of the clock a stored time may be before the clip counts as expired.
const MAX_SKEW_SECS: u64 = 24 * 3600;

const VERSION: u8 = 1;

/// Seals and opens one record.
pub trait Seal {
    fn seal(&self, plain: &[u8]) -> io::Result<Vec<u8>>;
    fn open(&self, sealed: &[u8]) -> io::Result<Vec<u8>>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub created: u64,
    pub text: String,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Stored {
    Yes,
    Off,
    Sensitive,
    OneTimeCode,
    TooLong,
    Empty,
}

pub struct ClipHistory<S: Seal> {
    path: PathBuf,
    seal: S,
    enabled: bool,
}

/// A short run of digits (optionally grouped by spaces or dashes) is a one-time code.
pub fn is_one_time_code(text: &str) -> bool {
    let t = text.trim();
    let digits = t.chars().filter(|c| c.is_ascii_digit()).count();
    (4..=10).contains(&digits)
        && t.chars()
            .all(|c| c.is_ascii_digit() || c == ' ' || c == '-')
}

impl<S: Seal> ClipHistory<S> {
    /// `enabled` is the user's `keep clipboard history` switch (off by default).
    pub fn new(dir: &Path, seal: S, enabled: bool) -> Self {
        Self {
            path: dir.join("clip_history.bin"),
            seal,
            enabled,
        }
    }

    /// Turning it off deletes the history.
    pub fn set_enabled(&mut self, on: bool) {
        self.enabled = on;
        if !on {
            let _ = std::fs::remove_file(&self.path);
        }
    }

    pub fn add(&self, text: &str, sensitive: bool, now: u64) -> io::Result<Stored> {
        if !self.enabled {
            return Ok(Stored::Off);
        }
        if sensitive {
            return Ok(Stored::Sensitive);
        }
        if text.trim().is_empty() {
            return Ok(Stored::Empty);
        }
        if text.len() > MAX_CLIP {
            return Ok(Stored::TooLong);
        }
        if is_one_time_code(text) {
            return Ok(Stored::OneTimeCode);
        }
        let mut entries = self.entries(now);
        entries.push(Entry {
            created: now,
            text: text.to_owned(),
        });
        if entries.len() > MAX_ENTRIES {
            let drop = entries.len() - MAX_ENTRIES;
            entries.drain(..drop);
        }
        self.write(&entries)?;
        Ok(Stored::Yes)
    }

    fn write(&self, entries: &[Entry]) -> io::Result<()> {
        let mut out = Vec::new();
        for e in entries {
            let mut rec = vec![VERSION];
            rec.extend_from_slice(&e.created.to_le_bytes());
            rec.extend_from_slice(e.text.as_bytes());
            let sealed = self.seal.seal(&rec)?;
            out.extend_from_slice(&(sealed.len() as u32).to_le_bytes());
            out.extend_from_slice(&sealed);
        }
        let tmp = self.path.with_extension("tmp");
        std::fs::write(&tmp, out)?;
        std::fs::rename(&tmp, &self.path)
    }

    /// The kept clips, oldest first. Expired and unreadable records are dropped; a file where
    /// no record opens (the key is lost) is deleted and reads as empty.
    pub fn entries(&self, now: u64) -> Vec<Entry> {
        if !self.enabled {
            return Vec::new();
        }
        let Ok(bytes) = std::fs::read(&self.path) else {
            return Vec::new();
        };
        let (mut out, mut total, mut opened) = (Vec::new(), 0usize, 0usize);
        let mut i = 0usize;
        while i + 4 <= bytes.len() {
            let n = u32::from_le_bytes(bytes[i..i + 4].try_into().unwrap()) as usize;
            i += 4;
            if n > bytes.len() - i {
                break;
            }
            total += 1;
            if let Some(e) = self.read_record(&bytes[i..i + n]) {
                opened += 1;
                // A clip dated in the future (the clock was set back) would never expire: it is dropped.
                if e.created <= now.saturating_add(MAX_SKEW_SECS)
                    && now.saturating_sub(e.created) < KEEP_SECS
                {
                    out.push(e);
                }
            }
            i += n;
        }
        if total > 0 && opened == 0 {
            let _ = std::fs::remove_file(&self.path);
        }
        out
    }

    fn read_record(&self, sealed: &[u8]) -> Option<Entry> {
        let plain = self.seal.open(sealed).ok()?;
        if plain.len() < 9 || plain[0] != VERSION {
            return None;
        }
        Some(Entry {
            created: u64::from_le_bytes(plain[1..9].try_into().ok()?),
            text: String::from_utf8(plain[9..].to_vec()).ok()?,
        })
    }

    /// Case-insensitive substring search, decrypted in memory only; newest first.
    pub fn search(&self, query: &str, now: u64) -> Vec<Entry> {
        let q = query.to_lowercase();
        let mut hits: Vec<Entry> = self
            .entries(now)
            .into_iter()
            .filter(|e| q.is_empty() || e.text.to_lowercase().contains(&q))
            .collect();
        hits.reverse();
        hits
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A software seal for the tests: reversible, and it refuses what another key sealed.
    struct Fake(u8);
    impl Seal for Fake {
        fn seal(&self, p: &[u8]) -> io::Result<Vec<u8>> {
            let mut v = vec![self.0];
            v.extend(p.iter().map(|b| b ^ self.0));
            Ok(v)
        }
        fn open(&self, s: &[u8]) -> io::Result<Vec<u8>> {
            match s.split_first() {
                Some((k, rest)) if *k == self.0 => Ok(rest.iter().map(|b| b ^ self.0).collect()),
                _ => Err(io::Error::from(io::ErrorKind::InvalidData)),
            }
        }
    }

    fn dir(n: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("pb-cliphist-{n}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn off_by_default_stores_nothing() {
        let d = dir("off");
        let h = ClipHistory::new(&d, Fake(7), false);
        assert_eq!(h.add("hello", false, 100).unwrap(), Stored::Off);
        assert!(!d.join("clip_history.bin").exists());
    }

    #[test]
    fn sensitive_clip_is_never_stored() {
        let d = dir("sens");
        let h = ClipHistory::new(&d, Fake(7), true);
        assert_eq!(h.add("hunter2", true, 100).unwrap(), Stored::Sensitive);
        assert!(h.entries(100).is_empty());
    }

    #[test]
    fn one_time_code_is_never_stored() {
        let d = dir("otp");
        let h = ClipHistory::new(&d, Fake(7), true);
        for code in ["123456", "123 456", "12-34-56", " 4821 "] {
            assert_eq!(
                h.add(code, false, 100).unwrap(),
                Stored::OneTimeCode,
                "{code}"
            );
        }
        assert_eq!(
            h.add("call me at 555 0100 tomorrow", false, 100).unwrap(),
            Stored::Yes
        );
        assert_eq!(h.entries(100).len(), 1);
    }

    #[test]
    fn stored_text_is_not_readable_in_the_file() {
        let d = dir("enc");
        let h = ClipHistory::new(&d, Fake(0x5a), true);
        h.add("the quick brown fox", false, 100).unwrap();
        let raw = std::fs::read(d.join("clip_history.bin")).unwrap();
        assert!(!raw.windows(5).any(|w| w == b"quick"));
    }

    #[test]
    fn entries_older_than_7_days_are_gone() {
        let d = dir("exp");
        let h = ClipHistory::new(&d, Fake(7), true);
        h.add("old", false, 1_000).unwrap();
        h.add("new", false, 1_000 + KEEP_SECS - 1).unwrap();
        let now = 1_000 + KEEP_SECS;
        let texts: Vec<_> = h.entries(now).into_iter().map(|e| e.text).collect();
        assert_eq!(texts, vec!["new"]);
    }

    #[test]
    fn a_clip_dated_in_the_future_does_not_live_forever() {
        let d = dir("future");
        let h = ClipHistory::new(&d, Fake(7), true);
        h.add("from a wrong clock", false, 10 * KEEP_SECS).unwrap();
        // The clock is corrected: that clip is a week and more ahead, so it is gone, not kept until then.
        assert!(h.entries(100).is_empty());
        h.add("now", false, 100).unwrap();
        assert_eq!(h.entries(100).len(), 1);
    }

    #[test]
    fn wrong_key_fails_closed_and_reads_as_empty_not_garbage() {
        let d = dir("key");
        ClipHistory::new(&d, Fake(1), true)
            .add("secret plans", false, 100)
            .unwrap();
        let h = ClipHistory::new(&d, Fake(2), true);
        assert!(h.entries(100).is_empty());
        assert!(
            !d.join("clip_history.bin").exists(),
            "an unreadable file is deleted"
        );
    }

    #[test]
    fn a_corrupt_tail_keeps_the_readable_records() {
        let d = dir("tail");
        let h = ClipHistory::new(&d, Fake(7), true);
        h.add("kept", false, 100).unwrap();
        let mut raw = std::fs::read(d.join("clip_history.bin")).unwrap();
        raw.extend_from_slice(&[9, 0, 0, 0, 1, 2]);
        std::fs::write(d.join("clip_history.bin"), raw).unwrap();
        assert_eq!(h.entries(100)[0].text, "kept");
    }

    #[test]
    fn search_matches_case_insensitively_newest_first() {
        let d = dir("search");
        let h = ClipHistory::new(&d, Fake(7), true);
        h.add("Invoice 2024 from ACME", false, 100).unwrap();
        h.add("lunch plan", false, 200).unwrap();
        h.add("acme contract draft", false, 300).unwrap();
        let hits: Vec<_> = h.search("AcMe", 300).into_iter().map(|e| e.text).collect();
        assert_eq!(hits, vec!["acme contract draft", "Invoice 2024 from ACME"]);
        assert_eq!(h.search("", 300).len(), 3);
    }

    #[test]
    fn turning_it_off_wipes_the_history() {
        let d = dir("wipe");
        let mut h = ClipHistory::new(&d, Fake(7), true);
        h.add("something", false, 100).unwrap();
        h.set_enabled(false);
        assert!(!d.join("clip_history.bin").exists());
        assert!(h.entries(100).is_empty());
    }

    #[test]
    fn size_and_count_are_capped() {
        let d = dir("cap");
        let h = ClipHistory::new(&d, Fake(7), true);
        assert_eq!(
            h.add(&"x".repeat(MAX_CLIP + 1), false, 100).unwrap(),
            Stored::TooLong
        );
        assert_eq!(h.add("   ", false, 100).unwrap(), Stored::Empty);
        for i in 0..MAX_ENTRIES + 5 {
            h.add(&format!("clip number {i}"), false, 100 + i as u64)
                .unwrap();
        }
        let e = h.entries(400);
        assert_eq!(e.len(), MAX_ENTRIES);
        assert_eq!(e[0].text, "clip number 5", "the oldest went first");
    }
}
