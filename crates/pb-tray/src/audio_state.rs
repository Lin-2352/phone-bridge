//! Audio sharing on the PC: the pure part. Which direction is which, what
//! state each is in, whether the PC must ask first, and every sentence the
//! user can read about it. No windows, no network: the runtime
//! (`audio.rs`) and the pages call these.
//!
//! The PC always dials and sends `AudioStart`. Phone audio on this PC: the
//! phone asks its own user. PC audio on the phone: this PC asks its user
//! (unless the Settings toggle skips that). The microphone is never offered
//! here: it is a phone-side mode that nothing on the PC can start.

use pb_protocol::control::{AudioReason, AUDIO_PC_TO_PHONE, AUDIO_PHONE_TO_PC};
use pb_transport::Error;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dir {
    /// The phone's audio plays on this PC.
    PhoneToPc,
    /// This PC's audio plays on the phone.
    PcToPhone,
}

impl Dir {
    pub const ALL: [Dir; 2] = [Dir::PhoneToPc, Dir::PcToPhone];

    /// `AudioStart.direction`.
    pub fn wire(self) -> u32 {
        match self {
            Dir::PhoneToPc => AUDIO_PHONE_TO_PC,
            Dir::PcToPhone => AUDIO_PC_TO_PHONE,
        }
    }

    pub fn index(self) -> usize {
        match self {
            Dir::PhoneToPc => 0,
            Dir::PcToPhone => 1,
        }
    }

    /// The menu entry, the Home card title and the Timeline wording.
    pub fn title(self) -> &'static str {
        match self {
            Dir::PhoneToPc => "Phone audio on this PC",
            Dir::PcToPhone => "PC audio on the phone",
        }
    }

    /// The short Home tile label.
    pub fn tile(self) -> &'static str {
        match self {
            Dir::PhoneToPc => "Phone audio",
            Dir::PcToPhone => "PC audio",
        }
    }

    /// The line under the title while it runs.
    pub fn running_meta(self) -> &'static str {
        match self {
            Dir::PhoneToPc => "Playing on this PC's speakers",
            Dir::PcToPhone => "Playing on the phone",
        }
    }

    /// Short log word.
    pub fn log_word(self) -> &'static str {
        match self {
            Dir::PhoneToPc => "phone_to_pc",
            Dir::PcToPhone => "pc_to_phone",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Phase {
    #[default]
    Idle,
    /// Asked; the phone (or this PC's own user) has not answered.
    Asking,
    Running,
}

/// One phase per direction: at most one session each.
#[derive(Clone, Copy, Debug, Default)]
pub struct Tracker([Phase; 2]);

impl Tracker {
    pub const fn new() -> Self {
        Tracker([Phase::Idle, Phase::Idle])
    }

    pub fn phase(&self, d: Dir) -> Phase {
        self.0[d.index()]
    }

    /// Start asking. False when that direction is already asking or running.
    pub fn begin(&mut self, d: Dir) -> bool {
        if self.0[d.index()] != Phase::Idle {
            return false;
        }
        self.0[d.index()] = Phase::Asking;
        true
    }

    /// The session is up. False when it was not being asked for (it ended).
    pub fn running(&mut self, d: Dir) -> bool {
        if self.0[d.index()] != Phase::Asking {
            return false;
        }
        self.0[d.index()] = Phase::Running;
        true
    }

    /// Whatever it was, it is over. Returns the phase it was in.
    pub fn end(&mut self, d: Dir) -> Phase {
        std::mem::take(&mut self.0[d.index()])
    }
}

/// Whether the PC shows its own confirmation first. Only PC audio on the
/// phone has one (the phone asks for its own audio); the Settings toggle
/// skips it. Nothing here ever involves the microphone.
pub fn needs_pc_confirm(d: Dir, skip_pc_to_phone: bool) -> bool {
    match d {
        Dir::PcToPhone => !skip_pc_to_phone,
        Dir::PhoneToPc => false,
    }
}

/// The PC's confirmation dialog: (title, body, yes, no).
pub fn confirm_text(phone: &str) -> (String, String, &'static str, &'static str) {
    (
        format!("Play this PC's audio on {phone}?"),
        "Everything this PC plays through its speakers will play on the phone until you stop it. \
         You can turn this question off in Relay settings."
            .into(),
        "Play",
        "Cancel",
    )
}

/// Why a request did not start, in words that say what to do next.
pub fn refused_text(d: Dir, phone: &str, e: &Error) -> String {
    match e {
        Error::Declined | Error::AudioRefused(AudioReason::Declined) => match d {
            Dir::PhoneToPc => format!(
                "{phone} didn't share its audio. The request was declined or nobody answered it on the phone."
            ),
            Dir::PcToPhone => format!("{phone} declined to play this PC's audio"),
        },
        Error::AudioRefused(AudioReason::Busy) => match d {
            Dir::PhoneToPc => format!("{phone} is already sharing its audio. Stop it there first."),
            Dir::PcToPhone => {
                format!("{phone} is already playing this PC's audio. Stop it first.")
            }
        },
        Error::AudioRefused(AudioReason::NoPermission) => format!(
            "Relay on {phone} may not capture audio. Allow it in Android settings, then try again."
        ),
        Error::AudioRefused(AudioReason::AppBlocksCapture) => format!(
            "The app playing audio on {phone} doesn't allow it to be shared. Try another app."
        ),
        Error::AudioRefused(AudioReason::BadParams) => {
            format!("{phone} didn't accept the audio format. Update Relay on both devices.")
        }
        Error::AudioRefused(AudioReason::Unavailable) => match d {
            Dir::PhoneToPc => format!(
                "{phone} can't share its audio now. It needs Android 10 or later and Relay open."
            ),
            Dir::PcToPhone => format!("{phone} can't play this PC's audio right now"),
        },
        Error::Unsupported => {
            format!("Relay on {phone} is too old for audio. Update it, then try again.")
        }
        Error::Timeout => format!("{phone} didn't answer the audio request"),
        Error::PeerChanged => {
            format!("The device answering is not {phone}. Check Devices in Relay.")
        }
        _ => format!("Couldn't reach {phone} for audio. Check that both are on the same network."),
    }
}

/// How a running session ended, for the user.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum End {
    /// The user pressed Stop here (or paused Relay).
    LocalStop,
    /// The phone stopped it.
    PeerStopped,
    /// The link dropped, or the stream ended.
    LinkLost,
    /// The PC's audio device failed or changed.
    DeviceFailed(String),
    /// The phone sent something the limits forbid.
    Violation,
}

/// What the user is told when it ends; `None` when they ended it
/// themselves (they know).
pub fn ended_text(d: Dir, phone: &str, end: &End) -> Option<String> {
    let t = d.title();
    match end {
        End::LocalStop => None,
        End::PeerStopped => Some(format!("{t} stopped on {phone}")),
        End::LinkLost => Some(format!(
            "{t} stopped: lost the connection to {phone}. Start it again when it is back."
        )),
        End::DeviceFailed(why) => Some(format!("{t} stopped: {why}")),
        End::Violation => Some(format!("{t} stopped: {phone} sent audio Relay can't use")),
    }
}

/// The audio device failed to start or kept running.
pub fn device_text(d: Dir, e: &str) -> String {
    match d {
        Dir::PhoneToPc => format!("This PC has no usable speakers for audio ({e})"),
        Dir::PcToPhone => format!("This PC's audio can't be captured ({e})"),
    }
}

/// The line on the PC's pill while the request waits. The PC cannot know
/// whether the phone will ask (its own setting may skip that question), so it
/// is true either way.
pub fn asking_text(d: Dir, phone: &str) -> String {
    match d {
        Dir::PhoneToPc => {
            format!("Asking {phone} to share its audio. If the phone asks, approve it there.")
        }
        Dir::PcToPhone => format!("Asking {phone} to play this PC's audio\u{2026}"),
    }
}

/// What the receiving end learned about frame arrival, for the log: how many
/// frames came and the longest silence between two of them. Jitter shows up
/// here before it shows as an underrun.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Arrivals {
    pub frames: u64,
    pub bytes: u64,
    pub max_gap_ms: u64,
    /// Loudest sample in this window, 0..=32767. A level, never the audio.
    pub peak: u16,
    last_ms: Option<u64>,
}

impl Arrivals {
    /// A frame of `len` bytes arrived `now_ms` after the session began.
    pub fn frame(&mut self, now_ms: u64, len: usize) {
        if let Some(last) = self.last_ms {
            self.max_gap_ms = self.max_gap_ms.max(now_ms.saturating_sub(last));
        }
        self.last_ms = Some(now_ms);
        self.frames += 1;
        self.bytes += len as u64;
    }

    /// Note the loudest sample of a PCM chunk (16-bit LE).
    pub fn level(&mut self, pcm: &[u8]) {
        for b in pcm.as_chunks::<2>().0 {
            let v = i16::from_le_bytes(*b).unsigned_abs().min(32767);
            self.peak = self.peak.max(v);
        }
    }

    /// One log line; the unit of every number is in its name.
    pub fn line(&self, dropped_ms: u64, underruns: u64) -> String {
        format!(
            "frames={} bytes={} max_gap_ms={} peak={} dropped_ms={} underruns={}",
            self.frames, self.bytes, self.max_gap_ms, self.peak, dropped_ms, underruns
        )
    }

    /// Start a new reporting window, keeping the last arrival time.
    pub fn reset_window(&mut self) {
        self.frames = 0;
        self.bytes = 0;
        self.max_gap_ms = 0;
        self.peak = 0;
    }
}

/// "12 s", "4 min 12 s", "1 h 3 min".
pub fn duration_text(secs: u64) -> String {
    match secs {
        0..60 => format!("{secs} s"),
        60..3600 => {
            let (m, s) = (secs / 60, secs % 60);
            if s == 0 {
                format!("{m} min")
            } else {
                format!("{m} min {s} s")
            }
        }
        _ => {
            let (h, m) = (secs / 3600, secs % 3600 / 60);
            if m == 0 {
                format!("{h} h")
            } else {
                format!("{h} h {m} min")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn directions_map_to_the_wire_codes() {
        assert_eq!(Dir::PhoneToPc.wire(), 0);
        assert_eq!(Dir::PcToPhone.wire(), 1);
        assert_eq!(Dir::PhoneToPc.index(), 0);
        assert_eq!(Dir::PcToPhone.index(), 1);
    }

    #[test]
    fn only_pc_audio_on_the_phone_asks_on_the_pc_and_the_toggle_skips_it() {
        assert!(needs_pc_confirm(Dir::PcToPhone, false));
        assert!(!needs_pc_confirm(Dir::PcToPhone, true));
        // The phone asks for its own audio; the PC never adds a dialog.
        assert!(!needs_pc_confirm(Dir::PhoneToPc, false));
        assert!(!needs_pc_confirm(Dir::PhoneToPc, true));
    }

    #[test]
    fn one_session_per_direction_and_the_other_direction_is_free() {
        let mut t = Tracker::new();
        assert!(t.begin(Dir::PhoneToPc));
        assert!(!t.begin(Dir::PhoneToPc), "already asking");
        assert!(t.begin(Dir::PcToPhone));
        assert!(t.running(Dir::PhoneToPc));
        assert!(!t.begin(Dir::PhoneToPc), "already running");
        assert_eq!(t.phase(Dir::PhoneToPc), Phase::Running);
        assert_eq!(t.phase(Dir::PcToPhone), Phase::Asking);
        assert_eq!(t.end(Dir::PhoneToPc), Phase::Running);
        assert_eq!(t.phase(Dir::PhoneToPc), Phase::Idle);
        assert!(t.begin(Dir::PhoneToPc), "free again");
    }

    #[test]
    fn a_session_that_ended_while_asking_cannot_turn_running() {
        let mut t = Tracker::new();
        assert!(t.begin(Dir::PcToPhone));
        assert_eq!(t.end(Dir::PcToPhone), Phase::Asking);
        assert!(!t.running(Dir::PcToPhone));
        assert_eq!(t.phase(Dir::PcToPhone), Phase::Idle);
    }

    #[test]
    fn every_refusal_names_the_phone_and_is_never_empty() {
        let errs = [
            Error::Declined,
            Error::Unsupported,
            Error::Timeout,
            Error::Closed,
            Error::PeerChanged,
            Error::AudioRefused(AudioReason::Busy),
            Error::AudioRefused(AudioReason::Unavailable),
            Error::AudioRefused(AudioReason::BadParams),
            Error::AudioRefused(AudioReason::NoPermission),
            Error::AudioRefused(AudioReason::AppBlocksCapture),
        ];
        for d in Dir::ALL {
            for e in &errs {
                let t = refused_text(d, "Pixel", e);
                assert!(t.contains("Pixel"), "{e:?}: {t}");
                assert!(!t.contains("Error") && !t.contains("error"), "{t}");
            }
        }
    }

    #[test]
    fn an_old_phone_and_a_blocking_app_say_what_to_do() {
        let t = refused_text(Dir::PcToPhone, "P", &Error::Unsupported);
        assert!(t.contains("Update"));
        let t = refused_text(
            Dir::PhoneToPc,
            "P",
            &Error::AudioRefused(AudioReason::AppBlocksCapture),
        );
        assert!(t.contains("another app"));
    }

    #[test]
    fn ending_by_yourself_is_silent_and_every_other_end_says_why() {
        assert_eq!(ended_text(Dir::PhoneToPc, "P", &End::LocalStop), None);
        for end in [
            End::PeerStopped,
            End::LinkLost,
            End::DeviceFailed("no device".into()),
            End::Violation,
        ] {
            let t = ended_text(Dir::PcToPhone, "P", &end).unwrap();
            assert!(t.starts_with("PC audio on the phone stopped"), "{t}");
        }
        assert!(ended_text(Dir::PhoneToPc, "P", &End::LinkLost)
            .unwrap()
            .contains("again"));
    }

    #[test]
    fn the_asking_line_is_true_whether_or_not_the_phone_asks() {
        let t = asking_text(Dir::PhoneToPc, "Pixel");
        assert!(t.contains("Pixel") && t.contains("If the phone asks"));
        assert!(!t.contains("approve it on the phone"));
        assert!(asking_text(Dir::PcToPhone, "Pixel").contains("Pixel"));
    }

    #[test]
    fn arrivals_note_the_loudest_sample() {
        let mut a = Arrivals::default();
        a.level(&[0, 0, 0x10, 0, 0xF0, 0xFF]);
        assert_eq!(a.peak, 16);
        a.level(&[0, 0x80]);
        assert_eq!(a.peak, 32767, "i16::MIN stays in range");
        assert!(a.line(0, 0).contains("peak=32767"));
        a.reset_window();
        assert_eq!(a.peak, 0);
    }

    #[test]
    fn arrivals_track_the_longest_gap_and_name_their_units() {
        let mut a = Arrivals::default();
        a.frame(0, 3840);
        a.frame(20, 3840);
        a.frame(520, 3840);
        a.frame(540, 3840);
        assert_eq!((a.frames, a.bytes, a.max_gap_ms), (4, 15_360, 500));
        let l = a.line(80, 3);
        assert!(
            l.contains("max_gap_ms=500")
                && l.contains("dropped_ms=80")
                && l.contains("underruns=3")
        );
        a.reset_window();
        assert_eq!((a.frames, a.max_gap_ms), (0, 0));
        a.frame(560, 1);
        assert_eq!(
            a.max_gap_ms, 20,
            "the gap across the window edge still counts"
        );
    }

    #[test]
    fn durations_read_naturally() {
        assert_eq!(duration_text(0), "0 s");
        assert_eq!(duration_text(59), "59 s");
        assert_eq!(duration_text(60), "1 min");
        assert_eq!(duration_text(252), "4 min 12 s");
        assert_eq!(duration_text(3600), "1 h");
        assert_eq!(duration_text(3780), "1 h 3 min");
    }

    #[test]
    fn the_confirmation_names_the_phone_and_the_off_switch() {
        let (title, body, yes, no) = confirm_text("Pixel");
        assert!(title.contains("Pixel"));
        assert!(body.contains("settings"));
        assert_ne!(yes, no);
    }

    #[test]
    fn wording_uses_the_user_facing_names() {
        assert_eq!(Dir::PhoneToPc.title(), "Phone audio on this PC");
        assert_eq!(Dir::PcToPhone.title(), "PC audio on the phone");
    }
}
