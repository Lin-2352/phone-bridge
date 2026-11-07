//! Pairing state machines (no I/O), so every rule is unit-testable.
//!
//! Flow (joiner scanned the host's [`PairingCode`]):
//! 1. joiner -> host `PairStart { token, commitment = H(nonce_j) }`
//! 2. host checks the one-time token, replies `PairNonce { nonce_h }`
//! 3. joiner -> host `PairReveal { nonce_j }`; host checks the commitment
//! 4. both show `SAS(host, nonce_h; joiner, nonce_j)`; users confirm on both sides.
//!
//! The commitment stops a man in the middle from choosing its nonce after
//! seeing the other one, so it cannot steer the 6-digit SAS.

use std::{
    collections::HashMap,
    fmt,
    net::IpAddr,
    time::{Duration, Instant},
};

use pb_identity::{sas, PeerId};
use pb_protocol::control::{PairNonce, PairReveal, PairStart, PairingCode};
use sha2::{Digest, Sha256};

/// How long a pairing code stays valid.
pub const PAIRING_TTL: Duration = Duration::from_secs(120);
/// Wrong tokens one source (remote IP) may send within [`SOURCE_WINDOW`] before it is blocked.
pub const MAX_BAD_PER_SOURCE: u32 = 3;
/// How long a source's count lasts; after this the source starts again from zero.
pub const SOURCE_WINDOW: Duration = Duration::from_secs(600);
/// Wrong tokens from anyone (all sources together) that burn the active code.
pub const MAX_BAD_GLOBAL: u32 = 10;
/// How many sources the table remembers; the oldest window is dropped past this.
pub const MAX_TRACKED_SOURCES: usize = 256;

#[derive(Debug, PartialEq, Eq)]
pub enum PairError {
    NoActiveCode,
    Expired,
    BadToken,
    TooManyAttempts,
    /// This source sent too many wrong tokens recently and is refused until its window ends.
    SourceBlocked,
    BadCommitment,
    Malformed,
    Random,
}

impl fmt::Display for PairError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::NoActiveCode => "no pairing code is active",
            Self::Expired => "pairing code expired",
            Self::BadToken => "wrong pairing code",
            Self::TooManyAttempts => "too many wrong attempts; pairing code burned",
            Self::SourceBlocked => "too many wrong attempts from this device; try again later",
            Self::BadCommitment => "peer's nonce does not match its commitment",
            Self::Malformed => "malformed pairing message",
            Self::Random => "random number generator failed",
        })
    }
}

impl std::error::Error for PairError {}

fn random<const N: usize>() -> Result<[u8; N], PairError> {
    let mut b = [0u8; N];
    getrandom::fill(&mut b).map_err(|_| PairError::Random)?;
    Ok(b)
}

fn commitment(nonce: &[u8; 32]) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(b"phone-bridge/commit/v0");
    h.update(nonce);
    h.finalize().into()
}

fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Pairing handshakes that may run at once. Each holds a slot until it ends.
pub const MAX_PAIRING_SLOTS: usize = 2;
/// The longest a slot may be held: the handshake, the user's confirmation window (120 s) and some room. A stalled
/// or silent peer cannot keep a slot past this, so slots always free.
pub const PAIRING_SLOT_LIMIT: Duration = Duration::from_secs(150);

/// Runs `fut` for at most [`PAIRING_SLOT_LIMIT`]; `None` if it did not finish in time.
pub async fn within_slot<F: std::future::Future>(fut: F) -> Option<F::Output> {
    tokio::time::timeout(PAIRING_SLOT_LIMIT, fut).await.ok()
}

/// What a wrong token did to the limits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// Counted; the source may try again.
    Retry,
    /// This source is now blocked.
    SourceBlocked,
    /// Anyone's wrong tokens reached the global cap: burn the code.
    CodeBurned,
}

/// Limits on wrong pairing tokens, kept on the node (not on a ticket): each source (remote IP) has its own
/// count, which outlives the rotation of the code, so rotating cannot reset an attacker; and one global count
/// (reset with each new ticket) burns the code when many sources together guess too often. One attacker can
/// block only itself, never the owner pairing from another machine.
#[derive(Default)]
pub struct BadTokenLimits {
    per_source: HashMap<IpAddr, (u32, Instant)>,
    global: u32,
}

impl BadTokenLimits {
    /// A new code was issued: the global count starts again; per-source counts stay.
    pub fn new_ticket(&mut self) {
        self.global = 0;
    }

    /// Whether `ip` may try now (blocked sources are refused until their window ends).
    pub fn allow(&self, ip: IpAddr, now: Instant) -> bool {
        match self.per_source.get(&ip) {
            Some((n, since)) if now.duration_since(*since) < SOURCE_WINDOW => {
                *n < MAX_BAD_PER_SOURCE
            }
            _ => true,
        }
    }

    /// Counts a wrong token from `ip`.
    pub fn record_bad(&mut self, ip: IpAddr, now: Instant) -> Verdict {
        if !self.per_source.contains_key(&ip) && self.per_source.len() >= MAX_TRACKED_SOURCES {
            self.make_room(now);
        }
        let entry = self.per_source.entry(ip).or_insert((0, now));
        if now.duration_since(entry.1) >= SOURCE_WINDOW {
            *entry = (0, now);
        }
        entry.0 += 1;
        let source_blocked = entry.0 >= MAX_BAD_PER_SOURCE;
        self.global += 1;
        if self.global >= MAX_BAD_GLOBAL {
            Verdict::CodeBurned
        } else if source_blocked {
            Verdict::SourceBlocked
        } else {
            Verdict::Retry
        }
    }

    /// Drops windows that ended; if the table is still full, the one that started longest ago.
    fn make_room(&mut self, now: Instant) {
        self.per_source
            .retain(|_, (_, since)| now.duration_since(*since) < SOURCE_WINDOW);
        if self.per_source.len() >= MAX_TRACKED_SOURCES {
            if let Some(oldest) = self
                .per_source
                .iter()
                .min_by_key(|(_, (_, since))| *since)
                .map(|(k, _)| *k)
            {
                self.per_source.remove(&oldest);
            }
        }
    }

    /// How many sources are remembered (tests).
    #[cfg(test)]
    pub fn tracked(&self) -> usize {
        self.per_source.len()
    }
}

/// A one-time pairing code issued by the host.
pub struct Ticket {
    token: [u8; 16],
    expires: Instant,
}

impl Ticket {
    pub fn new(now: Instant) -> Result<Self, PairError> {
        Self::with_ttl(now, PAIRING_TTL)
    }

    /// A code valid for `ttl` (short-lived, rotating codes use ~35 s).
    pub fn with_ttl(now: Instant, ttl: Duration) -> Result<Self, PairError> {
        Ok(Self {
            token: random()?,
            expires: now + ttl.min(PAIRING_TTL),
        })
    }

    pub fn token(&self) -> [u8; 16] {
        self.token
    }
}

/// Host side of one pairing attempt.
pub struct HostPairing {
    host: PeerId,
    joiner: PeerId,
    commitment: Vec<u8>,
    nonce_h: [u8; 32],
}

impl HostPairing {
    /// Check the joiner's `PairStart` against the active ticket. On success the
    /// ticket is consumed (`*ticket = None`) so the code is single-use. `source` is where the connection comes
    /// from; wrong tokens are counted per source and in total (`limits`).
    pub fn start(
        ticket: &mut Option<Ticket>,
        limits: &mut BadTokenLimits,
        source: IpAddr,
        host: PeerId,
        joiner: PeerId,
        msg: &PairStart,
        now: Instant,
    ) -> Result<(Self, PairNonce), PairError> {
        let t = ticket.as_mut().ok_or(PairError::NoActiveCode)?;
        if now >= t.expires {
            *ticket = None;
            return Err(PairError::Expired);
        }
        // A blocked source is refused before its token is looked at, so it cannot keep guessing.
        if !limits.allow(source, now) {
            return Err(PairError::SourceBlocked);
        }
        if !ct_eq(&msg.token, &t.token) {
            return Err(match limits.record_bad(source, now) {
                Verdict::Retry => PairError::BadToken,
                Verdict::SourceBlocked => PairError::SourceBlocked,
                Verdict::CodeBurned => {
                    *ticket = None;
                    PairError::TooManyAttempts
                }
            });
        }
        if msg.commitment.len() != 32 {
            return Err(PairError::Malformed);
        }
        *ticket = None;
        let nonce_h = random()?;
        let reply = PairNonce {
            nonce: nonce_h.to_vec(),
            device_name: String::new(),
        };
        Ok((
            Self {
                host,
                joiner,
                commitment: msg.commitment.clone(),
                nonce_h,
            },
            reply,
        ))
    }

    /// Verify the revealed nonce and return the SAS to show the user.
    pub fn reveal(&self, msg: &PairReveal) -> Result<String, PairError> {
        let nonce_j: [u8; 32] = msg
            .nonce
            .as_slice()
            .try_into()
            .map_err(|_| PairError::Malformed)?;
        if !ct_eq(&commitment(&nonce_j), &self.commitment) {
            return Err(PairError::BadCommitment);
        }
        Ok(sas((&self.host, &self.nonce_h), (&self.joiner, &nonce_j)))
    }
}

/// Joiner side of one pairing attempt.
pub struct JoinPairing {
    host: PeerId,
    joiner: PeerId,
    nonce_j: [u8; 32],
}

impl JoinPairing {
    pub fn start(
        code: &PairingCode,
        joiner: PeerId,
        device_name: String,
        addrs: Vec<String>,
    ) -> Result<(Self, PairStart), PairError> {
        let host = PeerId::from_bytes(code.host_id).map_err(|_| PairError::Malformed)?;
        let nonce_j = random()?;
        let msg = PairStart {
            token: code.token.to_vec(),
            commitment: commitment(&nonce_j).to_vec(),
            device_name,
            addrs,
        };
        Ok((
            Self {
                host,
                joiner,
                nonce_j,
            },
            msg,
        ))
    }

    /// Take the host's nonce; returns the reveal to send and the SAS to show.
    pub fn on_nonce(&self, msg: &PairNonce) -> Result<(PairReveal, String), PairError> {
        let nonce_h: [u8; 32] = msg
            .nonce
            .as_slice()
            .try_into()
            .map_err(|_| PairError::Malformed)?;
        let code = sas((&self.host, &nonce_h), (&self.joiner, &self.nonce_j));
        Ok((
            PairReveal {
                nonce: self.nonce_j.to_vec(),
            },
            code,
        ))
    }
}

#[cfg(test)]
mod tests {
    use std::net::IpAddr;

    use pb_identity::Identity;

    use super::*;

    fn ids() -> (PeerId, PeerId) {
        (
            Identity::from_seed([1; 32]).peer_id(),
            Identity::from_seed([2; 32]).peer_id(),
        )
    }

    fn src(n: u8) -> IpAddr {
        IpAddr::from([10, 0, 0, n])
    }

    /// The old one-argument-per-call shape with fresh limits and one fixed source.
    fn hs(
        ticket: &mut Option<Ticket>,
        host: PeerId,
        joiner: PeerId,
        msg: &PairStart,
        now: Instant,
    ) -> Result<(HostPairing, PairNonce), PairError> {
        HostPairing::start(
            ticket,
            &mut BadTokenLimits::default(),
            src(1),
            host,
            joiner,
            msg,
            now,
        )
    }

    fn bad_start(joiner: PeerId, host: PeerId, t: &Ticket) -> PairStart {
        let mut code = code_for(host, t);
        code.token[0] ^= 0xff;
        JoinPairing::start(&code, joiner, "P".into(), vec![])
            .unwrap()
            .1
    }

    fn good_start(joiner: PeerId, host: PeerId, t: &Ticket) -> PairStart {
        JoinPairing::start(&code_for(host, t), joiner, "P".into(), vec![])
            .unwrap()
            .1
    }

    fn code_for(host: PeerId, t: &Ticket) -> PairingCode {
        PairingCode {
            host_id: *host.as_bytes(),
            token: t.token(),
            addrs: vec![],
        }
    }

    #[test]
    fn happy_path_gives_both_sides_the_same_sas_and_burns_the_code() {
        let (host, joiner) = ids();
        let now = Instant::now();
        let mut ticket = Some(Ticket::new(now).unwrap());
        let code = code_for(host, ticket.as_ref().unwrap());

        let (join, start) = JoinPairing::start(&code, joiner, "Phone".into(), vec![]).unwrap();
        let (hostp, nonce) = hs(&mut ticket, host, joiner, &start, now).unwrap();
        assert!(ticket.is_none(), "code must be single-use");
        let (reveal, sas_j) = join.on_nonce(&nonce).unwrap();
        let sas_h = hostp.reveal(&reveal).unwrap();
        assert_eq!(sas_h, sas_j);
        assert_eq!(sas_h.len(), 6);

        // replaying the same start after use fails
        assert_eq!(
            hs(&mut ticket, host, joiner, &start, now).err(),
            Some(PairError::NoActiveCode)
        );
    }

    #[test]
    fn expired_code_is_rejected_and_cleared() {
        let (host, joiner) = ids();
        let now = Instant::now();
        let mut ticket = Some(Ticket::new(now).unwrap());
        let code = code_for(host, ticket.as_ref().unwrap());
        let (_, start) = JoinPairing::start(&code, joiner, "P".into(), vec![]).unwrap();
        let later = now + PAIRING_TTL;
        assert_eq!(
            hs(&mut ticket, host, joiner, &start, later).err(),
            Some(PairError::Expired)
        );
        assert!(ticket.is_none());
    }

    #[test]
    fn a_source_gets_three_bad_tokens() {
        let (host, joiner) = ids();
        let now = Instant::now();
        let mut ticket = Some(Ticket::new(now).unwrap());
        let mut limits = BadTokenLimits::default();
        let bad = bad_start(joiner, host, ticket.as_ref().unwrap());
        let good = good_start(joiner, host, ticket.as_ref().unwrap());
        for _ in 0..MAX_BAD_PER_SOURCE - 1 {
            let r = HostPairing::start(&mut ticket, &mut limits, src(1), host, joiner, &bad, now);
            assert_eq!(r.err(), Some(PairError::BadToken));
        }
        let r = HostPairing::start(&mut ticket, &mut limits, src(1), host, joiner, &bad, now);
        assert_eq!(r.err(), Some(PairError::SourceBlocked));
        // Blocked means blocked: even the right code is refused from this source now.
        let r = HostPairing::start(&mut ticket, &mut limits, src(1), host, joiner, &good, now);
        assert_eq!(r.err(), Some(PairError::SourceBlocked));
        assert!(
            ticket.is_some(),
            "one source cannot burn the code for everyone"
        );
    }

    #[test]
    fn another_source_can_still_pair() {
        let (host, joiner) = ids();
        let now = Instant::now();
        let mut ticket = Some(Ticket::new(now).unwrap());
        let mut limits = BadTokenLimits::default();
        let bad = bad_start(joiner, host, ticket.as_ref().unwrap());
        let good = good_start(joiner, host, ticket.as_ref().unwrap());
        for _ in 0..MAX_BAD_PER_SOURCE {
            let _ = HostPairing::start(&mut ticket, &mut limits, src(1), host, joiner, &bad, now);
        }
        let r = HostPairing::start(&mut ticket, &mut limits, src(2), host, joiner, &good, now);
        assert!(
            r.is_ok(),
            "the owner pairs from another machine: {:?}",
            r.err()
        );
    }

    #[test]
    fn the_global_cap_burns_the_code() {
        let (host, joiner) = ids();
        let now = Instant::now();
        let mut ticket = Some(Ticket::new(now).unwrap());
        let mut limits = BadTokenLimits::default();
        let bad = bad_start(joiner, host, ticket.as_ref().unwrap());
        let mut last = None;
        // Five sources, two bad tokens each: none is blocked, but together they reach the global cap.
        'outer: for n in 1..=5u8 {
            for _ in 0..2 {
                last =
                    HostPairing::start(&mut ticket, &mut limits, src(n), host, joiner, &bad, now)
                        .err();
                if ticket.is_none() {
                    break 'outer;
                }
            }
        }
        assert_eq!(last, Some(PairError::TooManyAttempts));
        assert!(ticket.is_none(), "the code is burned");
    }

    #[test]
    fn source_limits_survive_a_code_rotation() {
        let (host, joiner) = ids();
        let now = Instant::now();
        let mut limits = BadTokenLimits::default();
        let mut ticket = Some(Ticket::new(now).unwrap());
        let bad = bad_start(joiner, host, ticket.as_ref().unwrap());
        for _ in 0..MAX_BAD_PER_SOURCE {
            let _ = HostPairing::start(&mut ticket, &mut limits, src(1), host, joiner, &bad, now);
        }
        // The tray rotates the code every ~35 s: a new ticket, and the global count starts again.
        ticket = Some(Ticket::new(now).unwrap());
        limits.new_ticket();
        let good = good_start(joiner, host, ticket.as_ref().unwrap());
        let r = HostPairing::start(&mut ticket, &mut limits, src(1), host, joiner, &good, now);
        assert_eq!(
            r.err(),
            Some(PairError::SourceBlocked),
            "rotating the code does not reset a source"
        );
    }

    #[test]
    fn a_source_is_forgiven_after_ten_minutes() {
        let (host, joiner) = ids();
        let now = Instant::now();
        let mut ticket = Some(Ticket::new(now).unwrap());
        let mut limits = BadTokenLimits::default();
        let bad = bad_start(joiner, host, ticket.as_ref().unwrap());
        for _ in 0..MAX_BAD_PER_SOURCE {
            let _ = HostPairing::start(&mut ticket, &mut limits, src(1), host, joiner, &bad, now);
        }
        assert!(!limits.allow(src(1), now + SOURCE_WINDOW - Duration::from_secs(1)));
        assert!(
            limits.allow(src(1), now + SOURCE_WINDOW),
            "ten minutes later the source may try again"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_pairing_slot_frees_after_its_time_box() {
        use std::sync::Arc;
        let slots = Arc::new(tokio::sync::Semaphore::new(MAX_PAIRING_SLOTS));
        let permit = slots.clone().try_acquire_owned().unwrap();
        // A pairing that never ends (a stalled peer): the time box cuts it off and the slot frees.
        let stuck = tokio::spawn(async move {
            let _held = permit;
            within_slot(std::future::pending::<()>()).await
        });
        assert_eq!(
            slots.available_permits(),
            MAX_PAIRING_SLOTS - 1,
            "held while it runs"
        );
        tokio::time::advance(PAIRING_SLOT_LIMIT + Duration::from_secs(1)).await;
        assert_eq!(stuck.await.unwrap(), None, "timed out");
        assert_eq!(
            slots.available_permits(),
            MAX_PAIRING_SLOTS,
            "the slot is free again"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_pairing_that_finishes_in_time_is_not_cut_off() {
        let r = within_slot(async { 7 }).await;
        assert_eq!(r, Some(7));
        // The limit covers the user's confirmation window with room to spare.
        assert!(PAIRING_SLOT_LIMIT > Duration::from_secs(120));
    }

    #[test]
    fn many_sources_do_not_grow_the_table_without_bound() {
        let mut limits = BadTokenLimits::default();
        let now = Instant::now();
        for n in 0..(MAX_TRACKED_SOURCES as u32 * 3) {
            let ip = IpAddr::from([10, 1, (n >> 8) as u8, n as u8]);
            let _ = limits.record_bad(ip, now);
        }
        assert!(limits.tracked() <= MAX_TRACKED_SOURCES);
    }

    #[test]
    fn reveal_that_does_not_match_commitment_is_rejected() {
        let (host, joiner) = ids();
        let now = Instant::now();
        let mut ticket = Some(Ticket::new(now).unwrap());
        let code = code_for(host, ticket.as_ref().unwrap());
        let (_, start) = JoinPairing::start(&code, joiner, "P".into(), vec![]).unwrap();
        let (hostp, _) = hs(&mut ticket, host, joiner, &start, now).unwrap();
        let forged = PairReveal { nonce: vec![9; 32] };
        assert_eq!(hostp.reveal(&forged), Err(PairError::BadCommitment));
        assert_eq!(
            hostp.reveal(&PairReveal { nonce: vec![9; 5] }),
            Err(PairError::Malformed)
        );
    }

    #[test]
    fn malformed_messages_are_rejected() {
        let (host, joiner) = ids();
        let now = Instant::now();
        let mut ticket = Some(Ticket::new(now).unwrap());
        let code = code_for(host, ticket.as_ref().unwrap());
        let (join, mut start) = JoinPairing::start(&code, joiner, "P".into(), vec![]).unwrap();
        start.commitment.truncate(3);
        assert_eq!(
            hs(&mut ticket, host, joiner, &start, now).err(),
            Some(PairError::Malformed)
        );
        assert_eq!(
            join.on_nonce(&PairNonce {
                nonce: vec![1],
                device_name: String::new()
            })
            .err(),
            Some(PairError::Malformed)
        );
    }

    #[test]
    fn short_ttl_code_expires_and_ttl_is_capped() {
        let (host, joiner) = ids();
        let now = Instant::now();
        let mut ticket = Some(Ticket::with_ttl(now, Duration::from_secs(35)).unwrap());
        let code = code_for(host, ticket.as_ref().unwrap());
        let (_, start) = JoinPairing::start(&code, joiner, "P".into(), vec![]).unwrap();
        assert_eq!(
            hs(
                &mut ticket,
                host,
                joiner,
                &start,
                now + Duration::from_secs(36)
            )
            .err(),
            Some(PairError::Expired)
        );
        // A huge TTL is capped at PAIRING_TTL.
        let mut t2 = Some(Ticket::with_ttl(now, Duration::from_secs(3600)).unwrap());
        let code2 = code_for(host, t2.as_ref().unwrap());
        let (_, s2) = JoinPairing::start(&code2, joiner, "P".into(), vec![]).unwrap();
        assert_eq!(
            hs(&mut t2, host, joiner, &s2, now + PAIRING_TTL).err(),
            Some(PairError::Expired)
        );
    }
}
