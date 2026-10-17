//! Phase 0 spike: can iroh (QUIC) connect a Windows PC and an Android phone,
//! and what do latency / throughput / idle cost look like?
//!
//! Uses `presets::Minimal` on purpose: no n0 relay, no n0 DNS/pkarr publishing.
//! Nothing leaves the local link unless a relay is passed explicitly.
//!
//! serve   [--port N] [--relay URL]
//! connect --id ID --addr IP:PORT [--addr ...] [--relay URL] [--bulk-mib N] [--pings N]

use std::{
    collections::HashMap,
    net::SocketAddr,
    time::{Duration, Instant},
};

use iroh::{
    endpoint::{presets, Connection},
    Endpoint, EndpointAddr, EndpointId, RelayUrl, SecretKey, TransportAddr,
};
use iroh_spike::{fill_pattern, relay_mode, transport, ALPN, CHUNK};
use n0_error::{AnyError, Result, StdResultExt};
use sha2::{Digest, Sha256};

#[derive(Default)]
struct Args {
    port: Option<u16>,
    relay: Option<RelayUrl>,
    id: Option<EndpointId>,
    addrs: Vec<SocketAddr>,
    bulk_mib: u64,
    pings: u32,
    only: Option<String>,
    no_gso: bool,
}

fn parse_args(rest: &[String]) -> Result<Args> {
    let mut a = Args {
        bulk_mib: 64,
        pings: 200,
        ..Default::default()
    };
    let mut it = rest.iter();
    while let Some(flag) = it.next() {
        let mut val = || {
            it.next()
                .cloned()
                .ok_or_else(|| AnyError::from_display(format!("{flag} needs a value")))
        };
        match flag.as_str() {
            "--port" => a.port = Some(val()?.parse().anyerr()?),
            "--relay" => a.relay = Some(val()?.parse().anyerr()?),
            "--id" => a.id = Some(val()?.parse().anyerr()?),
            "--addr" => a.addrs.push(val()?.parse().anyerr()?),
            "--bulk-mib" => a.bulk_mib = val()?.parse().anyerr()?,
            "--pings" => a.pings = val()?.parse().anyerr()?,
            "--only" => a.only = Some(val()?),
            "--no-gso" => a.no_gso = true,
            other => return Err(AnyError::from_display(format!("unknown flag {other}"))),
        }
    }
    Ok(a)
}

#[tokio::main]
async fn main() -> Result<()> {
    // Set RUST_LOG (e.g. `noq_udp=debug,iroh=debug`) to see transport internals.
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_ansi(false)
        .init();
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let Some((mode, rest)) = argv.split_first() else {
        println!("usage: iroh-spike serve|connect [flags]");
        return Ok(());
    };
    let args = parse_args(rest)?;
    match mode.as_str() {
        "serve" => serve(args).await,
        "connect" => connect(args).await,
        _ => Err(AnyError::from_display("mode must be serve or connect")),
    }
}

async fn serve(args: Args) -> Result<()> {
    let endpoint = iroh_spike::start_server(args.port, args.relay, args.no_gso).await?;
    println!("ENDPOINT_ID={}", endpoint.id());
    println!("BOUND={:?}", endpoint.bound_sockets());
    println!("ADDR={:?}", endpoint.addr());
    std::future::pending::<()>().await;
    Ok(())
}

async fn connect(args: Args) -> Result<()> {
    let id = args
        .id
        .ok_or_else(|| AnyError::from_display("--id required"))?;
    let endpoint = Endpoint::builder(presets::Minimal)
        .secret_key(SecretKey::from_bytes(&rand::random()))
        .transport_config(transport(args.no_gso))
        .relay_mode(relay_mode(&args.relay))
        .bind()
        .await?;
    let mut parts: Vec<TransportAddr> = args.addrs.iter().copied().map(TransportAddr::Ip).collect();
    if let Some(url) = &args.relay {
        parts.push(TransportAddr::Relay(url.clone()));
    }
    let addr = EndpointAddr::from_parts(id, parts);

    let t = Instant::now();
    let conn = endpoint.connect(addr, ALPN).await?;
    println!("CONNECT_MS={:.1}", t.elapsed().as_secs_f64() * 1000.0);
    println!("MAX_DATAGRAM={:?}", conn.max_datagram_size());

    let run = |name: &str| {
        args.only
            .as_deref()
            .is_none_or(|o| o.split(',').any(|x| x == name))
    };
    if run("echo") {
        echo_test(&conn).await?;
    }
    if run("datagram") {
        datagram_test(&conn, args.pings).await?;
    }
    if run("upload") {
        upload_test(&conn, args.bulk_mib).await?;
    }
    if run("download") {
        download_test(&conn, args.bulk_mib).await?;
    }

    conn.close(0u32.into(), b"done");
    endpoint.close().await;
    Ok(())
}

async fn echo_test(conn: &Connection) -> Result<()> {
    let (mut send, mut recv) = conn.open_bi().await.anyerr()?;
    send.write_all(b"E").await.anyerr()?;
    send.write_all(b"hello phone bridge").await.anyerr()?;
    send.finish().anyerr()?;
    let got = recv.read_to_end(1024).await.anyerr()?;
    println!("ECHO_OK={}", got == b"hello phone bridge");
    Ok(())
}

async fn datagram_test(conn: &Connection, pings: u32) -> Result<()> {
    let mut sent: HashMap<u32, Instant> = HashMap::new();
    let mut rtts: Vec<f64> = Vec::new();
    for seq in 0..pings {
        sent.insert(seq, Instant::now());
        conn.send_datagram(seq.to_le_bytes().to_vec().into())
            .anyerr()?;
        // drain anything that came back while we wait the inter-ping gap
        let deadline = tokio::time::sleep(Duration::from_millis(5));
        tokio::pin!(deadline);
        loop {
            tokio::select! {
                _ = &mut deadline => break,
                d = conn.read_datagram() => {
                    let d = d.anyerr()?;
                    if let Ok(b) = <[u8; 4]>::try_from(&d[..]) {
                        if let Some(t0) = sent.remove(&u32::from_le_bytes(b)) {
                            rtts.push(t0.elapsed().as_secs_f64() * 1000.0);
                        }
                    }
                }
            }
        }
    }
    // final drain
    let _ = tokio::time::timeout(Duration::from_millis(300), async {
        while let Ok(d) = conn.read_datagram().await {
            if let Ok(b) = <[u8; 4]>::try_from(&d[..]) {
                if let Some(t0) = sent.remove(&u32::from_le_bytes(b)) {
                    rtts.push(t0.elapsed().as_secs_f64() * 1000.0);
                }
            }
        }
    })
    .await;
    rtts.sort_by(|a, b| a.total_cmp(b));
    let pct = |p: f64| {
        rtts.get(((rtts.len() as f64 - 1.0) * p) as usize)
            .copied()
            .unwrap_or(f64::NAN)
    };
    println!(
        "DATAGRAM sent={pings} recv={} loss={} rtt_ms p50={:.2} p95={:.2} max={:.2}",
        rtts.len(),
        pings as usize - rtts.len(),
        pct(0.5),
        pct(0.95),
        rtts.last().copied().unwrap_or(f64::NAN)
    );
    Ok(())
}

async fn upload_test(conn: &Connection, mib: u64) -> Result<()> {
    let total = mib * 1024 * 1024;
    let (mut send, mut recv) = conn.open_bi().await.anyerr()?;
    send.write_all(b"U").await.anyerr()?;
    let (mut buf, mut ctr) = (vec![0u8; CHUNK], 1u64);
    let mut hasher = Sha256::new();
    let t = Instant::now();
    let mut left = total;
    while left > 0 {
        let take = left.min(CHUNK as u64) as usize;
        fill_pattern(&mut buf[..take], &mut ctr);
        hasher.update(&buf[..take]);
        send.write_all(&buf[..take]).await.anyerr()?;
        left -= take as u64;
    }
    send.finish().anyerr()?;
    let reply = recv.read_to_end(64).await.anyerr()?;
    let secs = t.elapsed().as_secs_f64();
    let ok = reply.len() == 40
        && u64::from_le_bytes(reply[..8].try_into().unwrap()) == total
        && reply[8..] == hasher.finalize()[..];
    println!(
        "UPLOAD mib={mib} secs={secs:.2} MB/s={:.1} hash_ok={ok}",
        total as f64 / 1e6 / secs
    );
    Ok(())
}

async fn download_test(conn: &Connection, mib: u64) -> Result<()> {
    let total = mib * 1024 * 1024;
    let (mut send, mut recv) = conn.open_bi().await.anyerr()?;
    send.write_all(b"D").await.anyerr()?;
    send.write_all(&total.to_le_bytes()).await.anyerr()?;
    let (mut buf, mut ctr, mut expect_buf) = (vec![0u8; CHUNK], 1u64, vec![0u8; CHUNK]);
    let mut got = 0u64;
    let mut ok = true;
    let t = Instant::now();
    while got < total {
        let want = (total - got).min(CHUNK as u64) as usize;
        recv.read_exact(&mut buf[..want]).await.anyerr()?;
        fill_pattern(&mut expect_buf[..want], &mut ctr);
        ok &= buf[..want] == expect_buf[..want];
        got += want as u64;
    }
    let secs = t.elapsed().as_secs_f64();
    send.finish().anyerr()?;
    println!(
        "DOWNLOAD mib={mib} secs={secs:.2} MB/s={:.1} data_ok={ok}",
        total as f64 / 1e6 / secs
    );
    Ok(())
}
