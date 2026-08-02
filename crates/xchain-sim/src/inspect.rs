//! Optional views over canonical message bytes.
//!
//! Transport never calls these. A caller uses them when a test wants to know
//! what a delivered byte string decodes to.

extern crate alloc;

use alloc::vec::Vec;

use protocol_types::{MESSAGE_ID_DOMAIN, Message, MessageId, Timestamp, decode_message, keccak256};

use crate::error::SimError;

/// Canonical identifier of any byte string on the control lane.
///
/// This repeats the wire rule so malformed bytes still get a stable identity.
#[must_use]
pub fn message_identity(bytes: &[u8]) -> MessageId {
    let mut buffer: Vec<u8> =
        Vec::with_capacity(MESSAGE_ID_DOMAIN.len().saturating_add(bytes.len()));
    buffer.extend_from_slice(MESSAGE_ID_DOMAIN);
    buffer.extend_from_slice(bytes);
    MessageId::new(keccak256(&buffer))
}

/// Decodes delivered bytes, reporting a typed failure instead of panicking.
pub fn decode(bytes: &[u8]) -> Result<Message, SimError> {
    decode_message(bytes).map_err(SimError::Decode)
}

/// Reads the expiry stamp a canonical message carries.
///
/// The value is in seconds, which is not the simulated tick unit. The caller
/// decides how to map it onto a delivery deadline.
pub fn expiration(bytes: &[u8]) -> Result<Timestamp, SimError> {
    decode(bytes).map(|message| message.header.expires_at)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_bytes_still_get_an_identity() {
        assert_ne!(message_identity(&[]), MessageId::ZERO);
        assert_eq!(message_identity(b"abc"), message_identity(b"abc"));
        assert_ne!(message_identity(b"abc"), message_identity(b"abd"));
    }

    #[test]
    fn the_domain_tag_keeps_the_identity_away_from_a_plain_hash() {
        let bytes = [7u8; 16];
        assert_ne!(message_identity(&bytes).to_bytes(), keccak256(&bytes));
    }

    #[test]
    fn malformed_bytes_report_a_typed_failure() {
        assert!(matches!(decode(&[0u8; 8]), Err(SimError::Decode(_))));
        assert!(matches!(expiration(&[0u8; 8]), Err(SimError::Decode(_))));
    }
}
