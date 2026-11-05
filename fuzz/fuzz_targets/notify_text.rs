//! Notification text from the phone, read for one-time codes.
#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|text: &str| {
    let _ = pb_transport::notify::find_otp(text);
});
