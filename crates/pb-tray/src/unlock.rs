//! The PC's unlock gate (M7.2): whether Windows Hello can guard the sensitive actions on this PC, and why not
//! when it cannot. Pure: the Hello state and the Windows facts come in as values, so the rules are tested
//! everywhere. The verification call itself lives in pb-platform-win (a worker thread only).
#![cfg_attr(not(windows), allow(dead_code))]

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

/// After a successful unlock nothing asks again for this long.
pub const GRACE: std::time::Duration = std::time::Duration::from_secs(60);

/// The actions the gate guards (the phone also guards allow-list changes, which the PC does not have).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Guarded {
    AllowFile,
    AllowAudio,
    AllowControl,
    AlwaysAllow,
    AutoAccept,
    ConfirmPairing,
    Unpair,
    GateOff,
}

/// What to do for a guarded action.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GateStep {
    /// Go ahead: the gate is off, or the person unlocked a moment ago.
    Proceed,
    /// Ask for Windows Hello first.
    Prompt,
    /// The gate is on but cannot run (Hello was removed, Remote Desktop): the old consent still stands.
    ProceedNoGate,
}

pub fn gate_step(enabled: bool, available: bool, grace_ok: bool) -> GateStep {
    if !enabled {
        GateStep::Proceed
    } else if !available {
        GateStep::ProceedNoGate
    } else if grace_ok {
        GateStep::Proceed
    } else {
        GateStep::Prompt
    }
}

/// The 60 s after a successful unlock. A failed or cancelled unlock starts none; locking the session resets it; a
/// clock that moved backwards cannot prove it, so that asks too.
#[derive(Default)]
pub struct Grace {
    last: Option<std::time::Instant>,
}

impl Grace {
    pub const fn new() -> Self {
        Self { last: None }
    }

    pub fn needs_unlock(&self, now: std::time::Instant) -> bool {
        match self.last {
            None => true,
            Some(at) => now < at || now.duration_since(at) >= GRACE,
        }
    }

    pub fn passed(&mut self, now: std::time::Instant) {
        self.last = Some(now);
    }

    pub fn reset(&mut self) {
        self.last = None;
    }
}

static GRACE_STATE: std::sync::Mutex<Grace> = std::sync::Mutex::new(Grace::new());

fn grace() -> std::sync::MutexGuard<'static, Grace> {
    GRACE_STATE.lock().unwrap_or_else(|p| p.into_inner())
}

#[cfg(windows)]
static AVAILABILITY: std::sync::Mutex<Option<(std::time::Instant, GateAvailability)>> =
    std::sync::Mutex::new(None);

#[cfg(windows)]
/// How long a read of Windows Hello's state is reused (it is asked of Windows, so not on every repaint).
const AVAILABILITY_TTL: std::time::Duration = std::time::Duration::from_secs(10);

#[cfg(windows)]
/// The gate's availability for the settings row, without blocking the window: the last answer, refreshed on a
/// worker thread when it is older than ten seconds. Until the first answer arrives it reads "available".
pub fn cached_availability() -> GateAvailability {
    let now = std::time::Instant::now();
    let (known, fresh) = {
        let g = AVAILABILITY.lock().unwrap_or_else(|p| p.into_inner());
        match *g {
            Some((at, a)) => (a, now.duration_since(at) < AVAILABILITY_TTL),
            None => (GateAvailability::Available, false),
        }
    };
    if !fresh {
        // Mark it fresh first so a burst of repaints starts one refresh, not many.
        *AVAILABILITY.lock().unwrap_or_else(|p| p.into_inner()) = Some((now, known));
        std::thread::spawn(|| {
            let a = current_availability();
            *AVAILABILITY.lock().unwrap_or_else(|p| p.into_inner()) =
                Some((std::time::Instant::now(), a));
        });
    }
    known
}

/// Locking the session ends the grace: the next guarded action asks again.
pub fn reset_grace() {
    grace().reset();
}

/// Whether a guarded action would ask for Hello now (the gate is on and the last unlock is over). It does not know
/// whether Hello can run; `guard` finds that out and, when it cannot, lets the action through on the old consent.
pub fn would_prompt(enabled: bool) -> bool {
    enabled && grace().needs_unlock(std::time::Instant::now())
}

#[cfg(windows)]
fn hello_to_state(s: pb_platform_win::hello::HelloState) -> HelloState {
    use pb_platform_win::hello::HelloState as H;
    match s {
        H::Available => HelloState::Available,
        H::NotConfigured => HelloState::NotConfigured,
        H::DisabledByPolicy => HelloState::DisabledByPolicy,
        H::DeviceBusy => HelloState::DeviceBusy,
        H::Unavailable => HelloState::Unavailable,
    }
}

#[cfg(windows)]
/// Whether the gate can run on this PC right now (asks Windows: call it off the window thread).
pub fn current_availability() -> GateAvailability {
    availability(
        crate::fluent::os_build(),
        crate::fluent::remote_session(),
        hello_to_state(pb_platform_win::hello::hello_state()),
    )
}

#[cfg(windows)]
/// Runs the gate for a guarded action on a worker thread (Windows Hello blocks), then calls `then(true)` when the
/// action may go ahead and `then(false)` when the person did not unlock. A gate that is off, a grace that holds,
/// or a PC where Hello cannot run (the old consent stands) all go ahead; the last also starts the grace, so an
/// action re-queued after the check does not ask again.
pub fn guard(
    _what: Guarded,
    enabled: bool,
    hwnd: isize,
    message: String,
    then: impl FnOnce(bool) + Send + 'static,
) {
    // Whether Hello can run is not known yet: assume it can, and let the grace and the setting decide first.
    let grace_ok = !grace().needs_unlock(std::time::Instant::now());
    if gate_step(enabled, true, grace_ok) == GateStep::Proceed {
        then(true);
        return;
    }
    std::thread::spawn(move || {
        let available = current_availability() == GateAvailability::Available;
        let ok = match gate_step(true, available, false) {
            GateStep::Prompt => matches!(pb_platform_win::hello::verify(hwnd, &message), Ok(true)),
            // Cannot run: the consent dialog still stands, so the person is not locked out of their own PC.
            GateStep::ProceedNoGate | GateStep::Proceed => true,
        };
        if ok {
            grace().passed(std::time::Instant::now());
        }
        then(ok);
    });
}

#[cfg(windows)]
/// Wraps a dialog's answer: a "yes" goes through the gate first, and a failed or cancelled unlock turns it into a
/// "no" (nothing is allowed). A "no" never needs the gate.
pub fn gate_answer(
    what: Guarded,
    settings: crate::core::SharedSettings,
    hwnd: isize,
    message: &'static str,
    done: impl FnOnce(bool) + Send + 'static,
) -> impl FnOnce(bool) + Send + 'static {
    move |yes| {
        if !yes {
            done(false);
            return;
        }
        let enabled = settings.lock().map(|s| s.gate_on(what)).unwrap_or(false);
        guard(what, enabled, hwnd, message.to_owned(), done);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn asks_the_first_time_and_not_within_60_s() {
        let t0 = Instant::now();
        let mut g = Grace::default();
        assert!(g.needs_unlock(t0), "nothing unlocked yet");
        g.passed(t0);
        assert!(!g.needs_unlock(t0 + Duration::from_secs(59)));
        assert!(g.needs_unlock(t0 + GRACE), "asks again at 60 s");
    }

    #[test]
    fn a_failed_unlock_starts_no_grace_and_locking_resets_it() {
        let t0 = Instant::now();
        let mut g = Grace::default();
        // A failed or cancelled attempt never calls passed(): the next action asks again at once.
        assert!(g.needs_unlock(t0 + Duration::from_secs(1)));
        g.passed(t0);
        g.reset();
        assert!(
            g.needs_unlock(t0 + Duration::from_secs(1)),
            "locking the session resets the grace"
        );
    }

    #[test]
    fn gate_off_means_proceed_and_unavailable_means_proceed_with_reason() {
        assert_eq!(gate_step(false, true, false), GateStep::Proceed);
        assert_eq!(gate_step(true, false, false), GateStep::ProceedNoGate);
        assert_eq!(gate_step(true, true, false), GateStep::Prompt);
        assert_eq!(gate_step(true, true, true), GateStep::Proceed);
    }

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
