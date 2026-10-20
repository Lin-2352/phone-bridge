//! The frames file (spec section 4): a header, then two NV12 slots with a
//! seqlock each. The tray writes; the media source reads.

use std::sync::atomic::Ordering;

pub const MAGIC: [u8; 4] = *b"PBCF";
pub const VERSION: u32 = 1;
pub const DATA_OFFSET: usize = 4096;
pub const SLOT_BYTES: usize = 1920 * 1080 * 3 / 2;
pub const FILE_BYTES: usize = DATA_OFFSET + 2 * SLOT_BYTES;

#[derive(Debug, PartialEq, Eq)]
pub enum Read {
    New {
        seq: u64,
        w: u32,
        h: u32,
        pts_us: u64,
    },
    Same,
    Torn,
    Mismatch,
    Empty,
}

/// Word (u64) indices of the header fields (byte offset / 8).
const W_MAGIC_VERSION: usize = 0;
const W_WANTED: usize = 1;
const W_SEQ: usize = 3; // two slots: 3, 4
const W_NEWEST: usize = 5;
const W_PTS: usize = 6; // two slots: 6, 7
const W_BEAT: usize = 8;
const W_DIMS: usize = 9; // two slots: 9, 10
const W_FRAMES: usize = 11;
const HEADER_WORDS: usize = 12;
/// The tray beats while a session runs; during its 30 s reconnect wait the
/// last picture stays, then black.
const TRAY_TIMEOUT_MS: u64 = 35_000;

/// The mapped file as 8-byte words. Both processes map the same file; every
/// header field is an atomic word, and each slot is guarded by its seqlock.
pub struct Ring<'a> {
    ptr: *mut u64,
    words: usize,
    _map: std::marker::PhantomData<&'a mut [u64]>,
}

// SAFETY: the words are only accessed through atomics or inside the
// slot seqlock protocol.
#[allow(unsafe_code)]
unsafe impl Send for Ring<'_> {}

#[allow(unsafe_code)]
impl<'a> Ring<'a> {
    fn new(words: &'a mut [u64]) -> Option<Self> {
        (words.len() * 8 >= FILE_BYTES).then_some(Self {
            ptr: words.as_mut_ptr(),
            words: words.len(),
            _map: std::marker::PhantomData,
        })
    }

    fn word(&self, i: usize) -> &std::sync::atomic::AtomicU64 {
        assert!(i < self.words);
        // SAFETY: in bounds, 8-byte aligned (a u64 slice), lives as long as
        // the mapping; all access to header words is atomic.
        unsafe { std::sync::atomic::AtomicU64::from_ptr(self.ptr.add(i)) }
    }

    fn slot_ptr(&self, slot: usize) -> *mut u8 {
        // SAFETY: DATA_OFFSET + 2 * SLOT_BYTES == FILE_BYTES <= the mapping.
        unsafe { (self.ptr as *mut u8).add(DATA_OFFSET + slot * SLOT_BYTES) }
    }

    /// Format a fresh file: header zero, magic and version set.
    pub fn init(words: &'a mut [u64]) -> Self {
        let r = Self::new(words).expect("a buffer of FILE_BYTES");
        for i in 0..HEADER_WORDS {
            r.word(i).store(0, Ordering::Relaxed);
        }
        r.word(W_MAGIC_VERSION).store(
            u64::from(u32::from_le_bytes(MAGIC)) | (u64::from(VERSION) << 32),
            Ordering::Release,
        );
        r
    }

    /// An existing file; `None` unless magic and version match.
    pub fn open(words: &'a mut [u64]) -> Option<Self> {
        let r = Self::new(words)?;
        let mv = r.word(W_MAGIC_VERSION).load(Ordering::Acquire);
        (mv == u64::from(u32::from_le_bytes(MAGIC)) | (u64::from(VERSION) << 32)).then_some(r)
    }

    /// Write one frame into the slot that is not the newest. `fill` gets
    /// the slot's bytes (at least `w * h * 3 / 2` of them).
    pub fn write_frame(&mut self, w: u32, h: u32, pts_us: u64, fill: impl FnOnce(&mut [u8])) {
        let need = (w as usize) * (h as usize) * 3 / 2;
        assert!(need <= SLOT_BYTES, "frame larger than a slot");
        let frames = self.word(W_FRAMES).load(Ordering::Acquire);
        let newest = self.word(W_NEWEST).load(Ordering::Acquire) as usize;
        let slot = if frames == 0 { 0 } else { 1 - newest };
        let seq = self.word(W_SEQ + slot);
        seq.fetch_add(1, Ordering::AcqRel); // odd: being written
        self.word(W_DIMS + slot)
            .store(u64::from(w) | (u64::from(h) << 32), Ordering::Relaxed);
        self.word(W_PTS + slot).store(pts_us, Ordering::Relaxed);
        // SAFETY: this slot is odd: readers either skip it or discard the
        // copy; only this writer touches it.
        let bytes = unsafe { std::slice::from_raw_parts_mut(self.slot_ptr(slot), SLOT_BYTES) };
        fill(bytes);
        seq.fetch_add(1, Ordering::Release); // even: complete
        self.word(W_NEWEST).store(slot as u64, Ordering::Release);
        self.word(W_FRAMES).fetch_add(1, Ordering::Release);
    }

    /// A writer that died inside a write leaves its slot odd.
    pub fn begin_write_for_test(&mut self) {
        let newest = self.word(W_NEWEST).load(Ordering::Acquire) as usize;
        self.word(W_SEQ + (1 - newest))
            .fetch_add(1, Ordering::AcqRel);
    }

    /// The newest complete frame, if it is new and has the `wanted` size.
    /// `out` is only touched by a `New` result.
    pub fn read_newest(&self, last_seq: u64, wanted: (u32, u32), out: &mut Vec<u8>) -> Read {
        let frames = self.word(W_FRAMES).load(Ordering::Acquire);
        if frames == 0 {
            return Read::Empty;
        }
        if frames == last_seq {
            return Read::Same;
        }
        let slot = (self.word(W_NEWEST).load(Ordering::Acquire) as usize) & 1;
        let s1 = self.word(W_SEQ + slot).load(Ordering::Acquire);
        if s1 & 1 == 1 {
            return Read::Torn;
        }
        let dims = self.word(W_DIMS + slot).load(Ordering::Relaxed);
        let (w, h) = (dims as u32, (dims >> 32) as u32);
        let pts_us = self.word(W_PTS + slot).load(Ordering::Relaxed);
        let need = (w as usize) * (h as usize) * 3 / 2;
        if (w, h) != wanted || need == 0 || need > SLOT_BYTES {
            return Read::Mismatch;
        }
        let mut copy = vec![0u8; need];
        // SAFETY: within the slot; a concurrent write is caught by the
        // seqlock re-check below and the copy is discarded.
        unsafe {
            std::ptr::copy_nonoverlapping(self.slot_ptr(slot), copy.as_mut_ptr(), need);
        }
        std::sync::atomic::fence(Ordering::Acquire);
        if self.word(W_SEQ + slot).load(Ordering::Acquire) != s1 {
            return Read::Torn;
        }
        *out = copy;
        Read::New {
            seq: frames,
            w,
            h,
            pts_us,
        }
    }

    /// The size the app chose (written by the media source).
    pub fn set_wanted(&mut self, w: u32, h: u32) {
        self.word(W_WANTED)
            .store(u64::from(w) | (u64::from(h) << 32), Ordering::Release);
    }

    pub fn wanted(&self) -> (u32, u32) {
        let v = self.word(W_WANTED).load(Ordering::Acquire);
        (v as u32, (v >> 32) as u32)
    }

    /// The tray is alive (written about once a second).
    pub fn beat(&mut self, now_ms: u64) {
        self.word(W_BEAT).store(now_ms, Ordering::Release);
    }

    /// False if the tray never beat or stopped for 5 s: show black.
    pub fn tray_alive(&self, now_ms: u64) -> bool {
        let b = self.word(W_BEAT).load(Ordering::Acquire);
        b != 0 && now_ms.saturating_sub(b) <= TRAY_TIMEOUT_MS
    }

    /// Forget every frame (stop, tray start): nothing of the picture stays.
    pub fn zero(&mut self) {
        self.word(W_FRAMES).store(0, Ordering::Release);
        self.word(W_NEWEST).store(0, Ordering::Release);
        for slot in 0..2 {
            self.word(W_SEQ + slot).store(0, Ordering::Release);
            self.word(W_DIMS + slot).store(0, Ordering::Release);
            self.word(W_PTS + slot).store(0, Ordering::Release);
            // SAFETY: the tray is the only writer and no frame is current.
            unsafe { std::ptr::write_bytes(self.slot_ptr(slot), 0, SLOT_BYTES) };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words() -> Vec<u64> {
        vec![0u64; FILE_BYTES.div_ceil(8)]
    }

    fn frame(r: &mut Ring, w: u32, h: u32, pts: u64, v: u8) {
        r.write_frame(w, h, pts, |s| s[..(w * h * 3 / 2) as usize].fill(v));
    }

    #[test]
    fn header_round_trips() {
        let mut b = words();
        Ring::init(&mut b).set_wanted(1280, 720);
        let r = Ring::open(&mut b).expect("magic and version");
        assert_eq!(r.wanted(), (1280, 720));
        let mut zero = words();
        assert!(Ring::open(&mut zero).is_none(), "not initialised");
    }

    #[test]
    fn mismatched_size_frame_is_not_delivered() {
        let mut b = words();
        let mut r = Ring::init(&mut b);
        frame(&mut r, 1280, 720, 5, 9);
        let mut out = vec![7u8; 3];
        assert_eq!(r.read_newest(0, (1920, 1080), &mut out), Read::Mismatch);
        assert_eq!(out, vec![7u8; 3], "output untouched");
        assert!(matches!(
            r.read_newest(0, (1280, 720), &mut out),
            Read::New {
                w: 1280,
                h: 720,
                pts_us: 5,
                ..
            }
        ));
        assert_eq!(out.len(), 1280 * 720 * 3 / 2);
        assert!(out.iter().all(|b| *b == 9));
    }

    #[test]
    fn newest_complete_slot_wins() {
        let mut b = words();
        let mut r = Ring::init(&mut b);
        frame(&mut r, 1280, 720, 1, 1);
        frame(&mut r, 1280, 720, 2, 2);
        let mut out = Vec::new();
        assert!(matches!(
            r.read_newest(0, (1280, 720), &mut out),
            Read::New { pts_us: 2, .. }
        ));
        assert!(out.iter().all(|b| *b == 2));
    }

    #[test]
    fn empty_ring_reads_empty() {
        let mut b = words();
        let r = Ring::init(&mut b);
        assert_eq!(r.read_newest(0, (1280, 720), &mut Vec::new()), Read::Empty);
    }

    #[test]
    fn torn_read_is_rejected() {
        let mut b = words();
        let mut r = Ring::init(&mut b);
        frame(&mut r, 1280, 720, 1, 1);
        // The writer died inside the next write: its slot's seq stays odd,
        // and `newest` still points at the complete frame.
        r.begin_write_for_test();
        let mut out = Vec::new();
        let got = r.read_newest(0, (1280, 720), &mut out);
        assert!(matches!(got, Read::New { pts_us: 1, .. }), "{got:?}");
        assert!(out.iter().all(|b| *b == 1), "never the half-written bytes");
    }

    #[test]
    fn same_frame_reads_same() {
        let mut b = words();
        let mut r = Ring::init(&mut b);
        frame(&mut r, 1280, 720, 1, 1);
        let mut out = Vec::new();
        let Read::New { seq, .. } = r.read_newest(0, (1280, 720), &mut out) else {
            panic!()
        };
        assert_eq!(r.read_newest(seq, (1280, 720), &mut out), Read::Same);
    }

    #[test]
    fn tray_heartbeat_times_out_after_35s() {
        let mut b = words();
        let mut r = Ring::init(&mut b);
        assert!(!r.tray_alive(1000), "never beat");
        r.beat(1000);
        assert!(
            r.tray_alive(35_999),
            "the 30 s reconnect wait still shows the picture"
        );
        assert!(!r.tray_alive(36_001));
    }

    #[test]
    fn zero_clears_frames() {
        let mut b = words();
        let mut r = Ring::init(&mut b);
        frame(&mut r, 1280, 720, 1, 7);
        r.zero();
        assert_eq!(r.read_newest(0, (1280, 720), &mut Vec::new()), Read::Empty);
    }
}
