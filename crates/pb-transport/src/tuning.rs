//! QUIC transport knobs. [`Tuning::default`] is what the apps use; the bench
//! tool (`tools/pb-bench`) sweeps the others to measure what limits a
//! transfer on a given link (flow control, congestion control, CPU, MTU).

use std::sync::Arc;

use iroh::endpoint::{MtuDiscoveryConfig, QuicTransportConfig, VarInt};

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
}

/// Per-stream receive window the apps use. One file is one stream, so a
/// file's rate is bounded by window / round trip; the library default
/// (1.25 MB) capped transfers at 15-18 MB/s on a 5 GHz hotspot, where
/// round trips under load reach 50-100 ms. 4 MiB measured +34% PC -> phone
/// and +59% phone -> PC there; 8-16 MiB only added queueing (bench sweep,
/// docs/DECISIONS.md). Memory is used only while received data waits.
pub const STREAM_WINDOW: u64 = 4 * 1024 * 1024;

impl Default for Tuning {
    fn default() -> Self {
        Self {
            stream_window: Some(STREAM_WINDOW),
            conn_window: None,
            send_window: None,
            congestion: Congestion::Cubic,
            gso: !cfg!(target_os = "android"),
            mtu_max: None,
        }
    }
}

fn varint(v: u64) -> VarInt {
    VarInt::from_u64(v).unwrap_or(VarInt::MAX)
}

impl Tuning {
    pub fn transport_config(&self) -> QuicTransportConfig {
        let mut b = QuicTransportConfig::builder()
            .enable_segmentation_offload(self.gso)
            .keep_alive_interval(HEARTBEAT);
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
