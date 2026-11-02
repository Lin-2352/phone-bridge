//! The two Win32 reads the structured log needs: this process's working set
//! and the local UTC offset. Everything else about the log is safe code.
#![allow(unsafe_code)]

/// Working set of this process in bytes (0 when it cannot be read).
#[cfg(windows)]
pub fn working_set_bytes() -> u64 {
    use windows::Win32::System::{
        ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS},
        Threading::GetCurrentProcess,
    };
    let mut c = PROCESS_MEMORY_COUNTERS {
        cb: std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
        ..Default::default()
    };
    // SAFETY: `c` is a valid, correctly sized out-struct for the pseudo-handle
    // of the current process.
    let ok = unsafe { GetProcessMemoryInfo(GetCurrentProcess(), &mut c, c.cb) };
    if ok.is_ok() {
        c.WorkingSetSize as u64
    } else {
        0
    }
}

/// Seconds to add to UTC for local time now (daylight saving included).
#[cfg(windows)]
pub fn tz_offset_secs() -> i32 {
    use windows::Win32::System::Time::{GetTimeZoneInformation, TIME_ZONE_INFORMATION};
    let mut tz = TIME_ZONE_INFORMATION::default();
    // SAFETY: `tz` is a valid out-struct.
    let id = unsafe { GetTimeZoneInformation(&mut tz) };
    let bias = tz.Bias
        + match id {
            2 => tz.DaylightBias,
            _ => tz.StandardBias,
        };
    -bias * 60
}

#[cfg(not(windows))]
pub fn working_set_bytes() -> u64 {
    0
}

#[cfg(not(windows))]
pub fn tz_offset_secs() -> i32 {
    0
}
