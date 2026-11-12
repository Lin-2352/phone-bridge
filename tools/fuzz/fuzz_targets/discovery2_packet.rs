//! A keyed discovery (PBD2) packet from anyone on the local network: parsing and checking the mac never panic, and a
//! packet that parses is within the bounds.
#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = pb_transport::discovery2::fuzz_parse2(data);
});
