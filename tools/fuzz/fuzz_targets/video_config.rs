//! A video stream's codec configuration record (screen and camera).
#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = pb_video::Config::decode(data);
});
