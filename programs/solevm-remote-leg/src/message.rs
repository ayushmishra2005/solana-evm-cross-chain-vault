//! Shared validation of inbound canonical messages.
//!
//! Decoding, hashing and field rules all come from the shared protocol crate.
//! This module only binds a decoded message to the state of this deployment.

use anchor_lang::prelude::*;
use protocol_types::{DecodeError, Header, MESSAGE_ID_DOMAIN, Message, keccak256, layout};

use crate::control::{MessageClass, ReplayLane};
use crate::errors::RemoteLegError;
use crate::state::RemoteConfig;

/// Largest canonical message the protocol defines.
pub const MAX_MESSAGE_LEN: usize = layout::MAX_MESSAGE_LEN;

/// Width of the domain tag plus the largest message.
const PREIMAGE_LEN: usize = MESSAGE_ID_DOMAIN.len() + MAX_MESSAGE_LEN;

/// A message that passed every rule, with the id of its exact bytes.
#[derive(Clone, Copy, Debug)]
pub struct ValidatedMessage {
    pub message: Message,
    pub message_id: [u8; 32],
}

/// Domain tagged id of the exact bytes that arrived.
///
/// The bytes are hashed as received, so a re-encode can never change the id.
pub fn message_id(encoded: &[u8]) -> Result<[u8; 32]> {
    let domain = MESSAGE_ID_DOMAIN;
    let mut preimage = [0u8; PREIMAGE_LEN];
    let (head, tail) = preimage.split_at_mut(domain.len());
    head.copy_from_slice(domain);
    let body = tail
        .get_mut(..encoded.len())
        .ok_or(RemoteLegError::MessageTooLarge)?;
    body.copy_from_slice(encoded);

    let width = domain
        .len()
        .checked_add(encoded.len())
        .ok_or(RemoteLegError::ArithmeticOverflow)?;
    let slot = preimage
        .get(..width)
        .ok_or(RemoteLegError::MessageTooLarge)?;
    Ok(keccak256(slot))
}

/// Next link of the lane hash chain.
#[must_use]
pub fn next_commitment(previous: &[u8; 32], message_id: &[u8; 32]) -> [u8; 32] {
    protocol_types::next_commitment(
        protocol_types::Commitment::new(*previous),
        protocol_types::MessageId::new(*message_id),
    )
    .to_bytes()
}

/// Reads the chain clock and narrows it to the protocol time type.
pub fn current_time() -> Result<u64> {
    let seconds = Clock::get()?.unix_timestamp;
    u64::try_from(seconds).map_err(|_| RemoteLegError::InvalidTimestamp.into())
}

/// Maps a shared decode failure onto one typed program error.
fn decode_error(error: DecodeError) -> RemoteLegError {
    match error {
        DecodeError::UnsupportedProtocolVersion(_) => RemoteLegError::InvalidProtocolVersion,
        DecodeError::UnsupportedSchemaVersion(_) => RemoteLegError::InvalidSchemaVersion,
        DecodeError::UnknownMessageType(_) => RemoteLegError::UnsupportedMessageType,
        _ => RemoteLegError::InvalidMessage,
    }
}

/// Decodes one canonical message after bounding its width.
pub fn decode(bytes: &[u8]) -> Result<Message> {
    require!(!bytes.is_empty(), RemoteLegError::InvalidMessage);
    require_gte!(
        MAX_MESSAGE_LEN,
        bytes.len(),
        RemoteLegError::MessageTooLarge
    );
    protocol_types::decode_message(bytes).map_err(|error| decode_error(error).into())
}

/// Binds a header to the configured domain of this deployment.
fn check_domains(header: &Header, config: &RemoteConfig, lane: &ReplayLane) -> Result<()> {
    require_eq!(
        header.source_chain.get(),
        config.source_chain_id,
        RemoteLegError::InvalidSourceDomain
    );
    require_eq!(
        header.destination_chain.get(),
        config.destination_chain_id,
        RemoteLegError::InvalidDestinationDomain
    );
    require!(
        header.source_application.as_bytes() == &config.source_application_id,
        RemoteLegError::InvalidApplication
    );
    require!(
        header.destination_application.as_bytes() == &config.local_application_id,
        RemoteLegError::InvalidApplication
    );
    require!(
        header.deployment_id.as_bytes() == &config.deployment_id,
        RemoteLegError::InvalidDeployment
    );
    require!(
        header.vault_id.as_bytes() == &config.vault_id,
        RemoteLegError::InvalidVault
    );
    require_eq!(
        header.lane_id.get(),
        lane.lane_id,
        RemoteLegError::InvalidLane
    );
    Ok(())
}

/// Compares the envelope times with the chain clock.
fn check_times(header: &Header, now: u64) -> Result<()> {
    require_gte!(header.expires_at.get(), now, RemoteLegError::MessageExpired);
    require_gte!(
        now,
        header.published_at.get(),
        RemoteLegError::InvalidTimestamp
    );
    Ok(())
}

/// Applies every rule that does not depend on the message body.
///
/// The order follows the documented validation order for inbound messages.
pub fn validate_inbound(
    bytes: &[u8],
    class: MessageClass,
    config: &RemoteConfig,
    lane: &ReplayLane,
    now: u64,
) -> Result<ValidatedMessage> {
    let message = decode(bytes)?;

    require!(
        message.message_type() == class.message_type(),
        RemoteLegError::UnsupportedMessageType
    );

    check_domains(&message.header, config, lane)?;
    check_times(&message.header, now)?;

    let sequence = message.header.sequence.get();
    require_gte!(
        sequence,
        lane.minimum_acceptable_sequence,
        RemoteLegError::SequenceBelowWatermark
    );

    class
        .sequence_rule()
        .check(sequence, lane.highest_consumed_sequence)?;

    require!(
        message.header.previous_commitment.as_bytes() == &lane.message_commitment,
        RemoteLegError::InvalidPreviousCommitment
    );

    Ok(ValidatedMessage {
        message,
        message_id: message_id(bytes)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use protocol_types::{Commitment, MessageId};

    #[test]
    fn the_message_id_matches_the_shared_implementation() {
        let body = protocol_types::ConfigUpdateBody {
            config_version: protocol_types::ConfigVersion::new(2),
            previous_config_version: protocol_types::ConfigVersion::new(1),
            max_remote_allocation_bps: protocol_types::BasisPoints::new(5_000),
            max_upward_deviation_bps: protocol_types::BasisPoints::new(100),
            max_downward_deviation_bps: protocol_types::BasisPoints::new(200),
            max_report_age: 3_600,
            effective_timestamp: protocol_types::Timestamp::new(2_000),
            config_commitment: Commitment::new([0xAB; 32]),
        };
        let message = Message {
            header: sample_header(),
            body: protocol_types::Body::ConfigUpdate(body),
        };
        let encoded = message.encode().expect("message encodes");

        assert_eq!(
            message_id(encoded.as_bytes()).expect("id is computed"),
            encoded.message_id().to_bytes()
        );
    }

    #[test]
    fn the_message_id_covers_every_byte() {
        let mut bytes = [7u8; 64];
        let first = message_id(&bytes).expect("id is computed");
        bytes[63] = 8;
        assert_ne!(first, message_id(&bytes).expect("id is computed"));
    }

    #[test]
    fn an_empty_input_still_has_a_domain_tagged_id() {
        assert_ne!(
            message_id(&[]).expect("id is computed"),
            keccak256(&[]),
            "the domain tag must be part of the hash"
        );
    }

    #[test]
    fn an_input_above_the_protocol_maximum_is_rejected() {
        let bytes = vec![0u8; MAX_MESSAGE_LEN + 1];
        let error = message_id(&bytes).expect_err("oversized input should reject");
        assert_eq!(error, Error::from(RemoteLegError::MessageTooLarge));
    }

    #[test]
    fn the_next_commitment_matches_the_shared_implementation() {
        let previous = [3u8; 32];
        let id = [5u8; 32];
        assert_eq!(
            next_commitment(&previous, &id),
            protocol_types::next_commitment(Commitment::new(previous), MessageId::new(id))
                .to_bytes()
        );
    }

    #[test]
    fn an_empty_message_is_rejected() {
        let error = decode(&[]).expect_err("empty input should reject");
        assert_eq!(error, Error::from(RemoteLegError::InvalidMessage));
    }

    #[test]
    fn a_message_above_the_protocol_maximum_is_rejected() {
        let bytes = vec![0u8; MAX_MESSAGE_LEN + 1];
        let error = decode(&bytes).expect_err("oversized input should reject");
        assert_eq!(error, Error::from(RemoteLegError::MessageTooLarge));
    }

    #[track_caller]
    fn assert_maps_to(error: DecodeError, expected: RemoteLegError) {
        assert_eq!(Error::from(decode_error(error)), Error::from(expected));
    }

    #[test]
    fn an_unsupported_protocol_version_maps_to_its_own_error() {
        assert_maps_to(
            DecodeError::UnsupportedProtocolVersion(2),
            RemoteLegError::InvalidProtocolVersion,
        );
    }

    #[test]
    fn an_unsupported_schema_version_maps_to_its_own_error() {
        assert_maps_to(
            DecodeError::UnsupportedSchemaVersion(4),
            RemoteLegError::InvalidSchemaVersion,
        );
    }

    #[test]
    fn an_unknown_message_type_maps_to_its_own_error() {
        assert_maps_to(
            DecodeError::UnknownMessageType(9),
            RemoteLegError::UnsupportedMessageType,
        );
    }

    #[test]
    fn every_other_decode_failure_maps_to_the_generic_error() {
        for error in [
            DecodeError::InvalidMagic,
            DecodeError::BodyHashMismatch,
            DecodeError::LengthOverflow,
            DecodeError::Truncated {
                needed: 247,
                found: 8,
            },
            DecodeError::TrailingBytes {
                expected: 317,
                found: 318,
            },
            DecodeError::Invalid(protocol_types::ValidationError::ReservedFlagsSet),
        ] {
            assert_maps_to(error, RemoteLegError::InvalidMessage);
        }
    }

    fn sample_header() -> Header {
        Header {
            protocol_version: protocol_types::PROTOCOL_VERSION,
            schema_version: protocol_types::SCHEMA_VERSION,
            flags: protocol_types::Flags::new(0),
            source_chain: protocol_types::ChainId::new(8453),
            destination_chain: protocol_types::ChainId::new(900),
            source_application: protocol_types::ApplicationId::new([3u8; 32]),
            destination_application: protocol_types::ApplicationId::new([4u8; 32]),
            deployment_id: protocol_types::DeploymentId::new([1u8; 32]),
            vault_id: protocol_types::VaultId::new([2u8; 32]),
            lane_id: protocol_types::LaneId::new(1),
            sequence: protocol_types::Sequence::new(1),
            previous_commitment: Commitment::ZERO,
            observed_at: protocol_types::Timestamp::new(1_000),
            published_at: protocol_types::Timestamp::new(1_100),
            expires_at: protocol_types::Timestamp::new(9_000),
        }
    }
}
