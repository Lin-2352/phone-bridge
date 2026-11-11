//! Control-stream messages (session and pairing) and the pairing code format.

use std::fmt;

use prost::{Message, Oneof};

use crate::Hello;

/// Largest clipboard text we send or accept in one message.
pub const MAX_CLIPBOARD_TEXT: usize = 60 * 1024;

/// Largest PNG a clipboard image may be. It travels as a file stream, not in
/// a control frame.
pub const MAX_CLIPBOARD_IMAGE: usize = 10 * 1024 * 1024;

/// Envelope for every message on a session control stream.
#[derive(Clone, PartialEq, Message)]
pub struct Control {
    #[prost(
        oneof = "ControlMsg",
        tags = "1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29"
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
    /// receiver answers with `OfferReply { id, accept, ..Default::default() }`; if accepted, the
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
    /// `OfferReply { accept: false, ..Default::default() }`. Accepting also lets the PC's window
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
    /// PC -> phone: use your camera as a webcam.
    #[prost(message, tag = "21")]
    CameraStart(CameraStart),
    /// Phone -> PC: the camera session's codec and size.
    #[prost(message, tag = "22")]
    CameraAccept(CameraAccept),
    /// PC -> phone, mid-stream: switch lens.
    #[prost(message, tag = "23")]
    CameraControl(CameraControl),
    /// Phone -> PC: the lens now in use (after any switch, and at the start).
    #[prost(message, tag = "24")]
    CameraLens(CameraLens),
    /// Requester -> responder: start an audio session (`direction` says which
    /// side sends the PCM).
    #[prost(message, tag = "25")]
    AudioStart(AudioStart),
    /// Responder -> requester: accepted (PCM follows on a uni stream) or refused
    /// with a reason.
    #[prost(message, tag = "26")]
    AudioAccept(AudioAccept),
    /// Either side: gracefully stop the audio stream.
    #[prost(message, tag = "27")]
    AudioStop(AudioStop),
    /// PC -> phone: send what is on your clipboard now. Android lets only the focused app read it, so the phone
    /// opens a see-through window for a moment, then answers with a `ClipboardUpdate` (text) or a clipboard
    /// picture, or with a `ClipboardPullResult` saying why not. Older peers ignore it.
    #[prost(message, tag = "28")]
    ClipboardPull(ClipboardPull),
    /// Phone -> PC: the answer to a `ClipboardPull` that sent nothing (locked, switch off, nothing copied).
    #[prost(message, tag = "29")]
    ClipboardPullResult(ClipboardPullResult),
}

#[derive(Clone, PartialEq, Message)]
pub struct ClipboardPull {
    /// The PC also takes a picture if one is on the clipboard.
    #[prost(bool, tag = "1")]
    pub want_image: bool,
    /// Sent by the PC itself when its user comes back to it, not by a key press: the phone answers quietly
    /// (nothing is shown at the PC when there is nothing new or the phone cannot read right now).
    #[prost(bool, tag = "2")]
    pub auto: bool,
    /// How long ago the clipboard of the PC last changed, in milliseconds (0: not known). A phone clip copied
    /// longer ago than this is older than what the PC holds, so the phone does not send it.
    #[prost(uint64, tag = "3")]
    pub pc_changed_ago_ms: u64,
}

#[derive(Clone, PartialEq, Message)]
pub struct ClipboardPullResult {
    /// True when something was sent (the clip itself arrives as its own message).
    #[prost(bool, tag = "1")]
    pub ok: bool,
    /// Why nothing was sent, in words for the person at the PC.
    #[prost(string, tag = "2")]
    pub reason: String,
}

#[derive(Clone, PartialEq, Message)]
pub struct CameraStart {
    #[prost(uint64, tag = "1")]
    pub id: u64,
    /// Codecs the PC decodes (`pb_video::Codec` values), best first.
    #[prost(uint32, repeated, tag = "2")]
    pub codecs: Vec<u32>,
    /// 0 back, 1 front.
    #[prost(uint32, tag = "3")]
    pub lens: u32,
    /// The 16:9 size wanted (1920x1080 or 1280x720).
    #[prost(uint32, tag = "4")]
    pub width: u32,
    #[prost(uint32, tag = "5")]
    pub height: u32,
    #[prost(uint32, tag = "6")]
    pub fps: u32,
}

#[derive(Clone, PartialEq, Message)]
pub struct CameraAccept {
    #[prost(uint64, tag = "1")]
    pub id: u64,
    #[prost(uint32, tag = "2")]
    pub codec: u32,
    #[prost(uint32, tag = "3")]
    pub width: u32,
    #[prost(uint32, tag = "4")]
    pub height: u32,
    /// 0, 90, 180 or 270.
    #[prost(uint32, tag = "5")]
    pub rotation: u32,
    #[prost(uint32, tag = "6")]
    pub lens: u32,
}

#[derive(Clone, PartialEq, Message)]
pub struct CameraControl {
    #[prost(uint32, tag = "1")]
    pub lens: u32,
}

/// The lens the phone is using: 0 back, 1 front. On the wire exactly like
/// `CameraControl` (uint32, field 1); written by hand so that decoding
/// rejects any other value.
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub struct CameraLens {
    pub lens: u8,
}

impl Message for CameraLens {
    fn encode_raw(&self, buf: &mut impl prost::bytes::BufMut) {
        if self.lens != 0 {
            prost::encoding::uint32::encode(1, &u32::from(self.lens), buf);
        }
    }

    fn merge_field(
        &mut self,
        tag: u32,
        wire_type: prost::encoding::WireType,
        buf: &mut impl prost::bytes::Buf,
        ctx: prost::encoding::DecodeContext,
    ) -> Result<(), prost::DecodeError> {
        if tag != 1 {
            return prost::encoding::skip_field(wire_type, tag, buf, ctx);
        }
        let mut lens = 0u32;
        prost::encoding::uint32::merge(wire_type, &mut lens, buf, ctx)?;
        // Checked before narrowing, so 256 cannot pass as 0.
        if lens > 1 {
            // The only public way to make this error; deprecated in prost
            // 0.14 for removal in its next breaking release.
            #[allow(deprecated)]
            return Err(prost::DecodeError::new("camera lens above 1"));
        }
        self.lens = lens as u8;
        Ok(())
    }

    fn encoded_len(&self) -> usize {
        if self.lens != 0 {
            prost::encoding::uint32::encoded_len(1, &u32::from(self.lens))
        } else {
            0
        }
    }

    fn clear(&mut self) {
        self.lens = 0;
    }
}

/// Audio flows from the phone to the PC.
pub const AUDIO_PHONE_TO_PC: u32 = 0;
/// Audio flows from the PC to the phone.
pub const AUDIO_PC_TO_PHONE: u32 = 1;
/// The audio is what the device plays (playback capture or loopback).
pub const AUDIO_SOURCE_PLAYBACK: u32 = 0;
/// The audio is the microphone: only ever with [`AUDIO_PHONE_TO_PC`], and only
/// when the user chose the explicit microphone mode.
pub const AUDIO_SOURCE_MIC: u32 = 1;
/// Largest PCM frame on the audio stream, in bytes.
pub const AUDIO_MAX_FRAME: usize = 64 * 1024;
/// Raw 16-bit PCM.
pub const AUDIO_CODEC_PCM: u32 = 0;
/// One Opus packet per datagram, always 48 kHz. Raw stereo PCM needs about
/// 1.5 Mbit/s, and a phone can cap a background app far below that.
pub const AUDIO_CODEC_OPUS: u32 = 1;
/// The largest Opus packet of one frame (RFC 6716).
pub const AUDIO_MAX_OPUS_PACKET: usize = 1275;

/// Why an audio request was not accepted (`AudioAccept.reason`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum AudioReason {
    /// The user said no.
    Declined = 1,
    /// This device cannot share audio now (old Android, feature off).
    Unavailable = 2,
    /// An audio session in that direction is already running.
    Busy = 3,
    /// A field of the request was out of range.
    BadParams = 4,
    /// The capture permission was not granted.
    NoPermission = 5,
    /// The app playing the audio forbids capturing it.
    AppBlocksCapture = 6,
}

impl AudioReason {
    pub fn code(self) -> u32 {
        self as u32
    }

    /// Unknown codes (a newer peer) read as `Declined`.
    pub fn from_code(code: u32) -> Self {
        match code {
            2 => Self::Unavailable,
            3 => Self::Busy,
            4 => Self::BadParams,
            5 => Self::NoPermission,
            6 => Self::AppBlocksCapture,
            _ => Self::Declined,
        }
    }
}

/// A peer-supplied audio field outside its allowed range.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AudioParamError {
    SampleRate(u32),
    Channels(u32),
    Direction(u32),
    Source(u32),
    /// The microphone was requested in the PC to phone direction.
    MicWrongDirection,
    /// Not [`AUDIO_CODEC_PCM`] or [`AUDIO_CODEC_OPUS`], or Opus below 48 kHz.
    Codec(u32),
}

impl fmt::Display for AudioParamError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SampleRate(r) => write!(f, "audio sample rate {r} is not 44100 or 48000"),
            Self::Channels(c) => write!(f, "audio channel count {c} is not 1 or 2"),
            Self::Direction(d) => write!(f, "audio direction {d} is not 0 or 1"),
            Self::Source(s) => write!(f, "audio source {s} is not 0 or 1"),
            Self::MicWrongDirection => f.write_str("microphone audio only goes phone to PC"),
            Self::Codec(c) => write!(f, "audio codec {c} is not usable with this format"),
        }
    }
}

impl std::error::Error for AudioParamError {}

/// Sample rate and channel count must be 44100 or 48000 Hz and 1 or 2.
pub fn validate_audio_format(sample_rate: u32, channels: u32) -> Result<(), AudioParamError> {
    if sample_rate != 44_100 && sample_rate != 48_000 {
        return Err(AudioParamError::SampleRate(sample_rate));
    }
    if channels != 1 && channels != 2 {
        return Err(AudioParamError::Channels(channels));
    }
    Ok(())
}

/// A PCM frame is 1..=[`AUDIO_MAX_FRAME`] bytes and a whole number of
/// 16-bit sample groups (a multiple of `2 * channels`).
pub fn audio_frame_len_ok(len: usize, channels: u32) -> bool {
    let group = 2 * channels as usize;
    group != 0 && (1..=AUDIO_MAX_FRAME).contains(&len) && len.is_multiple_of(group)
}

/// Whether a packet's payload fits the session's codec: whole PCM samples, or
/// one Opus packet.
pub fn audio_payload_ok(codec: u32, len: usize, channels: u32) -> bool {
    if codec == AUDIO_CODEC_OPUS {
        (1..=AUDIO_MAX_OPUS_PACKET).contains(&len)
    } else {
        audio_frame_len_ok(len, channels)
    }
}

#[derive(Clone, PartialEq, Message)]
pub struct AudioStart {
    #[prost(uint64, tag = "1")]
    pub id: u64,
    /// Requested sample rate Hz (44100 or 48000).
    #[prost(uint32, tag = "2")]
    pub sample_rate: u32,
    /// 1 = mono, 2 = stereo.
    #[prost(uint32, tag = "3")]
    pub channels: u32,
    // Tag 4 was `bitrate_kbps` (raw PCM, it never meant anything): not reused.
    /// [`AUDIO_PHONE_TO_PC`] or [`AUDIO_PC_TO_PHONE`].
    #[prost(uint32, tag = "5")]
    pub direction: u32,
    /// [`AUDIO_SOURCE_PLAYBACK`] or [`AUDIO_SOURCE_MIC`] (phone to PC only).
    #[prost(uint32, tag = "6")]
    pub source: u32,
}

impl AudioStart {
    /// Check every field the peer supplied.
    pub fn validate(&self) -> Result<(), AudioParamError> {
        validate_audio_format(self.sample_rate, self.channels)?;
        if self.direction > AUDIO_PC_TO_PHONE {
            return Err(AudioParamError::Direction(self.direction));
        }
        if self.source > AUDIO_SOURCE_MIC {
            return Err(AudioParamError::Source(self.source));
        }
        if self.source == AUDIO_SOURCE_MIC && self.direction != AUDIO_PHONE_TO_PC {
            return Err(AudioParamError::MicWrongDirection);
        }
        Ok(())
    }
}

#[derive(Clone, PartialEq, Message)]
pub struct AudioAccept {
    #[prost(uint64, tag = "1")]
    pub id: u64,
    /// Actual sample rate the sending side will use.
    #[prost(uint32, tag = "2")]
    pub sample_rate: u32,
    /// Actual channels.
    #[prost(uint32, tag = "3")]
    pub channels: u32,
    /// False: the request was refused, `reason` says why (and the format
    /// fields are ignored). Absent (old peers) reads as refused.
    #[prost(bool, tag = "4")]
    pub accepted: bool,
    /// [`AudioReason`] code when `accepted` is false, else 0.
    #[prost(uint32, tag = "5")]
    pub reason: u32,
    /// What the datagrams carry: [`AUDIO_CODEC_PCM`] or [`AUDIO_CODEC_OPUS`].
    /// The sending side chooses.
    #[prost(uint32, tag = "6")]
    pub codec: u32,
}

impl AudioAccept {
    pub fn refused(id: u64, reason: AudioReason) -> Self {
        Self {
            id,
            sample_rate: 0,
            channels: 0,
            accepted: false,
            reason: reason.code(),
            codec: AUDIO_CODEC_PCM,
        }
    }

    /// Check the format of an accepting reply.
    pub fn validate(&self) -> Result<(), AudioParamError> {
        validate_audio_format(self.sample_rate, self.channels)?;
        let usable = self.codec == AUDIO_CODEC_PCM
            || (self.codec == AUDIO_CODEC_OPUS && self.sample_rate == 48_000);
        if !usable {
            return Err(AudioParamError::Codec(self.codec));
        }
        Ok(())
    }
}

#[derive(Clone, PartialEq, Message)]
pub struct AudioStop {
    #[prost(uint64, tag = "1")]
    pub id: u64,
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
    /// A PNG that the receiver puts on its clipboard instead of saving it.
    ClipboardImage = 3,
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
    /// Resumable transfers (capability `resume`): exactly 16 random bytes chosen by the sender and kept for every
    /// retry of this file. Empty from a peer without the capability, and for anything but a file.
    #[prost(bytes = "vec", tag = "7")]
    pub resume_token: Vec<u8>,
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
    /// Bytes of this file the receiver already holds (a kept partial), 0 when none. Only sent in answer to an
    /// offer that carried a `resume_token`.
    #[prost(uint64, tag = "4")]
    pub resume_from: u64,
    /// SHA-256 of those `resume_from` bytes (32 bytes), so the sender can check they are the same file prefix.
    #[prost(bytes = "vec", tag = "5")]
    pub prefix_sha256: Vec<u8>,
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
    /// Where in the file the bytes on this stream start: 0 (absent) for the whole file, or the `resume_from`
    /// the receiver offered after the sender checked the prefix hash.
    #[prost(uint64, tag = "2")]
    pub offset: u64,
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

/// Addresses kept from a pairing code.
pub const MAX_CODE_ADDRS: usize = 6;

#[derive(Debug, PartialEq, Eq)]
pub struct BadPairingCode;

impl fmt::Display for BadPairingCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("not a Relay pairing code")
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
        // Pasted text is not trusted: keep only real `ip:port` addresses
        // (canonical form, so no separator can ride along), at most six.
        let mut addrs: Vec<String> = Vec::new();
        for a in parts.next().unwrap_or("").split(',') {
            if let Ok(sa) = a.parse::<std::net::SocketAddr>() {
                if !addrs.contains(&sa.to_string()) && addrs.len() < MAX_CODE_ADDRS {
                    addrs.push(sa.to_string());
                }
            }
        }
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

    /// The offer as Relay 0.2.0 wrote it: fields 1 to 6 only.
    #[derive(Clone, PartialEq, Message)]
    struct OldOffer {
        #[prost(uint64, tag = "1")]
        id: u64,
        #[prost(string, tag = "3")]
        name: String,
        #[prost(uint64, tag = "4")]
        size: u64,
    }

    #[test]
    fn offer_without_resume_token_decodes_as_0_2_0() {
        let old = OldOffer {
            id: 5,
            name: "a.bin".into(),
            size: 10,
        };
        let new = Offer::decode(old.encode_to_vec().as_slice()).unwrap();
        assert_eq!((new.id, new.size), (5, 10));
        assert!(new.resume_token.is_empty(), "absent means no resume");
        // And a 0.2.0 reader ignores the field it does not know.
        let with_token = Offer {
            resume_token: vec![1; 16],
            ..new
        };
        let back = OldOffer::decode(with_token.encode_to_vec().as_slice()).unwrap();
        assert_eq!(back, old);
    }

    #[test]
    fn reply_and_header_resume_fields_are_optional_and_round_trip() {
        let none = OfferReply::decode(&[0x08, 0x07][..]).unwrap();
        assert_eq!((none.resume_from, none.prefix_sha256.len()), (0, 0));
        let r = OfferReply {
            id: 1,
            accept: true,
            resume_from: 4 << 30,
            prefix_sha256: vec![3; 32],
            ..Default::default()
        };
        assert_eq!(OfferReply::decode(r.encode_to_vec().as_slice()).unwrap(), r);
        let h = FileHeader {
            id: 1,
            offset: 4 << 30,
        };
        assert_eq!(FileHeader::decode(h.encode_to_vec().as_slice()).unwrap(), h);
        assert_eq!(FileHeader::decode(&[0x08, 0x01][..]).unwrap().offset, 0);
    }

    #[test]
    fn resume_is_advertised_now_and_no_longer_reserved() {
        assert!(crate::local_capabilities().contains(&crate::CAP_RESUME.to_string()));
        assert!(!crate::RESERVED_CAPS.contains(&crate::CAP_RESUME));
    }

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
                ..Default::default()
            }),
            ControlMsg::OfferReply(OfferReply {
                id: 9,
                accept: true,
                too_large: false,
                ..Default::default()
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
            ControlMsg::ClipboardPull(ClipboardPull {
                want_image: true,
                auto: false,
                pc_changed_ago_ms: 0,
            }),
            ControlMsg::ClipboardPullResult(ClipboardPullResult {
                ok: false,
                reason: "phone is locked".into(),
            }),
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
            ControlMsg::CameraStart(CameraStart {
                id: 6,
                codecs: vec![2, 3],
                lens: 1,
                width: 1920,
                height: 1080,
                fps: 30,
            }),
            ControlMsg::CameraAccept(CameraAccept {
                id: 6,
                codec: 2,
                width: 1920,
                height: 1080,
                rotation: 270,
                lens: 1,
            }),
            ControlMsg::CameraControl(CameraControl { lens: 0 }),
            ControlMsg::CameraLens(CameraLens { lens: 1 }),
            ControlMsg::AudioStart(AudioStart {
                id: 7,
                sample_rate: 48000,
                channels: 2,
                direction: AUDIO_PC_TO_PHONE,
                source: AUDIO_SOURCE_PLAYBACK,
            }),
            ControlMsg::AudioAccept(AudioAccept {
                id: 7,
                sample_rate: 48000,
                channels: 2,
                accepted: true,
                reason: 0,
                codec: AUDIO_CODEC_OPUS,
            }),
            ControlMsg::AudioAccept(AudioAccept::refused(7, AudioReason::Busy)),
            ControlMsg::AudioStop(AudioStop { id: 7 }),
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
    fn pairing_code_keeps_only_real_addresses_and_at_most_six() {
        let (id, tok) = ("cd".repeat(32), "ef".repeat(16));
        let planted = "ab".repeat(32);
        let text = format!(
            "pb1:{id}:{tok}:1.2.3.4:1\n{planted}\tMy Phone\t,junk,10.0.0.2:47100,[fe80::1]:5"
        );
        assert_eq!(
            text.parse::<PairingCode>().unwrap().addrs,
            vec!["10.0.0.2:47100".to_string(), "[fe80::1]:5".to_string()]
        );
        let many: Vec<String> = (1..=9).map(|i| format!("10.0.0.{i}:1")).collect();
        let text = format!("pb1:{id}:{tok}:{}", many.join(","));
        assert_eq!(
            text.parse::<PairingCode>().unwrap().addrs,
            many[..6].to_vec()
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

    /// A `Control` frame carrying `CameraLens` with `lens` as a raw varint.
    fn raw_camera_lens_frame(lens: u64) -> Vec<u8> {
        let mut inner = vec![0x08];
        prost::encoding::encode_varint(lens, &mut inner);
        // Field 24, wire type 2: tag bytes 0xC2 0x01.
        let mut body = vec![0xC2, 0x01, inner.len() as u8];
        body.extend(inner);
        let mut frame = (body.len() as u32).to_le_bytes().to_vec();
        frame.extend(body);
        frame
    }

    #[test]
    fn camera_lens_round_trips() {
        for lens in [0u8, 1] {
            let m = ControlMsg::CameraLens(CameraLens { lens });
            let bytes = encode_frame(&Control::from(m.clone())).unwrap();
            let (got, used): (Control, usize) = decode_frame(&bytes).unwrap().unwrap();
            assert_eq!(got.msg, Some(m.clone()));
            assert_eq!(used, bytes.len());
            // A sender that writes the default 0 explicitly is read the same.
            let (got, _): (Control, usize) = decode_frame(&raw_camera_lens_frame(lens.into()))
                .unwrap()
                .unwrap();
            assert_eq!(got.msg, Some(m));
        }
        // Field 24 holding a uint32 field 1, like `CameraControl`.
        let front = encode_frame(&Control::from(ControlMsg::CameraLens(CameraLens {
            lens: 1,
        })));
        assert_eq!(front.unwrap(), raw_camera_lens_frame(1));
    }

    #[test]
    fn camera_lens_rejects_a_lens_above_one() {
        // 256 and 300 would wrap to 0 and 44 if narrowed before the check.
        for lens in [2u64, 7, 256, 300, u64::from(u32::MAX)] {
            assert_eq!(
                decode_frame::<Control>(&raw_camera_lens_frame(lens)),
                Err(crate::FrameError::Decode),
                "lens {lens}"
            );
        }
    }

    #[test]
    fn camera_lens_rejects_a_truncated_frame() {
        let bytes = raw_camera_lens_frame(1);
        for cut in 0..bytes.len() {
            assert_eq!(
                decode_frame::<Control>(&bytes[..cut]),
                Ok(None),
                "cut {cut}"
            );
        }
        // The frame is complete but the message inside it ends early.
        for body in [&[0xC2u8, 0x01, 0x02, 0x08][..], &[0xC2, 0x01, 0x01, 0x08]] {
            let mut frame = (body.len() as u32).to_le_bytes().to_vec();
            frame.extend_from_slice(body);
            assert_eq!(
                decode_frame::<Control>(&frame),
                Err(crate::FrameError::Decode),
                "{body:?}"
            );
        }
    }

    #[test]
    fn an_unknown_control_tag_is_ignored_not_fatal() {
        // Field 999 (a message from a newer peer), wire type 2, two bytes.
        let mut body = Vec::new();
        prost::encoding::encode_varint((999 << 3) | 2, &mut body);
        body.extend([0x02, 0x08, 0x01]);
        let mut frame = (body.len() as u32).to_le_bytes().to_vec();
        frame.extend(body);
        let (got, used): (Control, usize) = decode_frame(&frame).unwrap().unwrap();
        // `Session::recv` skips an empty envelope and reads on.
        assert_eq!(got.msg, None);
        assert_eq!(used, frame.len());
    }

    #[test]
    fn camera_start_uses_field_21() {
        let c = Control::from(ControlMsg::CameraStart(CameraStart {
            id: 1,
            ..Default::default()
        }));
        let bytes = c.encode_to_vec();
        // Field 21, wire type 2: tag byte 0xAA 0x01.
        assert_eq!(&bytes[..2], &[0xAA, 0x01]);
    }

    #[test]
    fn a_clipboard_image_offer_is_kind_3_and_round_trips() {
        assert_eq!(OfferKind::ClipboardImage as i32, 3);
        let offer = Offer {
            id: 9,
            kind: OfferKind::ClipboardImage as i32,
            name: "clipboard.png".into(),
            size: 1234,
            mime: "image/png".into(),
            text: String::new(),
            ..Default::default()
        };
        let bytes = Control::from(ControlMsg::Offer(offer.clone())).encode_to_vec();
        let got = Control::decode(bytes.as_slice()).unwrap();
        assert_eq!(got.msg, Some(ControlMsg::Offer(offer)));
        assert_eq!(OfferKind::try_from(3), Ok(OfferKind::ClipboardImage));
    }

    #[test]
    fn the_image_cap_is_ten_mebibytes() {
        assert_eq!(MAX_CLIPBOARD_IMAGE, 10 * 1024 * 1024);
    }

    fn start(rate: u32, ch: u32, dir: u32, src: u32) -> AudioStart {
        AudioStart {
            id: 1,
            sample_rate: rate,
            channels: ch,
            direction: dir,
            source: src,
        }
    }

    #[test]
    fn audio_start_accepts_only_the_allowed_fields() {
        for rate in [44_100, 48_000] {
            for ch in [1, 2] {
                assert_eq!(start(rate, ch, 0, 0).validate(), Ok(()));
                assert_eq!(start(rate, ch, 1, 0).validate(), Ok(()));
            }
        }
        assert_eq!(start(48_000, 1, 0, AUDIO_SOURCE_MIC).validate(), Ok(()));
        assert_eq!(
            start(96_000, 2, 0, 0).validate(),
            Err(AudioParamError::SampleRate(96_000))
        );
        assert_eq!(
            start(0, 2, 0, 0).validate(),
            Err(AudioParamError::SampleRate(0))
        );
        assert_eq!(
            start(48_000, 0, 0, 0).validate(),
            Err(AudioParamError::Channels(0))
        );
        assert_eq!(
            start(48_000, 3, 0, 0).validate(),
            Err(AudioParamError::Channels(3))
        );
        assert_eq!(
            start(48_000, 2, 2, 0).validate(),
            Err(AudioParamError::Direction(2))
        );
        assert_eq!(
            start(48_000, 2, 0, 2).validate(),
            Err(AudioParamError::Source(2))
        );
        assert_eq!(
            start(48_000, 2, AUDIO_PC_TO_PHONE, AUDIO_SOURCE_MIC).validate(),
            Err(AudioParamError::MicWrongDirection)
        );
    }

    #[test]
    fn audio_frames_are_sized_and_aligned() {
        assert!(audio_frame_len_ok(1920, 2));
        assert!(audio_frame_len_ok(2, 1));
        assert!(audio_frame_len_ok(AUDIO_MAX_FRAME, 2));
        assert!(!audio_frame_len_ok(0, 2));
        assert!(!audio_frame_len_ok(AUDIO_MAX_FRAME + 4, 2));
        assert!(!audio_frame_len_ok(1, 1));
        assert!(!audio_frame_len_ok(6, 2), "not a whole stereo sample");
        assert!(!audio_frame_len_ok(4, 0));
    }

    #[test]
    fn a_payload_is_checked_by_its_codec() {
        assert!(audio_payload_ok(AUDIO_CODEC_OPUS, 1, 2), "any byte count");
        assert!(audio_payload_ok(AUDIO_CODEC_OPUS, 3, 2));
        assert!(audio_payload_ok(AUDIO_CODEC_OPUS, AUDIO_MAX_OPUS_PACKET, 2));
        assert!(!audio_payload_ok(AUDIO_CODEC_OPUS, 0, 2));
        assert!(!audio_payload_ok(
            AUDIO_CODEC_OPUS,
            AUDIO_MAX_OPUS_PACKET + 1,
            2
        ));
        assert!(audio_payload_ok(AUDIO_CODEC_PCM, 4, 2));
        assert!(
            !audio_payload_ok(AUDIO_CODEC_PCM, 3, 2),
            "whole samples only"
        );
    }

    #[test]
    fn an_accept_names_a_usable_codec() {
        let a = |sample_rate, codec| AudioAccept {
            id: 1,
            sample_rate,
            channels: 2,
            accepted: true,
            reason: 0,
            codec,
        };
        assert_eq!(a(48_000, AUDIO_CODEC_OPUS).validate(), Ok(()));
        assert_eq!(a(44_100, AUDIO_CODEC_PCM).validate(), Ok(()));
        assert_eq!(
            a(44_100, AUDIO_CODEC_OPUS).validate(),
            Err(AudioParamError::Codec(AUDIO_CODEC_OPUS)),
            "Opus has no 44.1 kHz"
        );
        assert_eq!(a(48_000, 9).validate(), Err(AudioParamError::Codec(9)));
    }

    #[test]
    fn audio_reason_codes_round_trip_and_unknown_reads_as_declined() {
        for r in [
            AudioReason::Declined,
            AudioReason::Unavailable,
            AudioReason::Busy,
            AudioReason::BadParams,
            AudioReason::NoPermission,
            AudioReason::AppBlocksCapture,
        ] {
            assert_eq!(AudioReason::from_code(r.code()), r);
        }
        assert_eq!(AudioReason::from_code(0), AudioReason::Declined);
        assert_eq!(AudioReason::from_code(99), AudioReason::Declined);
    }

    #[test]
    fn an_accept_without_the_flag_reads_as_refused() {
        // An accept as the pre-direction code sent it: id, rate, channels only.
        let mut old = Vec::new();
        prost::encoding::uint64::encode(1, &7, &mut old);
        prost::encoding::uint32::encode(2, &48_000, &mut old);
        prost::encoding::uint32::encode(3, &2, &mut old);
        let a = AudioAccept::decode(old.as_slice()).unwrap();
        assert!(!a.accepted);
    }

    #[test]
    fn an_unknown_tag_decodes_to_no_message() {
        let mut raw = Vec::new();
        prost::encoding::uint32::encode(99, &1, &mut raw);
        let c = Control::decode(raw.as_slice()).unwrap();
        assert_eq!(c.msg, None);
    }

    #[test]
    fn an_automatic_pull_carries_its_mark_and_the_age_of_the_pcs_own_copy() {
        let m = ControlMsg::ClipboardPull(ClipboardPull {
            want_image: true,
            auto: true,
            pc_changed_ago_ms: 4_200,
        });
        let bytes = encode_frame(&Control::from(m.clone())).unwrap();
        let (got, _): (Control, usize) = decode_frame(&bytes).unwrap().unwrap();
        assert_eq!(got.msg, Some(m));
    }

    /// A 0.3.0 PC sends only `want_image`: the new fields read as "manual, age unknown".
    #[test]
    fn a_pull_from_an_older_pc_reads_as_manual_with_no_age() {
        #[derive(Clone, PartialEq, prost::Message)]
        struct Old {
            #[prost(bool, tag = "1")]
            want_image: bool,
        }
        let old = prost::Message::encode_to_vec(&Old { want_image: true });
        let new: ClipboardPull = prost::Message::decode(old.as_slice()).unwrap();
        assert_eq!(
            new,
            ClipboardPull {
                want_image: true,
                auto: false,
                pc_changed_ago_ms: 0
            }
        );
    }
}
