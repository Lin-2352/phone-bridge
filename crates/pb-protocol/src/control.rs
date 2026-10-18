//! Control-stream messages (session and pairing) and the pairing code format.

use std::fmt;

use prost::{Message, Oneof};

use crate::Hello;

/// Largest clipboard text we send or accept in one message.
pub const MAX_CLIPBOARD_TEXT: usize = 60 * 1024;

/// Envelope for every message on a session control stream.
#[derive(Clone, PartialEq, Message)]
pub struct Control {
    #[prost(oneof = "ControlMsg", tags = "1, 2, 3")]
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
