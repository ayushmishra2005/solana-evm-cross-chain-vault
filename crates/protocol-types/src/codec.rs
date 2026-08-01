//! Byte level reads and writes, then the two public entry points.
//!
//! Parsing lives here. Field rules live in [`crate::validation`].

use crate::body::Body;
use crate::commitment::message_id;
use crate::error::{DecodeError, EncodeError};
use crate::hash::keccak256;
use crate::identifier::{
    ApplicationId, BodyHash, ChainId, Commitment, DeploymentId, Flags, LaneId, MessageId,
    ProtocolVersion, SchemaVersion, Sequence, Timestamp, VaultId,
};
use crate::layout;
use crate::message::{Header, Message, MessageType};
use crate::validation;

#[cfg(feature = "alloc")]
use alloc::vec::Vec;

/// Narrows a length for an error field without ever wrapping.
pub(crate) fn saturating_u32(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

fn window(bytes: &[u8], offset: usize, width: usize) -> Result<&[u8], DecodeError> {
    let end = offset
        .checked_add(width)
        .ok_or(DecodeError::LengthOverflow)?;
    bytes.get(offset..end).ok_or(DecodeError::Truncated {
        needed: saturating_u32(end),
        found: saturating_u32(bytes.len()),
    })
}

pub(crate) fn read_array<const N: usize>(
    bytes: &[u8],
    offset: usize,
) -> Result<[u8; N], DecodeError> {
    let slice = window(bytes, offset, N)?;
    <[u8; N]>::try_from(slice).map_err(|_| DecodeError::LengthOverflow)
}

pub(crate) fn read_u8(bytes: &[u8], offset: usize) -> Result<u8, DecodeError> {
    read_array::<1>(bytes, offset).map(u8::from_be_bytes)
}

pub(crate) fn read_u16(bytes: &[u8], offset: usize) -> Result<u16, DecodeError> {
    read_array::<2>(bytes, offset).map(u16::from_be_bytes)
}

pub(crate) fn read_u32(bytes: &[u8], offset: usize) -> Result<u32, DecodeError> {
    read_array::<4>(bytes, offset).map(u32::from_be_bytes)
}

pub(crate) fn read_u64(bytes: &[u8], offset: usize) -> Result<u64, DecodeError> {
    read_array::<8>(bytes, offset).map(u64::from_be_bytes)
}

pub(crate) fn read_u128(bytes: &[u8], offset: usize) -> Result<u128, DecodeError> {
    read_array::<16>(bytes, offset).map(u128::from_be_bytes)
}

/// True when every byte of the range is zero.
pub(crate) fn reserved_is_clear(
    bytes: &[u8],
    offset: usize,
    width: usize,
) -> Result<bool, DecodeError> {
    Ok(window(bytes, offset, width)?.iter().all(|byte| *byte == 0))
}

pub(crate) fn write_bytes(out: &mut [u8], offset: usize, value: &[u8]) -> Result<(), EncodeError> {
    let available = saturating_u32(out.len());
    let end = offset
        .checked_add(value.len())
        .ok_or(EncodeError::BufferTooSmall {
            needed: u32::MAX,
            available,
        })?;
    let slot = out
        .get_mut(offset..end)
        .ok_or(EncodeError::BufferTooSmall {
            needed: saturating_u32(end),
            available,
        })?;
    slot.copy_from_slice(value);
    Ok(())
}

pub(crate) fn write_u8(out: &mut [u8], offset: usize, value: u8) -> Result<(), EncodeError> {
    write_bytes(out, offset, &value.to_be_bytes())
}

pub(crate) fn write_u16(out: &mut [u8], offset: usize, value: u16) -> Result<(), EncodeError> {
    write_bytes(out, offset, &value.to_be_bytes())
}

pub(crate) fn write_u32(out: &mut [u8], offset: usize, value: u32) -> Result<(), EncodeError> {
    write_bytes(out, offset, &value.to_be_bytes())
}

pub(crate) fn write_u64(out: &mut [u8], offset: usize, value: u64) -> Result<(), EncodeError> {
    write_bytes(out, offset, &value.to_be_bytes())
}

pub(crate) fn write_u128(out: &mut [u8], offset: usize, value: u128) -> Result<(), EncodeError> {
    write_bytes(out, offset, &value.to_be_bytes())
}

fn read_header(bytes: &[u8]) -> Result<Header, DecodeError> {
    Ok(Header {
        protocol_version: ProtocolVersion::new(read_u16(bytes, layout::PROTOCOL_VERSION_OFFSET)?),
        schema_version: SchemaVersion::new(read_u16(bytes, layout::SCHEMA_VERSION_OFFSET)?),
        flags: Flags::new(read_u16(bytes, layout::FLAGS_OFFSET)?),
        source_chain: ChainId::new(read_u32(bytes, layout::SOURCE_CHAIN_OFFSET)?),
        destination_chain: ChainId::new(read_u32(bytes, layout::DESTINATION_CHAIN_OFFSET)?),
        source_application: ApplicationId::new(read_array(
            bytes,
            layout::SOURCE_APPLICATION_OFFSET,
        )?),
        destination_application: ApplicationId::new(read_array(
            bytes,
            layout::DESTINATION_APPLICATION_OFFSET,
        )?),
        deployment_id: DeploymentId::new(read_array(bytes, layout::DEPLOYMENT_ID_OFFSET)?),
        vault_id: VaultId::new(read_array(bytes, layout::VAULT_ID_OFFSET)?),
        lane_id: LaneId::new(read_u32(bytes, layout::LANE_ID_OFFSET)?),
        sequence: Sequence::new(read_u64(bytes, layout::SEQUENCE_OFFSET)?),
        previous_commitment: Commitment::new(read_array(
            bytes,
            layout::PREVIOUS_COMMITMENT_OFFSET,
        )?),
        observed_at: Timestamp::new(read_u64(bytes, layout::OBSERVED_AT_OFFSET)?),
        published_at: Timestamp::new(read_u64(bytes, layout::PUBLISHED_AT_OFFSET)?),
        expires_at: Timestamp::new(read_u64(bytes, layout::EXPIRES_AT_OFFSET)?),
    })
}

fn write_header(
    out: &mut [u8],
    header: &Header,
    message_type: MessageType,
    body_hash: BodyHash,
) -> Result<(), EncodeError> {
    write_bytes(out, layout::MAGIC_OFFSET, &crate::MAGIC)?;
    write_u16(
        out,
        layout::PROTOCOL_VERSION_OFFSET,
        header.protocol_version.get(),
    )?;
    write_u16(
        out,
        layout::SCHEMA_VERSION_OFFSET,
        header.schema_version.get(),
    )?;
    write_u16(out, layout::MESSAGE_TYPE_OFFSET, message_type.to_u16())?;
    write_u16(out, layout::FLAGS_OFFSET, header.flags.bits())?;
    write_u32(out, layout::SOURCE_CHAIN_OFFSET, header.source_chain.get())?;
    write_u32(
        out,
        layout::DESTINATION_CHAIN_OFFSET,
        header.destination_chain.get(),
    )?;
    write_bytes(
        out,
        layout::SOURCE_APPLICATION_OFFSET,
        header.source_application.as_bytes(),
    )?;
    write_bytes(
        out,
        layout::DESTINATION_APPLICATION_OFFSET,
        header.destination_application.as_bytes(),
    )?;
    write_bytes(
        out,
        layout::DEPLOYMENT_ID_OFFSET,
        header.deployment_id.as_bytes(),
    )?;
    write_bytes(out, layout::VAULT_ID_OFFSET, header.vault_id.as_bytes())?;
    write_u32(out, layout::LANE_ID_OFFSET, header.lane_id.get())?;
    write_u64(out, layout::SEQUENCE_OFFSET, header.sequence.get())?;
    write_bytes(
        out,
        layout::PREVIOUS_COMMITMENT_OFFSET,
        header.previous_commitment.as_bytes(),
    )?;
    write_u64(out, layout::OBSERVED_AT_OFFSET, header.observed_at.get())?;
    write_u64(out, layout::PUBLISHED_AT_OFFSET, header.published_at.get())?;
    write_u64(out, layout::EXPIRES_AT_OFFSET, header.expires_at.get())?;
    write_u32(
        out,
        layout::BODY_LENGTH_OFFSET,
        saturating_u32(message_type.body_len()),
    )?;
    write_bytes(out, layout::BODY_HASH_OFFSET, body_hash.as_bytes())
}

/// A canonical message held in a fixed size buffer.
///
/// The buffer is sized for the largest message, so encoding needs no
/// allocation.
#[derive(Clone, Copy)]
pub struct EncodedMessage {
    bytes: [u8; layout::MAX_MESSAGE_LEN],
    len: u16,
}

impl EncodedMessage {
    /// The canonical bytes.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        self.bytes.get(..self.len()).unwrap_or(&[])
    }

    #[must_use]
    pub fn len(&self) -> usize {
        usize::from(self.len)
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Keccak-256 over the domain tag and the whole message.
    #[must_use]
    pub fn message_id(&self) -> MessageId {
        message_id(self.as_bytes())
    }

    #[cfg(feature = "alloc")]
    #[must_use]
    pub fn to_vec(&self) -> Vec<u8> {
        self.as_bytes().to_vec()
    }
}

impl core::fmt::Debug for EncodedMessage {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("EncodedMessage")
            .field("len", &self.len())
            .finish_non_exhaustive()
    }
}

impl PartialEq for EncodedMessage {
    fn eq(&self, other: &Self) -> bool {
        self.as_bytes() == other.as_bytes()
    }
}

impl Eq for EncodedMessage {}

impl Message {
    /// Validates the message, then writes its canonical bytes.
    pub fn encode(&self) -> Result<EncodedMessage, EncodeError> {
        let mut buffer = EncodedMessage {
            bytes: [0u8; layout::MAX_MESSAGE_LEN],
            len: 0,
        };
        let written = encode_into(self, &mut buffer.bytes)?;
        buffer.len = u16::try_from(written).unwrap_or(u16::MAX);
        Ok(buffer)
    }

    /// Keccak-256 of the encoded body.
    pub fn body_hash(&self) -> Result<BodyHash, EncodeError> {
        let mut body = [0u8; layout::MAX_BODY_LEN];
        let width = self.body.encoded_len();
        let slot = body.get_mut(..width).ok_or(EncodeError::BufferTooSmall {
            needed: saturating_u32(width),
            available: saturating_u32(layout::MAX_BODY_LEN),
        })?;
        self.body.encode_into(slot)?;
        Ok(BodyHash::new(keccak256(slot)))
    }

    /// Canonical identifier of the whole message.
    pub fn message_id(&self) -> Result<MessageId, EncodeError> {
        Ok(self.encode()?.message_id())
    }
}

/// Writes the canonical bytes of a valid message and returns the width used.
///
/// The message is validated first, so an invalid message never reaches bytes.
pub fn encode_into(message: &Message, out: &mut [u8]) -> Result<usize, EncodeError> {
    validation::validate_supported_versions_for_encode(&message.header)?;
    validation::validate_header(&message.header)?;
    message.body.validate(message.header.published_at)?;

    let width = message.encoded_len();
    let available = saturating_u32(out.len());
    let frame = out.get_mut(..width).ok_or(EncodeError::BufferTooSmall {
        needed: saturating_u32(width),
        available,
    })?;

    let (head, body) = frame.split_at_mut(layout::HEADER_LEN);
    message.body.encode_into(body)?;
    let body_hash = BodyHash::new(keccak256(body));
    write_header(head, &message.header, message.message_type(), body_hash)?;
    Ok(width)
}

/// Encodes into an owned buffer.
#[cfg(feature = "alloc")]
pub fn encode_message(message: &Message) -> Result<Vec<u8>, EncodeError> {
    Ok(message.encode()?.to_vec())
}

/// Parses and fully validates one canonical message.
///
/// Nothing is returned unless every structural and field rule holds.
pub fn decode_message(bytes: &[u8]) -> Result<Message, DecodeError> {
    let frame = validation::inspect_frame(bytes)?;
    let body_bytes = window(bytes, layout::HEADER_LEN, frame.body_len)?;

    let declared = BodyHash::new(read_array(bytes, layout::BODY_HASH_OFFSET)?);
    if declared != BodyHash::new(keccak256(body_bytes)) {
        return Err(DecodeError::BodyHashMismatch);
    }

    let header = read_header(bytes)?;
    let body = Body::decode(frame.message_type, body_bytes)?;

    validation::validate_header(&header)?;
    body.validate(header.published_at)?;
    Ok(Message { header, body })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integers_are_read_back_in_big_endian_order() {
        let bytes = [0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08];
        assert_eq!(read_u16(&bytes, 0), Ok(0x0102));
        assert_eq!(read_u32(&bytes, 0), Ok(0x0102_0304));
        assert_eq!(read_u64(&bytes, 0), Ok(0x0102_0304_0506_0708));
        assert_eq!(read_u8(&bytes, 1), Ok(0x02));
    }

    #[test]
    fn a_read_past_the_end_reports_how_much_was_needed() {
        let bytes = [0u8; 3];
        assert_eq!(
            read_u32(&bytes, 0),
            Err(DecodeError::Truncated {
                needed: 4,
                found: 3
            })
        );
    }

    #[test]
    fn a_write_past_the_end_reports_the_buffer_size() {
        let mut bytes = [0u8; 3];
        assert_eq!(
            write_u32(&mut bytes, 0, 1),
            Err(EncodeError::BufferTooSmall {
                needed: 4,
                available: 3
            })
        );
    }

    #[test]
    fn an_offset_near_the_address_limit_reports_overflow() {
        let bytes = [0u8; 8];
        assert_eq!(
            read_u32(&bytes, usize::MAX),
            Err(DecodeError::LengthOverflow)
        );
    }

    #[test]
    fn reserved_bytes_are_clear_only_when_every_byte_is_zero() {
        let bytes = [0u8, 0, 1];
        assert_eq!(reserved_is_clear(&bytes, 0, 2), Ok(true));
        assert_eq!(reserved_is_clear(&bytes, 0, 3), Ok(false));
    }

    #[test]
    fn a_length_that_does_not_fit_the_error_field_is_reported_as_the_maximum() {
        assert_eq!(saturating_u32(usize::MAX), u32::MAX);
        assert_eq!(saturating_u32(7), 7);
    }
}
