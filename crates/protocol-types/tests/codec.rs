//! Round trips, exact sizes and every documented rejection.

#![allow(clippy::unwrap_used)]

mod common;

use common::{header, message, resealed};
use protocol_types::layout::{
    ALLOCATE_MESSAGE_LEN, BODY_HASH_OFFSET, BODY_LENGTH_OFFSET, CONFIG_UPDATE_MESSAGE_LEN,
    DEPLOYMENT_ID_OFFSET, DESTINATION_APPLICATION_OFFSET, FLAGS_OFFSET, HEADER_LEN, MAGIC_OFFSET,
    MAX_MESSAGE_LEN, MESSAGE_TYPE_OFFSET, PROTOCOL_VERSION_OFFSET, RECALL_MESSAGE_LEN,
    RECALL_SENT_MESSAGE_LEN, REMOTE_REPORT_MESSAGE_LEN, SCHEMA_VERSION_OFFSET,
    SOURCE_APPLICATION_OFFSET,
};
use protocol_types::{
    AllocateBody, ApplicationId, AssetAmount, AssetId, Body, ChainId, Commitment, DecodeError,
    DeploymentId, EncodeError, Flags, IdentifierField, LaneId, Message, MessageType,
    ProtocolVersion, SchemaVersion, Sequence, Timestamp, TransferId, ValidationError, VaultId,
    decode_message, encode_into, encode_message,
};

fn patch(kind: MessageType, offset: usize, value: &[u8]) -> Vec<u8> {
    let mut bytes = common::encoded(kind);
    bytes
        .get_mut(offset..offset + value.len())
        .unwrap()
        .copy_from_slice(value);
    bytes
}

// Round trips and sizes

#[test]
fn every_message_type_survives_the_round_trip() {
    for kind in MessageType::ALL {
        let original = message(kind);
        let bytes = encode_message(&original).unwrap();
        assert_eq!(decode_message(&bytes), Ok(original), "{kind:?}");
    }
}

#[test]
fn every_message_type_has_its_declared_encoded_length() {
    let expected = [
        (MessageType::Allocate, ALLOCATE_MESSAGE_LEN, 380),
        (MessageType::Recall, RECALL_MESSAGE_LEN, 364),
        (MessageType::RemoteReport, REMOTE_REPORT_MESSAGE_LEN, 476),
        (MessageType::RecallSent, RECALL_SENT_MESSAGE_LEN, 412),
        (MessageType::ConfigUpdate, CONFIG_UPDATE_MESSAGE_LEN, 324),
    ];
    for (kind, declared, literal) in expected {
        assert_eq!(declared, literal, "{kind:?}");
        assert_eq!(common::encoded(kind).len(), literal, "{kind:?}");
        assert_eq!(message(kind).encoded_len(), literal, "{kind:?}");
    }
}

#[test]
fn decoding_then_encoding_returns_the_same_bytes() {
    for kind in MessageType::ALL {
        let bytes = common::encoded(kind);
        let decoded = decode_message(&bytes).unwrap();
        assert_eq!(encode_message(&decoded).unwrap(), bytes, "{kind:?}");
    }
}

#[test]
fn encoding_the_same_message_twice_gives_the_same_bytes() {
    for kind in MessageType::ALL {
        assert_eq!(common::encoded(kind), common::encoded(kind), "{kind:?}");
    }
}

#[test]
fn encoding_into_a_slice_writes_the_same_bytes_as_the_owned_helper() {
    for kind in MessageType::ALL {
        let mut buffer = [0u8; MAX_MESSAGE_LEN];
        let written = encode_into(&message(kind), &mut buffer).unwrap();
        assert_eq!(written, kind.message_len());
        assert_eq!(buffer.get(..written).unwrap(), common::encoded(kind));
    }
}

#[test]
fn encoding_into_a_short_slice_reports_the_needed_width() {
    let mut buffer = [0u8; 8];
    assert_eq!(
        encode_into(&message(MessageType::Allocate), &mut buffer),
        Err(EncodeError::BufferTooSmall {
            needed: 380,
            available: 8
        })
    );
}

// Wire layout

#[test]
fn the_magic_prefix_is_the_first_four_bytes() {
    let bytes = common::encoded(MessageType::Allocate);
    assert_eq!(bytes.get(0..4), Some(&b"SVE1"[..]));
}

#[test]
fn the_versions_and_type_are_big_endian_at_their_offsets() {
    let bytes = common::encoded(MessageType::RemoteReport);
    assert_eq!(
        bytes.get(PROTOCOL_VERSION_OFFSET..SCHEMA_VERSION_OFFSET),
        Some(&[0u8, 1][..])
    );
    assert_eq!(
        bytes.get(SCHEMA_VERSION_OFFSET..MESSAGE_TYPE_OFFSET),
        Some(&[0u8, 1][..])
    );
    assert_eq!(
        bytes.get(MESSAGE_TYPE_OFFSET..FLAGS_OFFSET),
        Some(&[0u8, 3][..])
    );
}

#[test]
fn the_declared_body_length_is_big_endian_and_matches_the_type() {
    for kind in MessageType::ALL {
        let bytes = common::encoded(kind);
        let declared = bytes
            .get(BODY_LENGTH_OFFSET..BODY_LENGTH_OFFSET + 4)
            .unwrap();
        let expected = u32::try_from(kind.body_len()).unwrap().to_be_bytes();
        assert_eq!(declared, expected, "{kind:?}");
    }
}

#[test]
fn an_evm_source_application_is_left_padded_on_the_wire() {
    let bytes = common::encoded(MessageType::Allocate);
    let field = bytes
        .get(SOURCE_APPLICATION_OFFSET..SOURCE_APPLICATION_OFFSET + 32)
        .unwrap();
    assert_eq!(field.get(..12), Some(&[0u8; 12][..]));
    assert_eq!(field.get(12..), Some(&common::EVM_APPLICATION[..]));
}

#[test]
fn a_solana_destination_application_keeps_all_thirty_two_bytes() {
    let bytes = common::encoded(MessageType::Allocate);
    let field = bytes
        .get(DESTINATION_APPLICATION_OFFSET..DESTINATION_APPLICATION_OFFSET + 32)
        .unwrap();
    assert_eq!(field, common::SOLANA_APPLICATION);
}

#[test]
fn an_evm_application_round_trips_through_a_decoded_header() {
    let decoded = decode_message(&common::encoded(MessageType::Allocate)).unwrap();
    assert_eq!(
        decoded.header.source_application.evm_address(),
        Some(common::EVM_APPLICATION)
    );
    assert_eq!(
        decoded.header.destination_application.to_solana_pubkey(),
        common::SOLANA_APPLICATION
    );
    assert_eq!(decoded.header.destination_application.evm_address(), None);
}

// Structural rejections

#[test]
fn an_empty_input_rejects_as_truncated() {
    assert_eq!(
        decode_message(&[]),
        Err(DecodeError::Truncated {
            needed: 252,
            found: 0
        })
    );
}

#[test]
fn a_truncated_header_rejects() {
    let bytes = common::encoded(MessageType::Allocate);
    let short = bytes.get(..HEADER_LEN - 1).unwrap();
    assert_eq!(
        decode_message(short),
        Err(DecodeError::Truncated {
            needed: 252,
            found: 251
        })
    );
}

#[test]
fn a_truncated_body_rejects() {
    let bytes = common::encoded(MessageType::Allocate);
    let short = bytes.get(..bytes.len() - 1).unwrap();
    assert_eq!(
        decode_message(short),
        Err(DecodeError::Truncated {
            needed: 380,
            found: 379
        })
    );
}

#[test]
fn a_valid_message_with_one_extra_byte_rejects() {
    let mut bytes = common::encoded(MessageType::Allocate);
    bytes.push(0);
    assert_eq!(
        decode_message(&bytes),
        Err(DecodeError::TrailingBytes {
            expected: 380,
            found: 381
        })
    );
}

#[test]
fn every_prefix_of_a_valid_message_rejects() {
    let bytes = common::encoded(MessageType::ConfigUpdate);
    for cut in 0..bytes.len() {
        let prefix = bytes.get(..cut).unwrap();
        assert!(
            decode_message(prefix).is_err(),
            "prefix of {cut} bytes decoded"
        );
    }
}

#[test]
fn a_wrong_magic_rejects() {
    let bytes = patch(MessageType::Allocate, MAGIC_OFFSET, b"SVE2");
    assert_eq!(decode_message(&bytes), Err(DecodeError::InvalidMagic));
}

#[test]
fn an_unsupported_protocol_version_rejects() {
    for version in [0u16, 2, u16::MAX] {
        let bytes = patch(
            MessageType::Allocate,
            PROTOCOL_VERSION_OFFSET,
            &version.to_be_bytes(),
        );
        assert_eq!(
            decode_message(&bytes),
            Err(DecodeError::UnsupportedProtocolVersion(version))
        );
    }
}

#[test]
fn an_unsupported_schema_version_rejects() {
    for version in [0u16, 2, u16::MAX] {
        let bytes = patch(
            MessageType::Allocate,
            SCHEMA_VERSION_OFFSET,
            &version.to_be_bytes(),
        );
        assert_eq!(
            decode_message(&bytes),
            Err(DecodeError::UnsupportedSchemaVersion(version))
        );
    }
}

#[test]
fn an_unknown_message_type_rejects() {
    for kind in [0u16, 6, u16::MAX] {
        let bytes = patch(
            MessageType::Allocate,
            MESSAGE_TYPE_OFFSET,
            &kind.to_be_bytes(),
        );
        assert_eq!(
            decode_message(&bytes),
            Err(DecodeError::UnknownMessageType(kind))
        );
    }
}

#[test]
fn a_known_type_with_the_wrong_body_length_rejects() {
    let bytes = patch(
        MessageType::Allocate,
        MESSAGE_TYPE_OFFSET,
        &2u16.to_be_bytes(),
    );
    assert_eq!(
        decode_message(&bytes),
        Err(DecodeError::BodyLengthMismatch {
            expected: 112,
            found: 128
        })
    );
}

#[test]
fn a_tampered_body_length_rejects() {
    let bytes = patch(
        MessageType::Allocate,
        BODY_LENGTH_OFFSET,
        &99u32.to_be_bytes(),
    );
    assert_eq!(
        decode_message(&bytes),
        Err(DecodeError::BodyLengthMismatch {
            expected: 128,
            found: 99
        })
    );
}

#[test]
fn a_corrupted_body_hash_rejects() {
    let mut bytes = common::encoded(MessageType::Allocate);
    *bytes.get_mut(BODY_HASH_OFFSET).unwrap() ^= 1;
    assert_eq!(decode_message(&bytes), Err(DecodeError::BodyHashMismatch));
}

#[test]
fn a_body_edit_without_a_new_hash_rejects() {
    let mut bytes = common::encoded(MessageType::Allocate);
    *bytes.get_mut(HEADER_LEN).unwrap() ^= 1;
    assert_eq!(decode_message(&bytes), Err(DecodeError::BodyHashMismatch));
}

#[test]
fn a_body_edit_with_a_new_hash_decodes_to_a_different_message() {
    let bytes = common::encoded(MessageType::Allocate);
    let mut edited = bytes.clone();
    *edited.get_mut(HEADER_LEN).unwrap() ^= 1;
    let sealed = resealed(&edited);
    let decoded = decode_message(&sealed).unwrap();
    assert_ne!(decoded, message(MessageType::Allocate));
    assert_ne!(sealed, bytes);
}

#[test]
fn a_message_type_swap_with_a_resealed_body_still_rejects_on_length() {
    let mut bytes = common::encoded(MessageType::RemoteReport);
    bytes
        .get_mut(MESSAGE_TYPE_OFFSET..MESSAGE_TYPE_OFFSET + 2)
        .unwrap()
        .copy_from_slice(&1u16.to_be_bytes());
    assert_eq!(
        decode_message(&resealed(&bytes)),
        Err(DecodeError::BodyLengthMismatch {
            expected: 128,
            found: 224
        })
    );
}

#[test]
fn arbitrary_bytes_of_header_length_reject_without_panicking() {
    for filler in [0u8, 1, 0x55, 0xAA, 0xFF] {
        let bytes = vec![filler; HEADER_LEN];
        assert!(decode_message(&bytes).is_err());
    }
}

// Header field rules

fn expect_header_rejection(header: protocol_types::Header, expected: ValidationError) {
    let broken = Message {
        header,
        body: message(MessageType::Allocate).body,
    };
    assert_eq!(encode_message(&broken), Err(EncodeError::Invalid(expected)));
}

#[test]
fn a_reserved_flag_bit_rejects() {
    expect_header_rejection(
        protocol_types::Header {
            flags: Flags::new(1),
            ..header()
        },
        ValidationError::ReservedFlagsSet,
    );
}

#[test]
fn a_zero_source_chain_rejects() {
    expect_header_rejection(
        protocol_types::Header {
            source_chain: ChainId::ZERO,
            ..header()
        },
        ValidationError::ZeroIdentifier(IdentifierField::SourceChain),
    );
}

#[test]
fn a_zero_destination_chain_rejects() {
    expect_header_rejection(
        protocol_types::Header {
            destination_chain: ChainId::ZERO,
            ..header()
        },
        ValidationError::ZeroIdentifier(IdentifierField::DestinationChain),
    );
}

#[test]
fn equal_source_and_destination_chains_reject() {
    expect_header_rejection(
        protocol_types::Header {
            destination_chain: ChainId::new(1),
            ..header()
        },
        ValidationError::SourceEqualsDestinationChain,
    );
}

#[test]
fn a_zero_source_application_rejects() {
    expect_header_rejection(
        protocol_types::Header {
            source_application: ApplicationId::ZERO,
            ..header()
        },
        ValidationError::ZeroIdentifier(IdentifierField::SourceApplication),
    );
}

#[test]
fn a_zero_destination_application_rejects() {
    expect_header_rejection(
        protocol_types::Header {
            destination_application: ApplicationId::ZERO,
            ..header()
        },
        ValidationError::ZeroIdentifier(IdentifierField::DestinationApplication),
    );
}

#[test]
fn equal_source_and_destination_applications_reject() {
    let same = ApplicationId::from_evm_address(common::EVM_APPLICATION);
    expect_header_rejection(
        protocol_types::Header {
            destination_application: same,
            ..header()
        },
        ValidationError::SourceEqualsDestinationApplication,
    );
}

#[test]
fn a_zero_deployment_id_rejects() {
    expect_header_rejection(
        protocol_types::Header {
            deployment_id: DeploymentId::ZERO,
            ..header()
        },
        ValidationError::ZeroIdentifier(IdentifierField::Deployment),
    );
}

#[test]
fn a_zero_vault_id_rejects() {
    expect_header_rejection(
        protocol_types::Header {
            vault_id: VaultId::ZERO,
            ..header()
        },
        ValidationError::ZeroIdentifier(IdentifierField::Vault),
    );
}

#[test]
fn a_zero_lane_id_rejects() {
    expect_header_rejection(
        protocol_types::Header {
            lane_id: LaneId::ZERO,
            ..header()
        },
        ValidationError::ZeroIdentifier(IdentifierField::Lane),
    );
}

#[test]
fn a_zero_sequence_rejects() {
    expect_header_rejection(
        protocol_types::Header {
            sequence: Sequence::ZERO,
            ..header()
        },
        ValidationError::ZeroIdentifier(IdentifierField::Sequence),
    );
}

#[test]
fn a_publication_time_before_observation_rejects() {
    expect_header_rejection(
        protocol_types::Header {
            observed_at: Timestamp::new(common::PUBLISHED_AT + 1),
            ..header()
        },
        ValidationError::PublicationBeforeObservation,
    );
}

#[test]
fn an_expiration_before_publication_rejects() {
    expect_header_rejection(
        protocol_types::Header {
            expires_at: Timestamp::new(common::PUBLISHED_AT - 1),
            ..header()
        },
        ValidationError::ExpirationBeforePublication,
    );
}

#[test]
fn a_zero_previous_commitment_is_allowed_for_the_first_message() {
    let first = Message {
        header: protocol_types::Header {
            previous_commitment: Commitment::ZERO,
            ..header()
        },
        body: message(MessageType::Allocate).body,
    };
    let bytes = encode_message(&first).unwrap();
    assert_eq!(decode_message(&bytes), Ok(first));
}

#[test]
fn timestamps_that_are_all_equal_are_allowed() {
    let flat = Timestamp::new(common::PUBLISHED_AT);
    let body = AllocateBody {
        deadline: flat,
        ..common::allocate_body()
    };
    let level = Message {
        header: protocol_types::Header {
            observed_at: flat,
            published_at: flat,
            expires_at: flat,
            ..header()
        },
        body: Body::Allocate(body),
    };
    let bytes = encode_message(&level).unwrap();
    assert_eq!(decode_message(&bytes), Ok(level));
}

// Version rules on the encoding side

#[test]
fn encoding_an_unsupported_protocol_version_rejects() {
    let broken = Message {
        header: protocol_types::Header {
            protocol_version: ProtocolVersion::new(2),
            ..header()
        },
        body: message(MessageType::Allocate).body,
    };
    assert_eq!(
        encode_message(&broken),
        Err(EncodeError::UnsupportedProtocolVersion(2))
    );
}

#[test]
fn encoding_an_unsupported_schema_version_rejects() {
    let broken = Message {
        header: protocol_types::Header {
            schema_version: SchemaVersion::new(9),
            ..header()
        },
        body: message(MessageType::Allocate).body,
    };
    assert_eq!(
        encode_message(&broken),
        Err(EncodeError::UnsupportedSchemaVersion(9))
    );
}

#[test]
fn encoding_an_invalid_body_rejects_before_any_bytes_are_produced() {
    let broken = Message {
        header: header(),
        body: Body::Allocate(AllocateBody {
            transfer_id: TransferId::ZERO,
            ..common::allocate_body()
        }),
    };
    let mut buffer = [0xEEu8; MAX_MESSAGE_LEN];
    assert_eq!(
        encode_into(&broken, &mut buffer),
        Err(EncodeError::Invalid(ValidationError::ZeroIdentifier(
            IdentifierField::Transfer
        )))
    );
    assert!(
        buffer.iter().all(|byte| *byte == 0xEE),
        "buffer was written"
    );
}

// Body field rules seen through the whole message

#[test]
fn a_zero_body_identifier_rejects_during_decoding() {
    let mut bytes = common::encoded(MessageType::Allocate);
    let start = HEADER_LEN;
    bytes.get_mut(start..start + 32).unwrap().fill(0);
    assert_eq!(
        decode_message(&resealed(&bytes)),
        Err(DecodeError::Invalid(ValidationError::ZeroIdentifier(
            IdentifierField::Transfer
        )))
    );
}

#[test]
fn a_zero_body_amount_rejects_during_decoding() {
    let mut bytes = common::encoded(MessageType::Allocate);
    let start = HEADER_LEN + protocol_types::layout::ALLOCATE_AMOUNT_OFFSET;
    bytes.get_mut(start..start + 16).unwrap().fill(0);
    assert_eq!(
        decode_message(&resealed(&bytes)),
        Err(DecodeError::Invalid(ValidationError::ZeroAmount(
            protocol_types::AmountField::Amount
        )))
    );
}

#[test]
fn basis_points_above_ten_thousand_reject_during_decoding() {
    let mut bytes = common::encoded(MessageType::ConfigUpdate);
    let start = HEADER_LEN + protocol_types::layout::CONFIG_MAX_REMOTE_ALLOCATION_BPS_OFFSET;
    bytes
        .get_mut(start..start + 2)
        .unwrap()
        .copy_from_slice(&10_001u16.to_be_bytes());
    assert_eq!(
        decode_message(&resealed(&bytes)),
        Err(DecodeError::Invalid(ValidationError::BasisPointsOutOfRange))
    );
}

#[test]
fn an_unknown_probe_status_rejects_during_decoding() {
    let mut bytes = common::encoded(MessageType::RemoteReport);
    let offset = HEADER_LEN + protocol_types::layout::REPORT_PROBE_STATUS_OFFSET;
    *bytes.get_mut(offset).unwrap() = 7;
    assert_eq!(
        decode_message(&resealed(&bytes)),
        Err(DecodeError::InvalidProbeStatus(7))
    );
}

#[test]
fn a_non_zero_reserved_body_byte_rejects_during_decoding() {
    let mut bytes = common::encoded(MessageType::RemoteReport);
    let offset = HEADER_LEN + protocol_types::layout::REPORT_RESERVED_OFFSET;
    *bytes.get_mut(offset).unwrap() = 1;
    assert_eq!(
        decode_message(&resealed(&bytes)),
        Err(DecodeError::ReservedBytesSet)
    );
}

#[test]
fn a_deadline_before_publication_rejects_during_decoding() {
    let mut bytes = common::encoded(MessageType::Allocate);
    let start = HEADER_LEN + protocol_types::layout::ALLOCATE_DEADLINE_OFFSET;
    bytes
        .get_mut(start..start + 8)
        .unwrap()
        .copy_from_slice(&(common::PUBLISHED_AT - 1).to_be_bytes());
    assert_eq!(
        decode_message(&resealed(&bytes)),
        Err(DecodeError::Invalid(
            ValidationError::DeadlineBeforePublication
        ))
    );
}

// Identifiers and commitments

#[test]
fn the_message_id_changes_when_a_header_field_changes() {
    let original = message(MessageType::Allocate);
    let moved = Message {
        header: protocol_types::Header {
            sequence: Sequence::new(43),
            ..header()
        },
        ..original
    };
    assert_ne!(original.message_id().unwrap(), moved.message_id().unwrap());
}

#[test]
fn the_message_id_changes_when_a_body_field_changes() {
    let original = message(MessageType::Allocate);
    let edited = Message {
        header: header(),
        body: Body::Allocate(AllocateBody {
            amount: AssetAmount::new(1_000_001),
            ..common::allocate_body()
        }),
    };
    assert_ne!(original.message_id().unwrap(), edited.message_id().unwrap());
}

#[test]
fn the_message_id_is_stable_across_repeated_encodings() {
    for kind in MessageType::ALL {
        let subject = message(kind);
        assert_eq!(
            subject.message_id().unwrap(),
            subject.message_id().unwrap(),
            "{kind:?}"
        );
    }
}

#[test]
fn every_message_type_has_a_distinct_message_id() {
    let mut seen = Vec::new();
    for kind in MessageType::ALL {
        let id = message(kind).message_id().unwrap();
        assert!(!seen.contains(&id), "{kind:?} repeated an id");
        seen.push(id);
    }
}

#[test]
fn the_next_commitment_changes_when_either_input_changes() {
    let first = message(MessageType::Allocate).message_id().unwrap();
    let second = message(MessageType::Recall).message_id().unwrap();
    let previous = Commitment::new([1u8; 32]);
    let other = Commitment::new([2u8; 32]);

    let base = protocol_types::next_commitment(previous, first);
    assert_ne!(base, protocol_types::next_commitment(other, first));
    assert_ne!(base, protocol_types::next_commitment(previous, second));
    assert_eq!(base, protocol_types::next_commitment(previous, first));
}

#[test]
fn the_encoded_buffer_and_the_owned_vector_share_one_message_id() {
    for kind in MessageType::ALL {
        let subject = message(kind);
        let buffer = subject.encode().unwrap();
        assert_eq!(buffer.as_bytes(), encode_message(&subject).unwrap());
        assert_eq!(buffer.message_id(), subject.message_id().unwrap());
        assert!(!buffer.is_empty());
        assert_eq!(buffer.len(), kind.message_len());
    }
}

#[test]
fn an_encoded_buffer_compares_and_prints_by_its_bytes() {
    let allocate = message(MessageType::Allocate).encode().unwrap();
    let recall = message(MessageType::Recall).encode().unwrap();
    assert_eq!(allocate, message(MessageType::Allocate).encode().unwrap());
    assert_ne!(allocate, recall);
    assert_eq!(allocate.to_vec(), allocate.as_bytes());
    assert!(format!("{allocate:?}").contains("380"));
}

#[test]
fn a_message_carries_the_type_of_its_body() {
    for kind in MessageType::ALL {
        assert_eq!(message(kind).message_type(), kind);
    }
}

#[test]
fn the_body_hash_helper_matches_the_hash_written_into_the_header() {
    for kind in MessageType::ALL {
        let bytes = common::encoded(kind);
        let written = bytes.get(BODY_HASH_OFFSET..BODY_HASH_OFFSET + 32).unwrap();
        assert_eq!(
            message(kind).body_hash().unwrap().as_bytes(),
            written,
            "{kind:?}"
        );
    }
}

#[test]
fn every_body_field_offset_lands_inside_its_message() {
    for kind in MessageType::ALL {
        assert_eq!(kind.message_len(), HEADER_LEN + kind.body_len(), "{kind:?}");
        assert!(kind.message_len() <= MAX_MESSAGE_LEN, "{kind:?}");
    }
}

#[test]
fn a_deployment_id_change_is_visible_in_the_encoded_bytes() {
    let moved = Message {
        header: protocol_types::Header {
            deployment_id: DeploymentId::new([0xD1; 32]),
            ..header()
        },
        body: message(MessageType::Allocate).body,
    };
    let bytes = encode_message(&moved).unwrap();
    assert_eq!(
        bytes.get(DEPLOYMENT_ID_OFFSET..DEPLOYMENT_ID_OFFSET + 32),
        Some(&[0xD1u8; 32][..])
    );
    assert_ne!(bytes, common::encoded(MessageType::Allocate));
}

#[test]
fn an_asset_id_change_is_visible_in_the_encoded_bytes() {
    let edited = Message {
        header: header(),
        body: Body::Allocate(AllocateBody {
            asset_id: AssetId::new([0x23; 32]),
            ..common::allocate_body()
        }),
    };
    assert_ne!(
        encode_message(&edited).unwrap(),
        common::encoded(MessageType::Allocate)
    );
}
