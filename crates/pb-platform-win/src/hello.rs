//! Windows Hello as the unlock gate's verifier (M7.2). `hello_state` asks whether Hello can be used at all (no
//! window, safe anywhere); `verify` shows the Hello prompt over a window and waits for the person, so it blocks:
//! call it from a worker thread only, never from a window thread.
#![allow(unsafe_code)]

#[cfg(windows)]
use windows::{
    core::{factory, HSTRING},
    Security::Credentials::UI::{
        UserConsentVerificationResult, UserConsentVerifier, UserConsentVerifierAvailability,
    },
    Win32::{
        Foundation::HWND,
        System::WinRT::{IUserConsentVerifierInterop, RoInitialize, RO_INIT_MULTITHREADED},
    },
};
#[cfg(windows)]
use windows_future::IAsyncOperation;

/// What Windows reports about Hello (`UserConsentVerifierAvailability`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HelloState {
    Available,
    NotConfigured,
    DisabledByPolicy,
    DeviceBusy,
    Unavailable,
}

#[cfg(windows)]
fn io(e: windows::core::Error) -> std::io::Error {
    std::io::Error::other(e.to_string())
}

/// Whether Windows Hello can verify the user now.
#[cfg(windows)]
pub fn hello_state() -> HelloState {
    // A worker thread has no WinRT apartment yet; an error here means one already exists, which is fine.
    // SAFETY: plain apartment initialisation for this thread.
    let _ = unsafe { RoInitialize(RO_INIT_MULTITHREADED) };
    match UserConsentVerifier::CheckAvailabilityAsync().and_then(|op| op.join()) {
        Ok(UserConsentVerifierAvailability::Available) => HelloState::Available,
        Ok(UserConsentVerifierAvailability::NotConfiguredForUser) => HelloState::NotConfigured,
        Ok(UserConsentVerifierAvailability::DisabledByPolicy) => HelloState::DisabledByPolicy,
        Ok(UserConsentVerifierAvailability::DeviceBusy) => HelloState::DeviceBusy,
        _ => HelloState::Unavailable,
    }
}

/// Shows the Hello prompt over `hwnd` with `message` and waits: true only when the person verified. Blocks: worker
/// threads only.
#[cfg(windows)]
pub fn verify(hwnd: isize, message: &str) -> std::io::Result<bool> {
    // SAFETY: plain apartment initialisation for this thread (an error means one exists).
    let _ = unsafe { RoInitialize(RO_INIT_MULTITHREADED) };
    let interop = factory::<UserConsentVerifier, IUserConsentVerifierInterop>().map_err(io)?;
    // SAFETY: `hwnd` is a window of this process (or null); the message is a valid string for the call.
    let op: IAsyncOperation<UserConsentVerificationResult> = unsafe {
        interop.RequestVerificationForWindowAsync(HWND(hwnd as *mut _), &HSTRING::from(message))
    }
    .map_err(io)?;
    Ok(op.join().map_err(io)? == UserConsentVerificationResult::Verified)
}

/// Not Windows: there is no Hello.
#[cfg(not(windows))]
pub fn hello_state() -> HelloState {
    HelloState::Unavailable
}

/// Not Windows: nothing can verify.
#[cfg(not(windows))]
pub fn verify(_hwnd: isize, _message: &str) -> std::io::Result<bool> {
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hello_state_answers_without_a_window_or_a_panic() {
        // Whatever this machine says (a PC without Hello, a CI runner), asking is safe and gives one of the states.
        let s = hello_state();
        assert!(matches!(
            s,
            HelloState::Available
                | HelloState::NotConfigured
                | HelloState::DisabledByPolicy
                | HelloState::DeviceBusy
                | HelloState::Unavailable
        ));
    }
}
