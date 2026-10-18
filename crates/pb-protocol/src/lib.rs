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

/// Largest control frame body we will encode or accept (64 KiB).
pub const MAX_CONTROL_FRAME: usize = 64 * 1024;

const LEN_PREFIX: usize = 4;

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
}

impl Hello {
    pub fn new(device_name: impl Into<String>, capabilities: Vec<String>) -> Self {
        Self {
            protocol_version: u32::from(PROTOCOL_VERSION),
            device_name: device_name.into(),
            capabilities,
        }
    }

    /// Pre-1.0 policy: versions must match exactly.
    pub fn is_compatible(&self) -> bool {
        self.protocol_version == u32::from(PROTOCOL_VERSION)
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
    Key { scancode: u16, pressed: bool },
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
            k => return Err(DatagramError::UnknownKind(k)),
        };
        Ok(Self { seq, event })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert!(got.is_compatible());
    }

    #[test]
    fn incompatible_version_is_detected() {
        let mut h = hello();
        h.protocol_version = 99;
        assert!(!h.is_compatible());
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
        ];
        for (i, event) in events.into_iter().enumerate() {
            let d = InputDatagram {
                seq: 65530 + i as u16,
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
