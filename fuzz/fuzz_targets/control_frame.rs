//! The control stream's frame decoder: length prefix, size cap, protobuf body.
#![no_main]
use libfuzzer_sys::fuzz_target;
use pb_protocol::control::Control;

fuzz_target!(|data: &[u8]| {
    if let Ok(Some((_msg, used))) = pb_protocol::decode_frame::<Control>(data) {
        assert!(used <= data.len());
    }
});
