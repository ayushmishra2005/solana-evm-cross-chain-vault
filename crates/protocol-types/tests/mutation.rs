//! Field by field mutation of every message type.
//!
//! A mutated message must either reject or decode to a different message with
//! a different id. It must never decode back to the original.

#![allow(clippy::unwrap_used)]

mod common;

use common::{message, resealed};
use protocol_types::layout::{self, HEADER_LEN};
use protocol_types::{Commitment, MessageType, decode_message, next_commitment};

/// One named byte range of the wire format.
struct Region {
    name: &'static str,
    offset: usize,
    width: usize,
}

const fn region(name: &'static str, offset: usize, width: usize) -> Region {
    Region {
        name,
        offset,
        width,
    }
}

const HEADER_REGIONS: &[Region] = &[
    region("magic", layout::MAGIC_OFFSET, layout::MAGIC_LEN),
    region("protocol version", layout::PROTOCOL_VERSION_OFFSET, 2),
    region("schema version", layout::SCHEMA_VERSION_OFFSET, 2),
    region("message type", layout::MESSAGE_TYPE_OFFSET, 1),
    region("flags", layout::FLAGS_OFFSET, 2),
    region("source chain", layout::SOURCE_CHAIN_OFFSET, 4),
    region("destination chain", layout::DESTINATION_CHAIN_OFFSET, 4),
    region("source application", layout::SOURCE_APPLICATION_OFFSET, 32),
    region(
        "destination application",
        layout::DESTINATION_APPLICATION_OFFSET,
        32,
    ),
    region("deployment id", layout::DEPLOYMENT_ID_OFFSET, 32),
    region("vault id", layout::VAULT_ID_OFFSET, 32),
    region("lane id", layout::LANE_ID_OFFSET, 4),
    region("sequence", layout::SEQUENCE_OFFSET, 8),
    region(
        "previous commitment",
        layout::PREVIOUS_COMMITMENT_OFFSET,
        32,
    ),
    region("observed at", layout::OBSERVED_AT_OFFSET, 8),
    region("published at", layout::PUBLISHED_AT_OFFSET, 8),
    region("expires at", layout::EXPIRES_AT_OFFSET, 8),
    region("body hash", layout::BODY_HASH_OFFSET, 32),
];

const ALLOCATE_REGIONS: &[Region] = &[
    region("transfer id", layout::ALLOCATE_TRANSFER_ID_OFFSET, 32),
    region("amount", layout::ALLOCATE_AMOUNT_OFFSET, 16),
    region(
        "expected source balance",
        layout::ALLOCATE_EXPECTED_SOURCE_BALANCE_OFFSET,
        16,
    ),
    region(
        "minimum destination amount",
        layout::ALLOCATE_MINIMUM_DESTINATION_AMOUNT_OFFSET,
        16,
    ),
    region("deadline", layout::ALLOCATE_DEADLINE_OFFSET, 8),
    region("config version", layout::ALLOCATE_CONFIG_VERSION_OFFSET, 8),
];

const RECALL_REGIONS: &[Region] = &[
    region("transfer id", layout::RECALL_TRANSFER_ID_OFFSET, 32),
    region(
        "requested amount",
        layout::RECALL_REQUESTED_AMOUNT_OFFSET,
        16,
    ),
    region(
        "minimum return amount",
        layout::RECALL_MINIMUM_RETURN_AMOUNT_OFFSET,
        16,
    ),
    region("deadline", layout::RECALL_DEADLINE_OFFSET, 8),
    region("config version", layout::RECALL_CONFIG_VERSION_OFFSET, 8),
];

const REMOTE_REPORT_REGIONS: &[Region] = &[
    region("epoch id", layout::REPORT_EPOCH_ID_OFFSET, 8),
    region(
        "remote principal",
        layout::REPORT_REMOTE_PRINCIPAL_OFFSET,
        16,
    ),
    region("reported value", layout::REPORT_REPORTED_VALUE_OFFSET, 16),
    region("realized loss", layout::REPORT_REALIZED_LOSS_OFFSET, 16),
    region(
        "unattributed balance",
        layout::REPORT_UNATTRIBUTED_BALANCE_OFFSET,
        16,
    ),
    region(
        "latest completed transfer id",
        layout::REPORT_LATEST_COMPLETED_TRANSFER_ID_OFFSET,
        32,
    ),
    region("probe status", layout::REPORT_PROBE_STATUS_OFFSET, 1),
    region("probe timestamp", layout::REPORT_PROBE_TIMESTAMP_OFFSET, 8),
    region("config version", layout::REPORT_CONFIG_VERSION_OFFSET, 8),
    region(
        "remote state commitment",
        layout::REPORT_REMOTE_STATE_COMMITMENT_OFFSET,
        32,
    ),
];

const RECALL_SENT_REGIONS: &[Region] = &[
    region("transfer id", layout::RECALL_SENT_TRANSFER_ID_OFFSET, 32),
    region(
        "principal sent",
        layout::RECALL_SENT_PRINCIPAL_SENT_OFFSET,
        16,
    ),
    region(
        "actual amount sent",
        layout::RECALL_SENT_ACTUAL_AMOUNT_SENT_OFFSET,
        16,
    ),
    region(
        "realized loss",
        layout::RECALL_SENT_REALIZED_LOSS_OFFSET,
        16,
    ),
    region(
        "destination reference",
        layout::RECALL_SENT_DESTINATION_REFERENCE_OFFSET,
        32,
    ),
    region(
        "sent timestamp",
        layout::RECALL_SENT_SENT_TIMESTAMP_OFFSET,
        8,
    ),
    region(
        "config version",
        layout::RECALL_SENT_CONFIG_VERSION_OFFSET,
        8,
    ),
];

const CONFIG_UPDATE_REGIONS: &[Region] = &[
    region("config version", layout::CONFIG_VERSION_OFFSET, 8),
    region(
        "previous config version",
        layout::CONFIG_PREVIOUS_VERSION_OFFSET,
        8,
    ),
    region(
        "max remote allocation bps",
        layout::CONFIG_MAX_REMOTE_ALLOCATION_BPS_OFFSET,
        2,
    ),
    region(
        "max upward deviation bps",
        layout::CONFIG_MAX_UPWARD_DEVIATION_BPS_OFFSET,
        2,
    ),
    region(
        "max downward deviation bps",
        layout::CONFIG_MAX_DOWNWARD_DEVIATION_BPS_OFFSET,
        2,
    ),
    region("max report age", layout::CONFIG_MAX_REPORT_AGE_OFFSET, 8),
    region(
        "effective timestamp",
        layout::CONFIG_EFFECTIVE_TIMESTAMP_OFFSET,
        8,
    ),
    region("config commitment", layout::CONFIG_COMMITMENT_OFFSET, 32),
];

fn body_regions(kind: MessageType) -> &'static [Region] {
    match kind {
        MessageType::Allocate => ALLOCATE_REGIONS,
        MessageType::Recall => RECALL_REGIONS,
        MessageType::RemoteReport => REMOTE_REPORT_REGIONS,
        MessageType::RecallSent => RECALL_SENT_REGIONS,
        MessageType::ConfigUpdate => CONFIG_UPDATE_REGIONS,
    }
}

/// Flips the low bit of the first and of the last byte of a range.
fn flipped(bytes: &[u8], offset: usize, width: usize) -> Vec<Vec<u8>> {
    let last = offset.saturating_add(width).saturating_sub(1);
    [offset, last]
        .into_iter()
        .map(|position| {
            let mut edited = bytes.to_vec();
            *edited.get_mut(position).unwrap() ^= 1;
            edited
        })
        .collect()
}

/// Asserts a changed byte string never decodes back to the original message.
fn assert_diverges(original: &[u8], edited: &[u8], label: &str) {
    assert_ne!(original, edited, "{label} did not change the bytes");

    let Ok(decoded) = decode_message(edited) else {
        return;
    };
    let source = decode_message(original).unwrap();
    assert_ne!(decoded, source, "{label} decoded back to the original");

    let changed_id = decoded.message_id().unwrap();
    let original_id = source.message_id().unwrap();
    assert_ne!(changed_id, original_id, "{label} kept the original id");

    let previous = Commitment::new([0x5A; 32]);
    assert_ne!(
        next_commitment(previous, changed_id),
        next_commitment(previous, original_id),
        "{label} kept the original chain link"
    );
}

#[test]
fn flipping_a_bit_in_any_header_field_never_yields_the_original_message() {
    for kind in MessageType::ALL {
        let bytes = common::encoded(kind);
        for field in HEADER_REGIONS {
            for edited in flipped(&bytes, field.offset, field.width) {
                assert_diverges(&bytes, &edited, &format!("{kind:?} header {}", field.name));
            }
        }
    }
}

#[test]
fn flipping_a_bit_in_any_body_field_breaks_the_body_hash() {
    for kind in MessageType::ALL {
        let bytes = common::encoded(kind);
        for field in body_regions(kind) {
            for edited in flipped(&bytes, HEADER_LEN + field.offset, field.width) {
                assert_eq!(
                    decode_message(&edited),
                    Err(protocol_types::DecodeError::BodyHashMismatch),
                    "{kind:?} body {}",
                    field.name
                );
            }
        }
    }
}

#[test]
fn resealing_a_flipped_body_field_never_yields_the_original_message() {
    for kind in MessageType::ALL {
        let bytes = common::encoded(kind);
        for field in body_regions(kind) {
            for edited in flipped(&bytes, HEADER_LEN + field.offset, field.width) {
                let sealed = resealed(&edited);
                assert_diverges(&bytes, &sealed, &format!("{kind:?} body {}", field.name));
            }
        }
    }
}

#[test]
fn every_body_region_of_every_type_is_covered_by_the_mutation_lists() {
    for kind in MessageType::ALL {
        let regions = body_regions(kind);
        let covered: usize = regions.iter().map(|field| field.width).sum();
        assert_eq!(covered, kind.body_len(), "{kind:?}");

        let mut expected = 0;
        for field in regions {
            assert_eq!(field.offset, expected, "{kind:?} {}", field.name);
            expected = expected.saturating_add(field.width);
        }
    }
}

#[test]
fn the_header_mutation_list_covers_the_whole_header() {
    let covered: usize = HEADER_REGIONS.iter().map(|field| field.width).sum();
    assert_eq!(covered, HEADER_LEN);
}

#[test]
fn flipping_the_first_byte_rejects_on_the_magic() {
    for kind in MessageType::ALL {
        let mut bytes = common::encoded(kind);
        *bytes.first_mut().unwrap() ^= 1;
        assert_eq!(
            decode_message(&bytes),
            Err(protocol_types::DecodeError::InvalidMagic),
            "{kind:?}"
        );
    }
}

#[test]
fn flipping_the_final_byte_rejects_on_the_body_hash() {
    for kind in MessageType::ALL {
        let mut bytes = common::encoded(kind);
        *bytes.last_mut().unwrap() ^= 1;
        assert_eq!(
            decode_message(&bytes),
            Err(protocol_types::DecodeError::BodyHashMismatch),
            "{kind:?}"
        );
    }
}

#[test]
fn setting_any_single_flag_bit_rejects() {
    for shift in 0..16u32 {
        let mut bytes = common::encoded(MessageType::Allocate);
        let value: u16 = 1 << shift;
        bytes
            .get_mut(layout::FLAGS_OFFSET..layout::FLAGS_OFFSET + 2)
            .unwrap()
            .copy_from_slice(&value.to_be_bytes());
        assert_eq!(
            decode_message(&bytes),
            Err(protocol_types::DecodeError::Invalid(
                protocol_types::ValidationError::ReservedFlagsSet
            )),
            "bit {shift}"
        );
    }
}

#[test]
fn swapping_the_message_type_to_every_other_type_rejects_on_total_length() {
    for kind in MessageType::ALL {
        let bytes = common::encoded(kind);
        for other in MessageType::ALL {
            if other == kind {
                continue;
            }
            let mut edited = bytes.clone();
            *edited.get_mut(layout::MESSAGE_TYPE_OFFSET).unwrap() = other.to_u8();

            let wanted = u32::try_from(other.message_len()).unwrap();
            let present = u32::try_from(kind.message_len()).unwrap();
            let expected = if wanted > present {
                protocol_types::DecodeError::Truncated {
                    needed: wanted,
                    found: present,
                }
            } else {
                protocol_types::DecodeError::TrailingBytes {
                    expected: wanted,
                    found: present,
                }
            };
            assert_eq!(
                decode_message(&edited),
                Err(expected),
                "{kind:?} read as {other:?}"
            );
        }
    }
}

#[test]
fn every_unknown_message_type_byte_rejects() {
    let bytes = common::encoded(MessageType::Allocate);
    for value in 0..=u8::MAX {
        if (1..=5).contains(&value) {
            continue;
        }
        let mut edited = bytes.clone();
        *edited.get_mut(layout::MESSAGE_TYPE_OFFSET).unwrap() = value;
        assert_eq!(
            decode_message(&edited),
            Err(protocol_types::DecodeError::UnknownMessageType(value))
        );
    }
}

#[test]
fn every_unknown_probe_status_byte_rejects() {
    let bytes = common::encoded(MessageType::RemoteReport);
    let position = HEADER_LEN + layout::REPORT_PROBE_STATUS_OFFSET;
    for value in 4..=u8::MAX {
        let mut edited = bytes.clone();
        *edited.get_mut(position).unwrap() = value;
        let sealed = resealed(&edited);
        assert_eq!(
            decode_message(&sealed),
            Err(protocol_types::DecodeError::InvalidProbeStatus(value))
        );
    }
}

#[test]
fn a_byte_flip_across_two_newly_adjacent_fields_is_still_detected() {
    // Each boundary is where the padding used to sit, or where a width shrank.
    let boundaries = [
        (
            MessageType::RemoteReport,
            HEADER_LEN + layout::REPORT_PROBE_STATUS_OFFSET + 1,
            HEADER_LEN + layout::REPORT_PROBE_TIMESTAMP_OFFSET,
        ),
        (
            MessageType::ConfigUpdate,
            HEADER_LEN + layout::CONFIG_MAX_DOWNWARD_DEVIATION_BPS_OFFSET + 2,
            HEADER_LEN + layout::CONFIG_MAX_REPORT_AGE_OFFSET,
        ),
        (
            MessageType::Allocate,
            layout::MESSAGE_TYPE_OFFSET + 1,
            layout::FLAGS_OFFSET,
        ),
    ];

    for (kind, end_of_left, start_of_right) in boundaries {
        assert_eq!(end_of_left, start_of_right, "{kind:?} has a gap");
        let bytes = common::encoded(kind);
        for position in [start_of_right - 1, start_of_right] {
            let mut edited = bytes.clone();
            *edited.get_mut(position).unwrap() ^= 1;
            let sealed = if position >= HEADER_LEN {
                resealed(&edited)
            } else {
                edited
            };
            assert_diverges(&bytes, &sealed, &format!("{kind:?} byte {position}"));
        }
    }
}

#[test]
fn every_byte_position_survives_a_full_value_sweep_without_panicking() {
    let bytes = common::encoded(MessageType::ConfigUpdate);
    for position in 0..bytes.len() {
        for value in [0u8, 1, 0x7F, 0x80, 0xFF] {
            let mut edited = bytes.clone();
            let slot = edited.get_mut(position).unwrap();
            if *slot == value {
                continue;
            }
            *slot = value;
            let outcome = decode_message(&edited);
            if let Ok(decoded) = outcome {
                assert_ne!(decoded, message(MessageType::ConfigUpdate));
            }
        }
    }
}
