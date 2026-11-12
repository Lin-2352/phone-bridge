//! The camera frames file (shared with Frame Server): its header and the
//! newest-frame read, over whatever bytes the file holds.
#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let mut words: Vec<u64> = data
        .chunks(8)
        .map(|c| {
            let mut b = [0u8; 8];
            b[..c.len()].copy_from_slice(c);
            u64::from_le_bytes(b)
        })
        .collect();
    if let Some(ring) = pb_camera_frames::Ring::open(&mut words) {
        let mut out = Vec::new();
        let _ = ring.read_newest(0, (64, 48), &mut out);
    }
});
