//! Remote input records (mouse, keys) as the phone or PC receives them.
#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = pb_protocol::InputDatagram::decode(data);
});
