use sha3::{Digest, Keccak256};

/// Keccak-256 over one byte string.
#[must_use]
pub fn keccak256(bytes: &[u8]) -> [u8; 32] {
    let mut hasher = Keccak256::new();
    hasher.update(bytes);
    hasher.finalize().into()
}

/// Keccak-256 over several byte strings joined in order.
#[must_use]
pub(crate) fn keccak256_parts(parts: &[&[u8]]) -> [u8; 32] {
    let mut hasher = Keccak256::new();
    for part in parts {
        hasher.update(part);
    }
    hasher.finalize().into()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The published Keccak-256 digest of the empty string.
    const EMPTY: [u8; 32] = [
        0xc5, 0xd2, 0x46, 0x01, 0x86, 0xf7, 0x23, 0x3c, 0x92, 0x7e, 0x7d, 0xb2, 0xdc, 0xc7, 0x03,
        0xc0, 0xe5, 0x00, 0xb6, 0x53, 0xca, 0x82, 0x27, 0x3b, 0x7b, 0xfa, 0xd8, 0x04, 0x5d, 0x85,
        0xa4, 0x70,
    ];

    /// The published Keccak-256 digest of the ascii string abc.
    const ABC: [u8; 32] = [
        0x4e, 0x03, 0x65, 0x7a, 0xea, 0x45, 0xa9, 0x4f, 0xc7, 0xd4, 0x7b, 0xa8, 0x26, 0xc8, 0xd6,
        0x67, 0xc0, 0xd1, 0xe6, 0xe3, 0x3a, 0x64, 0xa0, 0x36, 0xec, 0x44, 0xf5, 0x8f, 0xa1, 0x2d,
        0x6c, 0x45,
    ];

    #[test]
    fn the_digest_matches_the_published_keccak_vectors() {
        assert_eq!(keccak256(b""), EMPTY);
        assert_eq!(keccak256(b"abc"), ABC);
    }

    #[test]
    fn joining_parts_matches_hashing_the_whole() {
        assert_eq!(keccak256_parts(&[b"ab", b"c"]), ABC);
        assert_eq!(keccak256_parts(&[b"", b"abc", b""]), ABC);
    }

    #[test]
    fn a_different_split_of_the_same_bytes_gives_the_same_digest() {
        let joined = keccak256(b"solevm message");
        assert_eq!(keccak256_parts(&[b"solevm", b" message"]), joined);
    }
}
