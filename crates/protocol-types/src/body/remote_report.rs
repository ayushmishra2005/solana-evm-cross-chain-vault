use crate::codec::{
    read_array, read_u8, read_u64, read_u128, reserved_is_clear, write_bytes, write_u8, write_u64,
    write_u128,
};
use crate::error::{DecodeError, EncodeError, IdentifierField, ValidationError};
use crate::identifier::{
    AssetAmount, AssetId, Commitment, ConfigVersion, EpochId, Timestamp, TransferId,
};
use crate::layout;

/// How fresh the last real withdrawal probe is.
///
/// This is a graded state, so it is a number rather than a flag.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ProbeStatus {
    #[default]
    NotRequired,
    Fresh,
    Stale,
    Failed,
}

impl ProbeStatus {
    /// Every status, in discriminant order.
    pub const ALL: [Self; 4] = [Self::NotRequired, Self::Fresh, Self::Stale, Self::Failed];

    #[must_use]
    pub const fn to_u8(self) -> u8 {
        match self {
            Self::NotRequired => 0,
            Self::Fresh => 1,
            Self::Stale => 2,
            Self::Failed => 3,
        }
    }

    /// Rejects any value outside the defined set.
    pub const fn from_u8(value: u8) -> Result<Self, DecodeError> {
        match value {
            0 => Ok(Self::NotRequired),
            1 => Ok(Self::Fresh),
            2 => Ok(Self::Stale),
            3 => Ok(Self::Failed),
            other => Err(DecodeError::InvalidProbeStatus(other)),
        }
    }
}

/// What the remote leg says it holds for one epoch.
///
/// Whether the value is economically acceptable is decided later.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RemoteReportBody {
    pub report_id: crate::identifier::ReportId,
    pub epoch_id: EpochId,
    pub asset_id: AssetId,
    pub remote_principal: AssetAmount,
    pub reported_value: AssetAmount,
    pub realized_loss: AssetAmount,
    pub unattributed_balance: AssetAmount,
    pub latest_completed_transfer_id: TransferId,
    pub probe_status: ProbeStatus,
    pub probe_timestamp: Timestamp,
    pub config_version: ConfigVersion,
    pub remote_state_commitment: Commitment,
}

impl RemoteReportBody {
    pub(crate) fn encode_into(&self, out: &mut [u8]) -> Result<(), EncodeError> {
        write_bytes(out, layout::REPORT_ID_OFFSET, self.report_id.as_bytes())?;
        write_u64(out, layout::REPORT_EPOCH_ID_OFFSET, self.epoch_id.get())?;
        write_bytes(
            out,
            layout::REPORT_ASSET_ID_OFFSET,
            self.asset_id.as_bytes(),
        )?;
        write_u128(
            out,
            layout::REPORT_REMOTE_PRINCIPAL_OFFSET,
            self.remote_principal.get(),
        )?;
        write_u128(
            out,
            layout::REPORT_REPORTED_VALUE_OFFSET,
            self.reported_value.get(),
        )?;
        write_u128(
            out,
            layout::REPORT_REALIZED_LOSS_OFFSET,
            self.realized_loss.get(),
        )?;
        write_u128(
            out,
            layout::REPORT_UNATTRIBUTED_BALANCE_OFFSET,
            self.unattributed_balance.get(),
        )?;
        write_bytes(
            out,
            layout::REPORT_LATEST_COMPLETED_TRANSFER_ID_OFFSET,
            self.latest_completed_transfer_id.as_bytes(),
        )?;
        write_u8(
            out,
            layout::REPORT_PROBE_STATUS_OFFSET,
            self.probe_status.to_u8(),
        )?;
        write_bytes(
            out,
            layout::REPORT_RESERVED_OFFSET,
            &[0u8; layout::REPORT_RESERVED_LEN],
        )?;
        write_u64(
            out,
            layout::REPORT_PROBE_TIMESTAMP_OFFSET,
            self.probe_timestamp.get(),
        )?;
        write_u64(
            out,
            layout::REPORT_CONFIG_VERSION_OFFSET,
            self.config_version.get(),
        )?;
        write_bytes(
            out,
            layout::REPORT_REMOTE_STATE_COMMITMENT_OFFSET,
            self.remote_state_commitment.as_bytes(),
        )
    }

    pub(crate) fn decode(bytes: &[u8]) -> Result<Self, DecodeError> {
        if !reserved_is_clear(
            bytes,
            layout::REPORT_RESERVED_OFFSET,
            layout::REPORT_RESERVED_LEN,
        )? {
            return Err(DecodeError::ReservedBytesSet);
        }
        Ok(Self {
            report_id: crate::identifier::ReportId::new(read_array(
                bytes,
                layout::REPORT_ID_OFFSET,
            )?),
            epoch_id: EpochId::new(read_u64(bytes, layout::REPORT_EPOCH_ID_OFFSET)?),
            asset_id: AssetId::new(read_array(bytes, layout::REPORT_ASSET_ID_OFFSET)?),
            remote_principal: AssetAmount::new(read_u128(
                bytes,
                layout::REPORT_REMOTE_PRINCIPAL_OFFSET,
            )?),
            reported_value: AssetAmount::new(read_u128(
                bytes,
                layout::REPORT_REPORTED_VALUE_OFFSET,
            )?),
            realized_loss: AssetAmount::new(read_u128(bytes, layout::REPORT_REALIZED_LOSS_OFFSET)?),
            unattributed_balance: AssetAmount::new(read_u128(
                bytes,
                layout::REPORT_UNATTRIBUTED_BALANCE_OFFSET,
            )?),
            latest_completed_transfer_id: TransferId::new(read_array(
                bytes,
                layout::REPORT_LATEST_COMPLETED_TRANSFER_ID_OFFSET,
            )?),
            probe_status: ProbeStatus::from_u8(read_u8(
                bytes,
                layout::REPORT_PROBE_STATUS_OFFSET,
            )?)?,
            probe_timestamp: Timestamp::new(read_u64(
                bytes,
                layout::REPORT_PROBE_TIMESTAMP_OFFSET,
            )?),
            config_version: ConfigVersion::new(read_u64(
                bytes,
                layout::REPORT_CONFIG_VERSION_OFFSET,
            )?),
            remote_state_commitment: Commitment::new(read_array(
                bytes,
                layout::REPORT_REMOTE_STATE_COMMITMENT_OFFSET,
            )?),
        })
    }

    pub(crate) fn validate(&self, published_at: Timestamp) -> Result<(), ValidationError> {
        if self.report_id.is_zero() {
            return Err(ValidationError::ZeroIdentifier(IdentifierField::Report));
        }
        if self.epoch_id.is_zero() {
            return Err(ValidationError::ZeroIdentifier(IdentifierField::Epoch));
        }
        if self.asset_id.is_zero() {
            return Err(ValidationError::ZeroIdentifier(IdentifierField::Asset));
        }
        if self.config_version.is_zero() {
            return Err(ValidationError::ZeroIdentifier(
                IdentifierField::ConfigVersion,
            ));
        }
        if self.remote_state_commitment.is_zero() {
            return Err(ValidationError::ZeroIdentifier(
                IdentifierField::RemoteStateCommitment,
            ));
        }
        if self.realized_loss > self.remote_principal {
            return Err(ValidationError::RealizedLossAbovePrincipal);
        }
        if self.probe_timestamp > published_at {
            return Err(ValidationError::ProbeTimestampAfterPublication);
        }
        if self.probe_status == ProbeStatus::Fresh && self.probe_timestamp.is_zero() {
            return Err(ValidationError::MissingProbeTimestamp);
        }
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn sample() -> Self {
        Self {
            report_id: crate::identifier::ReportId::new([0x55; 32]),
            epoch_id: EpochId::new(12),
            asset_id: AssetId::new([0x66; 32]),
            remote_principal: AssetAmount::new(2_000_000),
            reported_value: AssetAmount::new(2_050_000),
            realized_loss: AssetAmount::new(1_500),
            unattributed_balance: AssetAmount::new(25),
            latest_completed_transfer_id: TransferId::new([0x77; 32]),
            probe_status: ProbeStatus::Fresh,
            probe_timestamp: Timestamp::new(900),
            config_version: ConfigVersion::new(4),
            remote_state_commitment: Commitment::new([0x88; 32]),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PUBLISHED: Timestamp = Timestamp::new(1_000);

    fn encoded(body: &RemoteReportBody) -> [u8; layout::REMOTE_REPORT_BODY_LEN] {
        let mut bytes = [0u8; layout::REMOTE_REPORT_BODY_LEN];
        assert_eq!(body.encode_into(&mut bytes), Ok(()));
        bytes
    }

    #[test]
    fn every_probe_status_survives_the_round_trip() {
        for status in ProbeStatus::ALL {
            assert_eq!(ProbeStatus::from_u8(status.to_u8()), Ok(status));
        }
    }

    #[test]
    fn the_probe_status_discriminants_are_the_documented_numbers() {
        assert_eq!(ProbeStatus::NotRequired.to_u8(), 0);
        assert_eq!(ProbeStatus::Fresh.to_u8(), 1);
        assert_eq!(ProbeStatus::Stale.to_u8(), 2);
        assert_eq!(ProbeStatus::Failed.to_u8(), 3);
    }

    #[test]
    fn an_unknown_probe_status_rejects() {
        for value in [4u8, 5, 128, u8::MAX] {
            assert_eq!(
                ProbeStatus::from_u8(value),
                Err(DecodeError::InvalidProbeStatus(value))
            );
        }
    }

    #[test]
    fn a_sample_body_survives_the_round_trip() {
        let body = RemoteReportBody::sample();
        assert_eq!(RemoteReportBody::decode(&encoded(&body)), Ok(body));
    }

    #[test]
    fn the_reserved_bytes_are_written_as_zero() {
        let bytes = encoded(&RemoteReportBody::sample());
        let start = layout::REPORT_RESERVED_OFFSET;
        assert_eq!(
            bytes.get(start..start + layout::REPORT_RESERVED_LEN),
            Some(&[0u8; layout::REPORT_RESERVED_LEN][..])
        );
    }

    #[test]
    fn a_non_zero_reserved_byte_rejects() {
        let mut bytes = encoded(&RemoteReportBody::sample());
        if let Some(byte) = bytes.get_mut(layout::REPORT_RESERVED_OFFSET) {
            *byte = 1;
        }
        assert_eq!(
            RemoteReportBody::decode(&bytes),
            Err(DecodeError::ReservedBytesSet)
        );
    }

    #[test]
    fn an_unknown_probe_status_byte_rejects_during_decoding() {
        let mut bytes = encoded(&RemoteReportBody::sample());
        if let Some(byte) = bytes.get_mut(layout::REPORT_PROBE_STATUS_OFFSET) {
            *byte = 9;
        }
        assert_eq!(
            RemoteReportBody::decode(&bytes),
            Err(DecodeError::InvalidProbeStatus(9))
        );
    }

    #[test]
    fn a_sample_body_passes_validation() {
        assert_eq!(RemoteReportBody::sample().validate(PUBLISHED), Ok(()));
    }

    #[test]
    fn a_zero_report_id_rejects() {
        let body = RemoteReportBody {
            report_id: crate::identifier::ReportId::ZERO,
            ..RemoteReportBody::sample()
        };
        assert_eq!(
            body.validate(PUBLISHED),
            Err(ValidationError::ZeroIdentifier(IdentifierField::Report))
        );
    }

    #[test]
    fn a_zero_epoch_id_rejects() {
        let body = RemoteReportBody {
            epoch_id: EpochId::ZERO,
            ..RemoteReportBody::sample()
        };
        assert_eq!(
            body.validate(PUBLISHED),
            Err(ValidationError::ZeroIdentifier(IdentifierField::Epoch))
        );
    }

    #[test]
    fn a_zero_asset_id_rejects() {
        let body = RemoteReportBody {
            asset_id: AssetId::ZERO,
            ..RemoteReportBody::sample()
        };
        assert_eq!(
            body.validate(PUBLISHED),
            Err(ValidationError::ZeroIdentifier(IdentifierField::Asset))
        );
    }

    #[test]
    fn a_zero_config_version_rejects() {
        let body = RemoteReportBody {
            config_version: ConfigVersion::ZERO,
            ..RemoteReportBody::sample()
        };
        assert_eq!(
            body.validate(PUBLISHED),
            Err(ValidationError::ZeroIdentifier(
                IdentifierField::ConfigVersion
            ))
        );
    }

    #[test]
    fn a_zero_remote_state_commitment_rejects() {
        let body = RemoteReportBody {
            remote_state_commitment: Commitment::ZERO,
            ..RemoteReportBody::sample()
        };
        assert_eq!(
            body.validate(PUBLISHED),
            Err(ValidationError::ZeroIdentifier(
                IdentifierField::RemoteStateCommitment
            ))
        );
    }

    #[test]
    fn a_realized_loss_above_the_principal_rejects() {
        let body = RemoteReportBody {
            remote_principal: AssetAmount::new(10),
            realized_loss: AssetAmount::new(11),
            ..RemoteReportBody::sample()
        };
        assert_eq!(
            body.validate(PUBLISHED),
            Err(ValidationError::RealizedLossAbovePrincipal)
        );
    }

    #[test]
    fn a_realized_loss_equal_to_the_principal_is_allowed() {
        let body = RemoteReportBody {
            remote_principal: AssetAmount::new(10),
            realized_loss: AssetAmount::new(10),
            ..RemoteReportBody::sample()
        };
        assert_eq!(body.validate(PUBLISHED), Ok(()));
    }

    #[test]
    fn a_probe_timestamp_after_publication_rejects() {
        let body = RemoteReportBody {
            probe_timestamp: Timestamp::new(1_001),
            ..RemoteReportBody::sample()
        };
        assert_eq!(
            body.validate(PUBLISHED),
            Err(ValidationError::ProbeTimestampAfterPublication)
        );
    }

    #[test]
    fn a_fresh_probe_without_a_timestamp_rejects() {
        let body = RemoteReportBody {
            probe_status: ProbeStatus::Fresh,
            probe_timestamp: Timestamp::ZERO,
            ..RemoteReportBody::sample()
        };
        assert_eq!(
            body.validate(PUBLISHED),
            Err(ValidationError::MissingProbeTimestamp)
        );
    }

    #[test]
    fn a_not_required_probe_may_leave_the_timestamp_at_zero() {
        let body = RemoteReportBody {
            probe_status: ProbeStatus::NotRequired,
            probe_timestamp: Timestamp::ZERO,
            ..RemoteReportBody::sample()
        };
        assert_eq!(body.validate(PUBLISHED), Ok(()));
    }

    #[test]
    fn a_stale_or_failed_probe_may_leave_the_timestamp_at_zero() {
        for status in [ProbeStatus::Stale, ProbeStatus::Failed] {
            let body = RemoteReportBody {
                probe_status: status,
                probe_timestamp: Timestamp::ZERO,
                ..RemoteReportBody::sample()
            };
            assert_eq!(body.validate(PUBLISHED), Ok(()));
        }
    }

    #[test]
    fn a_reported_value_far_above_the_principal_is_left_to_later_logic() {
        let body = RemoteReportBody {
            remote_principal: AssetAmount::new(1),
            reported_value: AssetAmount::new(u128::MAX),
            realized_loss: AssetAmount::ZERO,
            ..RemoteReportBody::sample()
        };
        assert_eq!(body.validate(PUBLISHED), Ok(()));
    }

    #[test]
    fn a_zero_latest_completed_transfer_id_is_allowed() {
        let body = RemoteReportBody {
            latest_completed_transfer_id: TransferId::ZERO,
            ..RemoteReportBody::sample()
        };
        assert_eq!(body.validate(PUBLISHED), Ok(()));
    }

    #[test]
    fn a_body_shorter_than_its_layout_rejects() {
        let bytes = [0u8; layout::REMOTE_REPORT_BODY_LEN - 1];
        assert!(RemoteReportBody::decode(&bytes).is_err());
    }
}
