//! EcoQoS: ask Windows to run this process on efficient cores at low clock
//! speed. Right for a resident daemon that is idle almost all the time.
#![allow(unsafe_code)]

use windows::Win32::System::Threading::{
    GetCurrentProcess, GetCurrentThread, ProcessPowerThrottling, SetProcessInformation,
    SetThreadInformation, SetThreadPriority, ThreadPowerThrottling,
    PROCESS_POWER_THROTTLING_CURRENT_VERSION, PROCESS_POWER_THROTTLING_EXECUTION_SPEED,
    PROCESS_POWER_THROTTLING_STATE, THREAD_POWER_THROTTLING_CURRENT_VERSION,
    THREAD_POWER_THROTTLING_EXECUTION_SPEED, THREAD_POWER_THROTTLING_STATE,
    THREAD_PRIORITY_ABOVE_NORMAL,
};

/// CPU time this process has used (user + kernel), in seconds.
pub fn process_cpu_seconds() -> f64 {
    use windows::Win32::{Foundation::FILETIME, System::Threading::GetProcessTimes};
    let (mut created, mut exited, mut kernel, mut user) = (
        FILETIME::default(),
        FILETIME::default(),
        FILETIME::default(),
        FILETIME::default(),
    );
    // SAFETY: pseudo-handle and valid out-pointers.
    let ok = unsafe {
        GetProcessTimes(
            GetCurrentProcess(),
            &mut created,
            &mut exited,
            &mut kernel,
            &mut user,
        )
    };
    if ok.is_err() {
        return 0.0;
    }
    let ticks = |f: FILETIME| (u64::from(f.dwHighDateTime) << 32) | u64::from(f.dwLowDateTime);
    (ticks(kernel) + ticks(user)) as f64 / 1e7
}

/// Exempt the calling thread from EcoQoS and raise its priority: for the
/// input hook thread, which every key and mouse move on the PC waits for.
pub fn make_thread_responsive() -> windows::core::Result<()> {
    let state = THREAD_POWER_THROTTLING_STATE {
        Version: THREAD_POWER_THROTTLING_CURRENT_VERSION,
        ControlMask: THREAD_POWER_THROTTLING_EXECUTION_SPEED,
        StateMask: 0,
    };
    // SAFETY: valid pseudo-handle and a correctly sized, initialised struct.
    unsafe {
        SetThreadInformation(
            GetCurrentThread(),
            ThreadPowerThrottling,
            &state as *const _ as *const core::ffi::c_void,
            std::mem::size_of::<THREAD_POWER_THROTTLING_STATE>() as u32,
        )?;
        SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_ABOVE_NORMAL)
    }
}

/// Turn EcoQoS on (`true`) or let Windows decide (`false`).
pub fn set_eco_qos(on: bool) -> windows::core::Result<()> {
    let state = PROCESS_POWER_THROTTLING_STATE {
        Version: PROCESS_POWER_THROTTLING_CURRENT_VERSION,
        ControlMask: PROCESS_POWER_THROTTLING_EXECUTION_SPEED,
        StateMask: if on {
            PROCESS_POWER_THROTTLING_EXECUTION_SPEED
        } else {
            0
        },
    };
    // SAFETY: valid pseudo-handle and a correctly sized, initialised struct.
    unsafe {
        SetProcessInformation(
            GetCurrentProcess(),
            ProcessPowerThrottling,
            &state as *const _ as *const core::ffi::c_void,
            std::mem::size_of::<PROCESS_POWER_THROTTLING_STATE>() as u32,
        )
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn eco_qos_can_be_toggled() {
        super::set_eco_qos(true).unwrap();
        super::set_eco_qos(false).unwrap();
    }
}
