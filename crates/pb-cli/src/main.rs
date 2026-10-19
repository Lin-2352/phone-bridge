//! `pb`: Phone Bridge command-line client (test harness until the tray app).
//!
//! pb [--dir DIR] [--port N] [--name NAME] <command>
//!   pair-host [--auto-accept]   show a pairing code, confirm the SAS, pin the peer
//!   join <code> [--auto-accept] pair with a device showing a code
//!   peers                       list paired devices
//!   unpair <id-prefix>          revoke a device
//!   listen                      accept sessions and print received clipboard text
//!   send-clip <text> [--to <id-prefix>] [--sensitive]
//!   send-file <path> [--to <id-prefix>]
//!   clip-update <text> [--to <id-prefix>]     Universal Clipboard update
//!   pair-host-with <addr>       pair-host, also offering <addr> (emulator: 10.0.2.2:PORT)
//!   set-addr <id-prefix> <addr> where to reach a peer (e.g. an emulator port forward)
//!   control <secs> [drag] [--to <id>]  take control and draw circles (drag: button held, then a wheel burst)
//!
//! `--auto-accept` skips the SAS question and exists only for automated tests.

use std::{
    io::Write,
    path::PathBuf,
    sync::{Arc, Mutex},
};

#[cfg(not(windows))]
use pb_identity::load_or_create_identity;
use pb_identity::{FilePeerStore, PairedPeer};
use pb_protocol::control::{ClipboardText, ControlMsg, PairingCode, MAX_CLIPBOARD_TEXT};
use pb_transport::{Node, NodeConfig, NodeEvents, Session, SharedStore};
use tokio::io::{AsyncBufReadExt, BufReader};

type Res<T> = Result<T, Box<dyn std::error::Error>>;

struct Opts {
    dir: PathBuf,
    port: u16,
    name: String,
    auto_accept: bool,
    sensitive: bool,
    to: Option<String>,
    cmd: Vec<String>,
}

fn parse() -> Res<Opts> {
    let default_dir = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("PhoneBridge")
        .join("cli");
    let mut o = Opts {
        dir: default_dir,
        port: 47200,
        name: std::env::var("COMPUTERNAME").unwrap_or_else(|_| "PC".into()),
        auto_accept: false,
        sensitive: false,
        to: None,
        cmd: Vec::new(),
    };
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        let mut val = || it.next().ok_or_else(|| format!("{a} needs a value"));
        match a.as_str() {
            "--dir" => o.dir = val()?.into(),
            "--port" => o.port = val()?.parse()?,
            "--name" => o.name = val()?,
            "--to" => o.to = Some(val()?),
            "--auto-accept" => o.auto_accept = true,
            "--sensitive" => o.sensitive = true,
            f if f.starts_with("--") => return Err(format!("unknown flag {f}").into()),
            _ => o.cmd.push(a),
        }
    }
    Ok(o)
}

fn find_peer(store: &SharedStore, prefix: Option<&str>) -> Res<PairedPeer> {
    let peers = store.lock().map_err(|_| "store poisoned")?.list();
    let matches: Vec<_> = peers
        .into_iter()
        .filter(|p| prefix.is_none_or(|x| p.id.to_string().starts_with(&x.to_lowercase())))
        .collect();
    match matches.as_slice() {
        [one] => Ok(one.clone()),
        [] => Err("no matching paired device (see `pb peers`)".into()),
        _ => Err("several paired devices match; use --to <id-prefix>".into()),
    }
}

async fn confirm(sas: &str, peer_name: &str, auto: bool) -> Res<bool> {
    println!("SAS={sas}");
    println!("Pairing with \"{peer_name}\". Does the other device show {sas}? [y/N]");
    if auto {
        println!("(auto-accept: test mode)");
        return Ok(true);
    }
    std::io::stdout().flush()?;
    let mut line = String::new();
    BufReader::new(tokio::io::stdin())
        .read_line(&mut line)
        .await?;
    Ok(line.trim().eq_ignore_ascii_case("y"))
}

fn print_session(s: &Session) {
    println!("SESSION peer={} name=\"{}\"", s.peer, s.peer_name);
}

async fn pump(mut s: Session, inbox: PathBuf) {
    print_session(&s);
    loop {
        match s.recv().await {
            Ok(Some(ControlMsg::ClipboardText(c))) if c.sensitive => {
                println!(
                    "CLIP from {} (sensitive, {} chars, not shown)",
                    s.peer_name,
                    c.text.chars().count()
                )
            }
            Ok(Some(ControlMsg::ClipboardText(c))) => {
                println!("CLIP from {}: {}", s.peer_name, c.text)
            }
            Ok(Some(ControlMsg::ClipboardUpdate(u))) => {
                println!("CLIPSYNC from {}: {}", s.peer_name, u.text)
            }
            Ok(Some(ControlMsg::Offer(o))) => {
                let started = std::time::Instant::now();
                match pb_transport::files::receive_offer(
                    &mut s,
                    o,
                    true,
                    &inbox,
                    pb_transport::files::DEFAULT_MAX_FILE,
                    None,
                )
                .await
                {
                    Ok(pb_transport::files::Received::File { path, size }) => {
                        let secs = started.elapsed().as_secs_f64();
                        println!(
                            "FILE from {}: {} ({size} bytes, {:.1} MB/s)",
                            s.peer_name,
                            path.display(),
                            size as f64 / 1e6 / secs.max(1e-3)
                        );
                    }
                    Ok(other) => println!("OFFER from {}: {other:?}", s.peer_name),
                    Err(e) => println!("OFFER from {} failed: {e}", s.peer_name),
                }
                break;
            }
            Ok(Some(_)) => {}
            Ok(None) => break,
            Err(e) => {
                println!("session with {} ended: {e}", s.peer_name);
                break;
            }
        }
    }
}

async fn run(o: Opts) -> Res<()> {
    std::fs::create_dir_all(&o.dir)?;
    // Seed sealed with DPAPI on Windows; a Phase 1 plain file is migrated.
    #[cfg(windows)]
    let identity = pb_identity::vault::open_identity(&o.dir, &pb_platform_win::Dpapi)?;
    #[cfg(not(windows))]
    let identity = load_or_create_identity(&o.dir.join("identity.key"))?;
    let store: SharedStore = Arc::new(Mutex::new(FilePeerStore::open(o.dir.join("peers.txt"))?));
    let cmd: Vec<&str> = o.cmd.iter().map(String::as_str).collect();
    let config = NodeConfig {
        device_name: o.name.clone(),
        port: Some(o.port),
        discovery_port: None,
        tuning: Default::default(),
    };
    let (node, mut ev): (Node, NodeEvents) = Node::bind(&identity, store.clone(), config).await?;
    println!(
        "ME={} fingerprint={}",
        node.peer_id(),
        node.peer_id().fingerprint()
    );

    match cmd.as_slice() {
        ["peers"] => {
            for p in node.paired_peers() {
                println!("{} \"{}\" {}", p.id, p.name, p.addrs.join(","));
            }
        }
        ["unpair", prefix] => {
            let p = find_peer(&store, Some(prefix))?;
            node.unpair(&p.id);
            println!("UNPAIRED {}", p.id);
        }
        ["pair-host"] => {
            let code = node.open_pairing(node.addrs())?;
            println!("CODE={code}");
            println!("Waiting up to 2 minutes for the other device to join...");
            let pending =
                tokio::time::timeout(pb_transport::pairing::PAIRING_TTL, ev.pairings.recv())
                    .await
                    .map_err(|_| "pairing code expired")?
                    .ok_or("node stopped")?;
            let ok = confirm(&pending.sas, &pending.peer_name, o.auto_accept).await?;
            let peer = pending.decide(ok).await?;
            println!(
                "PAIRED {} \"{}\" {}",
                peer.id,
                peer.name,
                peer.addrs.join(",")
            );
        }
        ["pair-host-with", extra] => {
            // Test harness: also offer an address the joiner can reach (an
            // emulator reaches the host at 10.0.2.2).
            let mut addrs = node.addrs();
            addrs.push((*extra).to_owned());
            let code = node.open_pairing(addrs)?;
            println!("CODE={code}");
            let pending =
                tokio::time::timeout(pb_transport::pairing::PAIRING_TTL, ev.pairings.recv())
                    .await
                    .map_err(|_| "pairing code expired")?
                    .ok_or("node stopped")?;
            let ok = confirm(&pending.sas, &pending.peer_name, o.auto_accept).await?;
            let peer = pending.decide(ok).await?;
            println!("PAIRED {} \"{}\"", peer.id, peer.name);
        }
        ["set-addr", prefix, addr] => {
            // Test harness: where to reach a peer (e.g. an emulator port forward).
            let mut p = find_peer(&store, Some(prefix))?;
            p.addrs = vec![(*addr).to_owned()];
            store.lock().map_err(|_| "store")?.pin(p.clone());
            println!("ADDR {} {}", p.id, addr);
        }
        ["control", secs] | ["control", secs, "drag"] => {
            let drag = cmd.len() == 3;
            // Test harness: take control and draw circles like a 250 Hz mouse.
            let secs: f64 = secs.parse()?;
            let peer = find_peer(&store, o.to.as_deref())?;
            let t0 = std::time::Instant::now();
            let input = node.open_input(&peer.id).await?;
            println!("CONTROL accepted after {:.1}s", t0.elapsed().as_secs_f64());
            if drag {
                input.send(pb_protocol::InputEvent::MouseButton {
                    button: 0,
                    pressed: true,
                });
            }
            let start = std::time::Instant::now();
            let (mut px, mut py) = (0i32, 0i32);
            let mut sent = 0u64;
            let mut ended = None;
            while start.elapsed().as_secs_f64() < secs {
                let t = start.elapsed().as_secs_f64();
                let (x, y) = ((t * 3.0).cos() * 300.0, (t * 3.0).sin() * 300.0);
                let (nx, ny) = (x as i32, y as i32);
                if !input.send(pb_protocol::InputEvent::MouseMove {
                    dx: (nx - px) as i16,
                    dy: (ny - py) as i16,
                }) || !input.is_open()
                {
                    ended = Some(t);
                    break;
                }
                (px, py) = (nx, ny);
                sent += 1;
                tokio::time::sleep(std::time::Duration::from_millis(4)).await;
            }
            if drag {
                input.send(pb_protocol::InputEvent::MouseButton {
                    button: 0,
                    pressed: false,
                });
                // A burst of wheel notches: one continuous scroll on the phone.
                for _ in 0..6 {
                    input.send(pb_protocol::InputEvent::Wheel { dx: 0, dy: -120 });
                    tokio::time::sleep(std::time::Duration::from_millis(40)).await;
                }
                tokio::time::sleep(std::time::Duration::from_millis(800)).await;
            }
            match ended {
                Some(t) => println!("CONTROL ended by the phone after {t:.1}s ({sent} moves sent)"),
                None => println!("CONTROL done ({sent} moves in {secs}s)"),
            }
            input.stop();
        }
        ["join", code] => {
            let code: PairingCode = code.parse()?;
            let joining = node.join(&code).await?;
            let ok = confirm(&joining.sas, &joining.peer_name, o.auto_accept).await?;
            let peer = joining.decide(ok).await?;
            println!("PAIRED {} \"{}\"", peer.id, peer.name);
        }
        ["listen"] => {
            println!("LISTENING on port {}", node.port());
            while let Some(s) = ev.sessions.recv().await {
                tokio::spawn(pump(s, o.dir.join("inbox")));
            }
        }
        ["send-file", path] => {
            let peer = find_peer(&store, o.to.as_deref())?;
            let f = tokio::fs::File::open(path).await?;
            let size = f.metadata().await?.len();
            let name = std::path::Path::new(path)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "file".into());
            let started = std::time::Instant::now();
            let sha = node.send_file(&peer.id, &name, size, "", f, None).await?;
            let secs = started.elapsed().as_secs_f64();
            println!(
                "SENT FILE {name} {size} bytes in {secs:.2}s ({:.1} MB/s) sha256={}",
                size as f64 / 1e6 / secs.max(1e-3),
                sha.iter().map(|b| format!("{b:02x}")).collect::<String>()
            );
        }
        ["clip-update", text] => {
            let peer = find_peer(&store, o.to.as_deref())?;
            let s = node.connect(&peer.id).await?;
            s.send(ControlMsg::ClipboardUpdate(
                pb_protocol::control::ClipboardUpdate {
                    text: (*text).to_owned(),
                    hash: vec![],
                    sensitive: false,
                },
            ))
            .await?;
            s.finish().await;
            println!("SENT CLIPSYNC");
        }
        ["send-clip", text] => {
            if text.len() > MAX_CLIPBOARD_TEXT {
                return Err(format!("text is over {MAX_CLIPBOARD_TEXT} bytes").into());
            }
            let peer = find_peer(&store, o.to.as_deref())?;
            let s = node.connect(&peer.id).await?;
            print_session(&s);
            s.send(ControlMsg::ClipboardText(ClipboardText {
                text: (*text).to_owned(),
                sensitive: o.sensitive,
            }))
            .await?;
            println!("SENT {} bytes to \"{}\"", text.len(), peer.name);
            s.finish().await;
        }
        _ => return Err("usage: see the top of crates/pb-cli/src/main.rs".into()),
    }
    node.shutdown().await;
    Ok(())
}

fn main() {
    let result = parse().and_then(|o| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2) // resident footprint: idle default spawned 36 threads
            .enable_all()
            .build()?
            .block_on(run(o))
    });
    if let Err(e) = result {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}
