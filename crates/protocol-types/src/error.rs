use core::fmt;

/// Identifier that must never be zero.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum IdentifierField {
    SourceChain,
    DestinationChain,
    SourceApplication,
    DestinationApplication,
    Deployment,
    Vault,
    Lane,
    Sequence,
    Transfer,
    Epoch,
    ConfigVersion,
    RemoteStateCommitment,
    ConfigCommitment,
    DestinationReference,
}

impl IdentifierField {
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::SourceChain => "source chain",
            Self::DestinationChain => "destination chain",
            Self::SourceApplication => "source application",
            Self::DestinationApplication => "destination application",
            Self::Deployment => "deployment id",
            Self::Vault => "vault id",
            Self::Lane => "lane id",
            Self::Sequence => "sequence",
            Self::Transfer => "transfer id",
            Self::Epoch => "epoch id",
            Self::ConfigVersion => "config version",
            Self::RemoteStateCommitment => "remote state commitment",
            Self::ConfigCommitment => "config commitment",
            Self::DestinationReference => "destination reference",
        }
    }
}

/// Amount that must be greater than zero.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum AmountField {
    Amount,
    MinimumDestinationAmount,
    RequestedAmount,
    MinimumReturnAmount,
    PrincipalSent,
    ActualAmountSent,
    MaxReportAge,
}

impl AmountField {
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Amount => "amount",
            Self::MinimumDestinationAmount => "minimum destination amount",
            Self::RequestedAmount => "requested amount",
            Self::MinimumReturnAmount => "minimum return amount",
            Self::PrincipalSent => "principal sent",
            Self::ActualAmountSent => "actual amount sent",
            Self::MaxReportAge => "max report age",
        }
    }
}

/// A rule about field values that both encoding and decoding enforce.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum ValidationError {
    ZeroIdentifier(IdentifierField),
    ZeroAmount(AmountField),
    ReservedFlagsSet,
    SourceEqualsDestinationChain,
    SourceEqualsDestinationApplication,
    PublicationBeforeObservation,
    ExpirationBeforePublication,
    DeadlineBeforePublication,
    EffectiveTimestampBeforePublication,
    ProbeTimestampAfterObservation,
    SentTimestampAfterObservation,
    MissingProbeTimestamp,
    MinimumAboveAmount,
    RealizedLossAbovePrincipal,
    BasisPointsOutOfRange,
    ConfigVersionNotIncreasing,
}

impl fmt::Display for ValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroIdentifier(field) => {
                formatter.write_str(field.name())?;
                formatter.write_str(" must not be zero")
            }
            Self::ZeroAmount(field) => {
                formatter.write_str(field.name())?;
                formatter.write_str(" must be greater than zero")
            }
            Self::ReservedFlagsSet => formatter.write_str("a reserved flag bit is set"),
            Self::SourceEqualsDestinationChain => {
                formatter.write_str("source and destination chains are the same")
            }
            Self::SourceEqualsDestinationApplication => {
                formatter.write_str("source and destination applications are the same")
            }
            Self::PublicationBeforeObservation => {
                formatter.write_str("publication time is before observation time")
            }
            Self::ExpirationBeforePublication => {
                formatter.write_str("expiration time is before publication time")
            }
            Self::DeadlineBeforePublication => {
                formatter.write_str("deadline is before publication time")
            }
            Self::EffectiveTimestampBeforePublication => {
                formatter.write_str("effective time is before publication time")
            }
            Self::ProbeTimestampAfterObservation => {
                formatter.write_str("probe time is after publication time")
            }
            Self::SentTimestampAfterObservation => {
                formatter.write_str("sent time is after publication time")
            }
            Self::MissingProbeTimestamp => {
                formatter.write_str("a fresh probe needs a probe timestamp")
            }
            Self::MinimumAboveAmount => {
                formatter.write_str("the minimum is larger than the amount it bounds")
            }
            Self::RealizedLossAbovePrincipal => {
                formatter.write_str("realized loss is larger than the principal")
            }
            Self::BasisPointsOutOfRange => formatter.write_str("basis points exceed ten thousand"),
            Self::ConfigVersionNotIncreasing => {
                formatter.write_str("config version does not increase")
            }
        }
    }
}

impl core::error::Error for ValidationError {}

/// Why a message could not be encoded.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum EncodeError {
    BufferTooSmall { needed: u32, available: u32 },
    UnsupportedProtocolVersion(u16),
    UnsupportedSchemaVersion(u16),
    Invalid(ValidationError),
}

impl From<ValidationError> for EncodeError {
    fn from(value: ValidationError) -> Self {
        Self::Invalid(value)
    }
}

impl fmt::Display for EncodeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BufferTooSmall { needed, available } => {
                write!(
                    formatter,
                    "buffer holds {available} bytes but {needed} are needed"
                )
            }
            Self::UnsupportedProtocolVersion(found) => {
                write!(formatter, "protocol version {found} is not supported")
            }
            Self::UnsupportedSchemaVersion(found) => {
                write!(formatter, "schema version {found} is not supported")
            }
            Self::Invalid(reason) => reason.fmt(formatter),
        }
    }
}

impl core::error::Error for EncodeError {}

/// Why a byte string is not a canonical message.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum DecodeError {
    Truncated { needed: u32, found: u32 },
    TrailingBytes { expected: u32, found: u32 },
    InvalidMagic,
    UnsupportedProtocolVersion(u16),
    UnsupportedSchemaVersion(u16),
    UnknownMessageType(u8),
    InvalidProbeStatus(u8),
    BodyHashMismatch,
    LengthOverflow,
    Invalid(ValidationError),
}

impl From<ValidationError> for DecodeError {
    fn from(value: ValidationError) -> Self {
        Self::Invalid(value)
    }
}

impl fmt::Display for DecodeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Truncated { needed, found } => {
                write!(
                    formatter,
                    "input holds {found} bytes but {needed} are needed"
                )
            }
            Self::TrailingBytes { expected, found } => {
                write!(
                    formatter,
                    "message ends at {expected} bytes but input holds {found}"
                )
            }
            Self::InvalidMagic => formatter.write_str("magic prefix does not match"),
            Self::UnsupportedProtocolVersion(found) => {
                write!(formatter, "protocol version {found} is not supported")
            }
            Self::UnsupportedSchemaVersion(found) => {
                write!(formatter, "schema version {found} is not supported")
            }
            Self::UnknownMessageType(found) => {
                write!(formatter, "message type {found} is not known")
            }
            Self::InvalidProbeStatus(found) => {
                write!(formatter, "probe status {found} is not known")
            }
            Self::BodyHashMismatch => formatter.write_str("body hash does not match the body"),
            Self::LengthOverflow => formatter.write_str("declared length does not fit in memory"),
            Self::Invalid(reason) => reason.fmt(formatter),
        }
    }
}

impl core::error::Error for DecodeError {}

#[cfg(test)]
mod tests {
    use super::*;
    extern crate alloc;
    use alloc::string::ToString;

    const IDENTIFIERS: [IdentifierField; 14] = [
        IdentifierField::SourceChain,
        IdentifierField::DestinationChain,
        IdentifierField::SourceApplication,
        IdentifierField::DestinationApplication,
        IdentifierField::Deployment,
        IdentifierField::Vault,
        IdentifierField::Lane,
        IdentifierField::Sequence,
        IdentifierField::Transfer,
        IdentifierField::Epoch,
        IdentifierField::ConfigVersion,
        IdentifierField::RemoteStateCommitment,
        IdentifierField::ConfigCommitment,
        IdentifierField::DestinationReference,
    ];

    const AMOUNTS: [AmountField; 7] = [
        AmountField::Amount,
        AmountField::MinimumDestinationAmount,
        AmountField::RequestedAmount,
        AmountField::MinimumReturnAmount,
        AmountField::PrincipalSent,
        AmountField::ActualAmountSent,
        AmountField::MaxReportAge,
    ];

    const VALIDATIONS: [ValidationError; 16] = [
        ValidationError::ZeroIdentifier(IdentifierField::Vault),
        ValidationError::ZeroAmount(AmountField::Amount),
        ValidationError::ReservedFlagsSet,
        ValidationError::SourceEqualsDestinationChain,
        ValidationError::SourceEqualsDestinationApplication,
        ValidationError::PublicationBeforeObservation,
        ValidationError::ExpirationBeforePublication,
        ValidationError::DeadlineBeforePublication,
        ValidationError::EffectiveTimestampBeforePublication,
        ValidationError::ProbeTimestampAfterObservation,
        ValidationError::SentTimestampAfterObservation,
        ValidationError::MissingProbeTimestamp,
        ValidationError::MinimumAboveAmount,
        ValidationError::RealizedLossAbovePrincipal,
        ValidationError::BasisPointsOutOfRange,
        ValidationError::ConfigVersionNotIncreasing,
    ];

    const DECODES: [DecodeError; 10] = [
        DecodeError::Truncated {
            needed: 247,
            found: 8,
        },
        DecodeError::TrailingBytes {
            expected: 343,
            found: 344,
        },
        DecodeError::InvalidMagic,
        DecodeError::UnsupportedProtocolVersion(2),
        DecodeError::UnsupportedSchemaVersion(3),
        DecodeError::UnknownMessageType(9),
        DecodeError::InvalidProbeStatus(7),
        DecodeError::BodyHashMismatch,
        DecodeError::LengthOverflow,
        DecodeError::Invalid(ValidationError::ReservedFlagsSet),
    ];

    const ENCODES: [EncodeError; 4] = [
        EncodeError::BufferTooSmall {
            needed: 343,
            available: 8,
        },
        EncodeError::UnsupportedProtocolVersion(2),
        EncodeError::UnsupportedSchemaVersion(3),
        EncodeError::Invalid(ValidationError::MinimumAboveAmount),
    ];

    #[test]
    fn every_identifier_field_has_a_distinct_name() {
        for (index, field) in IDENTIFIERS.iter().enumerate() {
            assert!(!field.name().is_empty());
            for other in IDENTIFIERS.iter().skip(index.saturating_add(1)) {
                assert_ne!(field.name(), other.name());
            }
        }
    }

    #[test]
    fn every_amount_field_has_a_distinct_name() {
        for (index, field) in AMOUNTS.iter().enumerate() {
            assert!(!field.name().is_empty());
            for other in AMOUNTS.iter().skip(index.saturating_add(1)) {
                assert_ne!(field.name(), other.name());
            }
        }
    }

    #[test]
    fn every_validation_error_prints_a_distinct_message() {
        for (index, error) in VALIDATIONS.iter().enumerate() {
            let text = error.to_string();
            assert!(!text.is_empty());
            for other in VALIDATIONS.iter().skip(index.saturating_add(1)) {
                assert_ne!(text, other.to_string());
            }
        }
    }

    #[test]
    fn every_decode_error_prints_a_distinct_message() {
        for (index, error) in DECODES.iter().enumerate() {
            let text = error.to_string();
            assert!(!text.is_empty());
            for other in DECODES.iter().skip(index.saturating_add(1)) {
                assert_ne!(text, other.to_string());
            }
        }
    }

    #[test]
    fn every_encode_error_prints_a_distinct_message() {
        for (index, error) in ENCODES.iter().enumerate() {
            let text = error.to_string();
            assert!(!text.is_empty());
            for other in ENCODES.iter().skip(index.saturating_add(1)) {
                assert_ne!(text, other.to_string());
            }
        }
    }

    #[test]
    fn a_field_rule_prints_the_field_it_names() {
        let error = ValidationError::ZeroIdentifier(IdentifierField::Lane);
        assert_eq!(error.to_string(), "lane id must not be zero");
        let error = ValidationError::ZeroAmount(AmountField::PrincipalSent);
        assert_eq!(
            error.to_string(),
            "principal sent must be greater than zero"
        );
    }

    #[test]
    fn a_wrapped_rule_prints_the_same_text_in_both_error_types() {
        let rule = ValidationError::BasisPointsOutOfRange;
        assert_eq!(DecodeError::from(rule).to_string(), rule.to_string());
        assert_eq!(EncodeError::from(rule).to_string(), rule.to_string());
    }
}
