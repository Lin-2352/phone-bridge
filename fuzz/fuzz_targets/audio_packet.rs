//! Audio datagrams; the first two bytes pick the codec and channel count.
#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let [codec, channels, rest @ ..] = data else {
        return;
    };
    let _ = pb_transport::audio::decode_packet(rest, u32::from(*codec % 4), u32::from(*channels % 4));
});
