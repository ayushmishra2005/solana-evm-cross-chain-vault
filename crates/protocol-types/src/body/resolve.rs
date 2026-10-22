use crate::codec::{read_array, read_u8, read_u64, write_bytes, write_u8, write_u64};
use crate::error::{DecodeError, EncodeError, IdentifierField, ValidationError};
use crate::identifier::{ConfigVersion, Sequence, TransferId};
use crate::layout;

/// What a resolution asks the remote leg to do with one earlier transfer.
///
/// The discriminant is on the wire, so it must never be reordered.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ResolveAction {
    /// Carries out a quarantined transfer now, on its original terms.
    Admit,
}

impl ResolveAction {
    /// Every action, in discriminant order.
    pub const ALL: [Self; 1] = [Self::Admit];

    #[must_use]
    pub const fn to_u8(self) -> u8 {
        match self {
            Self::Admit => 1,
        }
    }

    /// Rejects zero and every action this build does not carry out.
    pub const fn from_u8(value: u8) -> Result<Self, DecodeError> {
        match value {
            1 => Ok(Self::Admit),
            other => Err(DecodeError::UnknownResolveAction(other)),
        }
    }
}

/// Settles one earlier transfer of the same lane without changing its terms.
///
/// The transfer id and the sequence of the message that carried it together
/// name exactly one obligation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResolveBody {
    pub transfer_id: TransferId,
    pub target_sequence: Sequence,
    pub action: ResolveAction,
    pub config_version: ConfigVersion,
}

impl ResolveBody {
    pub(crate) fn encode_into(&self, out: &mut [u8]) -> Result<(), EncodeError> {
        write_bytes(
            out,
            layout::RESOLVE_TRANSFER_ID_OFFSET,
            self.transfer_id.as_bytes(),
        )?;
        write_u64(
            out,
            layout::RESOLVE_TARGET_SEQUENCE_OFFSET,
            self.target_sequence.get(),
        )?;
        write_u8(out, layout::RESOLVE_ACTION_OFFSET, self.action.to_u8())?;
        write_u64(
            out,
            layout::RESOLVE_CONFIG_VERSION_OFFSET,
            self.config_version.get(),
        )
    }

    pub(crate) fn decode(bytes: &[u8]) -> Result<Self, DecodeError> {
        Ok(Self {
            transfer_id: TransferId::new(read_array(bytes, layout::RESOLVE_TRANSFER_ID_OFFSET)?),
            target_sequence: Sequence::new(read_u64(
                bytes,
                layout::RESOLVE_TARGET_SEQUENCE_OFFSET,
            )?),
            action: ResolveAction::from_u8(read_u8(bytes, layout::RESOLVE_ACTION_OFFSET)?)?,
            config_version: ConfigVersion::new(read_u64(
                bytes,
                layout::RESOLVE_CONFIG_VERSION_OFFSET,
            )?),
        })
    }

    /// The target must come earlier on the same lane than the resolution.
    pub(crate) fn validate(&self, sequence: Sequence) -> Result<(), ValidationError> {
        if self.transfer_id.is_zero() {
            return Err(ValidationError::ZeroIdentifier(IdentifierField::Transfer));
        }
        if self.target_sequence.is_zero() {
            return Err(ValidationError::ZeroIdentifier(
                IdentifierField::TargetSequence,
            ));
        }
        if self.target_sequence >= sequence {
            return Err(ValidationError::TargetNotBeforeResolution);
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
            transfer_id: TransferId::new([0x55; 32]),
            target_sequence: Sequence::new(41),
            action: ResolveAction::Admit,
            config_version: ConfigVersion::new(4),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEQUENCE: Sequence = Sequence::new(42);

    fn encoded(body: &ResolveBody) -> [u8; layout::RESOLVE_BODY_LEN] {
        let mut bytes = [0u8; layout::RESOLVE_BODY_LEN];
        assert_eq!(body.encode_into(&mut bytes), Ok(()));
        bytes
    }

    #[test]
    fn a_sample_body_survives_the_round_trip() {
        let body = ResolveBody::sample();
        assert_eq!(ResolveBody::decode(&encoded(&body)), Ok(body));
    }

    #[test]
    fn the_action_sits_at_its_declared_offset() {
        let bytes = encoded(&ResolveBody::sample());
        assert_eq!(bytes.get(layout::RESOLVE_ACTION_OFFSET), Some(&1));
    }

    #[test]
    fn the_target_sequence_sits_at_its_declared_offset_in_big_endian() {
        let body = ResolveBody {
            target_sequence: Sequence::new(0x0102_0304_0506_0708),
            ..ResolveBody::sample()
        };
        let bytes = encoded(&body);
        let start = layout::RESOLVE_TARGET_SEQUENCE_OFFSET;
        assert_eq!(
            bytes.get(start..start + 8),
            Some(&[1u8, 2, 3, 4, 5, 6, 7, 8][..])
        );
    }

    #[test]
    fn every_action_survives_the_round_trip() {
        for action in ResolveAction::ALL {
            assert_eq!(ResolveAction::from_u8(action.to_u8()), Ok(action));
        }
    }

    #[test]
    fn every_byte_outside_the_defined_actions_rejects() {
        for value in 0..=u8::MAX {
            let outcome = ResolveAction::from_u8(value);
            if value == 1 {
                assert_eq!(outcome, Ok(ResolveAction::Admit));
            } else {
                assert_eq!(outcome, Err(DecodeError::UnknownResolveAction(value)));
            }
        }
    }

    #[test]
    fn an_unknown_action_byte_fails_decoding() {
        let mut bytes = encoded(&ResolveBody::sample());
        if let Some(action) = bytes.get_mut(layout::RESOLVE_ACTION_OFFSET) {
            *action = 2;
        }
        assert_eq!(
            ResolveBody::decode(&bytes),
            Err(DecodeError::UnknownResolveAction(2))
        );
    }

    #[test]
    fn a_sample_body_passes_validation() {
        assert_eq!(ResolveBody::sample().validate(SEQUENCE), Ok(()));
    }

    #[test]
    fn a_zero_transfer_id_rejects() {
        let body = ResolveBody {
            transfer_id: TransferId::ZERO,
            ..ResolveBody::sample()
        };
        assert_eq!(
            body.validate(SEQUENCE),
            Err(ValidationError::ZeroIdentifier(IdentifierField::Transfer))
        );
    }

    #[test]
    fn a_zero_target_sequence_rejects() {
        let body = ResolveBody {
            target_sequence: Sequence::new(0),
            ..ResolveBody::sample()
        };
        assert_eq!(
            body.validate(SEQUENCE),
            Err(ValidationError::ZeroIdentifier(
                IdentifierField::TargetSequence
            ))
        );
    }

    #[test]
    fn a_target_at_or_after_the_resolution_rejects() {
        for target in [42, 43, u64::MAX] {
            let body = ResolveBody {
                target_sequence: Sequence::new(target),
                ..ResolveBody::sample()
            };
            assert_eq!(
                body.validate(SEQUENCE),
                Err(ValidationError::TargetNotBeforeResolution),
                "target {target}"
            );
        }
    }

    #[test]
    fn a_zero_config_version_rejects() {
        let body = ResolveBody {
            config_version: ConfigVersion::ZERO,
            ..ResolveBody::sample()
        };
        assert_eq!(
            body.validate(SEQUENCE),
            Err(ValidationError::ZeroIdentifier(
                IdentifierField::ConfigVersion
            ))
        );
    }

    #[test]
    fn a_body_shorter_than_its_layout_rejects() {
        let bytes = [0u8; layout::RESOLVE_BODY_LEN - 1];
        assert!(ResolveBody::decode(&bytes).is_err());
    }
}
