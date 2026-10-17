//! Shared server logic for the Phase 0 spikes (CLI binary and the Android FFI smoke test).
//!
//! Uses `presets::Minimal` on purpose: no n0 relay, no n0 DNS/pkarr publishing.
//! Nothing leaves the local link unless a relay is passed explicitly.

use std::{net::SocketAddr, sync::Mutex};

use iroh::{
    endpoint::{presets, Incoming, QuicTransportConfig},
    Endpoint, RelayMode, RelayUrl, SecretKey,
};
use n0_error::{Result, StdResultExt};
use sha2::{Digest, Sha256};

pub const ALPN: &[u8] = b"pb-spike/0";
pub const CHUNK: usize = 64 * 1024;

static EVENTS: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// Record a server event: printed, and buffered (last 200) for callers without a console.
pub fn log_event(msg: String) {
    println!("{msg}");
    if let Ok(mut e) = EVENTS.lock() {
        if e.len() >= 200 {
            e.remove(0);
        }
        e.push(msg);
    }
}

/// Take and clear buffered events.
pub fn drain_events() -> Vec<String> {
    EVENTS
        .lock()
        .map(|mut e| std::mem::take(&mut *e))
        .unwrap_or_default()
}

/// UDP segmentation offload (GSO) fails with EIO on some interfaces, notably the
/// phone's USB-tether `rndis0`; see docs/DECISIONS.md. `--no-gso` turns it off up front.
pub fn transport(no_gso: bool) -> QuicTransportConfig {
    QuicTransportConfig::builder()
        .enable_segmentation_offload(!no_gso)
        .build()
}

pub fn relay_mode(relay: &Option<RelayUrl>) -> RelayMode {
    match relay {
        Some(url) => RelayMode::custom([url.clone()]),
        None => RelayMode::Disabled,
    }
}

/// Deterministic, cheap-to-generate payload so both ends can hash it.
pub fn fill_pattern(buf: &mut [u8], counter: &mut u64) {
    for b in buf.iter_mut() {
        *counter = counter
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        *b = (*counter >> 56) as u8;
    }
}

/// Bind an endpoint and serve echo / datagram-echo / bulk streams in the background.
pub async fn start_server(
    port: Option<u16>,
    relay: Option<RelayUrl>,
    no_gso: bool,
) -> Result<Endpoint> {
    let mut builder = Endpoint::builder(presets::Minimal)
        .secret_key(SecretKey::from_bytes(&rand::random()))
        .alpns(vec![ALPN.to_vec()])
        .transport_config(transport(no_gso))
        .relay_mode(relay_mode(&relay));
    if let Some(port) = port {
        builder = builder
            .bind_addr(SocketAddr::from(([0, 0, 0, 0], port)))
            .anyerr()?;
    }
    let endpoint = builder.bind().await?;
    if relay.is_some() {
        endpoint.online().await;
    }
    tokio::spawn({
        let endpoint = endpoint.clone();
        async move {
            while let Some(incoming) = endpoint.accept().await {
                tokio::spawn(handle_connection(incoming));
            }
        }
    });
    Ok(endpoint)
}

async fn handle_connection(incoming: Incoming) {
    let conn = match incoming.await {
        Ok(c) => c,
        Err(e) => {
            log_event(format!("handshake failed: {e}"));
            return;
        }
    };
    log_event(format!("accepted {}", conn.remote_id()));
    let dg = conn.clone();
    tokio::spawn(async move {
        while let Ok(d) = dg.read_datagram().await {
            if dg.send_datagram(d).is_err() {
                break;
            }
        }
    });
    while let Ok((send, recv)) = conn.accept_bi().await {
        tokio::spawn(handle_stream(send, recv));
    }
    log_event("connection closed".into());
}

pub async fn handle_stream(
    mut send: iroh::endpoint::SendStream,
    mut recv: iroh::endpoint::RecvStream,
) {
    let r: Result<()> = async {
        let mut op = [0u8; 1];
        recv.read_exact(&mut op).await.anyerr()?;
        match op[0] {
            b'E' => {
                tokio::io::copy(&mut recv, &mut send).await.anyerr()?;
                send.finish().anyerr()?;
            }
            b'U' => {
                let mut hasher = Sha256::new();
                let mut buf = vec![0u8; CHUNK];
                let mut total = 0u64;
                loop {
                    let n = recv.read(&mut buf).await.anyerr()?.unwrap_or(0);
                    if n == 0 {
                        break;
                    }
                    hasher.update(&buf[..n]);
                    total += n as u64;
                }
                send.write_all(&total.to_le_bytes()).await.anyerr()?;
                send.write_all(&hasher.finalize()).await.anyerr()?;
                send.finish().anyerr()?;
            }
            b'D' => {
                let mut n = [0u8; 8];
                recv.read_exact(&mut n).await.anyerr()?;
                let mut remaining = u64::from_le_bytes(n);
                let (mut buf, mut ctr) = (vec![0u8; CHUNK], 1u64);
                while remaining > 0 {
                    let take = remaining.min(CHUNK as u64) as usize;
                    fill_pattern(&mut buf[..take], &mut ctr);
                    send.write_all(&buf[..take]).await.anyerr()?;
                    remaining -= take as u64;
                }
                send.finish().anyerr()?;
                // keep the stream alive until the peer has read everything
                let _ = recv.read_to_end(8).await;
            }
            _ => {}
        }
        Ok(())
    }
    .await;
    if let Err(e) = r {
        log_event(format!("stream error: {e}"));
    }
}
