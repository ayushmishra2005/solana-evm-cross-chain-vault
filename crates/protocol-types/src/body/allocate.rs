use crate::codec::{read_array, read_u64, read_u128, write_bytes, write_u64, write_u128};
use crate::error::{AmountField, DecodeError, EncodeError, IdentifierField, ValidationError};
use crate::identifier::{AssetAmount, AssetId, ConfigVersion, Timestamp, TransferId};
use crate::layout;

/// Authorises moving assets to the remote leg.
///
/// It authorises the move. It is not evidence that assets moved.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AllocateBody {
    pub transfer_id: TransferId,
    pub asset_id: AssetId,
    pub amount: AssetAmount,
    pub expected_source_balance: AssetAmount,
    pub minimum_destination_amount: AssetAmount,
    pub deadline: Timestamp,
    pub config_version: ConfigVersion,
}

impl AllocateBody {
    pub(crate) fn encode_into(&self, out: &mut [u8]) -> Result<(), EncodeError> {
        write_bytes(
            out,
            layout::ALLOCATE_TRANSFER_ID_OFFSET,
            self.transfer_id.as_bytes(),
        )?;
        write_bytes(
            out,
            layout::ALLOCATE_ASSET_ID_OFFSET,
            self.asset_id.as_bytes(),
        )?;
        write_u128(out, layout::ALLOCATE_AMOUNT_OFFSET, self.amount.get())?;
        write_u128(
            out,
            layout::ALLOCATE_EXPECTED_SOURCE_BALANCE_OFFSET,
            self.expected_source_balance.get(),
        )?;
        write_u128(
            out,
            layout::ALLOCATE_MINIMUM_DESTINATION_AMOUNT_OFFSET,
            self.minimum_destination_amount.get(),
        )?;
        write_u64(out, layout::ALLOCATE_DEADLINE_OFFSET, self.deadline.get())?;
        write_u64(
            out,
            layout::ALLOCATE_CONFIG_VERSION_OFFSET,
            self.config_version.get(),
        )
    }

    pub(crate) fn decode(bytes: &[u8]) -> Result<Self, DecodeError> {
        Ok(Self {
            transfer_id: TransferId::new(read_array(bytes, layout::ALLOCATE_TRANSFER_ID_OFFSET)?),
            asset_id: AssetId::new(read_array(bytes, layout::ALLOCATE_ASSET_ID_OFFSET)?),
            amount: AssetAmount::new(read_u128(bytes, layout::ALLOCATE_AMOUNT_OFFSET)?),
            expected_source_balance: AssetAmount::new(read_u128(
                bytes,
                layout::ALLOCATE_EXPECTED_SOURCE_BALANCE_OFFSET,
            )?),
            minimum_destination_amount: AssetAmount::new(read_u128(
                bytes,
                layout::ALLOCATE_MINIMUM_DESTINATION_AMOUNT_OFFSET,
            )?),
            deadline: Timestamp::new(read_u64(bytes, layout::ALLOCATE_DEADLINE_OFFSET)?),
            config_version: ConfigVersion::new(read_u64(
                bytes,
                layout::ALLOCATE_CONFIG_VERSION_OFFSET,
            )?),
        })
    }

    pub(crate) fn validate(&self, published_at: Timestamp) -> Result<(), ValidationError> {
        if self.transfer_id.is_zero() {
            return Err(ValidationError::ZeroIdentifier(IdentifierField::Transfer));
        }
        if self.asset_id.is_zero() {
            return Err(ValidationError::ZeroIdentifier(IdentifierField::Asset));
        }
        if self.amount.is_zero() {
            return Err(ValidationError::ZeroAmount(AmountField::Amount));
        }
        if self.minimum_destination_amount.is_zero() {
            return Err(ValidationError::ZeroAmount(
                AmountField::MinimumDestinationAmount,
            ));
        }
        if self.minimum_destination_amount > self.amount {
            return Err(ValidationError::MinimumAboveAmount);
        }
        if self.deadline < published_at {
            return Err(ValidationError::DeadlineBeforePublication);
        }
        if self.config_version.is_zero() {
            return Err(ValidationError::ZeroIdentifier(
                IdentifierField::ConfigVersion,
            ));
        }
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn sample() -> Self {
        Self {
            transfer_id: TransferId::new([0x11; 32]),
            asset_id: AssetId::new([0x22; 32]),
            amount: AssetAmount::new(1_000_000),
            expected_source_balance: AssetAmount::new(5_000_000),
            minimum_destination_amount: AssetAmount::new(999_000),
            deadline: Timestamp::new(2_000),
            config_version: ConfigVersion::new(4),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PUBLISHED: Timestamp = Timestamp::new(1_000);

    fn encoded(body: &AllocateBody) -> [u8; layout::ALLOCATE_BODY_LEN] {
        let mut bytes = [0u8; layout::ALLOCATE_BODY_LEN];
        assert_eq!(body.encode_into(&mut bytes), Ok(()));
        bytes
    }

    #[test]
    fn a_sample_body_survives_the_round_trip() {
        let body = AllocateBody::sample();
        assert_eq!(AllocateBody::decode(&encoded(&body)), Ok(body));
    }

    #[test]
    fn the_amount_sits_at_its_declared_offset_in_big_endian() {
        let body = AllocateBody {
            amount: AssetAmount::new(0x0102_0304),
            ..AllocateBody::sample()
        };
        let bytes = encoded(&body);
        let start = layout::ALLOCATE_AMOUNT_OFFSET;
        assert_eq!(bytes.get(start..start + 12), Some(&[0u8; 12][..]));
        assert_eq!(bytes.get(start + 12..start + 16), Some(&[1u8, 2, 3, 4][..]));
    }

    #[test]
    fn a_sample_body_passes_validation() {
        assert_eq!(AllocateBody::sample().validate(PUBLISHED), Ok(()));
    }

    #[test]
    fn a_zero_transfer_id_rejects() {
        let body = AllocateBody {
            transfer_id: TransferId::ZERO,
            ..AllocateBody::sample()
        };
        assert_eq!(
            body.validate(PUBLISHED),
            Err(ValidationError::ZeroIdentifier(IdentifierField::Transfer))
        );
    }

    #[test]
    fn a_zero_asset_id_rejects() {
        let body = AllocateBody {
            asset_id: AssetId::ZERO,
            ..AllocateBody::sample()
        };
        assert_eq!(
            body.validate(PUBLISHED),
            Err(ValidationError::ZeroIdentifier(IdentifierField::Asset))
        );
    }

    #[test]
    fn a_zero_amount_rejects() {
        let body = AllocateBody {
            amount: AssetAmount::ZERO,
            ..AllocateBody::sample()
        };
        assert_eq!(
            body.validate(PUBLISHED),
            Err(ValidationError::ZeroAmount(AmountField::Amount))
        );
    }

    #[test]
    fn a_zero_minimum_destination_amount_rejects() {
        let body = AllocateBody {
            minimum_destination_amount: AssetAmount::ZERO,
            ..AllocateBody::sample()
        };
        assert_eq!(
            body.validate(PUBLISHED),
            Err(ValidationError::ZeroAmount(
                AmountField::MinimumDestinationAmount
            ))
        );
    }

    #[test]
    fn a_minimum_above_the_amount_rejects() {
        let body = AllocateBody {
            amount: AssetAmount::new(10),
            minimum_destination_amount: AssetAmount::new(11),
            ..AllocateBody::sample()
        };
        assert_eq!(
            body.validate(PUBLISHED),
            Err(ValidationError::MinimumAboveAmount)
        );
    }

    #[test]
    fn a_minimum_equal_to_the_amount_is_allowed() {
        let body = AllocateBody {
            amount: AssetAmount::new(10),
            minimum_destination_amount: AssetAmount::new(10),
            ..AllocateBody::sample()
        };
        assert_eq!(body.validate(PUBLISHED), Ok(()));
    }

    #[test]
    fn a_deadline_before_publication_rejects() {
        let body = AllocateBody {
            deadline: Timestamp::new(999),
            ..AllocateBody::sample()
        };
        assert_eq!(
            body.validate(PUBLISHED),
            Err(ValidationError::DeadlineBeforePublication)
        );
    }

    #[test]
    fn a_deadline_equal_to_publication_is_allowed() {
        let body = AllocateBody {
            deadline: PUBLISHED,
            ..AllocateBody::sample()
        };
        assert_eq!(body.validate(PUBLISHED), Ok(()));
    }

    #[test]
    fn a_zero_config_version_rejects() {
        let body = AllocateBody {
            config_version: ConfigVersion::ZERO,
            ..AllocateBody::sample()
        };
        assert_eq!(
            body.validate(PUBLISHED),
            Err(ValidationError::ZeroIdentifier(
                IdentifierField::ConfigVersion
            ))
        );
    }

    #[test]
    fn a_body_shorter_than_its_layout_rejects() {
        let bytes = [0u8; layout::ALLOCATE_BODY_LEN - 1];
        assert!(AllocateBody::decode(&bytes).is_err());
    }
}
