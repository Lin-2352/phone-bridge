//! A UDP discovery packet from anyone on the local network.
#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = pb_transport::discovery::fuzz_parse(data);
});
