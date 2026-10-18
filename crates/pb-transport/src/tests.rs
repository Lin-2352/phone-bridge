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
        assert!(matches!(a, Err(Error::Declined)), "{a:?}");
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
