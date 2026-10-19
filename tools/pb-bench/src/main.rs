//! pb-bench: measures what limits a transfer between this PC and the phone.
//!
//! The phone runs the listening side (no Windows firewall rule needed: the PC
//! always starts the conversation, replies come back on the same flow).
//!
//!   pb-bench server [--port 47300] [knobs]            QUIC sink/source (phone)
//!   pb-bench client <id> <ip:port> [--dir up|down] [--mb 200] [--streams 1] [knobs]
//!   pb-bench udp-peer [--port 47301]                  raw UDP peer (phone)
//!   pb-bench udp-test <ip:port> [--rates 100,200,...] [--secs 3] [--size 1452]
//!
//! Knobs (both ends; windows act on the receiver, congestion control and GSO
//! on the sender): --stream-window 8M --conn-window 32M --send-window 32M
//! --cc cubic|bbr3|newreno --gso on|off --mtu-max 1472 [--eco] (Windows: run
//! under EcoQoS like the tray).
//!
//! `up` = PC sends to phone, `down` = phone sends to PC. Output is one
//! `key=value` line per run so a script can collect it.

use std::{
    net::{SocketAddr, UdpSocket},
    time::{Duration, Instant},
};

use iroh::{
    endpoint::{presets, Connection, PathStats},
    Endpoint, EndpointAddr, EndpointId, RelayMode, SecretKey, TransportAddr,
};
use pb_transport::tuning::Tuning;

const ALPN: &[u8] = b"pb-bench/0";
const CHUNK: usize = 256 * 1024;
const MIB: u64 = 1024 * 1024;

type Res<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

fn flag(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1).cloned())
}

/// "8M", "512k", "1250000".
fn size(v: &str) -> u64 {
    let (n, mul) = match v.as_bytes().last() {
        Some(b'M' | b'm') => (&v[..v.len() - 1], MIB),
        Some(b'K' | b'k') => (&v[..v.len() - 1], 1024),
        _ => (v, 1),
    };
    n.parse::<u64>().unwrap_or(0) * mul
}

fn tuning(args: &[String]) -> Tuning {
    let d = Tuning::default();
    Tuning {
        stream_window: flag(args, "--stream-window").map(|v| size(&v)),
        conn_window: flag(args, "--conn-window").map(|v| size(&v)),
        send_window: flag(args, "--send-window").map(|v| size(&v)),
        congestion: flag(args, "--cc")
            .and_then(|v| v.parse().ok())
            .unwrap_or(d.congestion),
        gso: flag(args, "--gso").map(|v| v == "on").unwrap_or(d.gso),
        mtu_max: flag(args, "--mtu-max").and_then(|v| v.parse().ok()),
    }
}

fn describe(t: &Tuning) -> String {
    format!(
        "sw={} cw={} snd={} cc={:?} gso={} mtu_max={}",
        t.stream_window.map_or("dflt".into(), |v| v.to_string()),
        t.conn_window.map_or("dflt".into(), |v| v.to_string()),
        t.send_window.map_or("dflt".into(), |v| v.to_string()),
        t.congestion,
        t.gso,
        t.mtu_max.map_or("dflt".into(), |v| v.to_string()),
    )
}

/// Process CPU time (user + kernel), seconds.
fn cpu_seconds() -> f64 {
    #[cfg(target_os = "linux")]
    {
        // /proc/self/stat fields 14 and 15: utime, stime in clock ticks (100 Hz).
        if let Ok(s) = std::fs::read_to_string("/proc/self/stat") {
            if let Some(rest) = s.rsplit(')').next() {
                let f: Vec<&str> = rest.split_whitespace().collect();
                let t = |i: usize| f.get(i).and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0);
                return (t(11) + t(12)) / 100.0;
            }
        }
        0.0
    }
    #[cfg(windows)]
    {
        pb_platform_win::power::process_cpu_seconds()
    }
    #[cfg(not(any(target_os = "linux", windows)))]
    0.0
}

async fn endpoint(port: u16, t: &Tuning) -> Res<Endpoint> {
    let mut seed = [0u8; 32];
    getrandom::fill(&mut seed).map_err(|e| e.to_string())?;
    let ep = Endpoint::builder(presets::Minimal)
        .secret_key(SecretKey::from_bytes(&seed))
        .alpns(vec![ALPN.to_vec()])
        .transport_config(t.transport_config())
        .relay_mode(RelayMode::Disabled)
        .bind_addr(SocketAddr::from(([0, 0, 0, 0], port)))?
        .bind()
        .await?;
    Ok(ep)
}

/// Stats of the path carrying data (the selected one).
fn path(conn: &Connection) -> PathStats {
    let paths = conn.paths();
    let mut best = None;
    for p in paths.iter() {
        if p.is_selected() {
            return p.stats();
        }
        best.get_or_insert(p.stats());
    }
    best.unwrap_or_default()
}

fn conn_summary(conn: &Connection) -> String {
    let p = path(conn);
    let s = conn.stats();
    format!(
        "cwnd_kb={} cong_events={} lost_pkts={} sent_pkts={} mtu={} rtt_ms={:.1} sdb_tx={} sdb_rx={} db_tx={} db_rx={} max_stream_data_rx={}",
        p.cwnd / 1024,
        p.congestion_events,
        s.lost_packets,
        s.udp_tx.datagrams,
        p.current_mtu,
        p.rtt.as_secs_f64() * 1000.0,
        s.frame_tx.stream_data_blocked,
        s.frame_rx.stream_data_blocked,
        s.frame_tx.data_blocked,
        s.frame_rx.data_blocked,
        s.frame_rx.max_stream_data,
    )
}

async fn server(args: &[String]) -> Res<()> {
    let port = flag(args, "--port")
        .and_then(|v| v.parse().ok())
        .unwrap_or(47300);
    let t = tuning(args);
    let ep = endpoint(port, &t).await?;
    println!("BENCH-SERVER id={} port={port} {}", ep.id(), describe(&t));
    while let Some(inc) = ep.accept().await {
        tokio::spawn(async move {
            let Ok(conn) = inc.await else { return };
            let cpu0 = cpu_seconds();
            serve(&conn).await;
            println!(
                "BENCH-SERVER-CONN cpu_s={:.2} {}",
                cpu_seconds() - cpu0,
                conn_summary(&conn)
            );
        });
    }
    Ok(())
}

async fn serve(conn: &Connection) {
    loop {
        tokio::select! {
            s = conn.accept_uni() => {
                let Ok(mut r) = s else { break };
                tokio::spawn(async move {
                    let mut buf = vec![0u8; CHUNK];
                    while let Ok(Some(_)) = r.read(&mut buf).await {}
                });
            }
            s = conn.accept_bi() => {
                let Ok((mut w, mut r)) = s else { break };
                tokio::spawn(async move {
                    let mut req = [0u8; 8];
                    if r.read_exact(&mut req).await.is_err() {
                        return;
                    }
                    let mut left = u64::from_le_bytes(req);
                    let buf = vec![0x5Au8; CHUNK];
                    while left > 0 {
                        let n = left.min(CHUNK as u64) as usize;
                        if w.write_all(&buf[..n]).await.is_err() {
                            return;
                        }
                        left -= n as u64;
                    }
                    let _ = w.finish();
                    let _ = w.stopped().await;
                });
            }
        }
    }
}

async fn client(args: &[String]) -> Res<()> {
    let id: EndpointId = args.get(2).ok_or("missing <id>")?.parse()?;
    let addr: SocketAddr = args.get(3).ok_or("missing <ip:port>")?.parse()?;
    let up = flag(args, "--dir").as_deref() != Some("down");
    let mb: u64 = flag(args, "--mb")
        .and_then(|v| v.parse().ok())
        .unwrap_or(200);
    let streams: u64 = flag(args, "--streams")
        .and_then(|v| v.parse().ok())
        .unwrap_or(1)
        .max(1);
    let t = tuning(args);
    let ep = endpoint(0, &t).await?;
    let conn = ep
        .connect(
            EndpointAddr::from_parts(id, [TransportAddr::Ip(addr)]),
            ALPN,
        )
        .await?;
    // Let the path settle (MTU discovery, RTT samples) before timing.
    tokio::time::sleep(Duration::from_millis(800)).await;
    let rtt_rest = path(&conn).rtt;

    // RTT under load, sampled while the transfer runs.
    let (stop_tx, mut stop_rx) = tokio::sync::oneshot::channel::<()>();
    let probe = conn.clone();
    let sampler = tokio::spawn(async move {
        let mut v = Vec::new();
        loop {
            tokio::select! {
                _ = &mut stop_rx => break,
                () = tokio::time::sleep(Duration::from_millis(100)) => v.push(path(&probe).rtt),
            }
        }
        v
    });

    let per = mb * MIB / streams;
    let cpu0 = cpu_seconds();
    let t0 = Instant::now();
    let mut tasks = Vec::new();
    for _ in 0..streams {
        let conn = conn.clone();
        tasks.push(tokio::spawn(async move {
            if up {
                let mut s = conn.open_uni().await?;
                let buf = vec![0x5Au8; CHUNK];
                let mut left = per;
                while left > 0 {
                    let n = left.min(CHUNK as u64) as usize;
                    s.write_all(&buf[..n]).await?;
                    left -= n as u64;
                }
                s.finish()?;
                // Resolves once the receiver has acknowledged every byte.
                let _ = s.stopped().await;
                Ok::<u64, Box<dyn std::error::Error + Send + Sync>>(per)
            } else {
                let (mut w, mut r) = conn.open_bi().await?;
                w.write_all(&per.to_le_bytes()).await?;
                w.finish()?;
                let mut buf = vec![0u8; CHUNK];
                let mut got = 0u64;
                while let Some(n) = r.read(&mut buf).await? {
                    got += n as u64;
                }
                Ok(got)
            }
        }));
    }
    let mut bytes = 0u64;
    for t in tasks {
        bytes += t.await??;
    }
    let secs = t0.elapsed().as_secs_f64();
    let cpu = cpu_seconds() - cpu0;
    let _ = stop_tx.send(());
    let rtts = sampler.await.unwrap_or_default();
    let (avg, max) = if rtts.is_empty() {
        (0.0, 0.0)
    } else {
        let ms: Vec<f64> = rtts.iter().map(|d| d.as_secs_f64() * 1000.0).collect();
        (
            ms.iter().sum::<f64>() / ms.len() as f64,
            ms.iter().cloned().fold(0.0, f64::max),
        )
    };
    println!(
        "BENCH dir={} mb={} streams={streams} MBps={:.1} secs={secs:.2} rtt_rest_ms={:.1} rtt_load_avg_ms={avg:.1} rtt_load_max_ms={max:.1} cpu_s={cpu:.2} {} {}",
        if up { "up" } else { "down" },
        bytes / MIB,
        bytes as f64 / MIB as f64 / secs,
        rtt_rest.as_secs_f64() * 1000.0,
        conn_summary(&conn),
        describe(&t),
    );
    conn.close(0u32.into(), b"done");
    ep.close().await;
    Ok(())
}

// --- Raw UDP: the path's capacity without any congestion control ---------
//
// Packets: b'R' rate_mbps:u32 secs:u32 size:u16  -> peer receives a blast
//          b'X' rate_mbps:u32 secs:u32 size:u16  -> peer sends a blast back
//          b'D' seq:u64 ...padding                 data
//          b'E' sent:u64                           end of a blast
//          b'S' count:u64 bytes:u64 span_us:u64    receiver's summary

fn udp_socket(port: u16) -> Res<UdpSocket> {
    use socket2::{Domain, Protocol, Socket, Type};
    let s = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
    // Large buffers: a full 64 KB default would drop packets at our end and
    // understate the path.
    let _ = s.set_recv_buffer_size(8 << 20);
    let _ = s.set_send_buffer_size(8 << 20);
    s.bind(&SocketAddr::from(([0, 0, 0, 0], port)).into())?;
    Ok(s.into())
}

/// Send `D` packets to `to` at `rate_mbps` for `secs`; returns packets sent.
fn blast(sock: &UdpSocket, to: SocketAddr, rate_mbps: u32, secs: u32, size: u16) -> u64 {
    let size = usize::from(size).max(9);
    let pps = f64::from(rate_mbps) * 1e6 / 8.0 / size as f64;
    let mut pkt = vec![0u8; size];
    pkt[0] = b'D';
    let start = Instant::now();
    let end = Duration::from_secs(u64::from(secs));
    let mut sent = 0u64;
    loop {
        let el = start.elapsed();
        if el >= end {
            break;
        }
        let due = (el.as_secs_f64() * pps) as u64;
        if sent >= due {
            std::hint::spin_loop();
            continue;
        }
        while sent < due {
            pkt[1..9].copy_from_slice(&sent.to_le_bytes());
            if sock.send_to(&pkt, to).is_err() {
                // Kernel queue full: count as sent-and-dropped at our end.
            }
            sent += 1;
        }
    }
    let mut e = [0u8; 9];
    e[0] = b'E';
    e[1..9].copy_from_slice(&sent.to_le_bytes());
    for _ in 0..5 {
        let _ = sock.send_to(&e, to);
        std::thread::sleep(Duration::from_millis(20));
    }
    sent
}

/// Count `D` packets until `E` or `timeout`; returns (count, bytes, span, sent-per-E).
fn sink(sock: &UdpSocket, timeout: Duration) -> (u64, u64, Duration, u64) {
    let _ = sock.set_read_timeout(Some(Duration::from_millis(500)));
    let mut buf = vec![0u8; 65536];
    let (mut count, mut bytes, mut sent) = (0u64, 0u64, 0u64);
    let (mut first, mut last) = (None, Instant::now());
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        match sock.recv_from(&mut buf) {
            Ok((n, _)) if n >= 9 && buf[0] == b'D' => {
                count += 1;
                bytes += n as u64;
                last = Instant::now();
                first.get_or_insert(last);
            }
            Ok((n, _)) if n >= 9 && buf[0] == b'E' => {
                sent = u64::from_le_bytes(buf[1..9].try_into().unwrap_or_default());
                break;
            }
            _ => {}
        }
    }
    let span = first.map_or(Duration::ZERO, |f| last.duration_since(f));
    (count, bytes, span, sent)
}

fn le_u32(b: &[u8]) -> u32 {
    u32::from_le_bytes(b.try_into().unwrap_or_default())
}

fn udp_peer(args: &[String]) -> Res<()> {
    let port = flag(args, "--port")
        .and_then(|v| v.parse().ok())
        .unwrap_or(47301);
    let sock = udp_socket(port)?;
    println!("UDP-PEER port={port}");
    let mut buf = vec![0u8; 65536];
    loop {
        let _ = sock.set_read_timeout(None);
        let Ok((n, from)) = sock.recv_from(&mut buf) else {
            continue;
        };
        if n < 11 {
            continue;
        }
        let (rate, secs) = (le_u32(&buf[1..5]), le_u32(&buf[5..9]));
        let size = u16::from_le_bytes([buf[9], buf[10]]);
        match buf[0] {
            b'R' => {
                let (count, bytes, span, sent) =
                    sink(&sock, Duration::from_secs(u64::from(secs) + 3));
                let mut s = vec![b'S'];
                s.extend_from_slice(&count.to_le_bytes());
                s.extend_from_slice(&bytes.to_le_bytes());
                s.extend_from_slice(&(span.as_micros() as u64).to_le_bytes());
                s.extend_from_slice(&sent.to_le_bytes());
                for _ in 0..3 {
                    let _ = sock.send_to(&s, from);
                }
            }
            b'X' => {
                std::thread::sleep(Duration::from_millis(100));
                blast(&sock, from, rate, secs, size);
            }
            _ => {}
        }
    }
}

fn udp_test(args: &[String]) -> Res<()> {
    let peer: SocketAddr = args.get(2).ok_or("missing <ip:port>")?.parse()?;
    let rates: Vec<u32> = flag(args, "--rates")
        .unwrap_or_else(|| "100,200,300,400,500,600,800".into())
        .split(',')
        .filter_map(|v| v.trim().parse().ok())
        .collect();
    let secs: u32 = flag(args, "--secs")
        .and_then(|v| v.parse().ok())
        .unwrap_or(3);
    let size: u16 = flag(args, "--size")
        .and_then(|v| v.parse().ok())
        .unwrap_or(1452);
    let sock = udp_socket(0)?;
    let ctl = |kind: u8, rate: u32| {
        let mut m = vec![kind];
        m.extend_from_slice(&rate.to_le_bytes());
        m.extend_from_slice(&secs.to_le_bytes());
        m.extend_from_slice(&size.to_le_bytes());
        m
    };
    for &rate in &rates {
        // Up: PC -> phone.
        let _ = sock.send_to(&ctl(b'R', rate), peer);
        std::thread::sleep(Duration::from_millis(200));
        let sent = blast(&sock, peer, rate, secs, size);
        let _ = sock.set_read_timeout(Some(Duration::from_secs(3)));
        let mut buf = [0u8; 64];
        let mut recv = 0u64;
        let mut span = 0f64;
        let mut rbytes = 0u64;
        let wait = Instant::now();
        while wait.elapsed() < Duration::from_secs(4) {
            if let Ok((n, _)) = sock.recv_from(&mut buf) {
                if n >= 25 && buf[0] == b'S' {
                    recv = u64::from_le_bytes(buf[1..9].try_into().unwrap_or_default());
                    rbytes = u64::from_le_bytes(buf[9..17].try_into().unwrap_or_default());
                    span =
                        u64::from_le_bytes(buf[17..25].try_into().unwrap_or_default()) as f64 / 1e6;
                    break;
                }
            }
        }
        let got_mbps = if span > 0.0 {
            rbytes as f64 * 8.0 / span / 1e6
        } else {
            0.0
        };
        println!(
            "UDP dir=up offered_mbps={rate} received_mbps={got_mbps:.0} loss_pct={:.1} sent={sent} recv={recv}",
            if sent > 0 { 100.0 * (1.0 - recv as f64 / sent as f64) } else { 0.0 }
        );
        std::thread::sleep(Duration::from_millis(300));

        // Down: phone -> PC.
        let _ = sock.send_to(&ctl(b'X', rate), peer);
        let (count, bytes, span, sent) = sink(&sock, Duration::from_secs(u64::from(secs) + 3));
        let secs_f = span.as_secs_f64();
        let got_mbps = if secs_f > 0.0 {
            bytes as f64 * 8.0 / secs_f / 1e6
        } else {
            0.0
        };
        println!(
            "UDP dir=down offered_mbps={rate} received_mbps={got_mbps:.0} loss_pct={:.1} sent={sent} recv={count}",
            if sent > 0 { 100.0 * (1.0 - count as f64 / sent as f64) } else { 0.0 }
        );
        std::thread::sleep(Duration::from_millis(300));
    }
    Ok(())
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    #[cfg(windows)]
    if args.iter().any(|a| a == "--eco") {
        let _ = pb_platform_win::power::set_eco_qos(true);
    }
    let r = match args.get(1).map(String::as_str) {
        Some("udp-peer") => udp_peer(&args),
        Some("udp-test") => udp_test(&args),
        Some(cmd @ ("server" | "client")) => tokio::runtime::Builder::new_multi_thread()
            // Same as the apps (TOKIO_WORKER_THREADS=2).
            .worker_threads(2)
            .enable_all()
            .build()
            .map_err(Into::into)
            .and_then(|rt| {
                rt.block_on(async {
                    if cmd == "server" {
                        server(&args).await
                    } else {
                        client(&args).await
                    }
                })
            }),
        _ => {
            eprintln!("usage: see the top of tools/pb-bench/src/main.rs");
            std::process::exit(2);
        }
    };
    if let Err(e) = r {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}
