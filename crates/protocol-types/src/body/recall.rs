use crate::codec::{read_array, read_u64, read_u128, write_bytes, write_u64, write_u128};
use crate::error::{AmountField, DecodeError, EncodeError, IdentifierField, ValidationError};
use crate::identifier::{AssetAmount, AssetId, ConfigVersion, Timestamp, TransferId};
use crate::layout;

/// Asks the remote leg to unwind and return assets.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RecallBody {
    pub transfer_id: TransferId,
    pub asset_id: AssetId,
    pub requested_amount: AssetAmount,
    pub minimum_return_amount: AssetAmount,
    pub deadline: Timestamp,
    pub config_version: ConfigVersion,
}

impl RecallBody {
    pub(crate) fn encode_into(&self, out: &mut [u8]) -> Result<(), EncodeError> {
        write_bytes(
            out,
            layout::RECALL_TRANSFER_ID_OFFSET,
            self.transfer_id.as_bytes(),
        )?;
        write_bytes(
            out,
            layout::RECALL_ASSET_ID_OFFSET,
            self.asset_id.as_bytes(),
        )?;
        write_u128(
            out,
            layout::RECALL_REQUESTED_AMOUNT_OFFSET,
            self.requested_amount.get(),
        )?;
        write_u128(
            out,
            layout::RECALL_MINIMUM_RETURN_AMOUNT_OFFSET,
            self.minimum_return_amount.get(),
        )?;
        write_u64(out, layout::RECALL_DEADLINE_OFFSET, self.deadline.get())?;
        write_u64(
            out,
            layout::RECALL_CONFIG_VERSION_OFFSET,
            self.config_version.get(),
        )
    }

    pub(crate) fn decode(bytes: &[u8]) -> Result<Self, DecodeError> {
        Ok(Self {
            transfer_id: TransferId::new(read_array(bytes, layout::RECALL_TRANSFER_ID_OFFSET)?),
            asset_id: AssetId::new(read_array(bytes, layout::RECALL_ASSET_ID_OFFSET)?),
            requested_amount: AssetAmount::new(read_u128(
                bytes,
                layout::RECALL_REQUESTED_AMOUNT_OFFSET,
            )?),
            minimum_return_amount: AssetAmount::new(read_u128(
                bytes,
                layout::RECALL_MINIMUM_RETURN_AMOUNT_OFFSET,
            )?),
            deadline: Timestamp::new(read_u64(bytes, layout::RECALL_DEADLINE_OFFSET)?),
            config_version: ConfigVersion::new(read_u64(
                bytes,
                layout::RECALL_CONFIG_VERSION_OFFSET,
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
        if self.requested_amount.is_zero() {
            return Err(ValidationError::ZeroAmount(AmountField::RequestedAmount));
        }
        if self.minimum_return_amount.is_zero() {
            return Err(ValidationError::ZeroAmount(
                AmountField::MinimumReturnAmount,
            ));
        }
        if self.minimum_return_amount > self.requested_amount {
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
            transfer_id: TransferId::new([0x33; 32]),
            asset_id: AssetId::new([0x44; 32]),
            requested_amount: AssetAmount::new(750_000),
            minimum_return_amount: AssetAmount::new(740_000),
            deadline: Timestamp::new(3_000),
            config_version: ConfigVersion::new(9),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PUBLISHED: Timestamp = Timestamp::new(1_000);

    fn encoded(body: &RecallBody) -> [u8; layout::RECALL_BODY_LEN] {
        let mut bytes = [0u8; layout::RECALL_BODY_LEN];
        assert_eq!(body.encode_into(&mut bytes), Ok(()));
        bytes
    }

    #[test]
    fn a_sample_body_survives_the_round_trip() {
        let body = RecallBody::sample();
        assert_eq!(RecallBody::decode(&encoded(&body)), Ok(body));
    }

    #[test]
    fn the_deadline_sits_at_its_declared_offset_in_big_endian() {
        let body = RecallBody {
            deadline: Timestamp::new(0x0102_0304_0506_0708),
            ..RecallBody::sample()
        };
        let bytes = encoded(&body);
        let start = layout::RECALL_DEADLINE_OFFSET;
        assert_eq!(
            bytes.get(start..start + 8),
            Some(&[1u8, 2, 3, 4, 5, 6, 7, 8][..])
        );
    }

    #[test]
    fn a_sample_body_passes_validation() {
        assert_eq!(RecallBody::sample().validate(PUBLISHED), Ok(()));
    }

    #[test]
    fn a_zero_transfer_id_rejects() {
        let body = RecallBody {
            transfer_id: TransferId::ZERO,
            ..RecallBody::sample()
        };
        assert_eq!(
            body.validate(PUBLISHED),
            Err(ValidationError::ZeroIdentifier(IdentifierField::Transfer))
        );
    }

    #[test]
    fn a_zero_asset_id_rejects() {
        let body = RecallBody {
            asset_id: AssetId::ZERO,
            ..RecallBody::sample()
        };
        assert_eq!(
            body.validate(PUBLISHED),
            Err(ValidationError::ZeroIdentifier(IdentifierField::Asset))
        );
    }

    #[test]
    fn a_zero_requested_amount_rejects() {
        let body = RecallBody {
            requested_amount: AssetAmount::ZERO,
            ..RecallBody::sample()
        };
        assert_eq!(
            body.validate(PUBLISHED),
            Err(ValidationError::ZeroAmount(AmountField::RequestedAmount))
        );
    }

    #[test]
    fn a_zero_minimum_return_amount_rejects() {
        let body = RecallBody {
            minimum_return_amount: AssetAmount::ZERO,
            ..RecallBody::sample()
        };
        assert_eq!(
            body.validate(PUBLISHED),
            Err(ValidationError::ZeroAmount(
                AmountField::MinimumReturnAmount
            ))
        );
    }

    #[test]
    fn a_minimum_above_the_requested_amount_rejects() {
        let body = RecallBody {
            requested_amount: AssetAmount::new(10),
            minimum_return_amount: AssetAmount::new(11),
            ..RecallBody::sample()
        };
        assert_eq!(
            body.validate(PUBLISHED),
            Err(ValidationError::MinimumAboveAmount)
        );
    }

    #[test]
    fn a_deadline_before_publication_rejects() {
        let body = RecallBody {
            deadline: Timestamp::new(1),
            ..RecallBody::sample()
        };
        assert_eq!(
            body.validate(PUBLISHED),
            Err(ValidationError::DeadlineBeforePublication)
        );
    }

    #[test]
    fn a_zero_config_version_rejects() {
        let body = RecallBody {
            config_version: ConfigVersion::ZERO,
            ..RecallBody::sample()
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
        let bytes = [0u8; layout::RECALL_BODY_LEN - 1];
        assert!(RecallBody::decode(&bytes).is_err());
    }
}
