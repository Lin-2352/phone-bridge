//! The file-transfer messages that carry resume fields: whatever bytes arrive, decoding never panics and a
//! decoded message encodes back to something that decodes to the same value.
#![no_main]
use libfuzzer_sys::fuzz_target;
use pb_protocol::control::{FileHeader, Offer, OfferReply};
use prost::Message;

fuzz_target!(|data: &[u8]| {
    if let Ok(o) = Offer::decode(data) {
        assert_eq!(Offer::decode(o.encode_to_vec().as_slice()).unwrap(), o);
    }
    if let Ok(r) = OfferReply::decode(data) {
        assert_eq!(OfferReply::decode(r.encode_to_vec().as_slice()).unwrap(), r);
    }
    if let Ok(h) = FileHeader::decode(data) {
        assert_eq!(FileHeader::decode(h.encode_to_vec().as_slice()).unwrap(), h);
    }
});
