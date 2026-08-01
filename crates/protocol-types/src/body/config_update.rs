use crate::codec::{
    read_array, read_u16, read_u64, reserved_is_clear, write_bytes, write_u16, write_u64,
};
use crate::error::{AmountField, DecodeError, EncodeError, IdentifierField, ValidationError};
use crate::identifier::{BasisPoints, Commitment, ConfigVersion, Timestamp};
use crate::layout;

/// Moves both legs to a new configuration generation.
///
/// The fields are fixed. There is no key value section.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ConfigUpdateBody {
    pub config_version: ConfigVersion,
    pub previous_config_version: ConfigVersion,
    pub max_remote_allocation_bps: BasisPoints,
    pub max_upward_deviation_bps: BasisPoints,
    pub max_downward_deviation_bps: BasisPoints,
    pub max_report_age: u64,
    pub effective_timestamp: Timestamp,
    pub config_commitment: Commitment,
}

impl ConfigUpdateBody {
    pub(crate) fn encode_into(&self, out: &mut [u8]) -> Result<(), EncodeError> {
        write_u64(
            out,
            layout::CONFIG_VERSION_OFFSET,
            self.config_version.get(),
        )?;
        write_u64(
            out,
            layout::CONFIG_PREVIOUS_VERSION_OFFSET,
            self.previous_config_version.get(),
        )?;
        write_u16(
            out,
            layout::CONFIG_MAX_REMOTE_ALLOCATION_BPS_OFFSET,
            self.max_remote_allocation_bps.get(),
        )?;
        write_u16(
            out,
            layout::CONFIG_MAX_UPWARD_DEVIATION_BPS_OFFSET,
            self.max_upward_deviation_bps.get(),
        )?;
        write_u16(
            out,
            layout::CONFIG_MAX_DOWNWARD_DEVIATION_BPS_OFFSET,
            self.max_downward_deviation_bps.get(),
        )?;
        write_bytes(
            out,
            layout::CONFIG_RESERVED_OFFSET,
            &[0u8; layout::CONFIG_RESERVED_LEN],
        )?;
        write_u64(
            out,
            layout::CONFIG_MAX_REPORT_AGE_OFFSET,
            self.max_report_age,
        )?;
        write_u64(
            out,
            layout::CONFIG_EFFECTIVE_TIMESTAMP_OFFSET,
            self.effective_timestamp.get(),
        )?;
        write_bytes(
            out,
            layout::CONFIG_COMMITMENT_OFFSET,
            self.config_commitment.as_bytes(),
        )
    }

    pub(crate) fn decode(bytes: &[u8]) -> Result<Self, DecodeError> {
        if !reserved_is_clear(
            bytes,
            layout::CONFIG_RESERVED_OFFSET,
            layout::CONFIG_RESERVED_LEN,
        )? {
            return Err(DecodeError::ReservedBytesSet);
        }
        Ok(Self {
            config_version: ConfigVersion::new(read_u64(bytes, layout::CONFIG_VERSION_OFFSET)?),
            previous_config_version: ConfigVersion::new(read_u64(
                bytes,
                layout::CONFIG_PREVIOUS_VERSION_OFFSET,
            )?),
            max_remote_allocation_bps: BasisPoints::new(read_u16(
                bytes,
                layout::CONFIG_MAX_REMOTE_ALLOCATION_BPS_OFFSET,
            )?),
            max_upward_deviation_bps: BasisPoints::new(read_u16(
                bytes,
                layout::CONFIG_MAX_UPWARD_DEVIATION_BPS_OFFSET,
            )?),
            max_downward_deviation_bps: BasisPoints::new(read_u16(
                bytes,
                layout::CONFIG_MAX_DOWNWARD_DEVIATION_BPS_OFFSET,
            )?),
            max_report_age: read_u64(bytes, layout::CONFIG_MAX_REPORT_AGE_OFFSET)?,
            effective_timestamp: Timestamp::new(read_u64(
                bytes,
                layout::CONFIG_EFFECTIVE_TIMESTAMP_OFFSET,
            )?),
            config_commitment: Commitment::new(read_array(
                bytes,
                layout::CONFIG_COMMITMENT_OFFSET,
            )?),
        })
    }

    pub(crate) fn validate(&self, published_at: Timestamp) -> Result<(), ValidationError> {
        if self.config_version.is_zero() {
            return Err(ValidationError::ZeroIdentifier(
                IdentifierField::ConfigVersion,
            ));
        }
        if self.config_version <= self.previous_config_version {
            return Err(ValidationError::ConfigVersionNotIncreasing);
        }
        let rates = [
            self.max_remote_allocation_bps,
            self.max_upward_deviation_bps,
            self.max_downward_deviation_bps,
        ];
        if rates.iter().any(|rate| !rate.is_in_range()) {
            return Err(ValidationError::BasisPointsOutOfRange);
        }
        if self.max_report_age == 0 {
            return Err(ValidationError::ZeroAmount(AmountField::MaxReportAge));
        }
        if self.effective_timestamp < published_at {
            return Err(ValidationError::EffectiveTimestampBeforePublication);
        }
        if self.config_commitment.is_zero() {
            return Err(ValidationError::ZeroIdentifier(
                IdentifierField::ConfigCommitment,
            ));
        }
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn sample() -> Self {
        Self {
            config_version: ConfigVersion::new(5),
            previous_config_version: ConfigVersion::new(4),
            max_remote_allocation_bps: BasisPoints::new(6_000),
            max_upward_deviation_bps: BasisPoints::new(200),
            max_downward_deviation_bps: BasisPoints::new(1_000),
            max_report_age: 3_600,
            effective_timestamp: Timestamp::new(1_500),
            config_commitment: Commitment::new([0xCC; 32]),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PUBLISHED: Timestamp = Timestamp::new(1_000);

    fn encoded(body: &ConfigUpdateBody) -> [u8; layout::CONFIG_UPDATE_BODY_LEN] {
        let mut bytes = [0u8; layout::CONFIG_UPDATE_BODY_LEN];
        assert_eq!(body.encode_into(&mut bytes), Ok(()));
        bytes
    }

    #[test]
    fn a_sample_body_survives_the_round_trip() {
        let body = ConfigUpdateBody::sample();
        assert_eq!(ConfigUpdateBody::decode(&encoded(&body)), Ok(body));
    }

    #[test]
    fn the_basis_point_fields_sit_at_their_declared_offsets() {
        let body = ConfigUpdateBody {
            max_remote_allocation_bps: BasisPoints::new(0x0102),
            max_upward_deviation_bps: BasisPoints::new(0x0304),
            max_downward_deviation_bps: BasisPoints::new(0x0506),
            ..ConfigUpdateBody::sample()
        };
        let bytes = encoded(&body);
        let start = layout::CONFIG_MAX_REMOTE_ALLOCATION_BPS_OFFSET;
        assert_eq!(bytes.get(start..start + 6), Some(&[1u8, 2, 3, 4, 5, 6][..]));
    }

    #[test]
    fn the_reserved_bytes_are_written_as_zero() {
        let bytes = encoded(&ConfigUpdateBody::sample());
        let start = layout::CONFIG_RESERVED_OFFSET;
        assert_eq!(
            bytes.get(start..start + layout::CONFIG_RESERVED_LEN),
            Some(&[0u8; layout::CONFIG_RESERVED_LEN][..])
        );
    }

    #[test]
    fn a_non_zero_reserved_byte_rejects() {
        let mut bytes = encoded(&ConfigUpdateBody::sample());
        if let Some(byte) = bytes.get_mut(layout::CONFIG_RESERVED_OFFSET) {
            *byte = 1;
        }
        assert_eq!(
            ConfigUpdateBody::decode(&bytes),
            Err(DecodeError::ReservedBytesSet)
        );
    }

    #[test]
    fn a_sample_body_passes_validation() {
        assert_eq!(ConfigUpdateBody::sample().validate(PUBLISHED), Ok(()));
    }

    #[test]
    fn a_zero_config_version_rejects() {
        let body = ConfigUpdateBody {
            config_version: ConfigVersion::ZERO,
            previous_config_version: ConfigVersion::ZERO,
            ..ConfigUpdateBody::sample()
        };
        assert_eq!(
            body.validate(PUBLISHED),
            Err(ValidationError::ZeroIdentifier(
                IdentifierField::ConfigVersion
            ))
        );
    }

    #[test]
    fn a_config_version_that_does_not_increase_rejects() {
        for previous in [5u64, 6] {
            let body = ConfigUpdateBody {
                config_version: ConfigVersion::new(5),
                previous_config_version: ConfigVersion::new(previous),
                ..ConfigUpdateBody::sample()
            };
            assert_eq!(
                body.validate(PUBLISHED),
                Err(ValidationError::ConfigVersionNotIncreasing)
            );
        }
    }

    #[test]
    fn a_first_config_may_follow_a_zero_previous_version() {
        let body = ConfigUpdateBody {
            config_version: ConfigVersion::new(1),
            previous_config_version: ConfigVersion::ZERO,
            ..ConfigUpdateBody::sample()
        };
        assert_eq!(body.validate(PUBLISHED), Ok(()));
    }

    #[test]
    fn each_basis_point_field_rejects_above_ten_thousand() {
        let fields: [fn(&mut ConfigUpdateBody, BasisPoints); 3] = [
            |body, value| body.max_remote_allocation_bps = value,
            |body, value| body.max_upward_deviation_bps = value,
            |body, value| body.max_downward_deviation_bps = value,
        ];
        for set in fields {
            let mut body = ConfigUpdateBody::sample();
            set(&mut body, BasisPoints::new(10_001));
            assert_eq!(
                body.validate(PUBLISHED),
                Err(ValidationError::BasisPointsOutOfRange)
            );
            set(&mut body, BasisPoints::new(10_000));
            assert_eq!(body.validate(PUBLISHED), Ok(()));
        }
    }

    #[test]
    fn a_zero_max_report_age_rejects() {
        let body = ConfigUpdateBody {
            max_report_age: 0,
            ..ConfigUpdateBody::sample()
        };
        assert_eq!(
            body.validate(PUBLISHED),
            Err(ValidationError::ZeroAmount(AmountField::MaxReportAge))
        );
    }

    #[test]
    fn an_effective_timestamp_before_publication_rejects() {
        let body = ConfigUpdateBody {
            effective_timestamp: Timestamp::new(999),
            ..ConfigUpdateBody::sample()
        };
        assert_eq!(
            body.validate(PUBLISHED),
            Err(ValidationError::EffectiveTimestampBeforePublication)
        );
    }

    #[test]
    fn an_effective_timestamp_equal_to_publication_is_allowed() {
        let body = ConfigUpdateBody {
            effective_timestamp: PUBLISHED,
            ..ConfigUpdateBody::sample()
        };
        assert_eq!(body.validate(PUBLISHED), Ok(()));
    }

    #[test]
    fn a_zero_config_commitment_rejects() {
        let body = ConfigUpdateBody {
            config_commitment: Commitment::ZERO,
            ..ConfigUpdateBody::sample()
        };
        assert_eq!(
            body.validate(PUBLISHED),
            Err(ValidationError::ZeroIdentifier(
                IdentifierField::ConfigCommitment
            ))
        );
    }

    #[test]
    fn a_body_shorter_than_its_layout_rejects() {
        let bytes = [0u8; layout::CONFIG_UPDATE_BODY_LEN - 1];
        assert!(ConfigUpdateBody::decode(&bytes).is_err());
    }
}
