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
