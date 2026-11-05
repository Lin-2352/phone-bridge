//! Versioned wire framing for Phone Bridge. See `docs/DECISIONS.md` (Q6).
//!
//! - Control messages: protobuf (prost) inside length-prefixed frames on QUIC streams.
//! - Input datagrams: fixed 8-byte layout, no allocation, on QUIC datagrams.
//!
//! Every decoder enforces a size limit before allocating and returns an error
//! instead of panicking on malformed input.

use std::fmt;

pub mod control;

use prost::Message;

/// Wire protocol version. Bumped on any incompatible change.
pub const PROTOCOL_VERSION: u16 = 0;

/// The oldest peer protocol version this build still talks to. Relay 0.2.0
/// required an exact match and sends no minimum (read as 0).
pub const MIN_COMPATIBLE: u16 = 0;

/// Close reason (code 426) when the receiver of the close must update.
pub const CLOSE_REASON_UPDATE_YOU: &[u8] = b"update:you";
/// Close reason (code 426) when the device closing must update.
pub const CLOSE_REASON_UPDATE_ME: &[u8] = b"update:me";

/// Largest control frame body we will encode or accept (64 KiB).
pub const MAX_CONTROL_FRAME: usize = 64 * 1024;

const LEN_PREFIX: usize = 4;

/// `Hello.capabilities` entry: the clipboard messages.
pub const CAP_CLIPBOARD: &str = "clipboard";

/// Reserved for keyed discovery (PBD2, milestone M7): advertised only by a build that has it.
pub const CAP_DISCOVERY2: &str = "discovery2";

/// Reserved for resumable transfers (offer resume token, queue pause and resume, milestone M9).
pub const CAP_RESUME: &str = "resume";

/// Names kept for features not built yet. A build advertises one only once it implements it.
/// The other planned features (unlock gate, Disconnect everything, pairing limits, quotas,
/// several devices, history, drag-and-drop) change nothing on the wire.
pub const RESERVED_CAPS: &[&str] = &[CAP_DISCOVERY2, CAP_RESUME];

/// What this build puts in `Hello.capabilities`.
pub fn local_capabilities() -> Vec<String> {
    vec![CAP_CLIPBOARD.into(), CAP_AUDIO.into()]
}

/// `Hello.capabilities` entry: this build understands the audio messages
/// (tags 25 to 27). A peer without it never answers them.
pub const CAP_AUDIO: &str = "audio";

/// Screen-session notice the phone sends before ending a share that Android
/// stopped because the phone locked (Android 15 QPR1 and later). The PC says
/// so instead of retrying; an older PC just shows it as the status line.
pub const SCREEN_LOCKED_NOTICE: &str = "Phone locked: screen sharing stopped";

/// First message on every connection, sent by both sides.
#[derive(Clone, PartialEq, Message)]
pub struct Hello {
    #[prost(uint32, tag = "1")]
    pub protocol_version: u32,
    /// User-visible device name. Only sent inside an authenticated session,
    /// never in discovery announcements.
    #[prost(string, tag = "2")]
    pub device_name: String,
    /// Feature names the sender supports (e.g. "clipboard", "files").
    #[prost(string, repeated, tag = "3")]
    pub capabilities: Vec<String>,
    /// The oldest peer `protocol_version` the sender talks to (0 from Relay 0.2.0).
    #[prost(uint32, tag = "4")]
    pub min_compatible: u32,
    /// The sender's Relay version, e.g. "0.3.0" (empty from Relay 0.2.0). For people, not for decisions.
    #[prost(string, tag = "5")]
    pub app_version: String,
}

/// Which device needs a newer Relay when two cannot talk.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UpdateSide {
    ThisDevice,
    OtherDevice,
    /// Unknown which (a Relay 0.2.0 peer says only "incompatible").
    Both,
}

/// `None` when the two devices can talk: each one's minimum is at most the other's version.
/// Otherwise the device whose version is below the other's minimum must update.
pub fn update_side(local: &Hello, remote: &Hello) -> Option<UpdateSide> {
    if remote.protocol_version < local.min_compatible {
        Some(UpdateSide::OtherDevice)
    } else if local.protocol_version < remote.min_compatible {
        Some(UpdateSide::ThisDevice)
    } else {
        None
    }
}

/// The close reason this device sends when [update_side] gave `side`.
pub fn close_reason_for(side: UpdateSide) -> &'static [u8] {
    match side {
        UpdateSide::OtherDevice => CLOSE_REASON_UPDATE_YOU,
        UpdateSide::ThisDevice => CLOSE_REASON_UPDATE_ME,
        UpdateSide::Both => b"incompatible protocol version",
    }
}

/// Reads a 426 close reason from the receiving device's side.
pub fn side_from_close_reason(reason: &[u8]) -> UpdateSide {
    match reason {
        CLOSE_REASON_UPDATE_YOU => UpdateSide::ThisDevice,
        CLOSE_REASON_UPDATE_ME => UpdateSide::OtherDevice,
        _ => UpdateSide::Both,
    }
}

impl Hello {
    pub fn new(device_name: impl Into<String>, capabilities: Vec<String>) -> Self {
        Self {
            protocol_version: u32::from(PROTOCOL_VERSION),
            device_name: device_name.into(),
            capabilities,
            min_compatible: u32::from(MIN_COMPATIBLE),
            app_version: env!("CARGO_PKG_VERSION").into(),
        }
    }

    /// Whether the sender listed `capability`.
    pub fn supports(&self, capability: &str) -> bool {
        self.capabilities.iter().any(|c| c == capability)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum FrameError {
    /// Declared or encoded body exceeds [`MAX_CONTROL_FRAME`].
    TooLarge(usize),
    /// Body is not a valid message of the expected type.
    Decode,
}

impl fmt::Display for FrameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooLarge(n) => write!(f, "frame of {n} bytes exceeds {MAX_CONTROL_FRAME}"),
            Self::Decode => f.write_str("malformed message"),
        }
    }
}

impl std::error::Error for FrameError {}

/// Encode `msg` as `u32 little-endian length || protobuf body`.
pub fn encode_frame<M: Message>(msg: &M) -> Result<Vec<u8>, FrameError> {
    let len = msg.encoded_len();
    if len > MAX_CONTROL_FRAME {
        return Err(FrameError::TooLarge(len));
    }
    let mut out = Vec::with_capacity(LEN_PREFIX + len);
    out.extend_from_slice(&(len as u32).to_le_bytes());
    msg.encode(&mut out).map_err(|_| FrameError::Decode)?;
    Ok(out)
}

/// Try to decode one frame from the front of `buf`.
///
/// Returns `Ok(None)` if more bytes are needed, otherwise the message and the
/// number of bytes consumed. A declared length over the limit is rejected
/// immediately, before any body bytes arrive.
pub fn decode_frame<M: Message + Default>(buf: &[u8]) -> Result<Option<(M, usize)>, FrameError> {
    let Some(prefix) = buf.first_chunk::<LEN_PREFIX>() else {
        return Ok(None);
    };
    let len = u32::from_le_bytes(*prefix) as usize;
    if len > MAX_CONTROL_FRAME {
        return Err(FrameError::TooLarge(len));
    }
    let Some(body) = buf.get(LEN_PREFIX..LEN_PREFIX + len) else {
        return Ok(None);
    };
    let msg = M::decode(body).map_err(|_| FrameError::Decode)?;
    Ok(Some((msg, LEN_PREFIX + len)))
}

/// A remote-input event carried in one QUIC datagram. Datagrams are unordered
/// and may be lost, so every event is self-contained and carries a sequence
/// number so the receiver can drop stale or duplicate ones.
///
/// Layout (8 bytes, little-endian): `kind u8 | flags u8 | seq u16 | a i16 | b i16`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputEvent {
    /// Relative pointer motion.
    MouseMove { dx: i16, dy: i16 },
    /// Button 0=left 1=right 2=middle 3=back 4=forward.
    MouseButton { button: u8, pressed: bool },
    /// Scroll in 1/120 notch units.
    Wheel { dx: i16, dy: i16 },
    /// Hardware scan code (not a character) for layout-independent keys.
    /// PC set 1; extended keys carry 0xE0 in the high byte (0xE048 = Up).
    Key { scancode: u16, pressed: bool },
    /// One UTF-16 code unit of typed text (surrogate pairs are two events).
    Char { unit: u16 },
    /// A finger at an absolute spot (the phone-screen window): x and y are
    /// 0..=65535 across the screen. Hover only moves the pointer.
    Touch { phase: TouchPhase, x: u16, y: u16 },
    /// Two fingers pinching out (zoom in) or in, centred on (x, y).
    Pinch { out: bool, x: u16, y: u16 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TouchPhase {
    Hover,
    Down,
    Move,
    Up,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InputDatagram {
    pub seq: u16,
    pub event: InputEvent,
}

pub const INPUT_DATAGRAM_LEN: usize = 8;

#[derive(Debug, PartialEq, Eq)]
pub enum DatagramError {
    WrongLength(usize),
    UnknownKind(u8),
}

impl fmt::Display for DatagramError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WrongLength(n) => write!(
                f,
                "input datagram must be {INPUT_DATAGRAM_LEN} bytes, got {n}"
            ),
            Self::UnknownKind(k) => write!(f, "unknown input datagram kind {k}"),
        }
    }
}

impl std::error::Error for DatagramError {}

impl InputDatagram {
    pub fn encode(&self) -> [u8; INPUT_DATAGRAM_LEN] {
        let (kind, flags, a, b) = match self.event {
            InputEvent::MouseMove { dx, dy } => (0u8, 0u8, dx, dy),
            InputEvent::MouseButton { button, pressed } => {
                (1, u8::from(pressed), i16::from(button), 0)
            }
            InputEvent::Wheel { dx, dy } => (2, 0, dx, dy),
            InputEvent::Key { scancode, pressed } => (3, u8::from(pressed), scancode as i16, 0),
            InputEvent::Char { unit } => (4, 0, unit as i16, 0),
            InputEvent::Touch { phase, x, y } => (5, phase as u8, x as i16, y as i16),
            InputEvent::Pinch { out, x, y } => (6, u8::from(out), x as i16, y as i16),
        };
        let mut out = [0u8; INPUT_DATAGRAM_LEN];
        out[0] = kind;
        out[1] = flags;
        out[2..4].copy_from_slice(&self.seq.to_le_bytes());
        out[4..6].copy_from_slice(&a.to_le_bytes());
        out[6..8].copy_from_slice(&b.to_le_bytes());
        out
    }

    pub fn decode(buf: &[u8]) -> Result<Self, DatagramError> {
        let b: &[u8; INPUT_DATAGRAM_LEN] = buf
            .try_into()
            .map_err(|_| DatagramError::WrongLength(buf.len()))?;
        let seq = u16::from_le_bytes([b[2], b[3]]);
        let a = i16::from_le_bytes([b[4], b[5]]);
        let c = i16::from_le_bytes([b[6], b[7]]);
        let pressed = b[1] & 1 == 1;
        let event = match b[0] {
            0 => InputEvent::MouseMove { dx: a, dy: c },
            1 => InputEvent::MouseButton {
                button: a as u8,
                pressed,
            },
            2 => InputEvent::Wheel { dx: a, dy: c },
            3 => InputEvent::Key {
                scancode: a as u16,
                pressed,
            },
            4 => InputEvent::Char { unit: a as u16 },
            5 => InputEvent::Touch {
                phase: match b[1] {
                    0 => TouchPhase::Hover,
                    1 => TouchPhase::Down,
                    2 => TouchPhase::Move,
                    3 => TouchPhase::Up,
                    _ => return Err(DatagramError::UnknownKind(5)),
                },
                x: a as u16,
                y: c as u16,
            },
            6 => InputEvent::Pinch {
                out: pressed,
                x: a as u16,
                y: c as u16,
            },
            k => return Err(DatagramError::UnknownKind(k)),
        };
        Ok(Self { seq, event })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hello_reports_listed_capabilities() {
        let h = Hello::new("PC", vec!["clipboard".into(), CAP_AUDIO.into()]);
        assert!(h.supports(CAP_AUDIO));
        assert!(!Hello::new("old", vec!["clipboard".into()]).supports(CAP_AUDIO));
    }

    #[test]
    fn touch_and_pinch_records_round_trip() {
        let events = [
            InputEvent::Touch {
                phase: TouchPhase::Hover,
                x: 0,
                y: 65535,
            },
            InputEvent::Touch {
                phase: TouchPhase::Down,
                x: 12,
                y: 34,
            },
            InputEvent::Touch {
                phase: TouchPhase::Move,
                x: 65535,
                y: 0,
            },
            InputEvent::Touch {
                phase: TouchPhase::Up,
                x: 32767,
                y: 32768,
            },
            InputEvent::Pinch {
                out: true,
                x: 100,
                y: 200,
            },
            InputEvent::Pinch {
                out: false,
                x: 65535,
                y: 65535,
            },
        ];
        for event in events {
            let d = InputDatagram { seq: 9, event };
            assert_eq!(InputDatagram::decode(&d.encode()), Ok(d));
        }
    }

    #[test]
    fn unknown_touch_phase_is_rejected() {
        let mut b = InputDatagram {
            seq: 1,
            event: InputEvent::Touch {
                phase: TouchPhase::Down,
                x: 1,
                y: 1,
            },
        }
        .encode();
        b[1] = 7;
        assert_eq!(
            InputDatagram::decode(&b),
            Err(DatagramError::UnknownKind(5))
        );
    }

    #[test]
    fn protocol_version_starts_at_zero() {
        assert_eq!(PROTOCOL_VERSION, 0);
    }

    fn hello() -> Hello {
        Hello::new("Desk PC", vec!["clipboard".into(), "files".into()])
    }

    #[test]
    fn hello_round_trips_through_a_frame() {
        let bytes = encode_frame(&hello()).unwrap();
        let (got, used): (Hello, usize) = decode_frame(&bytes).unwrap().unwrap();
        assert_eq!(got, hello());
        assert_eq!(used, bytes.len());
        assert_eq!(update_side(&hello(), &got), None);
    }

    fn hello_at(version: u32, min: u32) -> Hello {
        let mut h = hello();
        h.protocol_version = version;
        h.min_compatible = min;
        h
    }

    #[test]
    fn capability_names_are_unique_and_lowercase() {
        let mut all: Vec<&str> = vec![CAP_CLIPBOARD, CAP_AUDIO];
        all.extend(RESERVED_CAPS);
        let n = all.len();
        all.sort_unstable();
        all.dedup();
        assert_eq!(all.len(), n, "duplicate capability name");
        assert!(all.iter().all(|c| c
            .chars()
            .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit())));
    }

    #[test]
    fn reserved_capabilities_are_not_advertised_yet() {
        let local = local_capabilities();
        assert_eq!(local, vec!["clipboard".to_string(), "audio".to_string()]);
        for r in RESERVED_CAPS {
            assert!(
                !local.iter().any(|c| c == r),
                "{r} is advertised before it exists"
            );
        }
        assert!(RESERVED_CAPS.contains(&CAP_DISCOVERY2) && RESERVED_CAPS.contains(&CAP_RESUME));
    }

    #[test]
    fn same_versions_are_compatible() {
        assert_eq!(update_side(&hello(), &hello()), None);
        // A newer peer that still accepts this version: no update needed.
        assert_eq!(update_side(&hello_at(0, 0), &hello_at(3, 0)), None);
    }

    #[test]
    fn a_peer_below_my_minimum_must_update() {
        assert_eq!(
            update_side(&hello_at(2, 1), &hello_at(0, 0)),
            Some(UpdateSide::OtherDevice)
        );
    }

    #[test]
    fn this_device_below_the_peers_minimum_must_update() {
        assert_eq!(
            update_side(&hello_at(0, 0), &hello_at(2, 1)),
            Some(UpdateSide::ThisDevice)
        );
    }

    #[test]
    fn hello_carries_the_build_version() {
        let h = hello();
        assert_eq!(h.app_version, env!("CARGO_PKG_VERSION"));
        assert_eq!(h.min_compatible, u32::from(MIN_COMPATIBLE));
    }

    #[test]
    fn close_reasons_name_the_side() {
        // The closer says who must update; the receiver reads it from its own side.
        assert_eq!(
            side_from_close_reason(CLOSE_REASON_UPDATE_YOU),
            UpdateSide::ThisDevice
        );
        assert_eq!(
            side_from_close_reason(CLOSE_REASON_UPDATE_ME),
            UpdateSide::OtherDevice
        );
        assert_eq!(
            side_from_close_reason(b"incompatible protocol version"),
            UpdateSide::Both
        ); // Relay 0.2.0
        assert_eq!(
            close_reason_for(UpdateSide::OtherDevice),
            CLOSE_REASON_UPDATE_YOU
        );
        assert_eq!(
            close_reason_for(UpdateSide::ThisDevice),
            CLOSE_REASON_UPDATE_ME
        );
    }

    #[test]
    fn partial_frames_ask_for_more_bytes() {
        let bytes = encode_frame(&hello()).unwrap();
        for cut in 0..bytes.len() {
            assert_eq!(decode_frame::<Hello>(&bytes[..cut]), Ok(None), "cut {cut}");
        }
    }

    #[test]
    fn two_frames_back_to_back_decode_one_at_a_time() {
        let mut bytes = encode_frame(&hello()).unwrap();
        bytes.extend(encode_frame(&Hello::new("Phone", vec![])).unwrap());
        let (first, used): (Hello, usize) = decode_frame(&bytes).unwrap().unwrap();
        assert_eq!(first.device_name, "Desk PC");
        let (second, _): (Hello, usize) = decode_frame(&bytes[used..]).unwrap().unwrap();
        assert_eq!(second.device_name, "Phone");
    }

    #[test]
    fn oversized_declared_length_is_rejected_before_the_body_arrives() {
        let huge = ((MAX_CONTROL_FRAME + 1) as u32).to_le_bytes();
        assert_eq!(
            decode_frame::<Hello>(&huge),
            Err(FrameError::TooLarge(MAX_CONTROL_FRAME + 1))
        );
        assert_eq!(
            decode_frame::<Hello>(&u32::MAX.to_le_bytes()),
            Err(FrameError::TooLarge(u32::MAX as usize))
        );
    }

    #[test]
    fn oversized_message_is_rejected_on_encode() {
        let big = Hello::new("x".repeat(MAX_CONTROL_FRAME), vec![]);
        assert!(matches!(encode_frame(&big), Err(FrameError::TooLarge(_))));
    }

    #[test]
    fn garbage_bodies_error_instead_of_panicking() {
        // field 1 declared as varint but truncated / wrong wire types
        for body in [&[0x08u8][..], &[0x0a, 0xff, 0xff][..], &[0xff; 16][..]] {
            let mut frame = (body.len() as u32).to_le_bytes().to_vec();
            frame.extend_from_slice(body);
            assert_eq!(decode_frame::<Hello>(&frame), Err(FrameError::Decode));
        }
    }

    #[test]
    fn every_input_event_round_trips() {
        let events = [
            InputEvent::MouseMove { dx: -5, dy: 300 },
            InputEvent::MouseButton {
                button: 2,
                pressed: true,
            },
            InputEvent::MouseButton {
                button: 0,
                pressed: false,
            },
            InputEvent::Wheel { dx: 0, dy: -120 },
            InputEvent::Key {
                scancode: 0x1e,
                pressed: true,
            },
            InputEvent::Key {
                scancode: 0xe05b,
                pressed: false,
            },
            InputEvent::Char { unit: 0x20ac },
            InputEvent::Char { unit: 0xd83d },
        ];
        for (i, event) in events.into_iter().enumerate() {
            let d = InputDatagram {
                seq: 65530u16.wrapping_add(i as u16),
                event,
            };
            assert_eq!(InputDatagram::decode(&d.encode()), Ok(d));
        }
    }

    #[test]
    fn bad_datagrams_are_rejected() {
        assert_eq!(
            InputDatagram::decode(&[0; 7]),
            Err(DatagramError::WrongLength(7))
        );
        assert_eq!(
            InputDatagram::decode(&[0; 9]),
            Err(DatagramError::WrongLength(9))
        );
        let mut unknown = [0u8; 8];
        unknown[0] = 200;
        assert_eq!(
            InputDatagram::decode(&unknown),
            Err(DatagramError::UnknownKind(200))
        );
    }
}
