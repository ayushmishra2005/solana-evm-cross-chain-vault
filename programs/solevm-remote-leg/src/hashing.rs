//! Keccak that goes through the chain syscall.
//!
//! The digest is the same one the shared crate produces, so every id and
//! commitment stays identical. Only the cost differs.

/// Keccak-256 over several byte strings joined in order.
#[must_use]
pub fn keccak256_parts(parts: &[&[u8]]) -> [u8; 32] {
    solana_keccak_hasher::hashv(parts).to_bytes()
}

/// Keccak-256 over one byte string.
#[must_use]
pub fn keccak256(bytes: &[u8]) -> [u8; 32] {
    keccak256_parts(&[bytes])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_digest_matches_the_shared_implementation() {
        for input in [b"".as_slice(), b"abc", &[7u8; 400]] {
            assert_eq!(keccak256(input), protocol_types::keccak256(input));
        }
    }

    #[test]
    fn joining_parts_matches_hashing_the_whole() {
        assert_eq!(
            keccak256_parts(&[b"solevm", b" message"]),
            protocol_types::keccak256(b"solevm message")
        );
    }
}
