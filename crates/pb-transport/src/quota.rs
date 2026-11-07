//! Per-peer quotas (M7.8): a paired but misbehaving device cannot exhaust the node by opening sessions in a loop
//! or offering many large files at once, and other devices are unaffected. Pure and shared (`Arc`), so the
//! rules are tested without a network. Permits release what they hold when dropped.

use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use pb_identity::PeerId;

/// Sessions one paired peer may have open at once. Relay itself can hold close to ten for one phone at the extreme
/// (the main session, screen, camera, audio each way, control, concurrent file batches and clipboard sends), so the
/// limit leaves room above that and still bounds a misbehaving peer.
pub const MAX_SESSIONS_PER_PEER: usize = 16;
/// File offers one peer may make in a minute.
pub const MAX_OFFERS_PER_PEER_PER_MIN: usize = 30;
/// How long an offer counts against the per-minute limit.
pub const OFFER_WINDOW: Duration = Duration::from_secs(60);
/// Free space always left on the disk (the same margin as an incoming file's own check).
pub const KEEP_FREE_BYTES: u64 = 256 * 1024 * 1024;

/// Why a request was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QuotaRefusal {
    TooManySessions,
    TooManyOffers,
    NotEnoughSpace,
}

#[derive(Default)]
struct PeerState {
    sessions: usize,
    offers: VecDeque<Instant>,
    /// Bytes promised to offers that are still being received.
    reserved: u64,
}

#[derive(Default)]
struct Inner {
    peers: HashMap<PeerId, PeerState>,
}

/// The quotas of every paired peer, cloneable (a shared handle).
#[derive(Clone, Default)]
pub struct PeerQuotas {
    inner: Arc<Mutex<Inner>>,
}

/// One open session; dropping it frees the slot.
pub struct SessionPermit {
    quotas: PeerQuotas,
    peer: PeerId,
}

/// One accepted offer's reserved bytes; dropping it releases the reservation.
pub struct OfferPermit {
    quotas: PeerQuotas,
    peer: PeerId,
    size: u64,
}

impl PeerQuotas {
    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// A slot for a new session from `peer`, or `None` when it already has [`MAX_SESSIONS_PER_PEER`].
    pub fn open_session(&self, peer: PeerId) -> Option<SessionPermit> {
        let mut g = self.lock();
        let st = g.peers.entry(peer).or_default();
        if st.sessions >= MAX_SESSIONS_PER_PEER {
            return None;
        }
        st.sessions += 1;
        Some(SessionPermit {
            quotas: self.clone(),
            peer,
        })
    }

    /// Admits a file offer of `size` bytes from `peer` when `free` bytes are free on the disk. At most
    /// [`MAX_OFFERS_PER_PEER_PER_MIN`] offers a minute, and this peer's reservations plus `size` plus
    /// [`KEEP_FREE_BYTES`] must fit in `free`.
    pub fn offer(
        &self,
        peer: PeerId,
        size: u64,
        free: u64,
        now: Instant,
    ) -> Result<OfferPermit, QuotaRefusal> {
        let mut g = self.lock();
        let st = g.peers.entry(peer).or_default();
        while st
            .offers
            .front()
            .is_some_and(|t| now.duration_since(*t) >= OFFER_WINDOW)
        {
            st.offers.pop_front();
        }
        if st.offers.len() >= MAX_OFFERS_PER_PEER_PER_MIN {
            return Err(QuotaRefusal::TooManyOffers);
        }
        let needed = st
            .reserved
            .saturating_add(size)
            .saturating_add(KEEP_FREE_BYTES);
        if needed > free {
            return Err(QuotaRefusal::NotEnoughSpace);
        }
        st.offers.push_back(now);
        st.reserved = st.reserved.saturating_add(size);
        Ok(OfferPermit {
            quotas: self.clone(),
            peer,
            size,
        })
    }

    /// Drops a peer's state (it was unpaired).
    pub fn forget(&self, peer: &PeerId) {
        self.lock().peers.remove(peer);
    }
}

impl Drop for SessionPermit {
    fn drop(&mut self) {
        let mut g = self.quotas.lock();
        if let Some(st) = g.peers.get_mut(&self.peer) {
            st.sessions = st.sessions.saturating_sub(1);
        }
    }
}

impl Drop for OfferPermit {
    fn drop(&mut self) {
        let mut g = self.quotas.lock();
        if let Some(st) = g.peers.get_mut(&self.peer) {
            st.reserved = st.reserved.saturating_sub(self.size);
        }
    }
}

#[cfg(test)]
mod tests {
    use pb_identity::Identity;

    use super::*;

    fn peer(n: u8) -> PeerId {
        Identity::from_seed([n; 32]).peer_id()
    }

    const GB: u64 = 1024 * 1024 * 1024;

    #[test]
    fn the_session_past_the_quota_is_refused() {
        let q = PeerQuotas::default();
        let held: Vec<_> = (0..MAX_SESSIONS_PER_PEER)
            .map(|_| q.open_session(peer(1)).expect("within quota"))
            .collect();
        assert!(
            q.open_session(peer(1)).is_none(),
            "one past the quota is refused"
        );
        drop(held);
    }

    #[test]
    fn a_closed_session_frees_its_slot() {
        let q = PeerQuotas::default();
        let held: Vec<_> = (0..MAX_SESSIONS_PER_PEER)
            .map(|_| q.open_session(peer(1)).unwrap())
            .collect();
        assert!(q.open_session(peer(1)).is_none());
        drop(held.into_iter().next());
        assert!(q.open_session(peer(1)).is_some(), "one closed, one opens");
    }

    #[test]
    fn the_thirty_first_offer_in_a_minute_is_refused() {
        let q = PeerQuotas::default();
        let now = Instant::now();
        for _ in 0..MAX_OFFERS_PER_PEER_PER_MIN {
            drop(q.offer(peer(1), 1, 100 * GB, now).expect("within quota"));
        }
        assert_eq!(
            q.offer(peer(1), 1, 100 * GB, now).err(),
            Some(QuotaRefusal::TooManyOffers)
        );
        // A minute later the window has passed.
        assert!(q.offer(peer(1), 1, 100 * GB, now + OFFER_WINDOW).is_ok());
    }

    #[test]
    fn in_flight_offers_count_against_free_space() {
        let q = PeerQuotas::default();
        let now = Instant::now();
        let free = 10 * GB + KEEP_FREE_BYTES;
        let first = q.offer(peer(1), 6 * GB, free, now).expect("fits");
        // 6 GB is promised; another 6 GB would not fit in 10 GB.
        assert_eq!(
            q.offer(peer(1), 6 * GB, free, now).err(),
            Some(QuotaRefusal::NotEnoughSpace)
        );
        // Finishing the first releases its reservation.
        drop(first);
        assert!(q.offer(peer(1), 6 * GB, free, now).is_ok());
    }

    #[test]
    fn other_peers_are_not_affected() {
        let q = PeerQuotas::default();
        let now = Instant::now();
        let _held: Vec<_> = (0..MAX_SESSIONS_PER_PEER)
            .map(|_| q.open_session(peer(1)).unwrap())
            .collect();
        for _ in 0..MAX_OFFERS_PER_PEER_PER_MIN {
            drop(q.offer(peer(1), 1, 100 * GB, now).unwrap());
        }
        assert!(
            q.open_session(peer(2)).is_some(),
            "another peer still opens sessions"
        );
        assert!(
            q.offer(peer(2), 1, 100 * GB, now).is_ok(),
            "and still makes offers"
        );
    }

    #[test]
    fn an_unpaired_peer_is_forgotten() {
        let q = PeerQuotas::default();
        let _s = q.open_session(peer(1)).unwrap();
        q.forget(&peer(1));
        assert!(q.lock().peers.is_empty());
    }
}
