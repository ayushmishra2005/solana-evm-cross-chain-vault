//! Domain newtypes used across the public API.
//!
//! Wide identifiers are always 32 bytes. Scalars carry the exact wire width,
//! so no value can be widened or narrowed by accident.

/// Declares a 32 byte identifier.
macro_rules! wide_id {
    ($name:ident, $doc:literal) => {
        #[doc = $doc]
        #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name([u8; 32]);

        impl $name {
            /// The all zero value, which validation rejects.
            pub const ZERO: Self = Self([0u8; 32]);

            #[must_use]
            pub const fn new(bytes: [u8; 32]) -> Self {
                Self(bytes)
            }

            #[must_use]
            pub const fn to_bytes(self) -> [u8; 32] {
                self.0
            }

            #[must_use]
            pub const fn as_bytes(&self) -> &[u8; 32] {
                &self.0
            }

            #[must_use]
            pub fn is_zero(&self) -> bool {
                self.0 == [0u8; 32]
            }
        }

        impl From<[u8; 32]> for $name {
            fn from(bytes: [u8; 32]) -> Self {
                Self(bytes)
            }
        }

        impl From<$name> for [u8; 32] {
            fn from(value: $name) -> Self {
                value.0
            }
        }
    };
}

/// Declares a scalar newtype with a fixed wire width.
macro_rules! scalar_id {
    ($name:ident, $inner:ty, $doc:literal) => {
        #[doc = $doc]
        #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name($inner);

        impl $name {
            pub const ZERO: Self = Self(0);

            #[must_use]
            pub const fn new(value: $inner) -> Self {
                Self(value)
            }

            #[must_use]
            pub const fn get(self) -> $inner {
                self.0
            }

            #[must_use]
            pub const fn is_zero(self) -> bool {
                self.0 == 0
            }
        }

        impl From<$inner> for $name {
            fn from(value: $inner) -> Self {
                Self(value)
            }
        }

        impl From<$name> for $inner {
            fn from(value: $name) -> Self {
                value.0
            }
        }
    };
}

wide_id!(ApplicationId, "Application endpoint on one chain.");
wide_id!(DeploymentId, "Distinguishes one deployment from another.");
wide_id!(VaultId, "Vault the message belongs to.");
wide_id!(TransferId, "Identifies one allocation or recall.");
wide_id!(
    DestinationReference,
    "Opaque receipt the destination chain can be searched by."
);
wide_id!(Commitment, "Hash chain link or configuration digest.");
wide_id!(BodyHash, "Keccak-256 of the encoded body.");
wide_id!(MessageId, "Keccak-256 identifier of a whole message.");

scalar_id!(ProtocolVersion, u16, "Envelope format version.");
scalar_id!(SchemaVersion, u16, "Body schema version.");
scalar_id!(ChainId, u32, "Protocol internal chain number.");
scalar_id!(LaneId, u32, "Ordered stream between two applications.");
scalar_id!(Sequence, u64, "Position within a lane.");
scalar_id!(Timestamp, u64, "Seconds since the unix epoch.");
scalar_id!(ConfigVersion, u64, "Configuration generation.");
scalar_id!(EpochId, u64, "Vault epoch a report covers.");
scalar_id!(AssetAmount, u128, "Amount in the smallest asset unit.");
scalar_id!(BasisPoints, u16, "Rate in hundredths of a percent.");

/// One basis point per hundredth of a percent, so this is one hundred percent.
pub const MAX_BASIS_POINTS: u16 = 10_000;

impl BasisPoints {
    /// True when the rate is at most one hundred percent.
    #[must_use]
    pub const fn is_in_range(self) -> bool {
        self.0 <= MAX_BASIS_POINTS
    }
}

/// An application endpoint is always 32 opaque bytes on the wire.
///
/// The chain family is never read out of those bytes. A Solana key may start
/// with twelve zero bytes, so a zero prefix proves nothing. Callers learn the
/// family from configuration and then pick the matching operation below.
impl ApplicationId {
    /// Maps a 20 byte EVM address into the low bytes, with a zero prefix.
    #[must_use]
    pub fn from_evm_address(address: [u8; 20]) -> Self {
        Self(core::array::from_fn(|index| {
            index
                .checked_sub(12)
                .and_then(|low| address.get(low).copied())
                .unwrap_or(0)
        }))
    }

    /// Keeps a Solana public key unchanged.
    #[must_use]
    pub const fn from_solana_pubkey(key: [u8; 32]) -> Self {
        Self(key)
    }

    /// Returns the last twenty bytes and claims nothing about the chain.
    ///
    /// Use it only after configuration says this endpoint is an EVM address.
    #[must_use]
    pub fn low_20_bytes(&self) -> [u8; 20] {
        let mut low = [0u8; 20];
        let source = self.0.split_at(12).1;
        low.copy_from_slice(source);
        low
    }

    /// Reports whether the first twelve bytes are zero.
    ///
    /// This is a fact about the bytes. It is not proof of a chain family.
    #[must_use]
    pub fn has_zero_high_12_bytes(&self) -> bool {
        self.0.split_at(12).0 == [0u8; 12]
    }
}

/// Header bits reserved for later use.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Flags(u16);

impl Flags {
    /// No flag bit is defined yet, so every bit must stay clear.
    pub const RESERVED_MASK: u16 = u16::MAX;

    pub const NONE: Self = Self(0);

    #[must_use]
    pub const fn new(bits: u16) -> Self {
        Self(bits)
    }

    #[must_use]
    pub const fn bits(self) -> u16 {
        self.0
    }

    #[must_use]
    pub const fn has_reserved_bits(self) -> bool {
        self.0 & Self::RESERVED_MASK != 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_evm_address_keeps_its_bytes_in_the_low_twenty() {
        let address = [0xAB; 20];
        let id = ApplicationId::from_evm_address(address);
        assert_eq!(id.as_bytes().get(..12), Some(&[0u8; 12][..]));
        assert_eq!(id.low_20_bytes(), address);
    }

    #[test]
    fn a_solana_key_survives_the_round_trip_unchanged() {
        let key = core::array::from_fn(|index| u8::try_from(index).unwrap_or(0));
        let id = ApplicationId::from_solana_pubkey(key);
        assert_eq!(id.to_bytes(), key);
    }

    #[test]
    fn a_solana_key_that_starts_with_twelve_zero_bytes_stays_a_generic_identifier() {
        let mut key = [0u8; 32];
        key[12..].copy_from_slice(&[0x5D; 20]);
        let solana = ApplicationId::from_solana_pubkey(key);
        let evm = ApplicationId::from_evm_address([0x5D; 20]);
        assert_eq!(solana, evm);
        assert_eq!(solana.to_bytes(), key);
    }

    #[test]
    fn a_zero_prefix_is_reported_as_a_byte_fact_and_never_as_a_chain() {
        let padded = ApplicationId::from_evm_address([0x7E; 20]);
        let full = ApplicationId::from_solana_pubkey([0x11; 32]);
        assert!(padded.has_zero_high_12_bytes());
        assert!(!full.has_zero_high_12_bytes());
    }

    #[test]
    fn the_low_twenty_bytes_are_returned_for_any_identifier() {
        let full = ApplicationId::from_solana_pubkey([0x11; 32]);
        assert_eq!(full.low_20_bytes(), [0x11; 20]);
        assert_eq!(full.to_bytes().get(12..), Some(&full.low_20_bytes()[..]));
    }

    #[test]
    fn a_zero_evm_address_still_maps_to_a_zero_identifier() {
        let id = ApplicationId::from_evm_address([0u8; 20]);
        assert!(id.is_zero());
    }

    #[test]
    fn basis_points_above_one_hundred_percent_are_out_of_range() {
        assert!(BasisPoints::new(10_000).is_in_range());
        assert!(!BasisPoints::new(10_001).is_in_range());
    }

    #[test]
    fn a_wide_identifier_converts_both_ways() {
        let raw = [0x3C; 32];
        let id = VaultId::from(raw);
        assert_eq!(id, VaultId::new(raw));
        assert_eq!(<[u8; 32]>::from(id), raw);
        assert_eq!(id.to_bytes(), raw);
        assert_eq!(id.as_bytes(), &raw);
        assert!(!id.is_zero());
        assert!(VaultId::ZERO.is_zero());
        assert_eq!(VaultId::default(), VaultId::ZERO);
    }

    #[test]
    fn a_scalar_identifier_converts_both_ways() {
        let id = Sequence::from(88u64);
        assert_eq!(id, Sequence::new(88));
        assert_eq!(u64::from(id), 88);
        assert_eq!(id.get(), 88);
        assert!(!id.is_zero());
        assert!(Sequence::ZERO.is_zero());
        assert_eq!(Sequence::default(), Sequence::ZERO);
    }

    #[test]
    fn the_widest_amount_keeps_its_value() {
        let amount = AssetAmount::new(u128::MAX);
        assert_eq!(u128::from(amount), u128::MAX);
    }

    #[test]
    fn every_reserved_flag_bit_is_detected() {
        assert!(!Flags::NONE.has_reserved_bits());
        assert!(Flags::new(1).has_reserved_bits());
        assert!(Flags::new(0x8000).has_reserved_bits());
    }
}
