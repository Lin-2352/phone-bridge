//! The priority of the thread that feeds the sound device.
//!
//! cpal 0.15.3 means to raise its audio thread but passes the thread's ID where
//! Windows wants a handle, and ignores the error, so the thread stays at normal
//! priority and a busy PC can run the device dry. The fix has to come from code
//! running on that thread: the data callback.
#![allow(unsafe_code)]

use windows::core::w;
use windows::Win32::System::Threading::{
    AvSetMmThreadCharacteristicsW, GetCurrentThread, GetThreadPriority, SetThreadPriority,
    THREAD_PRIORITY_TIME_CRITICAL,
};

/// This thread's priority as Windows reports it. 0 is normal.
pub fn this_thread() -> i32 {
    // SAFETY: the pseudo handle from GetCurrentThread is always valid here.
    unsafe { GetThreadPriority(GetCurrentThread()) }
}

/// Asks for what audio players get. Best effort: without it the sound still
/// plays, only less reliably on a busy PC. Lasts as long as the thread.
pub fn raise_this_thread_for_audio() {
    let mut task = 0u32;
    // SAFETY: the name is a static wide string and `task` outlives the call.
    // The returned handle needs no closing: the system lets go when the thread ends.
    let _ = unsafe { AvSetMmThreadCharacteristicsW(w!("Pro Audio"), &mut task) };
    // SAFETY: the pseudo handle from GetCurrentThread is always valid here.
    let _ = unsafe { SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_TIME_CRITICAL) };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raising_really_raises_this_thread() {
        std::thread::spawn(|| {
            assert_eq!(this_thread(), 0, "a new thread starts at normal priority");
            raise_this_thread_for_audio();
            assert!(this_thread() > 0, "still at {}", this_thread());
        })
        .join()
        .unwrap();
    }
}
