//! Lines of peers.txt (the trusted devices), and the name and address cleaners.
#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|text: &str| {
    for line in text.lines() {
        let _ = pb_identity::fuzz_parse_peer_line(line);
    }
    let _ = pb_identity::clean_peer_name(text);
    let _ = pb_identity::clean_peer_addrs(&text.split(',').collect::<Vec<_>>());
});
