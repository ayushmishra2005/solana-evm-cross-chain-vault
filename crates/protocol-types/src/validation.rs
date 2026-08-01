//! Rules that decide whether a header is well formed.
//!
//! Everything here is stateless. Authorisation, replay and stored sequence
//! state belong to the application, not to the codec.

use crate::codec::{read_array, read_u16, read_u32, saturating_u32};
use crate::error::{DecodeError, EncodeError, IdentifierField, ValidationError};
use crate::layout;
use crate::message::{Header, MessageType};

/// What structural inspection learned before any field was parsed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Frame {
    pub(crate) message_type: MessageType,
    pub(crate) body_len: usize,
}

/// Checks the frame of a byte string without trusting any field.
///
/// It settles the magic, the versions, the kind and the exact total width.
pub(crate) fn inspect_frame(bytes: &[u8]) -> Result<Frame, DecodeError> {
    if bytes.len() < layout::HEADER_LEN {
        return Err(DecodeError::Truncated {
            needed: saturating_u32(layout::HEADER_LEN),
            found: saturating_u32(bytes.len()),
        });
    }

    if read_array::<{ layout::MAGIC_LEN }>(bytes, layout::MAGIC_OFFSET)? != crate::MAGIC {
        return Err(DecodeError::InvalidMagic);
    }

    let protocol = read_u16(bytes, layout::PROTOCOL_VERSION_OFFSET)?;
    if protocol != crate::PROTOCOL_VERSION.get() {
        return Err(DecodeError::UnsupportedProtocolVersion(protocol));
    }

    let schema = read_u16(bytes, layout::SCHEMA_VERSION_OFFSET)?;
    if schema != crate::SCHEMA_VERSION.get() {
        return Err(DecodeError::UnsupportedSchemaVersion(schema));
    }

    let message_type = MessageType::from_u16(read_u16(bytes, layout::MESSAGE_TYPE_OFFSET)?)?;
    let body_len = message_type.body_len();

    let declared = read_u32(bytes, layout::BODY_LENGTH_OFFSET)?;
    let expected = saturating_u32(body_len);
    if declared != expected {
        return Err(DecodeError::BodyLengthMismatch {
            expected,
            found: declared,
        });
    }

    let total = layout::HEADER_LEN
        .checked_add(body_len)
        .ok_or(DecodeError::LengthOverflow)?;
    if bytes.len() < total {
        return Err(DecodeError::Truncated {
            needed: saturating_u32(total),
            found: saturating_u32(bytes.len()),
        });
    }
    if bytes.len() > total {
        return Err(DecodeError::TrailingBytes {
            expected: saturating_u32(total),
            found: saturating_u32(bytes.len()),
        });
    }

    Ok(Frame {
        message_type,
        body_len,
    })
}

/// Refuses to encode a header that names a version this build cannot read.
pub(crate) fn validate_supported_versions_for_encode(header: &Header) -> Result<(), EncodeError> {
    if header.protocol_version != crate::PROTOCOL_VERSION {
        return Err(EncodeError::UnsupportedProtocolVersion(
            header.protocol_version.get(),
        ));
    }
    if header.schema_version != crate::SCHEMA_VERSION {
        return Err(EncodeError::UnsupportedSchemaVersion(
            header.schema_version.get(),
        ));
    }
    Ok(())
}

/// Field rules that hold for every message kind.
pub(crate) fn validate_header(header: &Header) -> Result<(), ValidationError> {
    if header.flags.has_reserved_bits() {
        return Err(ValidationError::ReservedFlagsSet);
    }

    if header.source_chain.is_zero() {
        return Err(ValidationError::ZeroIdentifier(
            IdentifierField::SourceChain,
        ));
    }
    if header.destination_chain.is_zero() {
        return Err(ValidationError::ZeroIdentifier(
            IdentifierField::DestinationChain,
        ));
    }
    if header.source_chain == header.destination_chain {
        return Err(ValidationError::SourceEqualsDestinationChain);
    }

    if header.source_application.is_zero() {
        return Err(ValidationError::ZeroIdentifier(
            IdentifierField::SourceApplication,
        ));
    }
    if header.destination_application.is_zero() {
        return Err(ValidationError::ZeroIdentifier(
            IdentifierField::DestinationApplication,
        ));
    }
    if header.source_application == header.destination_application {
        return Err(ValidationError::SourceEqualsDestinationApplication);
    }

    if header.deployment_id.is_zero() {
        return Err(ValidationError::ZeroIdentifier(IdentifierField::Deployment));
    }
    if header.vault_id.is_zero() {
        return Err(ValidationError::ZeroIdentifier(IdentifierField::Vault));
    }
    if header.lane_id.is_zero() {
        return Err(ValidationError::ZeroIdentifier(IdentifierField::Lane));
    }
    if header.sequence.is_zero() {
        return Err(ValidationError::ZeroIdentifier(IdentifierField::Sequence));
    }

    if header.published_at < header.observed_at {
        return Err(ValidationError::PublicationBeforeObservation);
    }
    if header.expires_at < header.published_at {
        return Err(ValidationError::ExpirationBeforePublication);
    }
    Ok(())
}
