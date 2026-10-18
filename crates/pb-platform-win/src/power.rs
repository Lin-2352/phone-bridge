//! EcoQoS: ask Windows to run this process on efficient cores at low clock
//! speed. Right for a resident daemon that is idle almost all the time.
#![allow(unsafe_code)]

use windows::Win32::System::Threading::{
    GetCurrentProcess, ProcessPowerThrottling, SetProcessInformation,
    PROCESS_POWER_THROTTLING_CURRENT_VERSION, PROCESS_POWER_THROTTLING_EXECUTION_SPEED,
    PROCESS_POWER_THROTTLING_STATE,
};

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
