//! `pb`: Phone Bridge command-line client (test harness until the tray app).
//!
//! pb [--dir DIR] [--port N] [--name NAME] <command>
//!   pair-host [--auto-accept]   show a pairing code, confirm the SAS, pin the peer
//!   join <code> [--auto-accept] pair with a device showing a code
//!   peers                       list paired devices
//!   unpair <id-prefix>          revoke a device
//!   listen                      accept sessions and print received clipboard text
//!   send-clip <text> [--to <id-prefix>] [--sensitive]
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

async fn pump(mut s: Session) {
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
            let pending = tokio::time::timeout(pb_transport::pairing::PAIRING_TTL, ev.pairings.recv())
                .await
                .map_err(|_| "pairing code expired")?
                .ok_or("node stopped")?;
            let ok = confirm(&pending.sas, &pending.peer_name, o.auto_accept).await?;
            let peer = pending.decide(ok).await?;
            println!("PAIRED {} \"{}\" {}", peer.id, peer.name, peer.addrs.join(","));
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
                tokio::spawn(pump(s));
            }
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
        _ => {
            return Err("usage: pb [--dir D] [--port N] [--name N] pair-host|join <code>|peers|unpair <id>|listen|send-clip <text> [--to id] [--sensitive]".into())
        }
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
