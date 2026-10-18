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
    fmt,
    time::{Duration, Instant},
};

use pb_identity::{sas, PeerId};
use pb_protocol::control::{PairNonce, PairReveal, PairStart, PairingCode};
use sha2::{Digest, Sha256};

/// How long a pairing code stays valid.
pub const PAIRING_TTL: Duration = Duration::from_secs(120);
/// Wrong-token attempts allowed before the code is burned.
pub const MAX_BAD_ATTEMPTS: u32 = 3;

#[derive(Debug, PartialEq, Eq)]
pub enum PairError {
    NoActiveCode,
    Expired,
    BadToken,
    TooManyAttempts,
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

/// A one-time pairing code issued by the host.
pub struct Ticket {
    token: [u8; 16],
    expires: Instant,
    bad_attempts: u32,
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
            bad_attempts: 0,
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
    /// ticket is consumed (`*ticket = None`) so the code is single-use.
    pub fn start(
        ticket: &mut Option<Ticket>,
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
        if !ct_eq(&msg.token, &t.token) {
            t.bad_attempts += 1;
            if t.bad_attempts >= MAX_BAD_ATTEMPTS {
                *ticket = None;
                return Err(PairError::TooManyAttempts);
            }
            return Err(PairError::BadToken);
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
    use pb_identity::Identity;

    use super::*;

    fn ids() -> (PeerId, PeerId) {
        (
            Identity::from_seed([1; 32]).peer_id(),
            Identity::from_seed([2; 32]).peer_id(),
        )
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
        let (hostp, nonce) = HostPairing::start(&mut ticket, host, joiner, &start, now).unwrap();
        assert!(ticket.is_none(), "code must be single-use");
        let (reveal, sas_j) = join.on_nonce(&nonce).unwrap();
        let sas_h = hostp.reveal(&reveal).unwrap();
        assert_eq!(sas_h, sas_j);
        assert_eq!(sas_h.len(), 6);

        // replaying the same start after use fails
        assert_eq!(
            HostPairing::start(&mut ticket, host, joiner, &start, now).err(),
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
            HostPairing::start(&mut ticket, host, joiner, &start, later).err(),
            Some(PairError::Expired)
        );
        assert!(ticket.is_none());
    }

    #[test]
    fn wrong_tokens_burn_the_code_after_three_attempts() {
        let (host, joiner) = ids();
        let now = Instant::now();
        let mut ticket = Some(Ticket::new(now).unwrap());
        let mut code = code_for(host, ticket.as_ref().unwrap());
        let real = code.token;
        code.token[0] ^= 0xff;
        let (_, bad) = JoinPairing::start(&code, joiner, "P".into(), vec![]).unwrap();
        for _ in 0..MAX_BAD_ATTEMPTS - 1 {
            assert_eq!(
                HostPairing::start(&mut ticket, host, joiner, &bad, now).err(),
                Some(PairError::BadToken)
            );
        }
        assert_eq!(
            HostPairing::start(&mut ticket, host, joiner, &bad, now).err(),
            Some(PairError::TooManyAttempts)
        );
        code.token = real;
        let (_, good) = JoinPairing::start(&code, joiner, "P".into(), vec![]).unwrap();
        assert_eq!(
            HostPairing::start(&mut ticket, host, joiner, &good, now).err(),
            Some(PairError::NoActiveCode)
        );
    }

    #[test]
    fn reveal_that_does_not_match_commitment_is_rejected() {
        let (host, joiner) = ids();
        let now = Instant::now();
        let mut ticket = Some(Ticket::new(now).unwrap());
        let code = code_for(host, ticket.as_ref().unwrap());
        let (_, start) = JoinPairing::start(&code, joiner, "P".into(), vec![]).unwrap();
        let (hostp, _) = HostPairing::start(&mut ticket, host, joiner, &start, now).unwrap();
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
            HostPairing::start(&mut ticket, host, joiner, &start, now).err(),
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
            HostPairing::start(
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
            HostPairing::start(&mut t2, host, joiner, &s2, now + PAIRING_TTL).err(),
            Some(PairError::Expired)
        );
    }
}
