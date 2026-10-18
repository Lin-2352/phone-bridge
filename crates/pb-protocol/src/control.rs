//! Control-stream messages (session and pairing) and the pairing code format.

use std::fmt;

use prost::{Message, Oneof};

use crate::Hello;

/// Largest clipboard text we send or accept in one message.
pub const MAX_CLIPBOARD_TEXT: usize = 60 * 1024;

/// Envelope for every message on a session control stream.
#[derive(Clone, PartialEq, Message)]
pub struct Control {
    #[prost(oneof = "ControlMsg", tags = "1, 2, 3, 4, 5, 6, 7, 8")]
    pub msg: Option<ControlMsg>,
}

#[derive(Clone, PartialEq, Oneof)]
pub enum ControlMsg {
    #[prost(message, tag = "1")]
    Hello(Hello),
    #[prost(message, tag = "2")]
    Heartbeat(Heartbeat),
    #[prost(message, tag = "3")]
    ClipboardText(ClipboardText),
    #[prost(message, tag = "4")]
    Offer(Offer),
    #[prost(message, tag = "5")]
    OfferReply(OfferReply),
    #[prost(message, tag = "6")]
    TransferDone(TransferDone),
    #[prost(message, tag = "7")]
    Cancel(Cancel),
    #[prost(message, tag = "8")]
    ClipboardUpdate(ClipboardUpdate),
}

/// What an offer carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq, prost::Enumeration)]
#[repr(i32)]
pub enum OfferKind {
    File = 0,
    Text = 1,
    Link = 2,
}

/// "I want to send you this." Files follow on their own QUIC stream after an
/// accepting [`OfferReply`]; text and links travel inline in `text`.
#[derive(Clone, PartialEq, Message)]
pub struct Offer {
    #[prost(uint64, tag = "1")]
    pub id: u64,
    #[prost(enumeration = "OfferKind", tag = "2")]
    pub kind: i32,
    /// Display name only; the receiver sanitises it before touching disk.
    #[prost(string, tag = "3")]
    pub name: String,
    #[prost(uint64, tag = "4")]
    pub size: u64,
    #[prost(string, tag = "5")]
    pub mime: String,
    #[prost(string, tag = "6")]
    pub text: String,
}

#[derive(Clone, PartialEq, Message)]
pub struct OfferReply {
    #[prost(uint64, tag = "1")]
    pub id: u64,
    #[prost(bool, tag = "2")]
    pub accept: bool,
}

/// Sent after the last file byte; the receiver compares the hash before
/// moving the file into place.
#[derive(Clone, PartialEq, Message)]
pub struct TransferDone {
    #[prost(uint64, tag = "1")]
    pub id: u64,
    #[prost(bytes = "vec", tag = "2")]
    pub sha256: Vec<u8>,
}

#[derive(Clone, PartialEq, Message)]
pub struct Cancel {
    #[prost(uint64, tag = "1")]
    pub id: u64,
}

/// Automatic Universal Clipboard sync. `hash` lets the receiver ignore an
/// update that only echoes what it just sent (no ping-pong loops).
#[derive(Clone, PartialEq, Message)]
pub struct ClipboardUpdate {
    #[prost(string, tag = "1")]
    pub text: String,
    #[prost(bytes = "vec", tag = "2")]
    pub hash: Vec<u8>,
    /// Marked private where it was copied (password, one-time code): the
    /// receiver keeps it out of clipboard history and hides it in notifications.
    #[prost(bool, tag = "3")]
    pub sensitive: bool,
}

/// First frame on a file data stream: which accepted offer these bytes belong to.
#[derive(Clone, PartialEq, Message)]
pub struct FileHeader {
    #[prost(uint64, tag = "1")]
    pub id: u64,
}

impl From<ControlMsg> for Control {
    fn from(msg: ControlMsg) -> Self {
        Self { msg: Some(msg) }
    }
}

/// Liveness ping; either side sends one every few seconds.
#[derive(Clone, PartialEq, Message)]
pub struct Heartbeat {
    #[prost(uint64, tag = "1")]
    pub seq: u64,
}

/// Clipboard text sent on an explicit user action.
#[derive(Clone, PartialEq, Message)]
pub struct ClipboardText {
    #[prost(string, tag = "1")]
    pub text: String,
    /// Sender marked it sensitive (password, OTP): receiver must not show a
    /// preview or keep it in history.
    #[prost(bool, tag = "2")]
    pub sensitive: bool,
}

/// Envelope for every message on a pairing (`pb-pair/0`) stream.
#[derive(Clone, PartialEq, Message)]
pub struct Pair {
    #[prost(oneof = "PairMsg", tags = "1, 2, 3, 4")]
    pub msg: Option<PairMsg>,
}

#[derive(Clone, PartialEq, Oneof)]
pub enum PairMsg {
    /// Joiner -> host: one-time token from the pairing code and a commitment
    /// `SHA-256("phone-bridge/commit/v0" || nonce)` to the joiner's nonce.
    #[prost(message, tag = "1")]
    Start(PairStart),
    /// Host -> joiner: host nonce, sent only after the commitment arrived.
    #[prost(message, tag = "2")]
    Nonce(PairNonce),
    /// Joiner -> host: the committed nonce.
    #[prost(message, tag = "3")]
    Reveal(PairReveal),
    /// Either side: the user's decision after comparing the SAS.
    #[prost(message, tag = "4")]
    Confirm(PairConfirm),
}

impl From<PairMsg> for Pair {
    fn from(msg: PairMsg) -> Self {
        Self { msg: Some(msg) }
    }
}

#[derive(Clone, PartialEq, Message)]
pub struct PairStart {
    #[prost(bytes = "vec", tag = "1")]
    pub token: Vec<u8>,
    #[prost(bytes = "vec", tag = "2")]
    pub commitment: Vec<u8>,
    #[prost(string, tag = "3")]
    pub device_name: String,
    /// Joiner's direct addresses so the host can dial it later.
    #[prost(string, repeated, tag = "4")]
    pub addrs: Vec<String>,
}

#[derive(Clone, PartialEq, Message)]
pub struct PairNonce {
    #[prost(bytes = "vec", tag = "1")]
    pub nonce: Vec<u8>,
    #[prost(string, tag = "2")]
    pub device_name: String,
}

#[derive(Clone, PartialEq, Message)]
pub struct PairReveal {
    #[prost(bytes = "vec", tag = "1")]
    pub nonce: Vec<u8>,
}

#[derive(Clone, PartialEq, Message)]
pub struct PairConfirm {
    #[prost(bool, tag = "1")]
    pub accepted: bool,
}

/// What the host shows (as QR, or text for now) and the joiner reads:
/// `pb1:<host id hex>:<token hex>:<addr>,<addr>`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PairingCode {
    pub host_id: [u8; 32],
    pub token: [u8; 16],
    pub addrs: Vec<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct BadPairingCode;

impl fmt::Display for BadPairingCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("not a Phone Bridge pairing code")
    }
}

impl std::error::Error for BadPairingCode {}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex<const N: usize>(s: &str) -> Result<[u8; N], BadPairingCode> {
    if s.len() != 2 * N || !s.is_ascii() {
        return Err(BadPairingCode);
    }
    let mut out = [0u8; N];
    for (i, b) in out.iter_mut().enumerate() {
        *b = u8::from_str_radix(&s[2 * i..2 * i + 2], 16).map_err(|_| BadPairingCode)?;
    }
    Ok(out)
}

impl fmt::Display for PairingCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "pb1:{}:{}:{}",
            hex(&self.host_id),
            hex(&self.token),
            self.addrs.join(",")
        )
    }
}

impl std::str::FromStr for PairingCode {
    type Err = BadPairingCode;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.len() > 2048 {
            return Err(BadPairingCode);
        }
        let rest = s.trim().strip_prefix("pb1:").ok_or(BadPairingCode)?;
        let mut parts = rest.splitn(3, ':');
        let host_id = unhex::<32>(parts.next().ok_or(BadPairingCode)?)?;
        let token = unhex::<16>(parts.next().ok_or(BadPairingCode)?)?;
        let addrs = parts
            .next()
            .unwrap_or("")
            .split(',')
            .filter(|a| !a.is_empty())
            .map(str::to_owned)
            .collect();
        Ok(Self {
            host_id,
            token,
            addrs,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{decode_frame, encode_frame};

    #[test]
    fn control_envelope_round_trips_each_variant() {
        let msgs = [
            ControlMsg::Hello(Hello::new("PC", vec!["clipboard".into()])),
            ControlMsg::Heartbeat(Heartbeat { seq: 7 }),
            ControlMsg::ClipboardText(ClipboardText {
                text: "héllo".into(),
                sensitive: true,
            }),
            ControlMsg::Offer(Offer {
                id: 9,
                kind: OfferKind::File as i32,
                name: "a.jpg".into(),
                size: 1 << 33,
                mime: "image/jpeg".into(),
                text: String::new(),
            }),
            ControlMsg::OfferReply(OfferReply {
                id: 9,
                accept: true,
            }),
            ControlMsg::TransferDone(TransferDone {
                id: 9,
                sha256: vec![7; 32],
            }),
            ControlMsg::Cancel(Cancel { id: 9 }),
            ControlMsg::ClipboardUpdate(ClipboardUpdate {
                text: "x".into(),
                hash: vec![1; 32],
                sensitive: true,
            }),
        ];
        for m in msgs {
            let bytes = encode_frame(&Control::from(m.clone())).unwrap();
            let (got, _): (Control, usize) = decode_frame(&bytes).unwrap().unwrap();
            assert_eq!(got.msg, Some(m));
        }
    }

    #[test]
    fn empty_envelope_decodes_to_none() {
        let bytes = encode_frame(&Control { msg: None }).unwrap();
        let (got, _): (Control, usize) = decode_frame(&bytes).unwrap().unwrap();
        assert_eq!(got.msg, None);
    }

    #[test]
    fn pair_envelope_round_trips() {
        let m = PairMsg::Start(PairStart {
            token: vec![1; 16],
            commitment: vec![2; 32],
            device_name: "Phone".into(),
            addrs: vec!["10.0.0.2:47100".into()],
        });
        let bytes = encode_frame(&Pair::from(m.clone())).unwrap();
        let (got, _): (Pair, usize) = decode_frame(&bytes).unwrap().unwrap();
        assert_eq!(got.msg, Some(m));
    }

    #[test]
    fn pairing_code_round_trips() {
        let code = PairingCode {
            host_id: [0xab; 32],
            token: [0x01; 16],
            addrs: vec!["10.107.168.101:47200".into(), "[fe80::1]:47200".into()],
        };
        let text = code.to_string();
        assert!(text.starts_with("pb1:abab"));
        assert_eq!(text.parse::<PairingCode>(), Ok(code));
    }

    #[test]
    fn pairing_code_without_addresses_is_allowed() {
        let text = format!("pb1:{}:{}:", "cd".repeat(32), "ef".repeat(16));
        assert_eq!(
            text.parse::<PairingCode>().unwrap().addrs,
            Vec::<String>::new()
        );
    }

    #[test]
    fn bad_pairing_codes_are_rejected() {
        let good_id = "cd".repeat(32);
        let good_tok = "ef".repeat(16);
        for bad in [
            String::new(),
            "pb2:x".into(),
            format!("pb1:{good_id}"),
            format!("pb1:{}:{good_tok}:", "cd".repeat(31)),
            format!("pb1:{good_id}:{}:", "zz".repeat(16)),
            format!("pb1:{good_id}:{good_tok}:{}", "a".repeat(3000)),
        ] {
            assert_eq!(bad.parse::<PairingCode>(), Err(BadPairingCode), "{bad:.40}");
        }
    }
}
