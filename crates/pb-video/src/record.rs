//! Video records on a stream: the header `PBV1`, then
//! `u32 len | i64 pts_us | u8 flags | payload` (little-endian, `len` = payload
//! bytes). A codec-config record carries [`Config`] and is always followed by
//! a key frame.

use std::fmt;

pub const MAGIC: &[u8; 4] = b"PBV1";
/// Largest payload accepted; more ends the session.
pub const MAX_RECORD: usize = 4 * 1024 * 1024;
pub const FLAG_KEY: u8 = 1;
pub const FLAG_CONFIG: u8 = 2;
/// The encoder repeated the previous frame to sharpen a still screen.
pub const FLAG_REPEAT: u8 = 4;

const HEADER: usize = 4 + 8 + 1;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Record {
    pub pts_us: i64,
    pub flags: u8,
    pub payload: Vec<u8>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum RecordError {
    BadMagic,
    TooLarge(usize),
}

impl fmt::Display for RecordError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadMagic => f.write_str("not a Phone Bridge video stream"),
            Self::TooLarge(n) => write!(f, "video record of {n} bytes is too large"),
        }
    }
}

impl std::error::Error for RecordError {}

/// Append one record (without the stream's magic).
pub fn encode(r: &Record, out: &mut Vec<u8>) {
    out.extend_from_slice(&(r.payload.len() as u32).to_le_bytes());
    out.extend_from_slice(&r.pts_us.to_le_bytes());
    out.push(r.flags);
    out.extend_from_slice(&r.payload);
}

/// Splits a byte stream into records; checks the magic once.
#[derive(Default)]
pub struct Decoder {
    buf: Vec<u8>,
    magic_ok: bool,
}

impl Decoder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, bytes: &[u8]) -> Result<Vec<Record>, RecordError> {
        self.buf.extend_from_slice(bytes);
        let mut at = 0;
        if !self.magic_ok {
            let n = self.buf.len().min(MAGIC.len());
            if self.buf[..n] != MAGIC[..n] {
                return Err(RecordError::BadMagic);
            }
            if n < MAGIC.len() {
                return Ok(Vec::new());
            }
            self.magic_ok = true;
            at = MAGIC.len();
        }
        let mut out = Vec::new();
        while let Some(head) = self.buf.get(at..at + HEADER) {
            let len = u32::from_le_bytes([head[0], head[1], head[2], head[3]]) as usize;
            if len > MAX_RECORD {
                return Err(RecordError::TooLarge(len));
            }
            let Some(payload) = self.buf.get(at + HEADER..at + HEADER + len) else {
                break;
            };
            let mut pts = [0u8; 8];
            pts.copy_from_slice(&head[4..12]);
            out.push(Record {
                pts_us: i64::from_le_bytes(pts),
                flags: head[12],
                payload: payload.to_vec(),
            });
            at += HEADER + len;
        }
        self.buf.drain(..at);
        Ok(out)
    }
}

/// The codec-config payload: `u16 rotation | u16 width | u16 height | headers`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Config {
    pub rotation: u16,
    pub width: u16,
    pub height: u16,
    /// VPS/SPS/PPS (H.265) or SPS/PPS (H.264), Annex-B.
    pub headers: Vec<u8>,
}

impl Config {
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(6 + self.headers.len());
        for v in [self.rotation, self.width, self.height] {
            out.extend_from_slice(&v.to_le_bytes());
        }
        out.extend_from_slice(&self.headers);
        out
    }

    pub fn decode(p: &[u8]) -> Option<Config> {
        let word = |i: usize| Some(u16::from_le_bytes([*p.get(i)?, *p.get(i + 1)?]));
        let (rotation, width, height) = (word(0)?, word(2)?, word(4)?);
        if ![0, 90, 180, 270].contains(&rotation) {
            return None;
        }
        Some(Config {
            rotation,
            width,
            height,
            headers: p[6..].to_vec(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(pts: i64, flags: u8, n: usize) -> Record {
        Record {
            pts_us: pts,
            flags,
            payload: (0..n).map(|i| i as u8).collect(),
        }
    }

    #[test]
    fn records_round_trip_across_split_reads() {
        let want = vec![
            rec(0, FLAG_CONFIG, 7),
            rec(33_333, FLAG_KEY, 300),
            rec(66_666, 0, 0),
        ];
        let mut bytes = MAGIC.to_vec();
        for r in &want {
            encode(r, &mut bytes);
        }
        let mut d = Decoder::new();
        let mut got = Vec::new();
        for b in &bytes {
            got.extend(d.push(std::slice::from_ref(b)).unwrap());
        }
        assert_eq!(got, want);
    }

    #[test]
    fn bad_magic_and_oversized_records_are_errors() {
        assert_eq!(Decoder::new().push(b"XXXX"), Err(RecordError::BadMagic));
        let mut bytes = MAGIC.to_vec();
        bytes.extend_from_slice(&(5u32 * 1024 * 1024).to_le_bytes());
        bytes.extend_from_slice(&[0; 9]);
        assert_eq!(
            Decoder::new().push(&bytes),
            Err(RecordError::TooLarge(5 * 1024 * 1024))
        );
    }

    #[test]
    fn config_record_round_trips_and_resets_size() {
        let c = Config {
            rotation: 90,
            width: 1920,
            height: 864,
            headers: vec![1, 2, 3],
        };
        assert_eq!(Config::decode(&c.encode()), Some(c));
        let bad = Config {
            rotation: 45,
            width: 1,
            height: 1,
            headers: vec![],
        };
        assert_eq!(Config::decode(&bad.encode()), None);
        assert_eq!(Config::decode(&[1, 2, 3]), None, "too short");
    }
}
