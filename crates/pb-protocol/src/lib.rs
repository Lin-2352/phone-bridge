//! Versioned wire framing and capability negotiation.
//!
//! Placeholder: control messages (prost) and fixed-layout datagram structs
//! arrive after the wire spec is written. See `docs/DECISIONS.md`.

/// Wire protocol version. Bumped on any incompatible change.
pub const PROTOCOL_VERSION: u16 = 0;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protocol_version_starts_at_zero() {
        assert_eq!(PROTOCOL_VERSION, 0);
    }
}
