//! Control-stream messages (session and pairing) and the pairing code format.

use std::fmt;

use prost::{Message, Oneof};

use crate::Hello;

/// Largest clipboard text we send or accept in one message.
pub const MAX_CLIPBOARD_TEXT: usize = 60 * 1024;

/// Envelope for every message on a session control stream.
#[derive(Clone, PartialEq, Message)]
pub struct Control {
    #[prost(
        oneof = "ControlMsg",
        tags = "1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20"
    )]
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
    /// The sender wants to control this device's pointer and keyboard. The
    /// receiver answers with `OfferReply { id, accept }`; if accepted, the
    /// sender opens one unidirectional stream of 8-byte `InputDatagram`
    /// records (see `pb_protocol::InputDatagram`) and the session lasts until
    /// that stream ends.
    #[prost(message, tag = "9")]
    InputStart(InputStart),
    /// Phone -> PC: a notification was posted or updated on the phone.
    #[prost(message, tag = "10")]
    Notification(Notification),
    /// Phone -> PC: an app icon, sent once per session before the first
    /// notification that names its hash.
    #[prost(message, tag = "11")]
    NotifyIcon(NotifyIcon),
    /// Phone -> PC: the notification is gone on the phone.
    #[prost(message, tag = "12")]
    NotifyRemoved(NotifyRemoved),
    /// Phone -> PC: every key active on the phone; the PC removes the rest.
    /// First message of every notify session.
    #[prost(message, tag = "13")]
    NotifySync(NotifySync),
    /// PC -> phone, once at tray start: answer with quiet copies of what is
    /// active, then `NotifySync`.
    #[prost(message, tag = "14")]
    NotifySyncRequest(NotifySyncRequest),
    /// PC -> phone: act on a notification; answered by `NotifyActionResult`.
    #[prost(message, tag = "15")]
    NotifyAction(NotifyAction),
    #[prost(message, tag = "16")]
    NotifyActionResult(NotifyActionResult),
    /// PC -> phone: show me your screen. Answered by `ScreenAccept`, or by
    /// `OfferReply { accept: false }`. Accepting also lets the PC's window
    /// drive the phone for this session (input on its own uni stream).
    #[prost(message, tag = "17")]
    ScreenStart(ScreenStart),
    /// Phone -> PC: accepted; video records follow on a uni stream.
    #[prost(message, tag = "18")]
    ScreenAccept(ScreenAccept),
    /// PC -> phone, every 100 ms: the newest frame presented.
    #[prost(message, tag = "19")]
    ScreenFeedback(ScreenFeedback),
    /// Window and session state, both ways (see the fields).
    #[prost(message, tag = "20")]
    ScreenState(ScreenState),
}

#[derive(Clone, PartialEq, Message)]
pub struct ScreenStart {
    #[prost(uint64, tag = "1")]
    pub id: u64,
    /// Long side of the video; 0 = native ("Sharp").
    #[prost(uint32, tag = "2")]
    pub max_long_side: u32,
    #[prost(uint32, tag = "3")]
    pub fps: u32,
    /// Codecs the PC decodes (`pb_video::Codec` values), best first.
    #[prost(uint32, repeated, tag = "4")]
    pub codecs: Vec<u32>,
    /// Dim the phone's backlight while mirroring.
    #[prost(bool, tag = "5")]
    pub dim: bool,
    #[prost(bool, tag = "6")]
    pub sharp: bool,
}

#[derive(Clone, PartialEq, Message)]
pub struct ScreenAccept {
    #[prost(uint64, tag = "1")]
    pub id: u64,
    #[prost(uint32, tag = "2")]
    pub codec: u32,
    #[prost(uint32, tag = "3")]
    pub width: u32,
    #[prost(uint32, tag = "4")]
    pub height: u32,
    #[prost(uint32, tag = "5")]
    pub rotation: u32,
}

#[derive(Clone, PartialEq, Message)]
pub struct ScreenFeedback {
    #[prost(int64, tag = "1")]
    pub presented_pts_us: i64,
}

#[derive(Clone, PartialEq, Message)]
pub struct ScreenState {
    /// PC -> phone: the window is visible (minimised = false: pause capture).
    #[prost(bool, tag = "1")]
    pub active: bool,
    /// PC -> phone: dim the phone's backlight.
    #[prost(bool, tag = "2")]
    pub dim: bool,
    /// Phone -> PC: something for the window to say ("Lower quality to save memory").
    #[prost(string, tag = "3")]
    pub notice: String,
    /// PC -> phone: decoding failed; switch to the PC's next codec.
    #[prost(bool, tag = "4")]
    pub codec_failed: bool,
}

/// Limits for notification messages; the sender trims to them so a whole
/// notification always fits one control frame.
pub const MAX_NOTIFY_KEY: usize = 256;
pub const MAX_NOTIFY_TITLE: usize = 512;
pub const MAX_NOTIFY_TEXT: usize = 4096;
pub const MAX_NOTIFY_LINES: usize = 5;
pub const MAX_NOTIFY_LINE: usize = 512;
pub const MAX_NOTIFY_PICTURE: usize = 48 * 1024;
pub const MAX_NOTIFY_ICON: usize = 16 * 1024;
pub const MAX_NOTIFY_SYNC_KEYS: usize = 200;
pub const MAX_REPLY_TEXT: usize = 4096;

/// A phone notification as the PC shows it.
#[derive(Clone, PartialEq, Message)]
pub struct Notification {
    /// The phone's stable key for this notification.
    #[prost(string, tag = "1")]
    pub key: String,
    #[prost(string, tag = "2")]
    pub package: String,
    #[prost(string, tag = "3")]
    pub app_name: String,
    /// Sender or title.
    #[prost(string, tag = "4")]
    pub title: String,
    #[prost(string, tag = "5")]
    pub text: String,
    /// When it was posted on the phone (Unix milliseconds).
    #[prost(int64, tag = "6")]
    pub when_ms: i64,
    /// Messages from one chat share this; the PC keeps one toast per chat.
    #[prost(string, tag = "7")]
    pub conversation: String,
    /// Latest messages of the conversation ("sender: text"), oldest first.
    #[prost(string, repeated, tag = "8")]
    pub lines: Vec<String>,
    /// Silent on the phone, arrived during its Do Not Disturb, or a re-send.
    #[prost(bool, tag = "9")]
    pub quiet: bool,
    /// The app hides it on the lock screen; only its public version is sent.
    #[prost(bool, tag = "10")]
    pub private: bool,
    #[prost(bool, tag = "11")]
    pub call: bool,
    /// A one-time code found in the text, else empty.
    #[prost(string, tag = "12")]
    pub otp: String,
    #[prost(message, repeated, tag = "13")]
    pub actions: Vec<NotifyActionDesc>,
    /// SHA-256 of the app icon PNG (see `NotifyIcon`), or empty.
    #[prost(bytes = "vec", tag = "14")]
    pub icon_hash: Vec<u8>,
    /// Avatar or photo preview: JPEG, at most `MAX_NOTIFY_PICTURE`.
    #[prost(bytes = "vec", tag = "15")]
    pub picture: Vec<u8>,
}

/// A button the app offers on its notification.
#[derive(Clone, PartialEq, Message)]
pub struct NotifyActionDesc {
    #[prost(string, tag = "1")]
    pub label: String,
    /// Takes typed text (an inline reply).
    #[prost(bool, tag = "2")]
    pub reply: bool,
}

#[derive(Clone, PartialEq, Message)]
pub struct NotifyIcon {
    #[prost(bytes = "vec", tag = "1")]
    pub hash: Vec<u8>,
    #[prost(bytes = "vec", tag = "2")]
    pub png: Vec<u8>,
}

#[derive(Clone, PartialEq, Message)]
pub struct NotifyRemoved {
    #[prost(string, tag = "1")]
    pub key: String,
}

#[derive(Clone, PartialEq, Message)]
pub struct NotifySync {
    #[prost(string, repeated, tag = "1")]
    pub keys: Vec<String>,
}

#[derive(Clone, PartialEq, Message)]
pub struct NotifySyncRequest {}

#[derive(Clone, Copy, Debug, PartialEq, Eq, prost::Enumeration)]
#[repr(i32)]
pub enum NotifyActionKind {
    Dismiss = 0,
    /// The app's button at `index`.
    Button = 1,
    /// Send `text` through the reply button at `index`.
    Reply = 2,
    /// Open the notification on the phone.
    Open = 3,
    /// Silence the ringing phone without declining the call.
    Silence = 4,
}

#[derive(Clone, PartialEq, Message)]
pub struct NotifyAction {
    #[prost(uint64, tag = "1")]
    pub id: u64,
    #[prost(string, tag = "2")]
    pub key: String,
    #[prost(enumeration = "NotifyActionKind", tag = "3")]
    pub kind: i32,
    #[prost(uint32, tag = "4")]
    pub index: u32,
    #[prost(string, tag = "5")]
    pub text: String,
}

#[derive(Clone, PartialEq, Message)]
pub struct NotifyActionResult {
    #[prost(uint64, tag = "1")]
    pub id: u64,
    #[prost(bool, tag = "2")]
    pub ok: bool,
    /// Why it failed, for the user ("That notification was cleared on the phone").
    #[prost(string, tag = "3")]
    pub reason: String,
}

#[derive(Clone, PartialEq, Message)]
pub struct InputStart {
    #[prost(uint64, tag = "1")]
    pub id: u64,
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
    /// Declined because the file is over the receiver's limit or free space
    /// (so the sender can say why). Older peers leave it false.
    #[prost(bool, tag = "3")]
    pub too_large: bool,
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
                too_large: false,
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
            ControlMsg::Notification(sample_notification()),
            ControlMsg::NotifyIcon(NotifyIcon {
                hash: vec![3; 32],
                png: vec![1, 2, 3],
            }),
            ControlMsg::NotifyRemoved(NotifyRemoved { key: "k1".into() }),
            ControlMsg::NotifySync(NotifySync {
                keys: vec!["k1".into(), "k2".into()],
            }),
            ControlMsg::NotifySyncRequest(NotifySyncRequest {}),
            ControlMsg::NotifyAction(NotifyAction {
                id: 4,
                key: "k1".into(),
                kind: NotifyActionKind::Reply as i32,
                index: 1,
                text: "on my way".into(),
            }),
            ControlMsg::NotifyActionResult(NotifyActionResult {
                id: 4,
                ok: false,
                reason: "gone".into(),
            }),
            ControlMsg::ScreenStart(ScreenStart {
                id: 5,
                max_long_side: 1920,
                fps: 30,
                codecs: vec![1, 2, 3],
                dim: true,
                sharp: false,
            }),
            ControlMsg::ScreenAccept(ScreenAccept {
                id: 5,
                codec: 2,
                width: 864,
                height: 1920,
                rotation: 0,
            }),
            ControlMsg::ScreenFeedback(ScreenFeedback {
                presented_pts_us: 33_333,
            }),
            ControlMsg::ScreenState(ScreenState {
                active: false,
                dim: true,
                notice: "Lower quality to save memory".into(),
                codec_failed: true,
            }),
        ];
        for m in msgs {
            let bytes = encode_frame(&Control::from(m.clone())).unwrap();
            let (got, _): (Control, usize) = decode_frame(&bytes).unwrap().unwrap();
            assert_eq!(got.msg, Some(m));
        }
    }

    fn sample_notification() -> Notification {
        Notification {
            key: "0|com.whatsapp|1|null|10123".into(),
            package: "com.whatsapp".into(),
            app_name: "WhatsApp".into(),
            title: "Mom".into(),
            text: "Dinner at 8?".into(),
            when_ms: 1_792_000_000_000,
            conversation: "Mom".into(),
            lines: vec!["Mom: Dinner at 8?".into()],
            quiet: true,
            private: false,
            call: false,
            otp: String::new(),
            actions: vec![NotifyActionDesc {
                label: "Reply".into(),
                reply: true,
            }],
            icon_hash: vec![3; 32],
            picture: vec![9; 10],
        }
    }

    #[test]
    fn notification_with_max_picture_fits_one_frame() {
        let n = Notification {
            key: "k".repeat(MAX_NOTIFY_KEY),
            package: "p".repeat(MAX_NOTIFY_TITLE),
            app_name: "a".repeat(MAX_NOTIFY_TITLE),
            title: "t".repeat(MAX_NOTIFY_TITLE),
            text: "x".repeat(MAX_NOTIFY_TEXT),
            conversation: "c".repeat(MAX_NOTIFY_TITLE),
            lines: vec!["l".repeat(MAX_NOTIFY_LINE); MAX_NOTIFY_LINES],
            otp: "12345678".into(),
            actions: (0..5)
                .map(|_| NotifyActionDesc {
                    label: "b".repeat(64),
                    reply: false,
                })
                .collect(),
            picture: vec![0xAB; MAX_NOTIFY_PICTURE],
            ..sample_notification()
        };
        let bytes = encode_frame(&Control::from(ControlMsg::Notification(n.clone()))).unwrap();
        let (got, _): (Control, usize) = decode_frame(&bytes).unwrap().unwrap();
        assert_eq!(got.msg, Some(ControlMsg::Notification(n)));
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
