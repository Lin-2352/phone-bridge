//! QUIC transport knobs. [`Tuning::default`] is what the apps use; the bench
//! tool (`tools/pb-bench`) sweeps the others to measure what limits a
//! transfer on a given link (flow control, congestion control, CPU, MTU).

use std::sync::Arc;

use std::time::Duration;

use iroh::endpoint::{IdleTimeout, MtuDiscoveryConfig, QuicTransportConfig, VarInt};

use crate::HEARTBEAT;

/// Congestion controller.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Congestion {
    Cubic,
    Bbr3,
    NewReno,
}

impl std::str::FromStr for Congestion {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "cubic" => Ok(Self::Cubic),
            "bbr3" | "bbr" => Ok(Self::Bbr3),
            "newreno" | "reno" => Ok(Self::NewReno),
            other => Err(format!("unknown congestion controller {other:?}")),
        }
    }
}

/// Transport settings for a node. `None` keeps the QUIC library default.
#[derive(Clone, Debug)]
pub struct Tuning {
    /// Per-stream receive window (bytes); bounds one file's rate to window / RTT.
    pub stream_window: Option<u64>,
    /// Connection-wide receive window (bytes).
    pub conn_window: Option<u64>,
    /// Bytes queued for sending across all streams.
    pub send_window: Option<u64>,
    pub congestion: Congestion,
    /// UDP segmentation offload (off on Android: docs/DECISIONS.md finding 1).
    pub gso: bool,
    /// Upper bound for path MTU discovery (UDP payload bytes).
    pub mtu_max: Option<u16>,
    /// Bytes of unsent datagrams QUIC keeps (oldest dropped first).
    pub datagram_send_buffer: usize,
}

/// Per-stream receive window the apps use. One file is one stream, so a
/// file's rate is bounded by window / round trip; the library default
/// (1.25 MB) capped transfers at 15-18 MB/s on a 5 GHz hotspot, where
/// round trips under load reach 50-100 ms. 4 MiB measured +34% PC -> phone
/// and +59% phone -> PC there; 8-16 MiB only added queueing (bench sweep,
/// docs/DECISIONS.md). Memory is used only while received data waits.
pub const STREAM_WINDOW: u64 = 4 * 1024 * 1024;

/// Unsent datagrams QUIC keeps, oldest dropped first. Only audio uses them;
/// this is about 100 ms of 48 kHz stereo. The library default (1 MiB, about
/// 5 s) would turn every Wi-Fi stall into seconds of late audio.
pub const DATAGRAM_SEND_BUFFER: usize = 20 * 1024;
/// What one Opus packet takes in the send buffer: about 400 bytes of audio at 160 kbit/s and
/// 20 ms frames, and the 8-byte packet header.
pub const OPUS_TYPICAL_PACKET: usize = 408;
/// The phone sends Opus, and 20 KiB of it is a full second. A Wi-Fi stall left that queued and the
/// sender then skipped the newest audio to keep the oldest, so what came after the stall was stale.
/// With room for about 140 ms (7 packets) QUIC drops the oldest by itself and what leaves is recent.
pub const OPUS_DATAGRAM_BUFFER: usize = 7 * OPUS_TYPICAL_PACKET;
/// How long a silent connection is kept before QUIC gives up on it. The library's 30 s ended a
/// share at 22:33:41 on 2026-11-05 during a Wi-Fi outage of about 45 s. Both ends send a keep-alive
/// every [`HEARTBEAT`], so a live link never comes near it.
pub const IDLE_TIMEOUT: Duration = Duration::from_secs(60);

/// The send buffer for this platform: the phone sends Opus, the PC sends uncompressed audio.
pub fn datagram_send_buffer_for(android: bool) -> usize {
    if android {
        OPUS_DATAGRAM_BUFFER
    } else {
        DATAGRAM_SEND_BUFFER
    }
}
/// Received datagrams waiting to be read, oldest dropped first.
pub const DATAGRAM_RECV_BUFFER: usize = 64 * 1024;

impl Default for Tuning {
    fn default() -> Self {
        Self {
            stream_window: Some(STREAM_WINDOW),
            // Bounds what one peer can make us buffer (streams x window).
            conn_window: Some(4 * STREAM_WINDOW),
            send_window: None,
            congestion: Congestion::Cubic,
            gso: !cfg!(target_os = "android"),
            mtu_max: None,
            datagram_send_buffer: datagram_send_buffer_for(cfg!(target_os = "android")),
        }
    }
}

fn varint(v: u64) -> VarInt {
    VarInt::from_u64(v).unwrap_or(VarInt::MAX)
}

impl Tuning {
    pub fn transport_config(&self) -> QuicTransportConfig {
        // A session uses one bidirectional stream plus one file or input
        // stream; a few more allow the bench's parallel streams.
        let mut b = QuicTransportConfig::builder()
            .enable_segmentation_offload(self.gso)
            .keep_alive_interval(HEARTBEAT)
            .max_concurrent_bidi_streams(4u32.into())
            .max_concurrent_uni_streams(4u32.into())
            .datagram_send_buffer_size(self.datagram_send_buffer)
            .max_idle_timeout(IdleTimeout::try_from(IDLE_TIMEOUT).ok())
            .datagram_receive_buffer_size(Some(DATAGRAM_RECV_BUFFER));
        if let Some(w) = self.stream_window {
            b = b.stream_receive_window(varint(w));
        }
        if let Some(w) = self.conn_window {
            b = b.receive_window(varint(w));
        }
        if let Some(w) = self.send_window {
            b = b.send_window(w);
        }
        b = match self.congestion {
            Congestion::Cubic => b,
            Congestion::Bbr3 => b.congestion_controller_factory(Arc::new(
                noq_proto::congestion::Bbr3Config::default(),
            )),
            Congestion::NewReno => b.congestion_controller_factory(Arc::new(
                noq_proto::congestion::NewRenoConfig::default(),
            )),
        };
        if let Some(m) = self.mtu_max {
            let mut mtu = MtuDiscoveryConfig::default();
            mtu.upper_bound(m);
            b = b.mtu_discovery_config(Some(mtu));
        }
        b.build()
    }
}
