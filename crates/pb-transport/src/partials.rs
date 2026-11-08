//! Kept partial files for resumable transfers (M9.2).
//!
//! A receiver that loses the connection mid-file keeps what it has, so the next attempt continues from there.
//! Everything here is local state: a partial is found only by the same peer, the same token, the same name and
//! the same size; it lives for at most 24 hours; and the total kept is capped per peer, overall, and against
//! the free space on the disk. File names on disk are built from the peer and the token, never from the name
//! the peer sent.

use std::path::{Path, PathBuf};

use pb_identity::PeerId;

/// The first byte after which a kept partial counts as stale: 24 hours.
pub const KEEP_SECS: u64 = 24 * 60 * 60;
/// Most bytes kept for one peer.
pub const PER_PEER_CAP: u64 = 2 * 1024 * 1024 * 1024;
/// Most bytes kept in all.
pub const TOTAL_CAP: u64 = 8 * 1024 * 1024 * 1024;
/// Free space that must remain on the disk after what is kept.
pub const KEEP_FREE_BYTES: u64 = 256 * 1024 * 1024;

/// Most partials kept for one peer, whatever their sizes (empty offers cost nothing against the byte caps).
pub const MAX_ENTRIES_PER_PEER: usize = 64;

const INDEX: &str = "resume.txt";

/// The limits, as values so tests can use small ones.
#[derive(Clone, Copy, Debug)]
pub struct Caps {
    pub keep_secs: u64,
    pub per_peer: u64,
    pub total: u64,
    pub keep_free: u64,
}

impl Default for Caps {
    fn default() -> Self {
        Self {
            keep_secs: KEEP_SECS,
            per_peer: PER_PEER_CAP,
            total: TOTAL_CAP,
            keep_free: KEEP_FREE_BYTES,
        }
    }
}

/// Why a partial could not be kept (the transfer then simply is not resumable).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeepRefusal {
    /// Over the per-peer cap on its own.
    TooBig,
    /// Not enough free space left for it.
    NoSpace,
    Io,
}

/// A kept partial: where it is and how many bytes it holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Claim {
    pub path: PathBuf,
    pub len: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Entry {
    peer: PeerId,
    token: [u8; 16],
    name: String,
    size: u64,
    created: u64,
}

impl Entry {
    fn file_name(&self) -> String {
        file_name(&self.peer, &self.token)
    }
}

fn file_name(peer: &PeerId, token: &[u8; 16]) -> String {
    format!("{}-{}.part", &peer.to_string()[..8], hex(token))
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn unhex_token(s: &str) -> Option<[u8; 16]> {
    if s.len() != 32 || !s.is_ascii() {
        return None;
    }
    let mut out = [0u8; 16];
    for (i, o) in out.iter_mut().enumerate() {
        *o = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(out)
}

/// A display name with no tab or line break, so one index line is exactly one entry.
fn clean(name: &str) -> String {
    name.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .take(255)
        .collect()
}

pub struct PartialStore {
    dir: PathBuf,
    caps: Caps,
    entries: Vec<Entry>,
    /// Partials a receive is using now: never claimed twice, evicted or swept.
    locked: std::collections::HashSet<(PeerId, [u8; 16])>,
}

impl PartialStore {
    /// Marks the partial as in use by a running receive; false when another receive already has it.
    pub fn lock(&mut self, peer: PeerId, token: [u8; 16]) -> bool {
        self.locked.insert((peer, token))
    }

    /// The receive ended but the partial is kept for a later attempt.
    pub fn unlock(&mut self, peer: PeerId, token: [u8; 16]) {
        self.locked.remove(&(peer, token));
    }

    fn is_locked(&self, e: &Entry) -> bool {
        self.locked.contains(&(e.peer, e.token))
    }

    /// The folder the partial files live in (its disk is the one whose free space matters).
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Opens (creating the folder) and loads the index; entries whose file is gone are dropped.
    pub fn open(dir: &Path, now: u64) -> std::io::Result<Self> {
        Self::open_with(dir, now, Caps::default())
    }

    pub fn open_with(dir: &Path, now: u64, caps: Caps) -> std::io::Result<Self> {
        std::fs::create_dir_all(dir)?;
        let mut entries = Vec::new();
        if let Ok(text) = std::fs::read_to_string(dir.join(INDEX)) {
            for line in text.lines() {
                let f: Vec<&str> = line.split('\t').collect();
                if f.len() != 5 {
                    continue;
                }
                let (Ok(peer), Some(token), Ok(size), Ok(created)) = (
                    PeerId::from_hex(f[0]),
                    unhex_token(f[1]),
                    f[2].parse::<u64>(),
                    f[3].parse::<u64>(),
                ) else {
                    continue;
                };
                entries.push(Entry {
                    peer,
                    token,
                    name: clean(f[4]),
                    size,
                    created,
                });
            }
        }
        let mut s = Self {
            dir: dir.to_path_buf(),
            caps,
            entries,
            locked: Default::default(),
        };
        s.sweep(now, u64::MAX);
        Ok(s)
    }

    fn path_of(&self, e: &Entry) -> PathBuf {
        self.dir.join(e.file_name())
    }

    fn len_of(&self, e: &Entry) -> Option<u64> {
        std::fs::metadata(self.path_of(e)).ok().map(|m| m.len())
    }

    fn save(&self) {
        let mut out = String::new();
        for e in &self.entries {
            out.push_str(&format!(
                "{}\t{}\t{}\t{}\t{}\n",
                e.peer,
                hex(&e.token),
                e.size,
                e.created,
                e.name
            ));
        }
        let tmp = self.dir.join(format!("{INDEX}.tmp"));
        if std::fs::write(&tmp, out).is_ok() {
            let _ = std::fs::rename(&tmp, self.dir.join(INDEX));
        }
    }

    fn remove_at(&mut self, i: usize) {
        let e = self.entries.remove(i);
        let _ = std::fs::remove_file(self.path_of(&e));
    }

    /// The kept partial for exactly this peer, token, name and size, with the bytes it holds.
    pub fn claim(&mut self, peer: PeerId, token: [u8; 16], name: &str, size: u64) -> Option<Claim> {
        let name = clean(name);
        let i = self
            .entries
            .iter()
            .position(|e| e.peer == peer && e.token == token && e.name == name && e.size == size)?;
        match self.len_of(&self.entries[i]) {
            Some(len) if len <= size => Some(Claim {
                path: self.path_of(&self.entries[i]),
                len,
            }),
            _ => {
                // Gone or longer than the file: nothing to continue from.
                self.remove_at(i);
                self.save();
                None
            }
        }
    }

    /// Starts a new (empty) partial, evicting this peer's oldest and then the oldest overall while a cap
    /// would be exceeded. `free` is the free space on the disk now.
    pub fn create(
        &mut self,
        peer: PeerId,
        token: [u8; 16],
        name: &str,
        size: u64,
        now: u64,
        free: u64,
    ) -> Result<PathBuf, KeepRefusal> {
        if size > self.caps.per_peer {
            return Err(KeepRefusal::TooBig);
        }
        if free < size.saturating_add(self.caps.keep_free) {
            return Err(KeepRefusal::NoSpace);
        }
        // The same key again starts over.
        if let Some(i) = self
            .entries
            .iter()
            .position(|e| e.peer == peer && e.token == token)
        {
            if self.is_locked(&self.entries[i]) {
                return Err(KeepRefusal::Io);
            }
            self.remove_at(i);
        }
        while self.entries.iter().filter(|e| e.peer == peer).count() >= MAX_ENTRIES_PER_PEER {
            let Some(i) = self.oldest(Some(&peer)) else {
                break;
            };
            self.remove_at(i);
        }
        let held = |s: &Self, p: Option<&PeerId>| -> u64 {
            s.entries
                .iter()
                .filter(|e| p.is_none_or(|p| &e.peer == p))
                .map(|e| e.size)
                .sum()
        };
        while held(self, Some(&peer)).saturating_add(size) > self.caps.per_peer {
            let Some(i) = self.oldest(Some(&peer)) else {
                break;
            };
            self.remove_at(i);
        }
        while held(self, None).saturating_add(size) > self.caps.total {
            let Some(i) = self.oldest(None) else { break };
            self.remove_at(i);
        }
        // What is left may be partials a transfer is writing; those are never evicted, so refuse instead.
        if held(self, Some(&peer)).saturating_add(size) > self.caps.per_peer
            || held(self, None).saturating_add(size) > self.caps.total
        {
            return Err(KeepRefusal::TooBig);
        }
        let e = Entry {
            peer,
            token,
            name: clean(name),
            size,
            created: now,
        };
        let path = self.path_of(&e);
        std::fs::File::create(&path).map_err(|_| KeepRefusal::Io)?;
        self.entries.push(e);
        self.save();
        Ok(path)
    }

    fn oldest(&self, peer: Option<&PeerId>) -> Option<usize> {
        self.entries
            .iter()
            .enumerate()
            .filter(|(_, e)| peer.is_none_or(|p| &e.peer == p))
            .filter(|(_, e)| !self.is_locked(e))
            .min_by_key(|(_, e)| e.created)
            .map(|(i, _)| i)
    }

    /// The transfer ended for good (finished, cancelled, or the content did not match): forget the partial and
    /// delete its file.
    pub fn finish(&mut self, peer: PeerId, token: [u8; 16]) {
        if let Some(i) = self
            .entries
            .iter()
            .position(|e| e.peer == peer && e.token == token)
        {
            self.remove_at(i);
            self.save();
        }
        self.locked.remove(&(peer, token));
    }

    /// Drops what is stale: older than the keep time, missing or oversized files, files in the folder that no
    /// entry owns, and (oldest first) whatever holds the disk below its free-space margin.
    pub fn sweep(&mut self, now: u64, free: u64) {
        let mut i = 0;
        while i < self.entries.len() {
            let e = &self.entries[i];
            let stale = now.saturating_sub(e.created) > self.caps.keep_secs;
            let bad = self.len_of(e).is_none_or(|l| l > e.size);
            if (stale || bad) && !self.is_locked(&self.entries[i]) {
                self.remove_at(i);
            } else {
                i += 1;
            }
        }
        let mut free = free;
        while free < self.caps.keep_free {
            let Some(i) = self.oldest(None) else { break };
            free = free.saturating_add(self.len_of(&self.entries[i]).unwrap_or(0));
            self.remove_at(i);
        }
        if let Ok(rd) = std::fs::read_dir(&self.dir) {
            let owned: std::collections::HashSet<String> =
                self.entries.iter().map(Entry::file_name).collect();
            for f in rd.flatten() {
                let n = f.file_name().to_string_lossy().into_owned();
                if n.ends_with(".part") && !owned.contains(&n) {
                    let _ = std::fs::remove_file(f.path());
                }
            }
        }
        self.save();
    }

    /// Bytes currently kept for `peer` (the files' real lengths).
    pub fn kept_bytes(&self, peer: &PeerId) -> u64 {
        self.entries
            .iter()
            .filter(|e| &e.peer == peer)
            .filter_map(|e| self.len_of(e))
            .sum()
    }
}

#[cfg(test)]
mod tests {
    use pb_identity::Identity;

    use super::*;

    fn peer(n: u8) -> PeerId {
        Identity::from_seed([n; 32]).peer_id()
    }

    fn dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("pb-partials-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    const TOKEN: [u8; 16] = [7; 16];
    const GIB: u64 = 1024 * 1024 * 1024;

    fn small() -> Caps {
        Caps {
            keep_secs: 100,
            per_peer: 1000,
            total: 1500,
            keep_free: 10,
        }
    }

    #[test]
    fn a_partial_in_use_is_locked_and_never_evicted_or_swept() {
        let d = dir("lock");
        let mut s = PartialStore::open_with(&d, 0, small()).unwrap();
        s.create(peer(1), TOKEN, "a.bin", 600, 1, u64::MAX).unwrap();
        assert!(s.lock(peer(1), TOKEN));
        assert!(!s.lock(peer(1), TOKEN), "a second receive cannot share it");
        // A new 600-byte file would push this peer over its cap: the locked one is not the victim.
        assert_eq!(
            s.create(peer(1), [8; 16], "b.bin", 600, 2, u64::MAX),
            Err(KeepRefusal::TooBig)
        );
        // The disk is short on space: sweep may not delete what a transfer is writing.
        s.sweep(3, 0);
        assert!(s.claim(peer(1), TOKEN, "a.bin", 600).is_some());
        // Even stale, a locked partial stays until its transfer ends.
        s.sweep(10_000, u64::MAX);
        assert!(s.claim(peer(1), TOKEN, "a.bin", 600).is_some());
        s.unlock(peer(1), TOKEN);
        assert!(s.lock(peer(1), TOKEN));
        s.finish(peer(1), TOKEN);
        assert!(s.lock(peer(1), TOKEN), "finish releases the lock too");
    }

    #[test]
    fn a_peer_cannot_pile_up_empty_partials() {
        let d = dir("many");
        let mut s = PartialStore::open_with(&d, 0, small()).unwrap();
        for i in 0..200u32 {
            let mut t = [0u8; 16];
            t[..4].copy_from_slice(&i.to_le_bytes());
            s.create(peer(1), t, "z.bin", 0, i as u64, u64::MAX)
                .unwrap();
        }
        let kept = std::fs::read_dir(&d)
            .unwrap()
            .flatten()
            .filter(|f| f.file_name().to_string_lossy().ends_with(".part"))
            .count();
        assert!(kept <= MAX_ENTRIES_PER_PEER, "kept {kept}");
    }

    #[test]
    fn claim_returns_length_for_same_peer_token_name_size() {
        let d = dir("claim");
        let mut s = PartialStore::open_with(&d, 0, small()).unwrap();
        let p = s.create(peer(1), TOKEN, "a.bin", 100, 1, u64::MAX).unwrap();
        std::fs::write(&p, [1u8; 40]).unwrap();
        let c = s.claim(peer(1), TOKEN, "a.bin", 100).unwrap();
        assert_eq!((c.len, c.path), (40, p));
        assert!(
            s.claim(peer(1), TOKEN, "a.bin", 101).is_none(),
            "another size"
        );
        assert!(
            s.claim(peer(1), TOKEN, "b.bin", 100).is_none(),
            "another name"
        );
        assert!(
            s.claim(peer(1), [8; 16], "a.bin", 100).is_none(),
            "another token"
        );
    }

    #[test]
    fn token_of_another_peer_never_matches() {
        let d = dir("peer");
        let mut s = PartialStore::open_with(&d, 0, small()).unwrap();
        s.create(peer(1), TOKEN, "a.bin", 100, 1, u64::MAX).unwrap();
        assert!(s.claim(peer(2), TOKEN, "a.bin", 100).is_none());
    }

    #[test]
    fn partial_names_cannot_escape() {
        let d = dir("escape");
        let mut s = PartialStore::open_with(&d, 0, small()).unwrap();
        for name in ["..\\x", "a/../b", "..", "con\tn\nl", &"x".repeat(300)] {
            let p = s.create(peer(1), TOKEN, name, 10, 1, u64::MAX).unwrap();
            assert_eq!(p.parent().unwrap(), d, "{name:?}");
            assert!(p.file_name().unwrap().to_string_lossy().ends_with(".part"));
        }
        // The index stays one line per entry whatever the name held.
        let text = std::fs::read_to_string(d.join(INDEX)).unwrap();
        assert_eq!(text.lines().count(), 1);
        assert!(text.lines().all(|l| l.split('\t').count() == 5));
    }

    #[test]
    fn expires_after_24_hours() {
        let d = dir("ttl");
        let mut s = PartialStore::open(&d, 0).unwrap();
        let p = s
            .create(peer(1), TOKEN, "a.bin", 100, 1_000, u64::MAX)
            .unwrap();
        s.sweep(1_000 + KEEP_SECS, u64::MAX);
        assert!(p.exists(), "exactly 24 h is still kept");
        s.sweep(1_000 + KEEP_SECS + 1, u64::MAX);
        assert!(!p.exists());
        assert!(s.claim(peer(1), TOKEN, "a.bin", 100).is_none());
    }

    #[test]
    fn per_peer_cap_evicts_oldest() {
        let d = dir("peercap");
        let mut s = PartialStore::open_with(&d, 0, small()).unwrap();
        let first = s.create(peer(1), [1; 16], "a", 600, 1, u64::MAX).unwrap();
        let other = s.create(peer(2), [2; 16], "b", 600, 2, u64::MAX).unwrap();
        let second = s.create(peer(1), [3; 16], "c", 600, 3, u64::MAX).unwrap();
        assert!(!first.exists(), "this peer's oldest made room");
        assert!(
            other.exists(),
            "another peer is untouched by a per-peer cap"
        );
        assert!(second.exists());
    }

    #[test]
    fn total_cap_evicts_oldest_overall() {
        let d = dir("totalcap");
        let mut s = PartialStore::open_with(&d, 0, small()).unwrap();
        let a = s.create(peer(1), [1; 16], "a", 800, 1, u64::MAX).unwrap();
        let b = s.create(peer(2), [2; 16], "b", 800, 2, u64::MAX).unwrap();
        assert!(!a.exists() && b.exists());
    }

    #[test]
    fn files_over_the_cap_are_not_kept() {
        let d = dir("big");
        let mut s = PartialStore::open_with(&d, 0, small()).unwrap();
        assert_eq!(
            s.create(peer(1), TOKEN, "a", 1001, 1, u64::MAX)
                .unwrap_err(),
            KeepRefusal::TooBig
        );
    }

    #[test]
    fn caps_hold_against_free_space() {
        let d = dir("free");
        let mut s = PartialStore::open_with(&d, 0, small()).unwrap();
        // 100 bytes plus the 10-byte margin do not fit in 109 free bytes.
        assert_eq!(
            s.create(peer(1), TOKEN, "a", 100, 1, 109).unwrap_err(),
            KeepRefusal::NoSpace
        );
        assert!(s.create(peer(1), TOKEN, "a", 100, 1, 110).is_ok());
        // A disk that fills up later gives back the oldest partial first, and stops once there is room.
        let a = d.join(file_name(&peer(1), &TOKEN));
        std::fs::write(&a, [0u8; 50]).unwrap();
        let b = s.create(peer(2), [2; 16], "b", 100, 5, u64::MAX).unwrap();
        std::fs::write(&b, [0u8; 100]).unwrap();
        s.sweep(6, 5);
        assert!(!a.exists(), "the oldest went first");
        assert!(b.exists(), "and the next one stayed: there was room");
    }

    #[test]
    fn persists_across_open() {
        let d = dir("persist");
        {
            let mut s = PartialStore::open_with(&d, 0, small()).unwrap();
            let p = s.create(peer(1), TOKEN, "a.bin", 100, 1, u64::MAX).unwrap();
            std::fs::write(p, [1u8; 30]).unwrap();
        }
        let mut s = PartialStore::open_with(&d, 2, small()).unwrap();
        assert_eq!(s.claim(peer(1), TOKEN, "a.bin", 100).unwrap().len, 30);
    }

    #[test]
    fn orphan_partials_are_swept() {
        let d = dir("orphan");
        let mut s = PartialStore::open_with(&d, 0, small()).unwrap();
        let stray = d.join("deadbeef-00000000000000000000000000000000.part");
        std::fs::write(&stray, b"x").unwrap();
        let other = d.join("notes.txt");
        std::fs::write(&other, b"keep").unwrap();
        s.sweep(1, u64::MAX);
        assert!(!stray.exists());
        assert!(other.exists(), "only .part files are ours to remove");
    }

    #[test]
    fn finish_deletes_the_file_and_the_entry() {
        let d = dir("finish");
        let mut s = PartialStore::open_with(&d, 0, small()).unwrap();
        let p = s.create(peer(1), TOKEN, "a", 10, 1, u64::MAX).unwrap();
        s.finish(peer(1), TOKEN);
        assert!(!p.exists());
        assert!(s.claim(peer(1), TOKEN, "a", 10).is_none());
    }

    #[test]
    fn a_partial_longer_than_its_file_is_dropped() {
        let d = dir("long");
        let mut s = PartialStore::open_with(&d, 0, small()).unwrap();
        let p = s.create(peer(1), TOKEN, "a", 10, 1, u64::MAX).unwrap();
        std::fs::write(p, [0u8; 11]).unwrap();
        assert!(s.claim(peer(1), TOKEN, "a", 10).is_none());
    }

    #[test]
    fn offset_math_is_u64_on_32_bit() {
        // A kept length past 4 GiB must come back whole (never truncated to usize).
        let d = dir("u64");
        let big = 4 * GIB + 5;
        let caps = Caps {
            per_peer: 8 * GIB,
            total: 8 * GIB,
            ..Caps::default()
        };
        let mut s = PartialStore::open_with(&d, 0, caps).unwrap();
        let p = s.create(peer(1), TOKEN, "big", big, 1, u64::MAX).unwrap();
        let f = std::fs::OpenOptions::new().write(true).open(&p).unwrap();
        f.set_len(4 * GIB + 1).unwrap();
        assert_eq!(
            s.claim(peer(1), TOKEN, "big", big).unwrap().len,
            4 * GIB + 1
        );
        // The file may be large on a disk without sparse files: leave nothing behind.
        let _ = std::fs::remove_dir_all(&d);
    }
}
