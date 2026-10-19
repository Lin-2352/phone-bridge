//! In-process tests: two real iroh nodes on loopback.

use std::sync::{Arc, Mutex};

use pb_identity::{Identity, MemoryPeerStore, PairedPeer};
use pb_protocol::control::{ClipboardText, ControlMsg};

use super::*;

async fn node(seed: u8, name: &str) -> (Node, NodeEvents, SharedStore) {
    let store: SharedStore = Arc::new(Mutex::new(MemoryPeerStore::default()));
    let (n, ev) = Node::bind(
        &Identity::from_seed([seed; 32]),
        store.clone(),
        NodeConfig {
            device_name: name.into(),
            port: None,
            discovery_port: None,
            tuning: Default::default(),
        },
    )
    .await
    .unwrap();
    (n, ev, store)
}

fn loopback(n: &Node) -> Vec<String> {
    vec![format!("127.0.0.1:{}", n.port())]
}

/// Pair `joiner` with `host`; both users answer with the given decisions.
async fn pair(
    host: &Node,
    host_ev: &mut NodeEvents,
    joiner: &Node,
    host_ok: bool,
    joiner_ok: bool,
) -> (
    Result<PairedPeer, Error>,
    Result<PairedPeer, Error>,
    String,
    String,
) {
    let code = host.open_pairing(loopback(host)).unwrap();
    let joining = joiner.join(&code).await.unwrap();
    let pending = host_ev.pairings.recv().await.unwrap();
    let (sas_h, sas_j) = (pending.sas.clone(), joining.sas.clone());
    assert_eq!(pending.peer, joiner.peer_id());
    let (h, j) = tokio::join!(pending.decide(host_ok), joining.decide(joiner_ok));
    (h, j, sas_h, sas_j)
}

#[test]
fn iroh_key_matches_our_identity() {
    let id = Identity::from_seed([7; 32]);
    let iroh_pub = SecretKey::from_bytes(&id.seed()).public();
    assert_eq!(iroh_pub.as_bytes(), id.peer_id().as_bytes());
}

#[tokio::test(flavor = "multi_thread")]
async fn pair_then_exchange_clipboard_both_ways() {
    let (pc, mut pc_ev, pc_store) = node(1, "PC").await;
    let (phone, mut phone_ev, phone_store) = node(2, "Phone").await;

    let (h, j, sas_h, sas_j) = pair(&pc, &mut pc_ev, &phone, true, true).await;
    assert_eq!(sas_h, sas_j, "both users must see the same code");
    assert_eq!(h.unwrap().name, "Phone");
    assert_eq!(j.unwrap().name, "PC");
    assert!(lock(&pc_store).get(&phone.peer_id()).is_some());
    assert!(lock(&phone_store).get(&pc.peer_id()).is_some());

    // phone -> PC (phone dials the address from the pairing code)
    let mut out = phone.connect(&pc.peer_id()).await.unwrap();
    let mut inc = pc_ev.sessions.recv().await.unwrap();
    assert_eq!(inc.peer, phone.peer_id());
    assert_eq!(inc.remote_hello.device_name, "Phone");
    out.send(ControlMsg::ClipboardText(ClipboardText {
        text: "from phone".into(),
        sensitive: false,
    }))
    .await
    .unwrap();
    match inc.recv().await.unwrap() {
        Some(ControlMsg::ClipboardText(c)) => assert_eq!(c.text, "from phone"),
        other => panic!("unexpected {other:?}"),
    }
    // and back on the same session
    inc.send(ControlMsg::ClipboardText(ClipboardText {
        text: "from pc".into(),
        sensitive: true,
    }))
    .await
    .unwrap();
    match out.recv().await.unwrap() {
        Some(ControlMsg::ClipboardText(c)) => assert!(c.sensitive && c.text == "from pc"),
        other => panic!("unexpected {other:?}"),
    }

    // PC -> phone using the address the phone sent during pairing; replace it
    // with loopback because the test machine's LAN addresses may be firewalled.
    let mut p = lock(&pc_store).get(&phone.peer_id()).unwrap();
    p.addrs = loopback(&phone);
    lock(&pc_store).pin(p);
    let _s = pc.connect(&phone.peer_id()).await.unwrap();
    let s2 = phone_ev.sessions.recv().await.unwrap();
    assert_eq!(s2.peer, pc.peer_id());
}

#[tokio::test(flavor = "multi_thread")]
async fn unpaired_peer_is_refused() {
    let (pc, mut pc_ev, _) = node(1, "PC").await;
    let (stranger, _sev, stranger_store) = node(9, "Stranger").await;
    // The stranger knows the PC's id and address, but the PC never paired it.
    lock(&stranger_store).pin(PairedPeer {
        id: pc.peer_id(),
        name: "PC".into(),
        addrs: loopback(&pc),
    });
    assert!(stranger.connect(&pc.peer_id()).await.is_err());
    assert!(
        tokio::time::timeout(Duration::from_millis(500), pc_ev.sessions.recv())
            .await
            .is_err(),
        "no session may be delivered for an unpaired peer"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn declining_on_either_side_pins_nothing() {
    for (host_ok, joiner_ok) in [(false, true), (true, false)] {
        let (pc, mut pc_ev, pc_store) = node(1, "PC").await;
        let (phone, _pev, phone_store) = node(2, "Phone").await;
        let (h, j, _, _) = pair(&pc, &mut pc_ev, &phone, host_ok, joiner_ok).await;
        assert!(
            matches!(h, Err(Error::Rejected)),
            "host {host_ok} joiner {joiner_ok}"
        );
        assert!(matches!(j, Err(Error::Rejected)));
        assert!(lock(&pc_store).list().is_empty());
        assert!(lock(&phone_store).list().is_empty());
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn pairing_code_is_single_use() {
    let (pc, mut pc_ev, _) = node(1, "PC").await;
    let (phone, _pev, _) = node(2, "Phone").await;
    let (other, _oev, _) = node(3, "Other").await;
    let code = pc.open_pairing(loopback(&pc)).unwrap();
    let joining = phone.join(&code).await.unwrap();
    let pending = pc_ev.pairings.recv().await.unwrap();
    // A second device replaying the same code is refused.
    assert!(other.join(&code).await.is_err());
    let (h, j) = tokio::join!(pending.decide(true), joining.decide(true));
    assert!(h.is_ok() && j.is_ok());
}

#[tokio::test(flavor = "multi_thread")]
async fn revoked_peer_can_no_longer_connect() {
    let (pc, mut pc_ev, _) = node(1, "PC").await;
    let (phone, _pev, _) = node(2, "Phone").await;
    let (h, j, _, _) = pair(&pc, &mut pc_ev, &phone, true, true).await;
    assert!(h.is_ok() && j.is_ok(), "{h:?} {j:?}");
    assert!(pc.unpair(&phone.peer_id()));
    let err = phone.connect(&pc.peer_id()).await.err();
    assert!(matches!(err, Some(Error::NotPaired)), "{err:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn unpairing_ends_a_session_that_is_open() {
    let (pc, mut pc_ev, _) = node(1, "PC").await;
    let (phone, _pev, _) = node(2, "Phone").await;
    let (h, j, _, _) = pair(&pc, &mut pc_ev, &phone, true, true).await;
    assert!(h.is_ok() && j.is_ok(), "{h:?} {j:?}");
    let mut s = phone.connect(&pc.peer_id()).await.unwrap();
    let _held = pc_ev.sessions.recv().await.unwrap();
    assert!(pc.unpair(&phone.peer_id()));
    let r = tokio::time::timeout(Duration::from_secs(5), s.recv()).await;
    assert!(
        matches!(r, Ok(Err(_) | Ok(None))),
        "the phone's session must end at once"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn rotating_codes_invalidate_the_previous_one_and_closing_kills_the_current() {
    let (pc, mut pc_ev, _) = node(1, "PC").await;
    let (phone, _pev, _) = node(2, "Phone").await;
    let old = pc
        .open_pairing_for(loopback(&pc), Duration::from_secs(35))
        .unwrap();
    let new = pc
        .open_pairing_for(loopback(&pc), Duration::from_secs(35))
        .unwrap();
    assert_ne!(old.token, new.token);
    assert!(
        phone.join(&old).await.is_err(),
        "rotated-out code must be dead"
    );
    pc.close_pairing();
    assert!(
        phone.join(&new).await.is_err(),
        "closing the window must kill the code"
    );
    let fresh = pc
        .open_pairing_for(loopback(&pc), Duration::from_secs(35))
        .unwrap();
    let joining = phone.join(&fresh).await.unwrap();
    let pending = pc_ev.pairings.recv().await.unwrap();
    let (h, j) = tokio::join!(pending.decide(true), joining.decide(true));
    assert!(h.is_ok() && j.is_ok());
}

mod transfers {
    use std::path::{Path, PathBuf};

    use pb_protocol::control::{ControlMsg, FileHeader, Offer, OfferKind, TransferDone};

    use super::*;
    use crate::files::{receive_offer, Received, DEFAULT_MAX_FILE};

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("pb-xfer-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn pattern(n: usize) -> Vec<u8> {
        let mut x = 0x9e37_79b9u32;
        (0..n)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                x as u8
            })
            .collect()
    }

    async fn paired() -> (Node, NodeEvents, Node, NodeEvents) {
        let (pc, mut pc_ev, _) = node(1, "PC").await;
        let (phone, phone_ev, _) = node(2, "Phone").await;
        let (h, j, _, _) = pair(&pc, &mut pc_ev, &phone, true, true).await;
        assert!(h.is_ok() && j.is_ok());
        (pc, pc_ev, phone, phone_ev)
    }

    /// Receiver side: take the next session, wait for its offer, handle it.
    async fn receive_one(
        ev: &mut NodeEvents,
        accept: bool,
        dir: &Path,
        max: u64,
    ) -> Result<Received, Error> {
        let mut s = ev.sessions.recv().await.unwrap();
        loop {
            match s.recv().await? {
                Some(ControlMsg::Offer(o)) => {
                    return receive_offer(&mut s, o, accept, dir, max, None).await
                }
                Some(_) => continue,
                None => return Err(Error::Closed),
            }
        }
    }

    fn no_partials(dir: &Path) -> bool {
        std::fs::read_dir(dir).unwrap().all(|e| {
            !e.unwrap()
                .file_name()
                .to_string_lossy()
                .contains(".partial-")
        })
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn files_of_various_sizes_arrive_intact() {
        let (pc, mut pc_ev, phone, _pev) = paired().await;
        let pcid = pc.peer_id();
        let dir = tmp("sizes");
        for (name, size) in [
            ("empty.bin", 0usize),
            ("small.txt", 1024),
            ("big.bin", 64 << 20),
        ] {
            let data = pattern(size);
            let send = phone.send_file(
                &pcid,
                name,
                size as u64,
                "application/octet-stream",
                &data[..],
                None,
            );
            let (sent, got) =
                tokio::join!(send, receive_one(&mut pc_ev, true, &dir, DEFAULT_MAX_FILE));
            sent.unwrap();
            match got.unwrap() {
                Received::File { path, size: n } => {
                    assert_eq!(n, size as u64);
                    assert_eq!(path, dir.join(name));
                    assert_eq!(std::fs::read(&path).unwrap(), data, "{name}");
                }
                other => panic!("{other:?}"),
            }
        }
        assert!(no_partials(&dir));
    }

    /// Throughput of the whole file path (read, hash, QUIC, hash, disk) on
    /// loopback. Run by hand: `cargo test -p pb-transport --release
    /// file_throughput -- --ignored --nocapture`.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[ignore]
    async fn file_throughput() {
        let (pc, mut pc_ev, phone, _pev) = paired().await;
        let pcid = pc.peer_id();
        let dir = tmp("throughput");
        let size = 512usize << 20;
        let data = pattern(size);
        for run in 0..3 {
            let t0 = std::time::Instant::now();
            let send = phone.send_file(&pcid, "t.bin", size as u64, "", &data[..], None);
            let (sent, got) =
                tokio::join!(send, receive_one(&mut pc_ev, true, &dir, DEFAULT_MAX_FILE));
            sent.unwrap();
            let secs = t0.elapsed().as_secs_f64();
            if let Received::File { path, .. } = got.unwrap() {
                let _ = std::fs::remove_file(path);
            }
            println!("run {run}: {:.0} MB/s", size as f64 / 1048576.0 / secs);
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn text_and_links_are_delivered_inline() {
        let (pc, mut pc_ev, phone, _pev) = paired().await;
        let pcid = pc.peer_id();
        let dir = tmp("text");
        let (a, b) = tokio::join!(
            phone.send_text(&pcid, "hello", false),
            receive_one(&mut pc_ev, true, &dir, DEFAULT_MAX_FILE)
        );
        a.unwrap();
        assert_eq!(b.unwrap(), Received::Text("hello".into()));
        let (a, b) = tokio::join!(
            phone.send_text(&pcid, "https://example.com", true),
            receive_one(&mut pc_ev, true, &dir, DEFAULT_MAX_FILE)
        );
        a.unwrap();
        assert_eq!(b.unwrap(), Received::Link("https://example.com".into()));
        // Finished transfers leave no connection open on either side.
        for _ in 0..50 {
            if pc.live_count() == 0 && phone.live_count() == 0 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        assert_eq!((pc.live_count(), phone.live_count()), (0, 0));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn declined_and_oversized_offers_save_nothing() {
        let (pc, mut pc_ev, phone, _pev) = paired().await;
        let pcid = pc.peer_id();
        let dir = tmp("decline");
        let data = pattern(4096);
        let (a, b) = tokio::join!(
            phone.send_file(&pcid, "x.bin", 4096, "", &data[..], None),
            receive_one(&mut pc_ev, false, &dir, DEFAULT_MAX_FILE)
        );
        assert!(matches!(a, Err(Error::Declined)), "{a:?}");
        assert!(matches!(b, Err(Error::Declined)));
        let (a, b) = tokio::join!(
            phone.send_file(&pcid, "x.bin", 4096, "", &data[..], None),
            receive_one(&mut pc_ev, true, &dir, 1000)
        );
        // The sender learns why: too large, not a plain no.
        assert!(matches!(a, Err(Error::TooLarge)), "{a:?}");
        assert!(matches!(b, Err(Error::TooLarge)));
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn hostile_names_stay_inside_the_folder() {
        let (pc, mut pc_ev, phone, _pev) = paired().await;
        let pcid = pc.peer_id();
        let dir = tmp("names");
        let data = pattern(10);
        let (a, b) = tokio::join!(
            phone.send_file(&pcid, r"..\..\..\Windows\evil.dll", 10, "", &data[..], None),
            receive_one(&mut pc_ev, true, &dir, DEFAULT_MAX_FILE)
        );
        a.unwrap();
        match b.unwrap() {
            Received::File { path, .. } => assert_eq!(path, dir.join("evil.dll")),
            other => panic!("{other:?}"),
        }
    }

    /// Hand-rolled sender that lies about the hash, or stops half way.
    async fn rogue_send(phone: &Node, pc: &PeerId, data: &[u8], wrong_hash: bool, stop_half: bool) {
        let mut s = phone.connect(pc).await.unwrap();
        s.send(ControlMsg::Offer(Offer {
            id: 7,
            kind: OfferKind::File as i32,
            name: "rogue.bin".into(),
            size: data.len() as u64,
            mime: String::new(),
            text: String::new(),
        }))
        .await
        .unwrap();
        loop {
            if let Some(ControlMsg::OfferReply(_)) = s.recv().await.unwrap() {
                break;
            }
        }
        let mut out = s.conn.open_uni().await.unwrap();
        write_frame(&mut out, &FileHeader { id: 7 }).await.unwrap();
        let n = if stop_half {
            data.len() / 2
        } else {
            data.len()
        };
        out.write_all(&data[..n]).await.unwrap();
        if stop_half {
            s.conn.close(0u32.into(), b"gone");
            return;
        }
        out.finish().unwrap();
        let hash = if wrong_hash {
            vec![0u8; 32]
        } else {
            vec![1u8; 32]
        };
        s.send(ControlMsg::TransferDone(TransferDone {
            id: 7,
            sha256: hash,
        }))
        .await
        .unwrap();
        let _ = tokio::time::timeout(Duration::from_secs(5), s.recv()).await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn corrupted_or_interrupted_transfers_leave_no_file() {
        let (pc, mut pc_ev, phone, _pev) = paired().await;
        let pcid = pc.peer_id();
        let dir = tmp("rogue");
        let data = pattern(1 << 20);
        let (_, b) = tokio::join!(
            rogue_send(&phone, &pcid, &data, true, false),
            receive_one(&mut pc_ev, true, &dir, DEFAULT_MAX_FILE)
        );
        assert!(matches!(b, Err(Error::HashMismatch)), "{b:?}");
        let (_, b) = tokio::join!(
            rogue_send(&phone, &pcid, &data, false, true),
            receive_one(&mut pc_ev, true, &dir, DEFAULT_MAX_FILE)
        );
        assert!(b.is_err(), "interrupted transfer must fail");
        assert_eq!(
            std::fs::read_dir(&dir).unwrap().count(),
            0,
            "no file and no partial left"
        );
    }
}

mod input {
    use pb_protocol::InputEvent;

    use super::*;
    use crate::input::receive_input;

    async fn paired() -> (Node, NodeEvents, Node, NodeEvents) {
        let (pc, mut pc_ev, _) = node(11, "PC").await;
        let (phone, phone_ev, _) = node(12, "Phone").await;
        let (h, j, _, _) = pair(&pc, &mut pc_ev, &phone, true, true).await;
        assert!(h.is_ok() && j.is_ok());
        (pc, pc_ev, phone, phone_ev)
    }

    /// Receiver: take the next session, expect InputStart, collect events.
    async fn receive(ev: &mut NodeEvents, accept: bool) -> (Result<(), Error>, Vec<InputEvent>) {
        let mut s = ev.sessions.recv().await.unwrap();
        loop {
            match s.recv().await {
                Ok(Some(ControlMsg::InputStart(start))) => {
                    let mut got = Vec::new();
                    let r = receive_input(&mut s, start, accept, |e| got.push(e)).await;
                    return (r, got);
                }
                Ok(Some(_)) => continue,
                _ => return (Err(Error::Closed), Vec::new()),
            }
        }
    }

    #[tokio::test]
    async fn events_arrive_in_order_and_moves_merge() {
        let (pc, mut pc_ev, phone, _) = paired().await;
        let rx = tokio::spawn(async move { receive(&mut pc_ev, true).await });
        let input = phone.open_input(&pc.peer_id()).await.unwrap();
        for _ in 0..100 {
            input.send(InputEvent::MouseMove { dx: 1, dy: -2 });
        }
        input.send(InputEvent::MouseButton {
            button: 0,
            pressed: true,
        });
        input.send(InputEvent::MouseButton {
            button: 0,
            pressed: false,
        });
        for u in "hé😀".encode_utf16() {
            input.send(InputEvent::Char { unit: u });
        }
        input.stop();
        let (r, got) = rx.await.unwrap();
        r.unwrap();
        let (mut x, mut y) = (0i32, 0i32);
        let mut rest = Vec::new();
        for e in got {
            match e {
                InputEvent::MouseMove { dx, dy } => {
                    assert!(rest.is_empty(), "moves come before the click");
                    x += i32::from(dx);
                    y += i32::from(dy);
                }
                other => rest.push(other),
            }
        }
        assert_eq!((x, y), (100, -200), "no motion lost by merging");
        let typed: Vec<u16> = rest
            .iter()
            .filter_map(|e| match e {
                InputEvent::Char { unit } => Some(*unit),
                _ => None,
            })
            .collect();
        assert_eq!(String::from_utf16(&typed).unwrap(), "hé😀");
        assert_eq!(
            &rest[..2],
            &[
                InputEvent::MouseButton {
                    button: 0,
                    pressed: true
                },
                InputEvent::MouseButton {
                    button: 0,
                    pressed: false
                }
            ]
        );
    }

    #[tokio::test]
    async fn the_receiver_can_end_a_running_control_session() {
        let (pc, mut pc_ev, phone, _) = paired().await;
        let (closer_tx, closer_rx) = tokio::sync::oneshot::channel();
        let rx = tokio::spawn(async move {
            let mut s = pc_ev.sessions.recv().await.unwrap();
            loop {
                if let Ok(Some(ControlMsg::InputStart(start))) = s.recv().await {
                    let _ = closer_tx.send(s.closer());
                    let mut got = 0usize;
                    let r = receive_input(&mut s, start, true, |_| got += 1).await;
                    return (r, got);
                }
            }
        });
        let input = phone.open_input(&pc.peer_id()).await.unwrap();
        input.send(InputEvent::MouseMove { dx: 3, dy: 4 });
        tokio::time::sleep(Duration::from_millis(300)).await;
        // The user turns control off on the receiving device.
        closer_rx.await.unwrap().close(b"stopped");
        let (r, got) = tokio::time::timeout(Duration::from_secs(5), rx)
            .await
            .expect("receiver loop ends")
            .unwrap();
        assert!(r.is_ok(), "{r:?}");
        assert_eq!(got, 1);
        // The controller notices within a moment.
        let mut closed = false;
        for _ in 0..50 {
            if !input.is_open() {
                closed = true;
                break;
            }
            input.send(InputEvent::MouseMove { dx: 1, dy: 0 });
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        assert!(closed, "the controller sees the session end");
    }

    #[tokio::test]
    async fn declined_input_reports_declined() {
        let (pc, mut pc_ev, phone, _) = paired().await;
        let rx = tokio::spawn(async move { receive(&mut pc_ev, false).await });
        let r = phone.open_input(&pc.peer_id()).await;
        assert!(matches!(r, Err(Error::Declined)));
        assert!(matches!(rx.await.unwrap().0, Err(Error::Declined)));
    }

    #[tokio::test]
    async fn a_long_paste_is_paced_and_arrives_whole() {
        let (pc, mut pc_ev, phone, _) = paired().await;
        let rx = tokio::spawn(async move { receive(&mut pc_ev, true).await });
        let input = phone.open_input(&pc.peer_id()).await.unwrap();
        // Over the receiver's 400 events/s budget if sent at once.
        let text = "paste ".repeat(100);
        for u in text.encode_utf16() {
            input.send(InputEvent::Char { unit: u });
        }
        input.stop();
        let (r, got) = rx.await.unwrap();
        r.unwrap();
        let typed: Vec<u16> = got
            .iter()
            .filter_map(|e| match e {
                InputEvent::Char { unit } => Some(*unit),
                _ => None,
            })
            .collect();
        assert_eq!(String::from_utf16(&typed).unwrap(), text);
    }

    #[tokio::test]
    async fn click_floods_end_the_session_and_excess_moves_are_dropped() {
        use pb_protocol::{control::InputStart, InputDatagram};
        let (pc, mut pc_ev, phone, _) = paired().await;
        let rx = tokio::spawn(async move { receive(&mut pc_ev, true).await });
        // A rogue controller that ignores pacing: raw records, all at once.
        let mut s = phone.connect(&pc.peer_id()).await.unwrap();
        s.send(ControlMsg::InputStart(InputStart { id: 7 }))
            .await
            .unwrap();
        assert!(crate::files::await_reply(&mut s, 7).await.unwrap());
        let mut out = s.conn.open_uni().await.unwrap();
        let mut buf = Vec::new();
        for i in 0..2000u16 {
            let event = InputEvent::MouseButton {
                button: 0,
                pressed: i % 2 == 0,
            };
            buf.extend_from_slice(&InputDatagram { seq: i, event }.encode());
        }
        let _ = out.write_all(&buf).await;
        let (r, got) = rx.await.unwrap();
        assert!(r.is_err(), "flood must end the session");
        assert!(
            got.len() <= 400,
            "at most the budget is injected, got {}",
            got.len()
        );
    }
}

mod rediscovery {
    use std::net::SocketAddr;

    use super::*;

    #[tokio::test]
    async fn a_probe_from_a_paired_peer_updates_its_address_and_gets_an_answer() {
        // Node A answers probes on a free port; B is paired with A.
        let probe_port = {
            let s = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
            s.local_addr().unwrap().port()
        };
        let id_a = Identity::from_seed([31; 32]);
        let store_a: SharedStore = Arc::new(Mutex::new(MemoryPeerStore::default()));
        let (a, mut a_ev) = Node::bind(
            &id_a,
            store_a.clone(),
            NodeConfig {
                device_name: "PC".into(),
                port: None,
                discovery_port: Some(probe_port),
                tuning: Default::default(),
            },
        )
        .await
        .unwrap();
        let (b, _b_ev, _) = node(32, "Phone").await;
        let (h, j, _, _) = pair(&a, &mut a_ev, &b, true, true).await;
        assert!(h.is_ok() && j.is_ok());

        // B "moved": pretend it now lives at 127.0.0.1:<moved_port>.
        let sock = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let moved_port: u16 = 40555;
        let mut probe = Vec::from(*b"PBD1");
        probe.push(1);
        probe.extend_from_slice(&moved_port.to_le_bytes());
        probe.extend_from_slice(&crate::discovery::test_tag(&b.peer_id()));
        sock.send_to(&probe, ("127.0.0.1", probe_port))
            .await
            .unwrap();

        let mut buf = [0u8; 64];
        let (n, _) = tokio::time::timeout(Duration::from_secs(3), sock.recv_from(&mut buf))
            .await
            .expect("answer")
            .unwrap();
        assert_eq!(n, 23);
        assert_eq!(&buf[..5], b"PBD1");
        // Learned as an unverified candidate; the saved addresses are untouched.
        assert_eq!(
            a.candidates(&b.peer_id()).first().copied(),
            Some(SocketAddr::from(([127, 0, 0, 1], moved_port)))
        );
        let saved = store_a.lock().unwrap().get(&b.peer_id()).unwrap().addrs;
        assert!(!saved.contains(&format!("127.0.0.1:{moved_port}")));

        // A probe tagged with an unpaired id is ignored (no answer).
        let mut stranger = probe.clone();
        stranger[7..].copy_from_slice(&[0xAB; 16]);
        sock.send_to(&stranger, ("127.0.0.1", probe_port))
            .await
            .unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(500), sock.recv_from(&mut buf))
                .await
                .is_err()
        );

        // B's saved address is stale; two candidates are known, one real and
        // one forged. Connecting saves only the one the connection uses.
        let mut peer_b = store_a.lock().unwrap().get(&b.peer_id()).unwrap();
        peer_b.addrs = vec!["127.0.0.1:1".into()];
        store_a.lock().unwrap().pin(peer_b);
        let real = SocketAddr::from(([127, 0, 0, 1], b.port()));
        let forged = SocketAddr::from(([127, 0, 0, 1], 40556));
        for port in [real.port(), forged.port()] {
            let mut p = probe.clone();
            p[5..7].copy_from_slice(&port.to_le_bytes());
            sock.send_to(&p, ("127.0.0.1", probe_port)).await.unwrap();
            let _ = tokio::time::timeout(Duration::from_secs(3), sock.recv_from(&mut buf)).await;
        }
        assert!(a.candidates(&b.peer_id()).contains(&forged));
        let s = a
            .connect(&b.peer_id())
            .await
            .expect("connects via the candidate");
        let used = crate::used_addr(&s.conn).expect("a direct path");
        drop(s);
        let saved = store_a.lock().unwrap().get(&b.peer_id()).unwrap().addrs;
        assert_eq!(saved.first(), Some(&used.to_string()), "{saved:?}");
        assert!(!saved.contains(&forged.to_string()), "{saved:?}");
        assert!(
            a.candidates(&b.peer_id()).is_empty(),
            "candidates are spent"
        );
    }
}
