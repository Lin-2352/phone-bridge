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

/// A store whose saves fail while `fail` is set (a locked or full disk).
#[derive(Default)]
struct FlakyStore {
    mem: MemoryPeerStore,
    fail: Arc<std::sync::atomic::AtomicBool>,
}

impl FlakyStore {
    fn failing(&self) -> bool {
        self.fail.load(std::sync::atomic::Ordering::SeqCst)
    }
}

impl PeerStore for FlakyStore {
    fn get(&self, id: &PeerId) -> Option<PairedPeer> {
        self.mem.get(id)
    }
    fn pin(&mut self, peer: PairedPeer) {
        self.mem.pin(peer);
    }
    fn remove(&mut self, id: &PeerId) -> bool {
        self.try_remove(id).unwrap_or(false)
    }
    fn list(&self) -> Vec<PairedPeer> {
        self.mem.list()
    }
    fn try_pin(&mut self, peer: PairedPeer) -> std::io::Result<()> {
        if self.failing() {
            return Err(std::io::Error::other("disk full"));
        }
        self.mem.pin(peer);
        Ok(())
    }
    fn try_remove(&mut self, id: &PeerId) -> std::io::Result<bool> {
        if self.failing() {
            return Err(std::io::Error::other("disk full"));
        }
        Ok(self.mem.remove(id))
    }
}

async fn flaky_node(
    seed: u8,
    name: &str,
) -> (
    Node,
    NodeEvents,
    SharedStore,
    Arc<std::sync::atomic::AtomicBool>,
) {
    let flaky = FlakyStore::default();
    let fail = flaky.fail.clone();
    let store: SharedStore = Arc::new(Mutex::new(flaky));
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
    (n, ev, store, fail)
}

#[tokio::test(flavor = "multi_thread")]
async fn an_unpair_that_cannot_be_saved_is_reported_and_changes_nothing() {
    let (pc, mut pc_ev, pc_store, fail) = flaky_node(1, "PC").await;
    let (phone, _pev, _) = node(2, "Phone").await;
    let (h, j, _, _) = pair(&pc, &mut pc_ev, &phone, true, true).await;
    assert!(h.is_ok() && j.is_ok(), "{h:?} {j:?}");
    let s = phone.connect(&pc.peer_id()).await.unwrap();
    let mut held = pc_ev.sessions.recv().await.unwrap();
    fail.store(true, std::sync::atomic::Ordering::SeqCst);
    assert!(
        matches!(pc.try_unpair(&phone.peer_id()), Err(Error::Storage(_))),
        "the user must hear that the removal failed"
    );
    assert!(!pc.unpair(&phone.peer_id()), "no success claimed");
    assert!(lock(&pc_store).get(&phone.peer_id()).is_some());
    // Still paired, so nothing it has open is cut either.
    s.send(ControlMsg::ClipboardText(ClipboardText {
        text: "still here".into(),
        sensitive: false,
    }))
    .await
    .unwrap();
    assert!(matches!(
        held.recv().await,
        Ok(Some(ControlMsg::ClipboardText(_)))
    ));
    // Once saving works again the removal goes through.
    fail.store(false, std::sync::atomic::Ordering::SeqCst);
    assert!(pc.try_unpair(&phone.peer_id()).unwrap());
    assert!(lock(&pc_store).get(&phone.peer_id()).is_none());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_pairing_the_host_cannot_save_is_reported_on_both_sides() {
    let (pc, mut pc_ev, pc_store, fail) = flaky_node(1, "PC").await;
    let (phone, _pev, phone_store) = node(2, "Phone").await;
    fail.store(true, std::sync::atomic::Ordering::SeqCst);
    let (h, j, _, _) = pair(&pc, &mut pc_ev, &phone, true, true).await;
    assert!(matches!(h, Err(Error::Storage(_))), "{h:?}");
    assert!(j.is_err(), "the phone must not claim a pairing: {j:?}");
    assert!(lock(&pc_store).list().is_empty());
    assert!(lock(&phone_store).list().is_empty(), "no half pairing");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_pairing_the_joiner_cannot_save_is_reported_on_both_sides() {
    let (pc, mut pc_ev, pc_store) = node(1, "PC").await;
    let (phone, _pev, phone_store, fail) = flaky_node(2, "Phone").await;
    fail.store(true, std::sync::atomic::Ordering::SeqCst);
    let (h, j, _, _) = pair(&pc, &mut pc_ev, &phone, true, true).await;
    assert!(matches!(j, Err(Error::Storage(_))), "{j:?}");
    assert!(h.is_err(), "the PC must not claim a pairing: {h:?}");
    assert!(lock(&phone_store).list().is_empty());
    assert!(lock(&pc_store).list().is_empty(), "no half pairing");
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

    use pb_protocol::control::{
        ControlMsg, FileHeader, Offer, OfferKind, OfferReply, TransferDone,
    };

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

    async fn within<T>(what: &str, f: impl std::future::Future<Output = T>) -> T {
        tokio::time::timeout(Duration::from_secs(20), f)
            .await
            .unwrap_or_else(|_| panic!("{what} hung"))
    }

    fn cancel_after(bytes: u64, c: &crate::files::Cancel) -> crate::files::Progress {
        let c = c.clone();
        Arc::new(move |done, _| {
            if done >= bytes {
                c.cancel();
            }
        })
    }

    fn dir_is_empty(dir: &Path) -> bool {
        std::fs::read_dir(dir).unwrap().next().is_none()
    }

    #[tokio::test]
    async fn a_cancel_handle_wakes_waiters_and_is_sticky() {
        let c = crate::files::Cancel::new();
        assert!(!c.is_cancelled());
        let c2 = c.clone();
        let waiter = tokio::spawn(async move { c2.cancelled().await });
        tokio::task::yield_now().await;
        c.cancel();
        c.cancel();
        within("waiter", waiter).await.unwrap();
        // Already cancelled: resolves at once.
        within("again", c.cancelled()).await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn the_sender_can_cancel_mid_send_and_nothing_is_left_behind() {
        use crate::files::TransferOpts;
        let (pc, mut pc_ev, phone, _pev) = paired().await;
        let pcid = pc.peer_id();
        let dir = tmp("cancel-send");
        let data = pattern(64 << 20);
        let cancel = crate::files::Cancel::new();
        let opts = TransferOpts::new(Some(cancel_after(1 << 20, &cancel)), Some(cancel.clone()));
        let send = phone.send_file_with(&pcid, "big.bin", data.len() as u64, "", &data[..], opts);
        let (sent, got) = within("cancelled send", async {
            tokio::join!(send, receive_one(&mut pc_ev, true, &dir, DEFAULT_MAX_FILE))
        })
        .await;
        assert!(matches!(sent, Err(Error::Cancelled)), "{sent:?}");
        assert!(matches!(got, Err(Error::PeerCancelled)), "{got:?}");
        assert!(
            dir_is_empty(&dir),
            "{:?}",
            std::fs::read_dir(&dir).unwrap().count()
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn the_receiver_can_cancel_mid_receive_and_nothing_is_left_behind() {
        use crate::files::{receive_offer_with, TransferOpts};
        let (pc, mut pc_ev, phone, _pev) = paired().await;
        let pcid = pc.peer_id();
        let dir = tmp("cancel-recv");
        let data = pattern(64 << 20);
        let cancel = crate::files::Cancel::new();
        let send = phone.send_file(&pcid, "big.bin", data.len() as u64, "", &data[..], None);
        let recv = async {
            let mut s = pc_ev.sessions.recv().await.unwrap();
            loop {
                match s.recv().await? {
                    Some(ControlMsg::Offer(o)) => {
                        let opts = TransferOpts::new(
                            Some(cancel_after(1 << 20, &cancel)),
                            Some(cancel.clone()),
                        );
                        return receive_offer_with(&mut s, o, true, &dir, DEFAULT_MAX_FILE, opts)
                            .await;
                    }
                    Some(_) => continue,
                    None => return Err(Error::Closed),
                }
            }
        };
        let (sent, got) = within("cancelled receive", async { tokio::join!(send, recv) }).await;
        assert!(matches!(got, Err(Error::Cancelled)), "{got:?}");
        assert!(matches!(sent, Err(Error::PeerCancelled)), "{sent:?}");
        assert!(dir_is_empty(&dir));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn the_sender_can_cancel_while_waiting_for_the_accept() {
        use crate::files::TransferOpts;
        let (pc, mut pc_ev, phone, _pev) = paired().await;
        let pcid = pc.peer_id();
        let dir = tmp("cancel-wait");
        let data = pattern(1 << 20);
        let cancel = crate::files::Cancel::new();
        let opts = TransferOpts::new(None, Some(cancel.clone()));
        let send = phone.send_file_with(&pcid, "w.bin", data.len() as u64, "", &data[..], opts);
        let recv = async {
            let mut s = pc_ev.sessions.recv().await.unwrap();
            let offer = loop {
                match s.recv().await? {
                    Some(ControlMsg::Offer(o)) => break o,
                    Some(_) => continue,
                    None => return Err(Error::Closed),
                }
            };
            // The user has not answered yet; the sender gives up.
            cancel.cancel();
            // A late "Save" on a dead offer ends cleanly, never hangs.
            tokio::time::sleep(Duration::from_millis(300)).await;
            receive_offer(&mut s, offer, true, &dir, DEFAULT_MAX_FILE, None).await
        };
        let (sent, got) = within("cancel while waiting", async { tokio::join!(send, recv) }).await;
        assert!(matches!(sent, Err(Error::Cancelled)), "{sent:?}");
        assert!(got.is_err(), "{got:?}");
        assert!(dir_is_empty(&dir));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_cancel_before_the_send_starts_returns_at_once() {
        use crate::files::TransferOpts;
        let (pc, _pc_ev, phone, _pev) = paired().await;
        let cancel = crate::files::Cancel::new();
        cancel.cancel();
        let opts = TransferOpts::new(None, Some(cancel));
        let r = within(
            "pre-cancelled send",
            phone.send_file_with(&pc.peer_id(), "x.bin", 3, "", &b"abc"[..], opts),
        )
        .await;
        assert!(matches!(r, Err(Error::Cancelled)), "{r:?}");
    }

    #[test]
    fn refusal_names_the_reason() {
        use crate::files::{refusal, Refusal};
        let d = std::env::temp_dir();
        assert_eq!(refusal(&d, 10, 5), Some(Refusal::OverCap));
        assert_eq!(refusal(&d, 10, 100), None);
        assert_eq!(refusal(&d, u64::MAX / 4, u64::MAX), Some(Refusal::NoSpace));
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
    async fn a_clipboard_image_arrives_in_memory_and_leaves_no_file() {
        let (pc, mut pc_ev, phone, _pev) = paired().await;
        let pcid = pc.peer_id();
        let dir = tmp("clipimg");
        let png = pattern(300_000);
        let (a, b) = tokio::join!(
            phone.send_clipboard_image(&pcid, png.clone()),
            receive_one(&mut pc_ev, true, &dir, DEFAULT_MAX_FILE)
        );
        a.unwrap();
        assert_eq!(b.unwrap(), Received::Image(png));
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn an_oversized_clipboard_image_is_refused_on_both_ends() {
        use pb_protocol::control::MAX_CLIPBOARD_IMAGE;
        let (pc, mut pc_ev, phone, _pev) = paired().await;
        let pcid = pc.peer_id();
        let dir = tmp("clipimg-big");
        // The sender will not even offer it.
        let big = vec![0u8; MAX_CLIPBOARD_IMAGE + 1];
        let a = phone.send_clipboard_image(&pcid, big).await;
        assert!(matches!(a, Err(Error::TooLarge)), "{a:?}");
        // A sender that offers it anyway is told "too large".
        let s = phone.connect(&pcid).await.unwrap();
        s.send(ControlMsg::Offer(Offer {
            id: 5,
            kind: OfferKind::ClipboardImage as i32,
            name: "clipboard.png".into(),
            size: MAX_CLIPBOARD_IMAGE as u64 + 1,
            mime: "image/png".into(),
            text: String::new(),
        }))
        .await
        .unwrap();
        let got = receive_one(&mut pc_ev, true, &dir, DEFAULT_MAX_FILE).await;
        assert!(matches!(got, Err(Error::TooLarge)), "{got:?}");
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
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

    #[tokio::test(flavor = "multi_thread")]
    async fn a_receiver_that_vanishes_before_saving_is_not_success() {
        let (pc, mut pc_ev, phone, _pev) = paired().await;
        let pcid = pc.peer_id();
        let data = pattern(256 * 1024);
        // Receiver: accept, read every byte and TransferDone, then drop the
        // session without saving (as when the app is switched off).
        let rx = tokio::spawn(async move {
            let mut s = pc_ev.sessions.recv().await.unwrap();
            let Ok(Some(ControlMsg::Offer(o))) = s.recv().await else {
                panic!("offer expected")
            };
            s.send(ControlMsg::OfferReply(OfferReply {
                id: o.id,
                accept: true,
                too_large: false,
            }))
            .await
            .unwrap();
            let mut r = s.conn.accept_uni().await.unwrap();
            let mut sink = vec![0u8; 64 * 1024];
            while let Ok(Some(_)) = r.read(&mut sink).await {}
            while let Ok(Some(m)) = s.recv().await {
                if matches!(m, ControlMsg::TransferDone(_)) {
                    break;
                }
            }
            drop(s);
            drop(pc_ev);
            pc.shutdown().await;
        });
        let sent = phone
            .send_file(&pcid, "lost.bin", data.len() as u64, "", &data[..], None)
            .await;
        rx.await.unwrap();
        assert!(sent.is_err(), "nothing was saved, so no success: {sent:?}");
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
    async fn on_asked_runs_once_the_request_reached_the_peer() {
        let (pc, mut pc_ev, phone, _) = paired().await;
        let rx = tokio::spawn(async move { receive(&mut pc_ev, false).await });
        let asked = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let a = asked.clone();
        let r = phone
            .open_input_with(&pc.peer_id(), move || {
                a.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            })
            .await;
        assert!(matches!(r, Err(Error::Declined)));
        assert_eq!(asked.load(std::sync::atomic::Ordering::SeqCst), 1);
        let _ = rx.await;
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
    async fn stop_drops_a_queued_paste() {
        let (pc, mut pc_ev, phone, _) = paired().await;
        let rx = tokio::spawn(async move { receive(&mut pc_ev, true).await });
        let input = phone.open_input(&pc.peer_id()).await.unwrap();
        for u in "x".repeat(4096).encode_utf16() {
            input.send(InputEvent::Char { unit: u });
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
        input.cancel();
        let (_, got) = tokio::time::timeout(Duration::from_secs(10), rx)
            .await
            .expect("the session ends soon after stop")
            .unwrap();
        assert!(
            got.len() < 300,
            "queued keys must not keep typing after stop: {}",
            got.len()
        );
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
    async fn touch_moves_merge_but_down_and_up_keep_order() {
        use pb_protocol::TouchPhase::*;
        let (pc, mut pc_ev, phone, _) = paired().await;
        let rx = tokio::spawn(async move { receive(&mut pc_ev, true).await });
        let input = phone.open_input(&pc.peer_id()).await.unwrap();
        input.send(InputEvent::Touch {
            phase: Down,
            x: 10,
            y: 10,
        });
        for i in 0..50u16 {
            input.send(InputEvent::Touch {
                phase: Move,
                x: 100 + i,
                y: 200,
            });
        }
        input.send(InputEvent::Touch {
            phase: Up,
            x: 149,
            y: 200,
        });
        input.stop();
        let (r, got) = rx.await.unwrap();
        assert!(r.is_ok());
        assert_eq!(
            got.first(),
            Some(&InputEvent::Touch {
                phase: Down,
                x: 10,
                y: 10
            })
        );
        assert_eq!(
            got.last(),
            Some(&InputEvent::Touch {
                phase: Up,
                x: 149,
                y: 200
            })
        );
        let moves: Vec<_> = got
            .iter()
            .filter(|e| matches!(e, InputEvent::Touch { phase: Move, .. }))
            .collect();
        assert!(
            !moves.is_empty() && moves.len() <= 10,
            "{} moves",
            moves.len()
        );
        assert_eq!(
            moves.last(),
            Some(&&InputEvent::Touch {
                phase: Move,
                x: 149,
                y: 200
            })
        );
    }

    #[tokio::test]
    async fn touch_flood_is_limited_like_mouse_moves() {
        use pb_protocol::{control::InputStart, InputDatagram, TouchPhase::Move};
        let (pc, mut pc_ev, phone, _) = paired().await;
        let rx = tokio::spawn(async move { receive(&mut pc_ev, true).await });
        let mut s = phone.connect(&pc.peer_id()).await.unwrap();
        s.send(ControlMsg::InputStart(InputStart { id: 8 }))
            .await
            .unwrap();
        assert!(crate::files::await_reply(&mut s, 8).await.unwrap());
        let mut out = s.conn.open_uni().await.unwrap();
        let mut buf = Vec::new();
        for i in 0..3000u16 {
            let event = InputEvent::Touch {
                phase: Move,
                x: i,
                y: i,
            };
            buf.extend_from_slice(&InputDatagram { seq: i, event }.encode());
        }
        let _ = out.write_all(&buf).await;
        let _ = out.finish();
        let (r, got) = rx.await.unwrap();
        assert!(r.is_ok(), "moves over the budget are dropped, not fatal");
        assert!(got.len() <= 2000, "got {}", got.len());
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
            got.len() <= 1000,
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

mod notify {
    use std::time::Duration;

    use pb_protocol::control::{
        Notification, NotifyAction, NotifyActionKind, NotifyActionResult, MAX_NOTIFY_LINES,
        MAX_NOTIFY_TEXT,
    };

    use super::*;
    use crate::notify::NotifyOutbox;

    const LINGER: Duration = Duration::from_millis(300);

    async fn paired() -> (Node, NodeEvents, SharedStore, Node, NodeEvents, SharedStore) {
        let (pc, mut pc_ev, pc_store) = node(21, "PC").await;
        let (phone, phone_ev, phone_store) = node(22, "Phone").await;
        let (h, j, _, _) = pair(&pc, &mut pc_ev, &phone, true, true).await;
        assert!(h.is_ok() && j.is_ok());
        // The PC dials the phone on loopback (LAN addresses may be firewalled).
        let mut p = lock(&pc_store).get(&phone.peer_id()).unwrap();
        p.addrs = loopback(&phone);
        lock(&pc_store).pin(p);
        (pc, pc_ev, pc_store, phone, phone_ev, phone_store)
    }

    fn note(key: &str, text: &str) -> Notification {
        Notification {
            key: key.into(),
            package: "com.example.chat".into(),
            app_name: "Chat".into(),
            title: "Mom".into(),
            text: text.into(),
            when_ms: 1_000,
            ..Default::default()
        }
    }

    /// PC side: everything the next incoming session carries.
    async fn collect(ev: &mut NodeEvents) -> Vec<ControlMsg> {
        let mut s = ev.sessions.recv().await.unwrap();
        let mut got = Vec::new();
        while let Ok(Some(m)) = s.recv().await {
            got.push(m);
        }
        got
    }

    fn posted(msgs: &[ControlMsg]) -> Vec<&Notification> {
        msgs.iter()
            .filter_map(|m| match m {
                ControlMsg::Notification(n) => Some(n),
                _ => None,
            })
            .collect()
    }

    const PNG: [u8; 4] = [0x89, 0x50, 0x4E, 0x47];

    #[tokio::test(flavor = "multi_thread")]
    async fn notify_session_starts_with_sync_then_posts() {
        let (_pc, mut pc_ev, _, phone, _, _) = paired().await;
        let out = NotifyOutbox::new(phone.clone()).with_linger(LINGER);
        out.seed(vec![note("a", "old")]);
        out.post(note("b", "new"), Some(PNG.to_vec()));
        let got = collect(&mut pc_ev).await;
        match &got[0] {
            ControlMsg::NotifySync(s) => {
                let mut keys = s.keys.clone();
                keys.sort();
                assert_eq!(keys, ["a", "b"]);
            }
            other => panic!("expected NotifySync first, got {other:?}"),
        }
        assert!(matches!(&got[1], ControlMsg::NotifyIcon(i) if i.png == PNG));
        match &got[2] {
            ControlMsg::Notification(n) => {
                assert_eq!(n.key, "b");
                assert_eq!(n.icon_hash.len(), 32);
            }
            other => panic!("expected the notification, got {other:?}"),
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn outbox_coalesces_updates_to_one_key() {
        let (_pc, mut pc_ev, _, phone, _, _) = paired().await;
        let out = NotifyOutbox::new(phone.clone()).with_linger(LINGER);
        for i in 0..30 {
            out.post(note("k", &format!("m{i}")), None);
        }
        let got = collect(&mut pc_ev).await;
        let ks: Vec<_> = posted(&got).into_iter().filter(|n| n.key == "k").collect();
        assert!(!ks.is_empty() && ks.len() <= 3, "got {} posts", ks.len());
        assert_eq!(ks.last().unwrap().text, "m29");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn remove_before_send_cancels_the_post() {
        let (_pc, mut pc_ev, _, phone, _, _) = paired().await;
        let out = NotifyOutbox::new(phone.clone()).with_linger(LINGER);
        out.post(note("x", "gone soon"), None);
        out.remove("x");
        out.post(note("y", "stays"), None);
        let got = collect(&mut pc_ev).await;
        assert!(posted(&got).iter().all(|n| n.key != "x"));
        assert!(got
            .iter()
            .all(|m| !matches!(m, ControlMsg::NotifySync(s) if s.keys.iter().any(|k| k == "x"))));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn missed_posts_are_resent_quietly() {
        let (pc, pc_ev, pc_store, phone, _, _) = paired().await;
        // The PC is down at first.
        let port = pc.port();
        pc.shutdown().await;
        drop((pc, pc_ev));
        let out = NotifyOutbox::new(phone.clone()).with_linger(LINGER);
        out.post(note("m", "while away"), None);
        tokio::time::timeout(Duration::from_secs(60), async {
            while out.busy() {
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        })
        .await
        .expect("the failed delivery should end");
        // Back on the same port with the same identity and pairings (the
        // old socket may take a moment to be released).
        let mut again = None;
        for _ in 0..50 {
            let bound = Node::bind(
                &Identity::from_seed([21; 32]),
                pc_store.clone(),
                NodeConfig {
                    device_name: "PC".into(),
                    port: Some(port),
                    discovery_port: None,
                    tuning: Default::default(),
                },
            )
            .await;
            if let Ok(b) = bound {
                again = Some(b);
                break;
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        let (_pc, mut pc_ev) = again.expect("the PC port should be free again");
        out.post(note("n", "back"), None);
        let got = collect(&mut pc_ev).await;
        let p = posted(&got);
        assert_eq!(p.len(), 2, "{got:?}");
        assert!(p[0].key == "m" && p[0].quiet);
        assert!(p[1].key == "n" && !p[1].quiet);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn oversized_notification_is_trimmed_to_fit_a_frame() {
        let (_pc, mut pc_ev, _, phone, _, _) = paired().await;
        let out = NotifyOutbox::new(phone.clone()).with_linger(LINGER);
        let mut n = note("big", &"😀".repeat(10_000));
        n.picture = vec![7; 200 * 1024];
        n.lines = (0..9).map(|i| format!("line {i}")).collect();
        out.post(n, None);
        let got = collect(&mut pc_ev).await;
        let n = posted(&got)[0];
        assert!(n.text.len() <= MAX_NOTIFY_TEXT);
        assert!(n.picture.is_empty());
        assert_eq!(n.lines.len(), MAX_NOTIFY_LINES);
        assert_eq!(n.lines.last().unwrap(), "line 8");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn sync_request_answers_with_quiet_snapshot() {
        let (pc, _pc_ev, _, phone, mut phone_ev, _) = paired().await;
        let out = NotifyOutbox::new(phone.clone()).with_linger(LINGER);
        out.seed(
            (0..60)
                .map(|i| Notification {
                    when_ms: i,
                    ..note(&format!("k{i}"), "x")
                })
                .collect(),
        );
        let answer = tokio::spawn(async move {
            let mut s = phone_ev.sessions.recv().await.unwrap();
            while let Ok(Some(m)) = s.recv().await {
                if matches!(m, ControlMsg::NotifySyncRequest(_)) {
                    out.answer_sync_request(&s).await.unwrap();
                    s.finish().await;
                    return;
                }
            }
        });
        let mut s = pc.notify_sync_request(&phone.peer_id()).await.unwrap();
        let mut got = Vec::new();
        while let Ok(Some(m)) = s.recv().await {
            got.push(m);
        }
        answer.await.unwrap();
        let p = posted(&got);
        assert_eq!(p.len(), 50);
        assert!(p.iter().all(|n| n.quiet));
        assert!(p.iter().any(|n| n.key == "k59"), "newest first");
        assert!(matches!(got.last(), Some(ControlMsg::NotifySync(s)) if s.keys.len() == 60));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn seed_replaces_the_active_set() {
        let (_pc, _pc_ev, _, phone, _, _) = paired().await;
        let out = NotifyOutbox::new(phone.clone());
        out.seed(vec![note("a", "x")]);
        out.seed(vec![note("b", "x")]);
        assert_eq!(out.active_keys(), ["b"]);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn note_and_forget_track_without_sending() {
        let (_pc, _pc_ev, _, phone, _, _) = paired().await;
        let out = NotifyOutbox::new(phone.clone());
        out.note(note("a", "x"));
        out.note(note("b", "x"));
        out.forget("a");
        assert_eq!(out.active_keys(), ["b"]);
        assert!(!out.busy(), "nothing is sent while sharing is off");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn action_round_trip() {
        let (pc, _pc_ev, _, phone, mut phone_ev, _) = paired().await;
        let answer = tokio::spawn(async move {
            let mut s = phone_ev.sessions.recv().await.unwrap();
            while let Ok(Some(m)) = s.recv().await {
                if let ControlMsg::NotifyAction(a) = m {
                    assert_eq!(a.text, "ok");
                    s.send(ControlMsg::NotifyActionResult(NotifyActionResult {
                        id: a.id,
                        ok: true,
                        reason: String::new(),
                    }))
                    .await
                    .unwrap();
                }
            }
        });
        let r = pc
            .notify_action(
                &phone.peer_id(),
                NotifyAction {
                    id: 0,
                    key: "k".into(),
                    kind: NotifyActionKind::Reply as i32,
                    index: 0,
                    text: "ok".into(),
                },
            )
            .await
            .unwrap();
        assert!(r.ok);
        answer.await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn action_to_unreachable_peer_reports_failure() {
        let (pc, _pc_ev, _, phone, _phone_ev, _) = paired().await;
        let id = phone.peer_id();
        phone.shutdown().await;
        let r = tokio::time::timeout(
            Duration::from_secs(40),
            pc.notify_action(
                &id,
                NotifyAction {
                    id: 0,
                    key: "k".into(),
                    kind: NotifyActionKind::Dismiss as i32,
                    index: 0,
                    text: String::new(),
                },
            ),
        )
        .await
        .expect("must not hang");
        assert!(r.is_err());
    }
}

mod screen {
    use std::time::Duration;

    use pb_protocol::{
        control::{ScreenAccept, ScreenStart},
        InputEvent, TouchPhase,
    };
    use pb_video::record::{Config, Record, FLAG_CONFIG, FLAG_KEY};

    use super::*;
    use crate::screen::{serve_screen, ScreenEvent, SendOutcome};

    pub(super) async fn paired() -> (Node, NodeEvents, Node, NodeEvents) {
        let (pc, mut pc_ev, pc_store) = node(31, "PC").await;
        let (phone, phone_ev, _) = node(32, "Phone").await;
        let (h, j, _, _) = pair(&pc, &mut pc_ev, &phone, true, true).await;
        assert!(h.is_ok() && j.is_ok());
        let mut p = lock(&pc_store).get(&phone.peer_id()).unwrap();
        p.addrs = loopback(&phone);
        lock(&pc_store).pin(p);
        (pc, pc_ev, phone, phone_ev)
    }

    fn start() -> ScreenStart {
        ScreenStart {
            id: 1,
            max_long_side: 1920,
            fps: 30,
            codecs: vec![1, 2, 3],
            dim: false,
            sharp: false,
        }
    }

    fn accept() -> ScreenAccept {
        ScreenAccept {
            id: 1,
            codec: 2,
            width: 864,
            height: 1920,
            rotation: 0,
        }
    }

    /// Phone side: take the next session and serve (or decline) its ScreenStart.
    async fn phone_side(
        ev: &mut NodeEvents,
        answer: Option<ScreenAccept>,
    ) -> Result<crate::screen::ScreenServer, Error> {
        let mut s = ev.sessions.recv().await.unwrap();
        loop {
            match s.recv().await? {
                Some(ControlMsg::ScreenStart(st)) => return serve_screen(s, st, answer).await,
                Some(_) => continue,
                None => return Err(Error::Closed),
            }
        }
    }

    pub(super) fn rec(pts: i64, flags: u8, payload: Vec<u8>) -> Record {
        Record {
            pts_us: pts,
            flags,
            payload,
        }
    }

    async fn next_event(server: &mut crate::screen::ScreenServer) -> ScreenEvent {
        tokio::time::timeout(Duration::from_secs(10), server.events.recv())
            .await
            .expect("an event")
            .expect("events open")
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn screen_records_arrive_in_order_after_accept() {
        let (pc, _pc_ev, phone, mut phone_ev) = paired().await;
        // The phone sends once the PC reads (its channel holds 16 records and
        // drops older non-key frames when the reader falls behind).
        let (go, ready) = tokio::sync::oneshot::channel::<()>();
        let phone_task = tokio::spawn(async move {
            let server = phone_side(&mut phone_ev, Some(accept())).await.unwrap();
            let _ = ready.await;
            let cfg = Config {
                rotation: 0,
                width: 864,
                height: 1920,
                headers: vec![0, 0, 0, 1],
            };
            assert!(matches!(
                server.send(rec(0, FLAG_CONFIG, cfg.encode())),
                SendOutcome::Queued(_)
            ));
            server.send(rec(0, FLAG_KEY, vec![1; 5000]));
            for i in 1..=20 {
                server.send(rec(i * 33_333, 0, vec![i as u8; 100]));
            }
            server
        });
        let mut view = pc
            .open_screen(&phone.peer_id(), start(), || {})
            .await
            .unwrap();
        assert_eq!(view.accept.width, 864);
        let _ = go.send(());
        let mut got = Vec::new();
        while got.len() < 22 {
            let r = tokio::time::timeout(Duration::from_secs(10), view.records.recv())
                .await
                .unwrap()
                .unwrap();
            got.push(r);
        }
        assert_eq!(got[0].flags, FLAG_CONFIG);
        assert_eq!(got[1].flags, FLAG_KEY);
        assert!(got.windows(2).skip(1).all(|w| w[0].pts_us < w[1].pts_us));
        let _server = phone_task.await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn declined_screen_reports_declined() {
        let (pc, _pc_ev, phone, mut phone_ev) = paired().await;
        let phone_task = tokio::spawn(async move { phone_side(&mut phone_ev, None).await });
        let r = pc.open_screen(&phone.peer_id(), start(), || {}).await;
        assert!(matches!(r, Err(Error::Declined)), "{:?}", r.err());
        assert!(matches!(phone_task.await.unwrap(), Err(Error::Declined)));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn either_side_can_stop() {
        // The PC closes: the phone hears a deliberate close (no resume grace).
        let (pc, _pc_ev, phone, mut phone_ev) = paired().await;
        let phone_task =
            tokio::spawn(async move { phone_side(&mut phone_ev, Some(accept())).await });
        let view = pc
            .open_screen(&phone.peer_id(), start(), || {})
            .await
            .unwrap();
        let mut server = phone_task.await.unwrap().unwrap();
        view.close();
        loop {
            if let ScreenEvent::Closed { by_peer } = next_event(&mut server).await {
                assert!(by_peer, "the PC closed on purpose");
                break;
            }
        }
        // The phone closes: the PC's records end.
        let (pc, _pc_ev, phone, mut phone_ev) = paired().await;
        let phone_task =
            tokio::spawn(async move { phone_side(&mut phone_ev, Some(accept())).await });
        let mut view = pc
            .open_screen(&phone.peer_id(), start(), || {})
            .await
            .unwrap();
        let mut server = phone_task.await.unwrap().unwrap();
        server.close();
        let end = tokio::time::timeout(Duration::from_secs(10), view.records.recv())
            .await
            .unwrap();
        assert!(end.is_none());
        loop {
            if let ScreenEvent::Closed { by_peer } = next_event(&mut server).await {
                assert!(!by_peer, "closed here, not by the PC");
                break;
            }
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn feedback_and_state_reach_the_phone() {
        let (pc, _pc_ev, phone, mut phone_ev) = paired().await;
        let phone_task =
            tokio::spawn(async move { phone_side(&mut phone_ev, Some(accept())).await });
        let view = pc
            .open_screen(&phone.peer_id(), start(), || {})
            .await
            .unwrap();
        let mut server = phone_task.await.unwrap().unwrap();
        view.feedback(33_000);
        view.set_state(false, true);
        let mut feedback = None;
        let mut state = None;
        while feedback.is_none() || state.is_none() {
            match next_event(&mut server).await {
                ScreenEvent::Feedback(p) => feedback = Some(p),
                ScreenEvent::State(s) => state = Some(s),
                _ => {}
            }
        }
        assert_eq!(feedback, Some(33_000));
        let s = state.unwrap();
        assert!(!s.active && s.dim);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn touches_reach_the_phone_through_the_session() {
        let (pc, _pc_ev, phone, mut phone_ev) = paired().await;
        let phone_task =
            tokio::spawn(async move { phone_side(&mut phone_ev, Some(accept())).await });
        let view = pc
            .open_screen(&phone.peer_id(), start(), || {})
            .await
            .unwrap();
        let mut server = phone_task.await.unwrap().unwrap();
        let down = InputEvent::Touch {
            phase: TouchPhase::Down,
            x: 100,
            y: 200,
        };
        view.input.send(down);
        loop {
            if let ScreenEvent::Input(e) = next_event(&mut server).await {
                assert_eq!(e, down);
                break;
            }
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn the_phone_notice_reaches_the_pc() {
        let (pc, _pc_ev, phone, mut phone_ev) = paired().await;
        let phone_task =
            tokio::spawn(async move { phone_side(&mut phone_ev, Some(accept())).await });
        let mut view = pc
            .open_screen(&phone.peer_id(), start(), || {})
            .await
            .unwrap();
        let server = phone_task.await.unwrap().unwrap();
        server.notice("Lower quality to save memory");
        let s = tokio::time::timeout(Duration::from_secs(10), view.state.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(s.notice, "Lower quality to save memory");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn screen_session_resumes_within_grace() {
        // A dropped session can be followed at once by a new one from the
        // same PC (the phone's grace bookkeeping is in pb-ffi).
        let (pc, _pc_ev, phone, mut phone_ev) = paired().await;
        let id = phone.peer_id();
        for _ in 0..2 {
            let ev = &mut phone_ev;
            let (view, server) = tokio::join!(
                pc.open_screen(&id, start(), || {}),
                phone_side(ev, Some(accept()))
            );
            let (view, server) = (view.unwrap(), server.unwrap());
            assert_eq!(view.accept.codec, 2);
            view.close();
            drop(server);
        }
    }
}

mod camera {
    use std::time::Duration;

    use pb_protocol::control::{CameraAccept, CameraStart};
    use pb_video::record::{FLAG_CONFIG, FLAG_KEY};

    use super::screen::{paired, rec};
    use super::*;
    use crate::camera::{serve_camera, CameraEvent, CameraServer};
    use crate::screen::SendOutcome;

    fn start() -> CameraStart {
        CameraStart {
            id: 1,
            codecs: vec![2, 3],
            lens: 0,
            width: 1920,
            height: 1080,
            fps: 30,
        }
    }

    fn accept() -> CameraAccept {
        CameraAccept {
            id: 1,
            codec: 2,
            width: 1920,
            height: 1080,
            rotation: 90,
            lens: 0,
        }
    }

    async fn phone_side(
        ev: &mut NodeEvents,
        answer: Option<CameraAccept>,
    ) -> Result<CameraServer, Error> {
        let mut s = ev.sessions.recv().await.unwrap();
        loop {
            match s.recv().await? {
                Some(ControlMsg::CameraStart(st)) => return serve_camera(s, st, answer).await,
                Some(_) => continue,
                None => return Err(Error::Closed),
            }
        }
    }

    async fn next_event(server: &mut CameraServer) -> CameraEvent {
        tokio::time::timeout(Duration::from_secs(10), server.events.recv())
            .await
            .expect("an event")
            .expect("events open")
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn camera_records_arrive_in_order() {
        let (pc, _pc_ev, phone, mut phone_ev) = paired().await;
        let (go, ready) = tokio::sync::oneshot::channel::<()>();
        let phone_task = tokio::spawn(async move {
            let server = phone_side(&mut phone_ev, Some(accept())).await.unwrap();
            let _ = ready.await;
            assert!(matches!(
                server.send(rec(0, FLAG_CONFIG, vec![0, 0, 0, 1])),
                SendOutcome::Queued(_)
            ));
            server.send(rec(0, FLAG_KEY, vec![1; 5000]));
            for i in 1..=20 {
                server.send(rec(i * 33_333, 0, vec![i as u8; 100]));
            }
            server
        });
        let mut view = pc
            .open_camera(&phone.peer_id(), start(), || {})
            .await
            .unwrap();
        assert_eq!(view.accept.rotation, 90);
        let _ = go.send(());
        let mut got = Vec::new();
        while got.len() < 22 {
            let r = tokio::time::timeout(Duration::from_secs(10), view.records.recv())
                .await
                .unwrap()
                .unwrap();
            got.push(r);
        }
        assert_eq!(got[0].flags, FLAG_CONFIG);
        assert_eq!(got[1].flags, FLAG_KEY);
        assert!(got.windows(2).skip(1).all(|w| w[0].pts_us < w[1].pts_us));
        let _server = phone_task.await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn declined_camera_reports_declined() {
        let (pc, _pc_ev, phone, mut phone_ev) = paired().await;
        let phone_task = tokio::spawn(async move { phone_side(&mut phone_ev, None).await });
        let r = pc.open_camera(&phone.peer_id(), start(), || {}).await;
        assert!(matches!(r, Err(Error::Declined)), "{:?}", r.err());
        assert!(matches!(phone_task.await.unwrap(), Err(Error::Declined)));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn lens_switch_and_feedback_reach_the_phone() {
        let (pc, _pc_ev, phone, mut phone_ev) = paired().await;
        let phone_task =
            tokio::spawn(async move { phone_side(&mut phone_ev, Some(accept())).await });
        let view = pc
            .open_camera(&phone.peer_id(), start(), || {})
            .await
            .unwrap();
        let mut server = phone_task.await.unwrap().unwrap();
        view.switch_lens(1);
        view.feedback(33_000);
        let (mut lens, mut fb) = (None, None);
        while lens.is_none() || fb.is_none() {
            match next_event(&mut server).await {
                CameraEvent::Lens(l) => lens = Some(l),
                CameraEvent::Feedback(p) => fb = Some(p),
                _ => {}
            }
        }
        assert_eq!((lens, fb), (Some(1), Some(33_000)));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn the_pc_learns_the_lens_the_phone_switched_to() {
        let (pc, _pc_ev, phone, mut phone_ev) = paired().await;
        let phone_task =
            tokio::spawn(async move { phone_side(&mut phone_ev, Some(accept())).await });
        let view = pc
            .open_camera(&phone.peer_id(), start(), || {})
            .await
            .unwrap();
        let server = phone_task.await.unwrap().unwrap();
        assert_eq!(view.lens(), None, "nothing reported yet");
        let mut updates = view.lens_updates();
        server.send_lens(1);
        tokio::time::timeout(Duration::from_secs(10), updates.wait_for(|l| *l == Some(1)))
            .await
            .expect("the front lens is reported")
            .unwrap();
        assert_eq!(view.lens(), Some(1));
        // The remote half sees it too, and the sink (what the FFI holds) sends.
        let (remote, _records) = view.into_parts();
        let (sink, _events) = server.into_parts();
        sink.send_lens(0);
        tokio::time::timeout(Duration::from_secs(10), updates.wait_for(|l| *l == Some(0)))
            .await
            .expect("the back lens is reported")
            .unwrap();
        assert_eq!(remote.lens(), Some(0));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn pc_close_is_by_peer_and_phone_close_is_not() {
        let (pc, _pc_ev, phone, mut phone_ev) = paired().await;
        let phone_task =
            tokio::spawn(async move { phone_side(&mut phone_ev, Some(accept())).await });
        let view = pc
            .open_camera(&phone.peer_id(), start(), || {})
            .await
            .unwrap();
        let mut server = phone_task.await.unwrap().unwrap();
        view.close();
        loop {
            if let CameraEvent::Closed { by_peer } = next_event(&mut server).await {
                assert!(by_peer, "the PC closed on purpose");
                break;
            }
        }
        let (pc, _pc_ev, phone, mut phone_ev) = paired().await;
        let phone_task =
            tokio::spawn(async move { phone_side(&mut phone_ev, Some(accept())).await });
        let mut view = pc
            .open_camera(&phone.peer_id(), start(), || {})
            .await
            .unwrap();
        let mut server = phone_task.await.unwrap().unwrap();
        server.close();
        let end = tokio::time::timeout(Duration::from_secs(10), view.records.recv())
            .await
            .unwrap();
        assert!(end.is_none());
        loop {
            if let CameraEvent::Closed { by_peer } = next_event(&mut server).await {
                assert!(!by_peer, "closed here, not by the PC");
                break;
            }
        }
    }
}

mod identity_changed {
    use super::*;

    #[test]
    fn the_handshake_failure_for_a_new_identity_is_recognised() {
        let seen = Error::Transport(
            "the cryptographic handshake failed: error 48: invalid peer certificate: UnknownIssuer"
                .into(),
        );
        assert!(is_identity_mismatch(&seen));
        assert!(!is_identity_mismatch(&Error::Timeout));
        assert!(!is_identity_mismatch(&Error::Transport(
            "connection reset".into()
        )));
    }

    #[tokio::test]
    async fn a_reinstalled_phone_is_reported_and_clears_when_the_right_device_answers() {
        let (pc, mut pc_ev, pc_store) = node(41, "PC").await;
        let (phone, _phone_ev, _) = node(42, "Phone").await;
        let (h, j, _, _) = pair(&pc, &mut pc_ev, &phone, true, true).await;
        assert!(h.is_ok() && j.is_ok());
        let id = phone.peer_id();
        assert!(!pc.peer_changed(&id));

        // The phone is uninstalled (it no longer answers) and reinstalled:
        // a node with a new identity answers at the saved address.
        phone.shutdown().await;
        let (impostor, _iev, _) = node(43, "Phone").await;
        let mut p = lock(&pc_store).get(&id).unwrap();
        p.addrs = loopback(&impostor);
        lock(&pc_store).pin(p);
        let r = pc.connect(&id).await;
        assert!(
            matches!(r, Err(Error::PeerChanged)),
            "got {:?}",
            r.err().map(|e| e.to_string())
        );
        assert!(pc.peer_changed(&id));

        // The other device has left the old address, and the paired phone
        // answers again (a restart on a new port): cleared. The impostor
        // must be gone first: iroh keeps the address the failed dial used
        // as a path for the phone's id and tries it on the next connect,
        // and an impostor still answering there can win the handshake race
        // (`PeerChanged` again). See the note in `Node::connect`.
        impostor.shutdown().await;
        let (again, _aev, again_store) = node(42, "Phone").await;
        // The restarted phone still knows the PC, so the session opens.
        lock(&again_store).pin(PairedPeer {
            id: pc.peer_id(),
            name: "PC".into(),
            addrs: loopback(&pc),
        });
        let mut p = lock(&pc_store).get(&id).unwrap();
        p.addrs = loopback(&again);
        lock(&pc_store).pin(p);
        let r = pc.connect(&id).await;
        assert!(r.is_ok(), "got {:?}", r.err().map(|e| e.to_string()));
        assert!(!pc.peer_changed(&id));
    }

    /// Leave the flag a failed dial to a new identity would leave (that
    /// path is the test above; iroh remembers a live peer's real address,
    /// so an impostor cannot be put in front of it here).
    fn mark_changed(from: &Node, peer: PeerId) {
        lock(&from.inner.changed).peers.push(peer);
        assert!(from.peer_changed(&peer));
    }

    #[tokio::test]
    async fn pairing_again_with_the_same_device_clears_the_flag_on_both_sides() {
        let (pc, mut pc_ev, pc_store) = node(44, "PC").await;
        let (phone, _phone_ev, phone_store) = node(45, "Phone").await;
        let (h, j, _, _) = pair(&pc, &mut pc_ev, &phone, true, true).await;
        assert!(h.is_ok() && j.is_ok());
        mark_changed(&phone, pc.peer_id());
        mark_changed(&pc, phone.peer_id());

        let (h, j, _, _) = pair(&pc, &mut pc_ev, &phone, true, true).await;
        assert!(h.is_ok() && j.is_ok());
        // Joiner (pinned in `Joining::decide`) and host (in `host_pairing`).
        assert!(!phone.peer_changed(&pc.peer_id()));
        assert!(!pc.peer_changed(&phone.peer_id()));
        assert!(lock(&phone_store).get(&pc.peer_id()).is_some());
        assert!(lock(&pc_store).get(&phone.peer_id()).is_some());
    }

    #[tokio::test]
    async fn an_authenticated_incoming_session_clears_a_stale_flag() {
        let (pc, mut pc_ev, _) = node(48, "PC").await;
        let (phone, _phone_ev, _) = node(49, "Phone").await;
        let (h, j, _, _) = pair(&pc, &mut pc_ev, &phone, true, true).await;
        assert!(h.is_ok() && j.is_ok());
        mark_changed(&pc, phone.peer_id());

        // The phone dials the PC: its TLS identity is the paired one.
        let _s = phone.connect(&pc.peer_id()).await.unwrap();
        let _incoming = pc_ev.sessions.recv().await.unwrap();
        assert!(!pc.peer_changed(&phone.peer_id()));
    }

    #[tokio::test]
    async fn a_forged_candidate_does_not_make_an_away_peer_look_reset() {
        let (pc, mut pc_ev, pc_store) = node(61, "PC").await;
        let (phone, _phone_ev, _) = node(62, "Phone").await;
        let (h, j, _, _) = pair(&pc, &mut pc_ev, &phone, true, true).await;
        assert!(h.is_ok() && j.is_ok());
        let id = phone.peer_id();

        // The phone is away: nothing answers at its saved address.
        phone.shutdown().await;
        let mut p = lock(&pc_store).get(&id).unwrap();
        p.addrs = vec!["127.0.0.1:1".into()];
        lock(&pc_store).pin(p);
        // An unpaired LAN host replays the phone's discovery tag from its own
        // address, where a device with another identity answers.
        let (impostor, _iev, _) = node(63, "Impostor").await;
        pc.test_add_candidate(id, SocketAddr::from(([127, 0, 0, 1], impostor.port())));

        let r = pc.connect(&id).await;
        assert!(r.is_err());
        assert!(
            !matches!(r, Err(Error::PeerChanged)),
            "a forged candidate must not read as a reset"
        );
        assert!(!pc.peer_changed(&id));
        // Asked again (iroh now remembers the forged address as a path):
        // still not a reset.
        let r = pc.connect(&id).await;
        assert!(!matches!(r, Err(Error::PeerChanged)));
        assert!(!pc.peer_changed(&id));
    }

    #[tokio::test]
    async fn a_mismatch_that_lands_after_unpair_or_a_new_pairing_is_dropped() {
        let (pc, mut pc_ev, _) = node(64, "PC").await;
        let (phone, _phone_ev, _) = node(65, "Phone").await;
        let (h, j, _, _) = pair(&pc, &mut pc_ev, &phone, true, true).await;
        assert!(h.is_ok() && j.is_ok());
        let id = phone.peer_id();

        // A dial in flight when the peer pairs again (or connects to us):
        // its late mismatch is about the device from before.
        let epoch = pc.changed_epoch(&id);
        pc.forget_changed(&id);
        assert!(!pc.note_mismatch(&id, epoch));
        assert!(!pc.peer_changed(&id));

        // A dial in flight when the user unpairs: nothing is left behind.
        let epoch = pc.changed_epoch(&id);
        assert!(pc.unpair(&id));
        assert!(!pc.note_mismatch(&id, epoch));
        assert!(lock(&pc.inner.changed).peers.is_empty());

        // Not paired at all: never flagged.
        assert!(!pc.note_mismatch(&id, pc.changed_epoch(&id)));
        assert!(lock(&pc.inner.changed).peers.is_empty());
    }

    #[tokio::test]
    async fn a_current_mismatch_for_a_paired_peer_is_flagged() {
        let (pc, mut pc_ev, _) = node(66, "PC").await;
        let (phone, _phone_ev, _) = node(67, "Phone").await;
        let (h, j, _, _) = pair(&pc, &mut pc_ev, &phone, true, true).await;
        assert!(h.is_ok() && j.is_ok());
        let id = phone.peer_id();
        assert!(pc.note_mismatch(&id, pc.changed_epoch(&id)));
        assert!(pc.peer_changed(&id));
    }

    /// Racy by nature (iroh sends the first packets to every path at once):
    /// without the retry in `dial_known` this failed 5 of 30 runs; with it,
    /// 150 of 150 passed.
    #[tokio::test]
    async fn a_forged_candidate_does_not_hide_the_real_peer_at_its_saved_address() {
        let (pc, mut pc_ev, pc_store) = node(68, "PC").await;
        let (phone, _phone_ev, _) = node(69, "Phone").await;
        let (h, j, _, _) = pair(&pc, &mut pc_ev, &phone, true, true).await;
        assert!(h.is_ok() && j.is_ok());
        let id = phone.peer_id();
        // The phone answers at its saved address; a LAN host plants a
        // candidate where a device with another identity answers.
        let mut p = lock(&pc_store).get(&id).unwrap();
        p.addrs = loopback(&phone);
        lock(&pc_store).pin(p);
        let (impostor, _iev, _) = node(70, "Impostor").await;
        pc.test_add_candidate(id, SocketAddr::from(([127, 0, 0, 1], impostor.port())));
        let r = pc.connect(&id).await;
        assert!(r.is_ok(), "got {:?}", r.err().map(|e| e.to_string()));
        assert!(!pc.peer_changed(&id));
    }

    #[tokio::test]
    async fn another_peer_proving_itself_does_not_hide_a_reset() {
        let (pc, mut pc_ev, _) = node(71, "PC").await;
        let (a, _aev, _) = node(72, "Phone A").await;
        let (b, _bev, _) = node(73, "Phone B").await;
        for p in [&a, &b] {
            let (h, j, _, _) = pair(&pc, &mut pc_ev, p, true, true).await;
            assert!(h.is_ok() && j.is_ok());
        }
        let (ida, idb) = (a.peer_id(), b.peer_id());
        // A's dial is in flight when B connects (or pairs again).
        let epoch = pc.changed_epoch(&ida);
        pc.forget_changed(&idb);
        assert!(
            pc.note_mismatch(&ida, epoch),
            "B's session says nothing about A"
        );
        assert!(pc.peer_changed(&ida));
    }

    #[tokio::test]
    async fn a_planted_path_stays_distrusted_after_remove_and_pair_again() {
        let (pc, mut pc_ev, _) = node(74, "PC").await;
        let (phone, _phone_ev, _) = node(75, "Phone").await;
        let (h, j, _, _) = pair(&pc, &mut pc_ev, &phone, true, true).await;
        assert!(h.is_ok() && j.is_ok());
        let id = phone.peer_id();
        // A planted candidate met a different identity earlier in this run.
        pc.distrust_mismatches(&id);
        pc.test_add_candidate(id, SocketAddr::from(([127, 0, 0, 1], 40557)));
        // The user removes the phone and pairs it again: iroh still has the
        // planted path (the endpoint lives on), so its answers prove nothing.
        assert!(pc.unpair(&id));
        assert!(pc.candidates(&id).is_empty(), "unpair drops its candidates");
        let (h, j, _, _) = pair(&pc, &mut pc_ev, &phone, true, true).await;
        assert!(h.is_ok() && j.is_ok());
        assert!(!pc.note_mismatch(&id, pc.changed_epoch(&id)));
        assert!(!pc.peer_changed(&id));
    }

    #[test]
    fn only_a_learned_address_that_is_not_saved_makes_a_mismatch_doubtful() {
        let a: SocketAddr = "192.168.1.5:4000".parse().unwrap();
        let b: SocketAddr = "192.168.1.9:4000".parse().unwrap();
        let saved = vec!["192.168.1.5:4000".to_string()];
        assert!(!has_unsaved_candidate(&[], &saved));
        assert!(!has_unsaved_candidate(&[a], &saved), "already saved");
        assert!(has_unsaved_candidate(&[b], &saved));
        assert!(has_unsaved_candidate(&[a, b], &saved));
        assert!(has_unsaved_candidate(&[a], &[]));
    }

    #[test]
    fn the_changed_error_does_not_claim_a_reset() {
        let m = Error::PeerChanged.to_string();
        assert!(m.starts_with("another device answers for this peer"), "{m}");
        assert!(m.contains("if it was reinstalled or reset"), "{m}");
    }

    #[tokio::test]
    async fn an_unknown_incoming_device_leaves_the_flag_alone() {
        let (pc, mut pc_ev, _) = node(51, "PC").await;
        let (phone, _phone_ev, _) = node(52, "Phone").await;
        let (h, j, _, _) = pair(&pc, &mut pc_ev, &phone, true, true).await;
        assert!(h.is_ok() && j.is_ok());
        mark_changed(&pc, phone.peer_id());

        // A stranger that thinks it is paired with the PC is refused.
        let (stranger, _sev, stranger_store) = node(54, "Stranger").await;
        lock(&stranger_store).pin(PairedPeer {
            id: pc.peer_id(),
            name: "PC".into(),
            addrs: loopback(&pc),
        });
        assert!(stranger.connect(&pc.peer_id()).await.is_err());
        assert!(pc.peer_changed(&phone.peer_id()));
    }

    mod audio {
        use std::time::{Duration, Instant};

        use pb_protocol::control::{
            AudioAccept, AudioReason, AudioStart, AudioStop, AUDIO_PC_TO_PHONE, AUDIO_PHONE_TO_PC,
            AUDIO_SOURCE_MIC,
        };

        use super::screen::paired;
        use super::*;
        use crate::audio::{
            encode_packet, request_audio, serve_audio, AudioEvent, AudioFrame, AudioSession,
        };

        // One frame fits one datagram, so frames arrive as they were sent.
        const FRAME: usize = 960;

        fn req(direction: u32) -> AudioStart {
            AudioStart {
                id: 0,
                sample_rate: 48000,
                channels: 2,
                direction,
                source: 0,
            }
        }

        fn ok_accept() -> AudioAccept {
            AudioAccept {
                id: 0,
                sample_rate: 48000,
                channels: 2,
                accepted: true,
                reason: 0,
            }
        }

        /// A frame whose first four bytes carry `i`.
        fn frame(i: u32) -> Vec<u8> {
            let mut f = vec![0u8; FRAME];
            f[..4].copy_from_slice(&i.to_le_bytes());
            f
        }

        fn index(f: &AudioFrame) -> u32 {
            u32::from_le_bytes(f.samples[..4].try_into().unwrap())
        }

        /// The responder: the node that received the `AudioStart`.
        async fn responder(
            node: &Node,
            ev: &mut NodeEvents,
            answer: Result<AudioAccept, AudioReason>,
        ) -> Result<AudioSession, Error> {
            let mut s = ev.sessions.recv().await.unwrap();
            loop {
                match s.recv().await? {
                    Some(ControlMsg::AudioStart(st)) => {
                        return serve_audio(s, st, answer, node.audio_slots()).await
                    }
                    Some(_) => continue,
                    None => return Err(Error::Closed),
                }
            }
        }

        /// Run the exchange: (requester session, responder session).
        async fn establish(
            pc: &Node,
            phone: &Node,
            phone_ev: &mut NodeEvents,
            direction: u32,
        ) -> (AudioSession, AudioSession) {
            let phone_id = phone.peer_id();
            let (a, b) = tokio::join!(
                pc.open_audio(&phone_id, req(direction), || {}),
                responder(phone, phone_ev, Ok(ok_accept())),
            );
            (a.expect("requester"), b.expect("responder"))
        }

        /// (sender, receiver) of an established pair.
        fn by_role(a: AudioSession, b: AudioSession) -> (AudioSession, AudioSession) {
            if a.is_sender() {
                (a, b)
            } else {
                (b, a)
            }
        }

        async fn next_event(s: &mut AudioSession) -> AudioEvent {
            tokio::time::timeout(Duration::from_secs(10), s.events.recv())
                .await
                .expect("an event")
                .expect("events open")
        }

        async fn next_frame(s: &mut AudioSession) -> AudioFrame {
            tokio::time::timeout(Duration::from_secs(10), s.recv_frame())
                .await
                .expect("a frame")
                .expect("stream open")
        }

        #[tokio::test(flavor = "multi_thread")]
        async fn frames_arrive_in_order_both_directions() {
            for dir in [AUDIO_PHONE_TO_PC, AUDIO_PC_TO_PHONE] {
                let (pc, _pc_ev, phone, mut phone_ev) = paired().await;
                let (a, b) = establish(&pc, &phone, &mut phone_ev, dir).await;
                assert_eq!(a.accept.sample_rate, 48000);
                let (tx, mut rx) = by_role(a, b);
                assert!(tx.is_sender());
                assert!(!rx.is_sender());
                assert_eq!(tx.start.direction, dir);
                for i in 0..10 {
                    tx.send_frame(frame(i)).unwrap();
                }
                for i in 0..10 {
                    assert_eq!(index(&next_frame(&mut rx).await), i, "direction {dir}");
                }
            }
        }

        #[tokio::test(flavor = "multi_thread")]
        async fn the_receiving_end_cannot_send_and_the_sending_end_cannot_receive() {
            let (pc, _pc_ev, phone, mut phone_ev) = paired().await;
            let (a, b) = establish(&pc, &phone, &mut phone_ev, AUDIO_PHONE_TO_PC).await;
            let (mut tx, rx) = by_role(a, b);
            assert!(rx.send_frame(frame(0)).is_err());
            assert!(tx.recv_frame().await.is_none());
        }

        #[tokio::test(flavor = "multi_thread")]
        async fn a_sent_frame_must_be_valid() {
            let (pc, _pc_ev, phone, mut phone_ev) = paired().await;
            let (a, b) = establish(&pc, &phone, &mut phone_ev, AUDIO_PHONE_TO_PC).await;
            let (tx, _rx) = by_role(a, b);
            assert!(tx.send_frame(Vec::new()).is_err());
            assert!(
                tx.send_frame(vec![0; 6]).is_err(),
                "not a whole stereo sample"
            );
            assert!(tx.send_frame(vec![0; 65_536 + 4]).is_err());
            assert!(tx.send_frame(vec![0; 4]).is_ok());
        }

        #[tokio::test(flavor = "multi_thread")]
        async fn a_declined_request_reports_declined_both_directions() {
            for dir in [AUDIO_PHONE_TO_PC, AUDIO_PC_TO_PHONE] {
                let (pc, _pc_ev, phone, mut phone_ev) = paired().await;
                let phone_id = phone.peer_id();
                let (a, b) = tokio::join!(
                    pc.open_audio(&phone_id, req(dir), || {}),
                    responder(&phone, &mut phone_ev, Err(AudioReason::Declined)),
                );
                assert!(matches!(a, Err(Error::Declined)));
                assert!(matches!(b, Err(Error::Declined)));
                assert!(!pc.audio_slots().is_busy(dir));
                assert!(!phone.audio_slots().is_busy(dir));
            }
        }

        #[tokio::test(flavor = "multi_thread")]
        async fn every_refusal_reason_reaches_the_requester() {
            for reason in [
                AudioReason::Unavailable,
                AudioReason::NoPermission,
                AudioReason::AppBlocksCapture,
            ] {
                let (pc, _pc_ev, phone, mut phone_ev) = paired().await;
                let phone_id = phone.peer_id();
                let (a, _b) = tokio::join!(
                    pc.open_audio(&phone_id, req(AUDIO_PHONE_TO_PC), || {}),
                    responder(&phone, &mut phone_ev, Err(reason)),
                );
                match a {
                    Err(Error::AudioRefused(r)) => assert_eq!(r, reason),
                    other => panic!("unexpected {:?}", other.map(|_| ())),
                }
            }
        }

        #[tokio::test(flavor = "multi_thread")]
        async fn a_second_session_in_the_same_direction_is_busy() {
            let (pc, _pc_ev, phone, mut phone_ev) = paired().await;
            let (a, b) = establish(&pc, &phone, &mut phone_ev, AUDIO_PHONE_TO_PC).await;

            // Same direction: the requester refuses locally, before any traffic.
            let local = pc
                .open_audio(&phone.peer_id(), req(AUDIO_PHONE_TO_PC), || {})
                .await;
            assert!(matches!(local, Err(Error::AudioRefused(AudioReason::Busy))));

            // A peer that does not check still gets Busy from the responder.
            let mut raw = pc.connect(&phone.peer_id()).await.unwrap();
            let mut st = req(AUDIO_PHONE_TO_PC);
            st.id = 77;
            raw.send(ControlMsg::AudioStart(st)).await.unwrap();
            let served = responder(&phone, &mut phone_ev, Ok(ok_accept())).await;
            assert!(matches!(
                served,
                Err(Error::AudioRefused(AudioReason::Busy))
            ));
            let acc = loop {
                if let Some(ControlMsg::AudioAccept(acc)) = raw.recv().await.unwrap() {
                    break acc;
                }
            };
            assert!(!acc.accepted);
            assert_eq!(acc.reason, AudioReason::Busy.code());

            // The other direction is independent.
            let (c, d) = establish(&pc, &phone, &mut phone_ev, AUDIO_PC_TO_PHONE).await;
            drop((a, b, c, d));
        }

        #[tokio::test(flavor = "multi_thread")]
        async fn the_slot_is_free_again_after_the_session_ends() {
            let (pc, _pc_ev, phone, mut phone_ev) = paired().await;
            let (a, b) = establish(&pc, &phone, &mut phone_ev, AUDIO_PHONE_TO_PC).await;
            assert!(pc.audio_slots().is_busy(AUDIO_PHONE_TO_PC));
            assert!(phone.audio_slots().is_busy(AUDIO_PHONE_TO_PC));
            drop((a, b));
            assert!(!pc.audio_slots().is_busy(AUDIO_PHONE_TO_PC));
            assert!(!phone.audio_slots().is_busy(AUDIO_PHONE_TO_PC));
            let (_a, _b) = establish(&pc, &phone, &mut phone_ev, AUDIO_PHONE_TO_PC).await;
        }

        #[tokio::test(flavor = "multi_thread")]
        async fn bad_parameters_are_refused_before_anything_is_sent() {
            let (pc, _pc_ev, phone, mut phone_ev) = paired().await;
            let phone_id = phone.peer_id();
            let mut bad = req(AUDIO_PHONE_TO_PC);
            bad.sample_rate = 96000;
            let res = pc.open_audio(&phone_id, bad, || {}).await;
            assert!(matches!(res, Err(Error::Protocol(_))));
            let mut mic = req(AUDIO_PC_TO_PHONE);
            mic.source = AUDIO_SOURCE_MIC;
            let res = pc.open_audio(&phone_id, mic, || {}).await;
            assert!(matches!(res, Err(Error::Protocol(_))));
            assert!(!pc.audio_slots().is_busy(AUDIO_PHONE_TO_PC));
            assert!(!pc.audio_slots().is_busy(AUDIO_PC_TO_PHONE));
            // No session was even dialled, so nothing reached the phone.
            let got =
                tokio::time::timeout(Duration::from_millis(500), phone_ev.sessions.recv()).await;
            assert!(got.is_err(), "no connection may have been made");
        }

        #[tokio::test(flavor = "multi_thread")]
        async fn a_hostile_start_is_refused_with_bad_params_by_the_responder() {
            let (pc, _pc_ev, phone, mut phone_ev) = paired().await;
            for (rate, ch, dir, src) in [
                (96000, 2, 0, 0),
                (48000, 3, 0, 0),
                (48000, 2, 5, 0),
                (48000, 2, 1, AUDIO_SOURCE_MIC),
            ] {
                let mut raw = pc.connect(&phone.peer_id()).await.unwrap();
                raw.send(ControlMsg::AudioStart(AudioStart {
                    id: 5,
                    sample_rate: rate,
                    channels: ch,
                    direction: dir,
                    source: src,
                }))
                .await
                .unwrap();
                let served = responder(&phone, &mut phone_ev, Ok(ok_accept())).await;
                assert!(
                    matches!(served, Err(Error::Protocol(_))),
                    "{rate} {ch} {dir} {src}"
                );
                let acc = loop {
                    match raw.recv().await {
                        Ok(Some(ControlMsg::AudioAccept(a))) => break a,
                        Ok(Some(_)) => continue,
                        other => panic!("no reply: {:?}", other.map(|_| ())),
                    }
                };
                assert!(!acc.accepted);
                assert_eq!(acc.reason, AudioReason::BadParams.code());
                assert!(!phone.audio_slots().is_busy(0) && !phone.audio_slots().is_busy(1));
            }
        }

        #[tokio::test(flavor = "multi_thread")]
        async fn stop_from_either_side_ends_both_both_directions() {
            for dir in [AUDIO_PHONE_TO_PC, AUDIO_PC_TO_PHONE] {
                for stopper_is_sender in [true, false] {
                    let (pc, _pc_ev, phone, mut phone_ev) = paired().await;
                    let (a, b) = establish(&pc, &phone, &mut phone_ev, dir).await;
                    let (mut tx, mut rx) = by_role(a, b);
                    if stopper_is_sender {
                        tx.stop();
                        assert!(matches!(
                            next_event(&mut tx).await,
                            AudioEvent::Stopped { by_peer: false }
                        ));
                        assert!(matches!(
                            next_event(&mut rx).await,
                            AudioEvent::Stopped { by_peer: true }
                        ));
                    } else {
                        rx.stop();
                        assert!(matches!(
                            next_event(&mut rx).await,
                            AudioEvent::Stopped { by_peer: false }
                        ));
                        assert!(matches!(
                            next_event(&mut tx).await,
                            AudioEvent::Stopped { by_peer: true }
                        ));
                    }
                    assert!(tx.send_frame(frame(1)).is_err(), "no frames after a stop");
                    assert!(rx.recv_frame().await.is_none());
                }
            }
        }

        /// The PC's pattern: press Stop, then let go of the session at once.
        /// The peer must hear "stopped by the peer", never a lost link.
        #[tokio::test(flavor = "multi_thread")]
        async fn stop_then_drop_at_once_still_reads_as_a_peer_stop() {
            for dir in [AUDIO_PHONE_TO_PC, AUDIO_PC_TO_PHONE] {
                for stopper_is_sender in [true, false] {
                    for _ in 0..5 {
                        let (pc, _pc_ev, phone, mut phone_ev) = paired().await;
                        let (a, b) = establish(&pc, &phone, &mut phone_ev, dir).await;
                        let (tx, rx) = by_role(a, b);
                        let (stopper, mut other) = if stopper_is_sender {
                            (tx, rx)
                        } else {
                            (rx, tx)
                        };
                        stopper.stop();
                        drop(stopper);
                        assert!(
                            matches!(
                                next_event(&mut other).await,
                                AudioEvent::Stopped { by_peer: true }
                            ),
                            "dir {dir} stopper_is_sender {stopper_is_sender}"
                        );
                    }
                }
            }
        }

        #[tokio::test(flavor = "multi_thread")]
        async fn dropping_the_receiver_closes_the_session_for_the_sender() {
            for dir in [AUDIO_PHONE_TO_PC, AUDIO_PC_TO_PHONE] {
                let (pc, _pc_ev, phone, mut phone_ev) = paired().await;
                let (a, b) = establish(&pc, &phone, &mut phone_ev, dir).await;
                let (mut tx, rx) = by_role(a, b);
                drop(rx);
                assert!(matches!(
                    next_event(&mut tx).await,
                    AudioEvent::Stopped { by_peer: false }
                ));
                assert!(tx.send_frame(frame(1)).is_err());
            }
        }

        #[tokio::test(flavor = "multi_thread")]
        async fn dropping_the_sender_ends_the_stream_for_the_receiver() {
            let (pc, _pc_ev, phone, mut phone_ev) = paired().await;
            let (a, b) = establish(&pc, &phone, &mut phone_ev, AUDIO_PC_TO_PHONE).await;
            let (tx, mut rx) = by_role(a, b);
            drop(tx);
            assert!(matches!(
                next_event(&mut rx).await,
                AudioEvent::Stopped { .. }
            ));
            assert!(rx.recv_frame().await.is_none());
        }

        #[tokio::test(flavor = "multi_thread")]
        async fn a_slow_consumer_never_stalls_the_sender_and_gets_the_newest_audio() {
            let (pc, _pc_ev, phone, mut phone_ev) = paired().await;
            let (a, b) = establish(&pc, &phone, &mut phone_ev, AUDIO_PHONE_TO_PC).await;
            let (tx, mut rx) = by_role(a, b);
            const N: u32 = 3000; // ~5.7 MB, far over ~300 ms of audio
            let t = Instant::now();
            for i in 0..N {
                tx.send_frame(frame(i)).unwrap();
                if i % 100 == 0 {
                    tokio::task::yield_now().await;
                }
            }
            assert!(
                t.elapsed() < Duration::from_secs(2),
                "send_frame must not block"
            );
            // The consumer only now starts reading: it must reach the last
            // frame, in order, having missed some of the older ones.
            let mut seen = 0u32;
            let mut last = 0;
            let mut prev = None;
            while last != N - 1 {
                let f = next_frame(&mut rx).await;
                if f.samples.iter().all(|&b| b == 0) {
                    continue; // silence for a lost packet (or frame 0)
                }
                last = index(&f);
                if let Some(p) = prev {
                    assert!(last > p, "frames never reorder");
                }
                prev = Some(last);
                seen += 1;
            }
            assert!(seen < N, "old frames were dropped ({seen} of {N})");
        }

        /// A UDP relay between two nodes that can black traffic out and cap
        /// its rate, standing in for a congested 2.4 GHz link.
        struct Impair {
            blackout: std::sync::atomic::AtomicBool,
            /// Bytes per second per direction.
            cap: u64,
            forwarded: std::sync::atomic::AtomicU64,
        }

        struct Bucket {
            tokens: f64,
            at: Instant,
        }

        impl Impair {
            /// Whether this packet survives. Over the cap it is lost, as on a
            /// full radio queue.
            fn pass(&self, bucket: &mut Bucket, len: usize) -> bool {
                use std::sync::atomic::Ordering::Relaxed;
                if self.blackout.load(Relaxed) {
                    return false;
                }
                let now = Instant::now();
                bucket.tokens = (bucket.tokens
                    + now.duration_since(bucket.at).as_secs_f64() * self.cap as f64)
                    .min(self.cap as f64 / 20.0);
                bucket.at = now;
                if bucket.tokens < len as f64 {
                    return false;
                }
                bucket.tokens -= len as f64;
                self.forwarded.fetch_add(len as u64, Relaxed);
                true
            }
        }

        async fn relay(target: u16, cap: u64) -> (u16, Arc<Impair>) {
            use tokio::net::UdpSocket;
            let front = Arc::new(UdpSocket::bind("127.0.0.1:0").await.unwrap());
            let back = Arc::new(UdpSocket::bind("127.0.0.1:0").await.unwrap());
            back.connect(("127.0.0.1", target)).await.unwrap();
            let port = front.local_addr().unwrap().port();
            let imp = Arc::new(Impair {
                blackout: false.into(),
                cap,
                forwarded: 0.into(),
            });
            let client = Arc::new(Mutex::new(None));
            let bucket = || Bucket {
                tokens: 0.0,
                at: Instant::now(),
            };
            {
                let (front, back, imp, client) =
                    (front.clone(), back.clone(), imp.clone(), client.clone());
                let mut b = bucket();
                tokio::spawn(async move {
                    let mut buf = vec![0u8; 65_536];
                    while let Ok((n, from)) = front.recv_from(&mut buf).await {
                        *lock(&client) = Some(from);
                        if imp.pass(&mut b, n) {
                            let _ = back.send(&buf[..n]).await;
                        }
                    }
                });
            }
            {
                let imp = imp.clone();
                let mut b = bucket();
                tokio::spawn(async move {
                    let mut buf = vec![0u8; 65_536];
                    while let Ok(n) = back.recv(&mut buf).await {
                        let to = *lock(&client);
                        if let (Some(to), true) = (to, imp.pass(&mut b, n)) {
                            let _ = front.send_to(&buf[..n], to).await;
                        }
                    }
                });
            }
            (port, imp)
        }

        /// The user's bug: after a Wi-Fi stall the PC kept playing old audio
        /// for many seconds. A 3 s blackout on a link only a little faster
        /// than the audio must not leave a backlog behind it.
        #[tokio::test(flavor = "multi_thread")]
        async fn audio_is_current_again_right_after_a_blackout() {
            use std::sync::atomic::Ordering::Relaxed;
            const STEREO_5MS: usize = 960;
            let (pc, mut pc_ev, pc_store) = super::node(61, "PC").await;
            let (phone, mut phone_ev, _) = super::node(62, "Phone").await;
            let (h, j, _, _) = super::pair(&pc, &mut pc_ev, &phone, true, true).await;
            assert!(h.is_ok() && j.is_ok());
            // 192 KB/s of audio over a 280 KB/s link.
            let (port, imp) = relay(phone.port(), 280_000).await;
            let mut p = lock(&pc_store).get(&phone.peer_id()).unwrap();
            p.addrs = vec![format!("127.0.0.1:{port}")];
            lock(&pc_store).pin(p);
            // A fresh endpoint with the same identity: the old one remembers
            // the phone's real address from pairing and would dial around
            // the relay.
            drop(pc_ev);
            pc.shutdown().await;
            let (pc, _pc_ev) = Node::bind(
                &Identity::from_seed([61; 32]),
                pc_store.clone(),
                NodeConfig {
                    device_name: "PC".into(),
                    port: None,
                    discovery_port: None,
                    tuning: Default::default(),
                },
            )
            .await
            .unwrap();

            let (a, b) = establish(&pc, &phone, &mut phone_ev, AUDIO_PHONE_TO_PC).await;
            let (tx, mut rx) = by_role(a, b);
            let began = Instant::now();
            let sender = tokio::spawn(async move {
                let mut i = 0u32;
                let mut tick = tokio::time::interval(Duration::from_millis(5));
                while began.elapsed() < Duration::from_secs(13) {
                    tick.tick().await;
                    let mut f = vec![1u8; STEREO_5MS];
                    f[..4].copy_from_slice(&i.to_le_bytes());
                    f[4..8].copy_from_slice(&(began.elapsed().as_millis() as u32).to_le_bytes());
                    if tx.send_frame(f).is_err() {
                        break;
                    }
                    i += 1;
                }
                tx
            });
            let (mut worst_before, mut worst_after, mut after) = (0u32, 0u32, 0u32);
            tokio::time::sleep(Duration::from_secs(2)).await;
            imp.blackout.store(true, Relaxed);
            let lifted = tokio::spawn({
                let imp = imp.clone();
                async move {
                    tokio::time::sleep(Duration::from_secs(3)).await;
                    imp.blackout.store(false, Relaxed);
                }
            });
            // From 1 s after the blackout (t = 6 s), every frame must be recent.
            while began.elapsed() < Duration::from_secs(12) {
                let Ok(Some(f)) =
                    tokio::time::timeout(Duration::from_millis(500), rx.recv_frame()).await
                else {
                    continue;
                };
                if f.samples.iter().all(|&b| b == 0) {
                    continue; // silence for a lost packet
                }
                let sent = u32::from_le_bytes(f.samples[4..8].try_into().unwrap());
                let now = began.elapsed().as_millis() as u32;
                let age = now.saturating_sub(sent);
                if now < 2_000 {
                    worst_before = worst_before.max(age);
                } else if now >= 6_000 {
                    worst_after = worst_after.max(age);
                    after += 1;
                }
            }
            lifted.await.unwrap();
            let forwarded = imp.forwarded.load(Relaxed);
            drop(sender);
            eprintln!(
                "blackout: worst age before {worst_before} ms, after {worst_after} ms,                  {after} frames after, {forwarded} bytes relayed"
            );
            assert!(forwarded > 500_000, "the audio must really cross the relay");
            assert!(after > 50, "audio must flow again after the blackout");
            assert!(
                worst_after < 400,
                "audio was {worst_after} ms old a second after the blackout"
            );
        }

        /// Raw responder for hostile-peer tests: accepts with `accept` itself.
        async fn raw_responder(ev: &mut NodeEvents, accept: AudioAccept) -> Session {
            let mut s = ev.sessions.recv().await.unwrap();
            loop {
                if let Some(ControlMsg::AudioStart(st)) = s.recv().await.unwrap() {
                    let mut acc = accept;
                    acc.id = st.id;
                    s.send(ControlMsg::AudioAccept(acc)).await.unwrap();
                    return s;
                }
            }
        }

        #[tokio::test(flavor = "multi_thread")]
        async fn malformed_packets_are_violations() {
            let bad: [Vec<u8>; 3] = [vec![1u8; 3], vec![1u8; 8], vec![1u8; 8 + 6]];
            for packet in bad {
                let (pc, _pc_ev, phone, mut phone_ev) = paired().await;
                let phone_id = phone.peer_id();
                let (view, raw) = tokio::join!(
                    pc.open_audio(&phone_id, req(AUDIO_PHONE_TO_PC), || {}),
                    raw_responder(&mut phone_ev, ok_accept()),
                );
                let mut view = view.unwrap();
                raw.conn
                    .send_datagram(encode_packet(0, 0, &frame(4)).into())
                    .unwrap();
                assert_eq!(index(&next_frame(&mut view).await), 4);
                let len = packet.len();
                raw.conn.send_datagram(packet.into()).unwrap();
                assert!(
                    matches!(next_event(&mut view).await, AudioEvent::Violation(_)),
                    "packet of {len} bytes"
                );
                assert!(view.recv_frame().await.is_none());
            }
        }

        #[tokio::test(flavor = "multi_thread")]
        async fn the_peer_going_away_ends_the_sender_and_refuses_frames() {
            let (pc, _pc_ev, phone, mut phone_ev) = paired().await;
            let phone_id = phone.peer_id();
            let (view, raw) = tokio::join!(
                pc.open_audio(&phone_id, req(AUDIO_PC_TO_PHONE), || {}),
                raw_responder(&mut phone_ev, ok_accept()),
            );
            let mut view = view.unwrap();
            raw.conn.close(0u32.into(), b"gone");
            let ev = next_event(&mut view).await;
            assert!(
                matches!(
                    ev,
                    AudioEvent::Stopped { by_peer: false } | AudioEvent::WriteFailed(_)
                ),
                "{ev:?}"
            );
            assert!(view.send_frame(frame(0)).is_err());
        }

        #[tokio::test(flavor = "multi_thread")]
        async fn a_hostile_accept_is_rejected() {
            let bad_accepts = [
                AudioAccept {
                    sample_rate: 96000,
                    ..ok_accept()
                },
                AudioAccept {
                    channels: 6,
                    ..ok_accept()
                },
                AudioAccept {
                    sample_rate: 0,
                    channels: 0,
                    ..ok_accept()
                },
            ];
            for bad in bad_accepts {
                let (pc, _pc_ev, phone, mut phone_ev) = paired().await;
                let phone_id = phone.peer_id();
                let (res, _raw) = tokio::join!(
                    pc.open_audio(&phone_id, req(AUDIO_PHONE_TO_PC), || {}),
                    raw_responder(&mut phone_ev, bad.clone()),
                );
                assert!(matches!(res, Err(Error::Protocol(_))), "{bad:?}");
                assert!(!pc.audio_slots().is_busy(AUDIO_PHONE_TO_PC));
            }
        }

        #[tokio::test(flavor = "multi_thread")]
        async fn an_unknown_reason_code_reads_as_declined() {
            let (pc, _pc_ev, phone, mut phone_ev) = paired().await;
            let phone_id = phone.peer_id();
            let (res, _raw) = tokio::join!(
                pc.open_audio(&phone_id, req(AUDIO_PHONE_TO_PC), || {}),
                raw_responder(
                    &mut phone_ev,
                    AudioAccept {
                        accepted: false,
                        reason: 99,
                        ..ok_accept()
                    }
                ),
            );
            assert!(matches!(res, Err(Error::Declined)));
        }

        #[tokio::test(flavor = "multi_thread")]
        async fn an_old_peer_without_the_audio_capability_fails_fast() {
            let (pc, _pc_ev, phone, mut phone_ev) = paired().await;
            let mut s = pc.connect(&phone.peer_id()).await.unwrap();
            let mut theirs = phone_ev.sessions.recv().await.unwrap();
            s.remote_hello.capabilities = vec!["clipboard".into()];
            let t = Instant::now();
            let res = request_audio(s, req(AUDIO_PHONE_TO_PC), pc.audio_slots(), || {}).await;
            assert!(matches!(res, Err(Error::Unsupported)));
            assert!(
                t.elapsed() < Duration::from_secs(2),
                "must not wait for a reply"
            );
            assert!(!pc.audio_slots().is_busy(AUDIO_PHONE_TO_PC));
            // Nothing was sent: the next thing the phone sees is the close.
            assert!(!matches!(
                theirs.recv().await,
                Ok(Some(ControlMsg::AudioStart(_)))
            ));
        }

        #[tokio::test(flavor = "multi_thread")]
        async fn this_build_announces_the_audio_capability() {
            let (pc, _pc_ev, phone, mut phone_ev) = paired().await;
            let s = pc.connect(&phone.peer_id()).await.unwrap();
            let theirs = phone_ev.sessions.recv().await.unwrap();
            assert!(theirs.remote_hello.supports(pb_protocol::CAP_AUDIO));
            assert!(s.remote_hello.supports(pb_protocol::CAP_AUDIO));
        }

        #[tokio::test(flavor = "multi_thread")]
        async fn an_unknown_control_tag_mid_session_does_not_end_the_audio_session() {
            let (pc, _pc_ev, phone, mut phone_ev) = paired().await;
            // The requester here is hand-driven, so we can inject raw frames.
            let mut raw_pc = pc.connect(&phone.peer_id()).await.unwrap();
            let mut st = req(AUDIO_PC_TO_PHONE);
            st.id = 9;
            raw_pc.send(ControlMsg::AudioStart(st)).await.unwrap();
            let served = tokio::spawn({
                let phone = phone.clone();
                async move {
                    let mut s = phone_ev.sessions.recv().await.unwrap();
                    let st = loop {
                        if let Some(ControlMsg::AudioStart(st)) = s.recv().await.unwrap() {
                            break st;
                        }
                    };
                    serve_audio(s, st, Ok(ok_accept()), phone.audio_slots()).await
                }
            });
            loop {
                if let Some(ControlMsg::AudioAccept(a)) = raw_pc.recv().await.unwrap() {
                    assert!(a.accepted);
                    break;
                }
            }
            let mut phone_side = served.await.unwrap().unwrap();

            // A tag only a newer peer knows, in a valid frame on the control stream.
            let mut body = Vec::new();
            prost::encoding::uint32::encode(99, &1, &mut body);
            let mut frame_bytes = (body.len() as u32).to_le_bytes().to_vec();
            frame_bytes.extend_from_slice(&body);
            raw_pc
                .send
                .lock()
                .await
                .write_all(&frame_bytes)
                .await
                .unwrap();

            // The session still carries audio, and an AudioStop still ends it.
            raw_pc
                .conn
                .send_datagram(encode_packet(0, 0, &[7u8; 8]).into())
                .unwrap();
            assert_eq!(next_frame(&mut phone_side).await.samples, vec![7u8; 8]);
            raw_pc
                .send(ControlMsg::AudioStop(AudioStop { id: 9 }))
                .await
                .unwrap();
            assert!(matches!(
                next_event(&mut phone_side).await,
                AudioEvent::Stopped { by_peer: true }
            ));
        }
    }
}
