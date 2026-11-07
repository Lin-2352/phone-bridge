//! The PC's unlock gate (M7.2): whether Windows Hello can guard the sensitive actions on this PC, and why not
//! when it cannot. Pure: the Hello state and the Windows facts come in as values, so the rules are tested
//! everywhere. The verification call itself lives in pb-platform-win (a worker thread only).

#![allow(dead_code)] // the settings row and the gate call these in the next M7.2 commit

/// Windows 11's build (22000) is the first with the Hello consent verifier interop.
pub const MIN_BUILD: u32 = 22000;

/// What Windows reports about Hello (`UserConsentVerifierAvailability`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HelloState {
    Available,
    NotConfigured,
    DisabledByPolicy,
    DeviceBusy,
    Unavailable,
}

/// Whether the gate can run on this PC, and the reason when it cannot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GateAvailability {
    Available,
    OldWindows,
    RemoteDesktop,
    NoHello(HelloState),
}

/// The gate runs when the build is Windows 11, the session is local and Hello is set up.
pub fn availability(build: u32, remote_session: bool, hello: HelloState) -> GateAvailability {
    if build < MIN_BUILD {
        GateAvailability::OldWindows
    } else if remote_session {
        GateAvailability::RemoteDesktop
    } else if hello != HelloState::Available {
        GateAvailability::NoHello(hello)
    } else {
        GateAvailability::Available
    }
}

/// The line the settings row shows under the switch (in English; strings move with the PC's strings table).
pub fn reason_text(a: &GateAvailability) -> &'static str {
    match a {
        GateAvailability::Available => "Ask for Windows Hello before sensitive actions",
        GateAvailability::OldWindows => "Needs Windows 11 (version 22000 or later)",
        GateAvailability::RemoteDesktop => "Not available over Remote Desktop",
        GateAvailability::NoHello(HelloState::NotConfigured) => {
            "Set up Windows Hello (PIN, face or fingerprint) first"
        }
        GateAvailability::NoHello(HelloState::DisabledByPolicy) => {
            "Windows Hello is turned off by your administrator"
        }
        GateAvailability::NoHello(HelloState::DeviceBusy) => {
            "Windows Hello is busy; try again in a moment"
        }
        GateAvailability::NoHello(_) => "Windows Hello is not available on this PC",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_10_is_unavailable_with_a_reason() {
        let a = availability(19045, false, HelloState::Available);
        assert_eq!(a, GateAvailability::OldWindows);
        assert!(reason_text(&a).contains("Windows 11"));
    }

    #[test]
    fn remote_desktop_is_unavailable_with_a_reason() {
        let a = availability(26100, true, HelloState::Available);
        assert_eq!(a, GateAvailability::RemoteDesktop);
        assert!(reason_text(&a).contains("Remote Desktop"));
    }

    #[test]
    fn hello_not_set_up_is_unavailable_with_a_reason() {
        let a = availability(26100, false, HelloState::NotConfigured);
        assert_eq!(a, GateAvailability::NoHello(HelloState::NotConfigured));
        assert!(reason_text(&a).contains("Set up Windows Hello"));
    }

    #[test]
    fn windows_11_with_hello_is_available() {
        assert_eq!(
            availability(22000, false, HelloState::Available),
            GateAvailability::Available
        );
        assert_eq!(
            availability(22000, false, HelloState::Available),
            GateAvailability::Available
        );
    }
}
