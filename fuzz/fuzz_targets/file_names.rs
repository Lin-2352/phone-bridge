//! File names offered by the other device: never a path, whatever they hold.
#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|name: &str| {
    let safe = pb_transport::files::sanitize_file_name(name);
    assert!(!safe.contains('/') && !safe.contains('\\'), "{safe:?}");
    let _ = pb_transport::files::classify(name);
});
