use crate::hash::keccak256_parts;
use crate::identifier::{Commitment, MessageId};

/// Domain tag that keeps message ids apart from other digests.
pub const MESSAGE_ID_DOMAIN: &[u8] = b"SOLEVM_MESSAGE_ID_V1";

/// Domain tag that keeps chain links apart from other digests.
pub const COMMITMENT_DOMAIN: &[u8] = b"SOLEVM_COMMITMENT_V1";

/// Identifier of a whole canonical message.
///
/// Every committed byte feeds the hash, so any edit changes the id.
#[must_use]
pub(crate) fn message_id(encoded: &[u8]) -> MessageId {
    MessageId::new(keccak256_parts(&[MESSAGE_ID_DOMAIN, encoded]))
}

/// Next link of a lane hash chain.
///
/// This is a pure function. It does not accept or record a sequence.
#[must_use]
pub fn next_commitment(previous: Commitment, message: MessageId) -> Commitment {
    Commitment::new(keccak256_parts(&[
        COMMITMENT_DOMAIN,
        previous.as_bytes(),
        message.as_bytes(),
    ]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_same_inputs_always_give_the_same_link() {
        let previous = Commitment::new([7u8; 32]);
        let message = MessageId::new([9u8; 32]);
        assert_eq!(
            next_commitment(previous, message),
            next_commitment(previous, message)
        );
    }

    #[test]
    fn a_different_previous_commitment_changes_the_link() {
        let message = MessageId::new([9u8; 32]);
        let first = next_commitment(Commitment::ZERO, message);
        let second = next_commitment(Commitment::new([1u8; 32]), message);
        assert_ne!(first, second);
    }

    #[test]
    fn a_different_message_id_changes_the_link() {
        let previous = Commitment::new([7u8; 32]);
        let first = next_commitment(previous, MessageId::new([1u8; 32]));
        let second = next_commitment(previous, MessageId::new([2u8; 32]));
        assert_ne!(first, second);
    }

    #[test]
    fn swapping_the_two_inputs_changes_the_link() {
        let left = [3u8; 32];
        let right = [4u8; 32];
        assert_ne!(
            next_commitment(Commitment::new(left), MessageId::new(right)),
            next_commitment(Commitment::new(right), MessageId::new(left))
        );
    }

    #[test]
    fn the_two_domain_tags_differ() {
        assert_ne!(MESSAGE_ID_DOMAIN, COMMITMENT_DOMAIN);
    }

    #[test]
    fn the_message_id_domain_tag_is_part_of_the_hash() {
        let encoded = [0xAAu8; 8];
        assert_ne!(message_id(&encoded).to_bytes(), crate::keccak256(&encoded));
    }
}
