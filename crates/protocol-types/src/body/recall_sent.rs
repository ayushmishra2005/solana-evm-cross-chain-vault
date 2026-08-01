use crate::codec::{read_array, read_u64, read_u128, write_bytes, write_u64, write_u128};
use crate::error::{AmountField, DecodeError, EncodeError, IdentifierField, ValidationError};
use crate::identifier::{AssetAmount, AssetId, Commitment, ConfigVersion, Timestamp, TransferId};
use crate::layout;

/// Says the remote leg released assets back towards the hub.
///
/// It is evidence of a send. It is not evidence of arrival.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RecallSentBody {
    pub transfer_id: TransferId,
    pub asset_id: AssetId,
    pub principal_sent: AssetAmount,
    pub actual_amount_sent: AssetAmount,
    pub realized_loss: AssetAmount,
    pub destination_reference: Commitment,
    pub sent_timestamp: Timestamp,
    pub config_version: ConfigVersion,
}

impl RecallSentBody {
    pub(crate) fn encode_into(&self, out: &mut [u8]) -> Result<(), EncodeError> {
        write_bytes(
            out,
            layout::RECALL_SENT_TRANSFER_ID_OFFSET,
            self.transfer_id.as_bytes(),
        )?;
        write_bytes(
            out,
            layout::RECALL_SENT_ASSET_ID_OFFSET,
            self.asset_id.as_bytes(),
        )?;
        write_u128(
            out,
            layout::RECALL_SENT_PRINCIPAL_SENT_OFFSET,
            self.principal_sent.get(),
        )?;
        write_u128(
            out,
            layout::RECALL_SENT_ACTUAL_AMOUNT_SENT_OFFSET,
            self.actual_amount_sent.get(),
        )?;
        write_u128(
            out,
            layout::RECALL_SENT_REALIZED_LOSS_OFFSET,
            self.realized_loss.get(),
        )?;
        write_bytes(
            out,
            layout::RECALL_SENT_DESTINATION_REFERENCE_OFFSET,
            self.destination_reference.as_bytes(),
        )?;
        write_u64(
            out,
            layout::RECALL_SENT_SENT_TIMESTAMP_OFFSET,
            self.sent_timestamp.get(),
        )?;
        write_u64(
            out,
            layout::RECALL_SENT_CONFIG_VERSION_OFFSET,
            self.config_version.get(),
        )
    }

    pub(crate) fn decode(bytes: &[u8]) -> Result<Self, DecodeError> {
        Ok(Self {
            transfer_id: TransferId::new(read_array(
                bytes,
                layout::RECALL_SENT_TRANSFER_ID_OFFSET,
            )?),
            asset_id: AssetId::new(read_array(bytes, layout::RECALL_SENT_ASSET_ID_OFFSET)?),
            principal_sent: AssetAmount::new(read_u128(
                bytes,
                layout::RECALL_SENT_PRINCIPAL_SENT_OFFSET,
            )?),
            actual_amount_sent: AssetAmount::new(read_u128(
                bytes,
                layout::RECALL_SENT_ACTUAL_AMOUNT_SENT_OFFSET,
            )?),
            realized_loss: AssetAmount::new(read_u128(
                bytes,
                layout::RECALL_SENT_REALIZED_LOSS_OFFSET,
            )?),
            destination_reference: Commitment::new(read_array(
                bytes,
                layout::RECALL_SENT_DESTINATION_REFERENCE_OFFSET,
            )?),
            sent_timestamp: Timestamp::new(read_u64(
                bytes,
                layout::RECALL_SENT_SENT_TIMESTAMP_OFFSET,
            )?),
            config_version: ConfigVersion::new(read_u64(
                bytes,
                layout::RECALL_SENT_CONFIG_VERSION_OFFSET,
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
        if self.principal_sent.is_zero() {
            return Err(ValidationError::ZeroAmount(AmountField::PrincipalSent));
        }
        if self.actual_amount_sent.is_zero() {
            return Err(ValidationError::ZeroAmount(AmountField::ActualAmountSent));
        }
        if self.realized_loss > self.principal_sent {
            return Err(ValidationError::RealizedLossAbovePrincipal);
        }
        if self.destination_reference.is_zero() {
            return Err(ValidationError::ZeroIdentifier(
                IdentifierField::DestinationReference,
            ));
        }
        if self.sent_timestamp > published_at {
            return Err(ValidationError::SentTimestampAfterPublication);
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
            transfer_id: TransferId::new([0x99; 32]),
            asset_id: AssetId::new([0xAA; 32]),
            principal_sent: AssetAmount::new(500_000),
            actual_amount_sent: AssetAmount::new(498_000),
            realized_loss: AssetAmount::new(2_000),
            destination_reference: Commitment::new([0xBB; 32]),
            sent_timestamp: Timestamp::new(950),
            config_version: ConfigVersion::new(4),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PUBLISHED: Timestamp = Timestamp::new(1_000);

    fn encoded(body: &RecallSentBody) -> [u8; layout::RECALL_SENT_BODY_LEN] {
        let mut bytes = [0u8; layout::RECALL_SENT_BODY_LEN];
        assert_eq!(body.encode_into(&mut bytes), Ok(()));
        bytes
    }

    #[test]
    fn a_sample_body_survives_the_round_trip() {
        let body = RecallSentBody::sample();
        assert_eq!(RecallSentBody::decode(&encoded(&body)), Ok(body));
    }

    #[test]
    fn a_sample_body_passes_validation() {
        assert_eq!(RecallSentBody::sample().validate(PUBLISHED), Ok(()));
    }

    #[test]
    fn a_zero_transfer_id_rejects() {
        let body = RecallSentBody {
            transfer_id: TransferId::ZERO,
            ..RecallSentBody::sample()
        };
        assert_eq!(
            body.validate(PUBLISHED),
            Err(ValidationError::ZeroIdentifier(IdentifierField::Transfer))
        );
    }

    #[test]
    fn a_zero_asset_id_rejects() {
        let body = RecallSentBody {
            asset_id: AssetId::ZERO,
            ..RecallSentBody::sample()
        };
        assert_eq!(
            body.validate(PUBLISHED),
            Err(ValidationError::ZeroIdentifier(IdentifierField::Asset))
        );
    }

    #[test]
    fn a_zero_principal_sent_rejects() {
        let body = RecallSentBody {
            principal_sent: AssetAmount::ZERO,
            ..RecallSentBody::sample()
        };
        assert_eq!(
            body.validate(PUBLISHED),
            Err(ValidationError::ZeroAmount(AmountField::PrincipalSent))
        );
    }

    #[test]
    fn a_zero_actual_amount_sent_rejects() {
        let body = RecallSentBody {
            actual_amount_sent: AssetAmount::ZERO,
            ..RecallSentBody::sample()
        };
        assert_eq!(
            body.validate(PUBLISHED),
            Err(ValidationError::ZeroAmount(AmountField::ActualAmountSent))
        );
    }

    #[test]
    fn a_realized_loss_above_the_principal_sent_rejects() {
        let body = RecallSentBody {
            principal_sent: AssetAmount::new(10),
            actual_amount_sent: AssetAmount::new(1),
            realized_loss: AssetAmount::new(11),
            ..RecallSentBody::sample()
        };
        assert_eq!(
            body.validate(PUBLISHED),
            Err(ValidationError::RealizedLossAbovePrincipal)
        );
    }

    #[test]
    fn a_zero_destination_reference_rejects() {
        let body = RecallSentBody {
            destination_reference: Commitment::ZERO,
            ..RecallSentBody::sample()
        };
        assert_eq!(
            body.validate(PUBLISHED),
            Err(ValidationError::ZeroIdentifier(
                IdentifierField::DestinationReference
            ))
        );
    }

    #[test]
    fn a_sent_timestamp_after_publication_rejects() {
        let body = RecallSentBody {
            sent_timestamp: Timestamp::new(1_001),
            ..RecallSentBody::sample()
        };
        assert_eq!(
            body.validate(PUBLISHED),
            Err(ValidationError::SentTimestampAfterPublication)
        );
    }

    #[test]
    fn a_sent_timestamp_equal_to_publication_is_allowed() {
        let body = RecallSentBody {
            sent_timestamp: PUBLISHED,
            ..RecallSentBody::sample()
        };
        assert_eq!(body.validate(PUBLISHED), Ok(()));
    }

    #[test]
    fn a_zero_config_version_rejects() {
        let body = RecallSentBody {
            config_version: ConfigVersion::ZERO,
            ..RecallSentBody::sample()
        };
        assert_eq!(
            body.validate(PUBLISHED),
            Err(ValidationError::ZeroIdentifier(
                IdentifierField::ConfigVersion
            ))
        );
    }

    #[test]
    fn an_amount_sent_above_the_principal_is_left_to_later_logic() {
        let body = RecallSentBody {
            principal_sent: AssetAmount::new(10),
            actual_amount_sent: AssetAmount::new(12),
            realized_loss: AssetAmount::ZERO,
            ..RecallSentBody::sample()
        };
        assert_eq!(body.validate(PUBLISHED), Ok(()));
    }

    #[test]
    fn a_body_shorter_than_its_layout_rejects() {
        let bytes = [0u8; layout::RECALL_SENT_BODY_LEN - 1];
        assert!(RecallSentBody::decode(&bytes).is_err());
    }
}
