//! A stranger's pairing messages, before anything is authenticated: once with
//! the bytes as they are (wrong token), once with the right token so the
//! commitment and reveal checks run too.
#![no_main]
use libfuzzer_sys::fuzz_target;
use pb_identity::Identity;
use pb_protocol::control::{PairReveal, PairStart};
use pb_transport::pairing::{BadTokenLimits, HostPairing, Ticket};
use prost::Message;

fuzz_target!(|data: &[u8]| {
    let (a, b) = data.split_at(data.len() / 2);
    let Ok(mut start) = PairStart::decode(a) else {
        return;
    };
    let host = Identity::from_seed([1; 32]).peer_id();
    let joiner = Identity::from_seed([2; 32]).peer_id();
    let now = std::time::Instant::now();
    let source = std::net::IpAddr::from([10, 0, 0, 1]);
    let mut limits = BadTokenLimits::default();
    let mut ticket = Some(Ticket::new(now).unwrap());
    let _ = HostPairing::start(&mut ticket, &mut limits, source, host, joiner, &start, now);
    let mut ticket = Some(Ticket::new(now).unwrap());
    start.token = ticket.as_ref().unwrap().token().to_vec();
    if let Ok((pairing, _nonce)) =
        HostPairing::start(&mut ticket, &mut limits, source, host, joiner, &start, now)
    {
        if let Ok(reveal) = PairReveal::decode(b) {
            let _ = pairing.reveal(&reveal);
        }
    }
});
