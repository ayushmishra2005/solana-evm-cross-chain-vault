//! Every byte offset and length of the wire format.
//!
//! Nothing outside this module may hardcode an offset. The assertions below
//! keep the fields contiguous and keep each declared size honest.

/// Width of a wide identifier.
pub const WIDE: usize = 32;

// Header field offsets

pub const MAGIC_OFFSET: usize = 0;
pub const MAGIC_LEN: usize = 4;

pub const PROTOCOL_VERSION_OFFSET: usize = MAGIC_OFFSET + MAGIC_LEN;
pub const SCHEMA_VERSION_OFFSET: usize = PROTOCOL_VERSION_OFFSET + 2;
pub const MESSAGE_TYPE_OFFSET: usize = SCHEMA_VERSION_OFFSET + 2;
pub const FLAGS_OFFSET: usize = MESSAGE_TYPE_OFFSET + 2;
pub const SOURCE_CHAIN_OFFSET: usize = FLAGS_OFFSET + 2;
pub const DESTINATION_CHAIN_OFFSET: usize = SOURCE_CHAIN_OFFSET + 4;
pub const SOURCE_APPLICATION_OFFSET: usize = DESTINATION_CHAIN_OFFSET + 4;
pub const DESTINATION_APPLICATION_OFFSET: usize = SOURCE_APPLICATION_OFFSET + WIDE;
pub const DEPLOYMENT_ID_OFFSET: usize = DESTINATION_APPLICATION_OFFSET + WIDE;
pub const VAULT_ID_OFFSET: usize = DEPLOYMENT_ID_OFFSET + WIDE;
pub const LANE_ID_OFFSET: usize = VAULT_ID_OFFSET + WIDE;
pub const SEQUENCE_OFFSET: usize = LANE_ID_OFFSET + 4;
pub const PREVIOUS_COMMITMENT_OFFSET: usize = SEQUENCE_OFFSET + 8;
pub const OBSERVED_AT_OFFSET: usize = PREVIOUS_COMMITMENT_OFFSET + WIDE;
pub const PUBLISHED_AT_OFFSET: usize = OBSERVED_AT_OFFSET + 8;
pub const EXPIRES_AT_OFFSET: usize = PUBLISHED_AT_OFFSET + 8;
pub const BODY_LENGTH_OFFSET: usize = EXPIRES_AT_OFFSET + 8;
pub const BODY_HASH_OFFSET: usize = BODY_LENGTH_OFFSET + 4;

/// Bytes before the body starts.
pub const HEADER_LEN: usize = BODY_HASH_OFFSET + WIDE;

// Allocate body

pub const ALLOCATE_TRANSFER_ID_OFFSET: usize = 0;
pub const ALLOCATE_ASSET_ID_OFFSET: usize = ALLOCATE_TRANSFER_ID_OFFSET + WIDE;
pub const ALLOCATE_AMOUNT_OFFSET: usize = ALLOCATE_ASSET_ID_OFFSET + WIDE;
pub const ALLOCATE_EXPECTED_SOURCE_BALANCE_OFFSET: usize = ALLOCATE_AMOUNT_OFFSET + 16;
pub const ALLOCATE_MINIMUM_DESTINATION_AMOUNT_OFFSET: usize =
    ALLOCATE_EXPECTED_SOURCE_BALANCE_OFFSET + 16;
pub const ALLOCATE_DEADLINE_OFFSET: usize = ALLOCATE_MINIMUM_DESTINATION_AMOUNT_OFFSET + 16;
pub const ALLOCATE_CONFIG_VERSION_OFFSET: usize = ALLOCATE_DEADLINE_OFFSET + 8;
pub const ALLOCATE_BODY_LEN: usize = ALLOCATE_CONFIG_VERSION_OFFSET + 8;

// Recall body

pub const RECALL_TRANSFER_ID_OFFSET: usize = 0;
pub const RECALL_ASSET_ID_OFFSET: usize = RECALL_TRANSFER_ID_OFFSET + WIDE;
pub const RECALL_REQUESTED_AMOUNT_OFFSET: usize = RECALL_ASSET_ID_OFFSET + WIDE;
pub const RECALL_MINIMUM_RETURN_AMOUNT_OFFSET: usize = RECALL_REQUESTED_AMOUNT_OFFSET + 16;
pub const RECALL_DEADLINE_OFFSET: usize = RECALL_MINIMUM_RETURN_AMOUNT_OFFSET + 16;
pub const RECALL_CONFIG_VERSION_OFFSET: usize = RECALL_DEADLINE_OFFSET + 8;
pub const RECALL_BODY_LEN: usize = RECALL_CONFIG_VERSION_OFFSET + 8;

// Remote report body

pub const REPORT_ID_OFFSET: usize = 0;
pub const REPORT_EPOCH_ID_OFFSET: usize = REPORT_ID_OFFSET + WIDE;
pub const REPORT_ASSET_ID_OFFSET: usize = REPORT_EPOCH_ID_OFFSET + 8;
pub const REPORT_REMOTE_PRINCIPAL_OFFSET: usize = REPORT_ASSET_ID_OFFSET + WIDE;
pub const REPORT_REPORTED_VALUE_OFFSET: usize = REPORT_REMOTE_PRINCIPAL_OFFSET + 16;
pub const REPORT_REALIZED_LOSS_OFFSET: usize = REPORT_REPORTED_VALUE_OFFSET + 16;
pub const REPORT_UNATTRIBUTED_BALANCE_OFFSET: usize = REPORT_REALIZED_LOSS_OFFSET + 16;
pub const REPORT_LATEST_COMPLETED_TRANSFER_ID_OFFSET: usize =
    REPORT_UNATTRIBUTED_BALANCE_OFFSET + 16;
pub const REPORT_PROBE_STATUS_OFFSET: usize = REPORT_LATEST_COMPLETED_TRANSFER_ID_OFFSET + WIDE;
/// Padding that puts the timestamps back on an eight byte boundary.
pub const REPORT_RESERVED_OFFSET: usize = REPORT_PROBE_STATUS_OFFSET + 1;
pub const REPORT_RESERVED_LEN: usize = 7;
pub const REPORT_PROBE_TIMESTAMP_OFFSET: usize = REPORT_RESERVED_OFFSET + REPORT_RESERVED_LEN;
pub const REPORT_CONFIG_VERSION_OFFSET: usize = REPORT_PROBE_TIMESTAMP_OFFSET + 8;
pub const REPORT_REMOTE_STATE_COMMITMENT_OFFSET: usize = REPORT_CONFIG_VERSION_OFFSET + 8;
pub const REMOTE_REPORT_BODY_LEN: usize = REPORT_REMOTE_STATE_COMMITMENT_OFFSET + WIDE;

// Recall sent body

pub const RECALL_SENT_TRANSFER_ID_OFFSET: usize = 0;
pub const RECALL_SENT_ASSET_ID_OFFSET: usize = RECALL_SENT_TRANSFER_ID_OFFSET + WIDE;
pub const RECALL_SENT_PRINCIPAL_SENT_OFFSET: usize = RECALL_SENT_ASSET_ID_OFFSET + WIDE;
pub const RECALL_SENT_ACTUAL_AMOUNT_SENT_OFFSET: usize = RECALL_SENT_PRINCIPAL_SENT_OFFSET + 16;
pub const RECALL_SENT_REALIZED_LOSS_OFFSET: usize = RECALL_SENT_ACTUAL_AMOUNT_SENT_OFFSET + 16;
pub const RECALL_SENT_DESTINATION_REFERENCE_OFFSET: usize = RECALL_SENT_REALIZED_LOSS_OFFSET + 16;
pub const RECALL_SENT_SENT_TIMESTAMP_OFFSET: usize =
    RECALL_SENT_DESTINATION_REFERENCE_OFFSET + WIDE;
pub const RECALL_SENT_CONFIG_VERSION_OFFSET: usize = RECALL_SENT_SENT_TIMESTAMP_OFFSET + 8;
pub const RECALL_SENT_BODY_LEN: usize = RECALL_SENT_CONFIG_VERSION_OFFSET + 8;

// Config update body

pub const CONFIG_VERSION_OFFSET: usize = 0;
pub const CONFIG_PREVIOUS_VERSION_OFFSET: usize = CONFIG_VERSION_OFFSET + 8;
pub const CONFIG_MAX_REMOTE_ALLOCATION_BPS_OFFSET: usize = CONFIG_PREVIOUS_VERSION_OFFSET + 8;
pub const CONFIG_MAX_UPWARD_DEVIATION_BPS_OFFSET: usize =
    CONFIG_MAX_REMOTE_ALLOCATION_BPS_OFFSET + 2;
pub const CONFIG_MAX_DOWNWARD_DEVIATION_BPS_OFFSET: usize =
    CONFIG_MAX_UPWARD_DEVIATION_BPS_OFFSET + 2;
/// Padding that puts max report age on an eight byte boundary.
pub const CONFIG_RESERVED_OFFSET: usize = CONFIG_MAX_DOWNWARD_DEVIATION_BPS_OFFSET + 2;
pub const CONFIG_RESERVED_LEN: usize = 2;
pub const CONFIG_MAX_REPORT_AGE_OFFSET: usize = CONFIG_RESERVED_OFFSET + CONFIG_RESERVED_LEN;
pub const CONFIG_EFFECTIVE_TIMESTAMP_OFFSET: usize = CONFIG_MAX_REPORT_AGE_OFFSET + 8;
pub const CONFIG_COMMITMENT_OFFSET: usize = CONFIG_EFFECTIVE_TIMESTAMP_OFFSET + 8;
pub const CONFIG_UPDATE_BODY_LEN: usize = CONFIG_COMMITMENT_OFFSET + WIDE;

// Whole message lengths

pub const ALLOCATE_MESSAGE_LEN: usize = HEADER_LEN + ALLOCATE_BODY_LEN;
pub const RECALL_MESSAGE_LEN: usize = HEADER_LEN + RECALL_BODY_LEN;
pub const REMOTE_REPORT_MESSAGE_LEN: usize = HEADER_LEN + REMOTE_REPORT_BODY_LEN;
pub const RECALL_SENT_MESSAGE_LEN: usize = HEADER_LEN + RECALL_SENT_BODY_LEN;
pub const CONFIG_UPDATE_MESSAGE_LEN: usize = HEADER_LEN + CONFIG_UPDATE_BODY_LEN;

/// Largest body across every message type.
pub const MAX_BODY_LEN: usize = REMOTE_REPORT_BODY_LEN;

/// Largest whole message across every message type.
pub const MAX_MESSAGE_LEN: usize = HEADER_LEN + MAX_BODY_LEN;

// Declared sizes

const _: () = assert!(HEADER_LEN == 252);
const _: () = assert!(ALLOCATE_BODY_LEN == 128);
const _: () = assert!(RECALL_BODY_LEN == 112);
const _: () = assert!(REMOTE_REPORT_BODY_LEN == 224);
const _: () = assert!(RECALL_SENT_BODY_LEN == 160);
const _: () = assert!(CONFIG_UPDATE_BODY_LEN == 72);

const _: () = assert!(ALLOCATE_MESSAGE_LEN == 380);
const _: () = assert!(RECALL_MESSAGE_LEN == 364);
const _: () = assert!(REMOTE_REPORT_MESSAGE_LEN == 476);
const _: () = assert!(RECALL_SENT_MESSAGE_LEN == 412);
const _: () = assert!(CONFIG_UPDATE_MESSAGE_LEN == 324);

// The body never outgrows the buffer the encoder reserves.

const _: () = assert!(ALLOCATE_BODY_LEN <= MAX_BODY_LEN);
const _: () = assert!(RECALL_BODY_LEN <= MAX_BODY_LEN);
const _: () = assert!(REMOTE_REPORT_BODY_LEN <= MAX_BODY_LEN);
const _: () = assert!(RECALL_SENT_BODY_LEN <= MAX_BODY_LEN);
const _: () = assert!(CONFIG_UPDATE_BODY_LEN <= MAX_BODY_LEN);
const _: () = assert!(MAX_MESSAGE_LEN == 476);

/// Every field of one region, as a start offset and a width.
type Field = (usize, usize);

/// True when the fields start at zero, touch without gaps and fill the region.
///
/// The callers below run at compile time, so a bad index fails the build.
#[allow(clippy::indexing_slicing, clippy::arithmetic_side_effects)]
const fn is_contiguous(fields: &[Field], total: usize) -> bool {
    let mut expected = 0;
    let mut index = 0;
    while index < fields.len() {
        let (start, width) = fields[index];
        if start != expected || width == 0 {
            return false;
        }
        expected += width;
        index += 1;
    }
    expected == total
}

const HEADER_FIELDS: &[Field] = &[
    (MAGIC_OFFSET, MAGIC_LEN),
    (PROTOCOL_VERSION_OFFSET, 2),
    (SCHEMA_VERSION_OFFSET, 2),
    (MESSAGE_TYPE_OFFSET, 2),
    (FLAGS_OFFSET, 2),
    (SOURCE_CHAIN_OFFSET, 4),
    (DESTINATION_CHAIN_OFFSET, 4),
    (SOURCE_APPLICATION_OFFSET, WIDE),
    (DESTINATION_APPLICATION_OFFSET, WIDE),
    (DEPLOYMENT_ID_OFFSET, WIDE),
    (VAULT_ID_OFFSET, WIDE),
    (LANE_ID_OFFSET, 4),
    (SEQUENCE_OFFSET, 8),
    (PREVIOUS_COMMITMENT_OFFSET, WIDE),
    (OBSERVED_AT_OFFSET, 8),
    (PUBLISHED_AT_OFFSET, 8),
    (EXPIRES_AT_OFFSET, 8),
    (BODY_LENGTH_OFFSET, 4),
    (BODY_HASH_OFFSET, WIDE),
];

const ALLOCATE_FIELDS: &[Field] = &[
    (ALLOCATE_TRANSFER_ID_OFFSET, WIDE),
    (ALLOCATE_ASSET_ID_OFFSET, WIDE),
    (ALLOCATE_AMOUNT_OFFSET, 16),
    (ALLOCATE_EXPECTED_SOURCE_BALANCE_OFFSET, 16),
    (ALLOCATE_MINIMUM_DESTINATION_AMOUNT_OFFSET, 16),
    (ALLOCATE_DEADLINE_OFFSET, 8),
    (ALLOCATE_CONFIG_VERSION_OFFSET, 8),
];

const RECALL_FIELDS: &[Field] = &[
    (RECALL_TRANSFER_ID_OFFSET, WIDE),
    (RECALL_ASSET_ID_OFFSET, WIDE),
    (RECALL_REQUESTED_AMOUNT_OFFSET, 16),
    (RECALL_MINIMUM_RETURN_AMOUNT_OFFSET, 16),
    (RECALL_DEADLINE_OFFSET, 8),
    (RECALL_CONFIG_VERSION_OFFSET, 8),
];

const REMOTE_REPORT_FIELDS: &[Field] = &[
    (REPORT_ID_OFFSET, WIDE),
    (REPORT_EPOCH_ID_OFFSET, 8),
    (REPORT_ASSET_ID_OFFSET, WIDE),
    (REPORT_REMOTE_PRINCIPAL_OFFSET, 16),
    (REPORT_REPORTED_VALUE_OFFSET, 16),
    (REPORT_REALIZED_LOSS_OFFSET, 16),
    (REPORT_UNATTRIBUTED_BALANCE_OFFSET, 16),
    (REPORT_LATEST_COMPLETED_TRANSFER_ID_OFFSET, WIDE),
    (REPORT_PROBE_STATUS_OFFSET, 1),
    (REPORT_RESERVED_OFFSET, REPORT_RESERVED_LEN),
    (REPORT_PROBE_TIMESTAMP_OFFSET, 8),
    (REPORT_CONFIG_VERSION_OFFSET, 8),
    (REPORT_REMOTE_STATE_COMMITMENT_OFFSET, WIDE),
];

const RECALL_SENT_FIELDS: &[Field] = &[
    (RECALL_SENT_TRANSFER_ID_OFFSET, WIDE),
    (RECALL_SENT_ASSET_ID_OFFSET, WIDE),
    (RECALL_SENT_PRINCIPAL_SENT_OFFSET, 16),
    (RECALL_SENT_ACTUAL_AMOUNT_SENT_OFFSET, 16),
    (RECALL_SENT_REALIZED_LOSS_OFFSET, 16),
    (RECALL_SENT_DESTINATION_REFERENCE_OFFSET, WIDE),
    (RECALL_SENT_SENT_TIMESTAMP_OFFSET, 8),
    (RECALL_SENT_CONFIG_VERSION_OFFSET, 8),
];

const CONFIG_UPDATE_FIELDS: &[Field] = &[
    (CONFIG_VERSION_OFFSET, 8),
    (CONFIG_PREVIOUS_VERSION_OFFSET, 8),
    (CONFIG_MAX_REMOTE_ALLOCATION_BPS_OFFSET, 2),
    (CONFIG_MAX_UPWARD_DEVIATION_BPS_OFFSET, 2),
    (CONFIG_MAX_DOWNWARD_DEVIATION_BPS_OFFSET, 2),
    (CONFIG_RESERVED_OFFSET, CONFIG_RESERVED_LEN),
    (CONFIG_MAX_REPORT_AGE_OFFSET, 8),
    (CONFIG_EFFECTIVE_TIMESTAMP_OFFSET, 8),
    (CONFIG_COMMITMENT_OFFSET, WIDE),
];

const _: () = assert!(is_contiguous(HEADER_FIELDS, HEADER_LEN));
const _: () = assert!(is_contiguous(ALLOCATE_FIELDS, ALLOCATE_BODY_LEN));
const _: () = assert!(is_contiguous(RECALL_FIELDS, RECALL_BODY_LEN));
const _: () = assert!(is_contiguous(REMOTE_REPORT_FIELDS, REMOTE_REPORT_BODY_LEN));
const _: () = assert!(is_contiguous(RECALL_SENT_FIELDS, RECALL_SENT_BODY_LEN));
const _: () = assert!(is_contiguous(CONFIG_UPDATE_FIELDS, CONFIG_UPDATE_BODY_LEN));

// Timestamps stay on an eight byte boundary inside padded bodies.

const _: () = assert!(REPORT_PROBE_TIMESTAMP_OFFSET.is_multiple_of(8));
const _: () = assert!(REPORT_CONFIG_VERSION_OFFSET.is_multiple_of(8));
const _: () = assert!(CONFIG_MAX_REPORT_AGE_OFFSET.is_multiple_of(8));
const _: () = assert!(CONFIG_EFFECTIVE_TIMESTAMP_OFFSET.is_multiple_of(8));

#[cfg(test)]
mod tests {
    use super::*;

    /// Repeats the const checks so a failure names the region.
    #[test]
    fn every_region_is_contiguous_and_full() {
        assert!(is_contiguous(HEADER_FIELDS, HEADER_LEN), "header");
        assert!(
            is_contiguous(ALLOCATE_FIELDS, ALLOCATE_BODY_LEN),
            "allocate"
        );
        assert!(is_contiguous(RECALL_FIELDS, RECALL_BODY_LEN), "recall");
        assert!(
            is_contiguous(REMOTE_REPORT_FIELDS, REMOTE_REPORT_BODY_LEN),
            "remote report"
        );
        assert!(
            is_contiguous(RECALL_SENT_FIELDS, RECALL_SENT_BODY_LEN),
            "recall sent"
        );
        assert!(
            is_contiguous(CONFIG_UPDATE_FIELDS, CONFIG_UPDATE_BODY_LEN),
            "config update"
        );
    }

    #[test]
    fn a_gap_between_fields_is_rejected() {
        assert!(!is_contiguous(&[(0, 4), (8, 4)], 12));
    }

    #[test]
    fn an_overlap_between_fields_is_rejected() {
        assert!(!is_contiguous(&[(0, 4), (2, 4)], 6));
    }

    #[test]
    fn a_region_shorter_than_its_fields_is_rejected() {
        assert!(!is_contiguous(&[(0, 4), (4, 4)], 12));
    }

    #[test]
    fn a_zero_width_field_is_rejected() {
        assert!(!is_contiguous(&[(0, 4), (4, 0)], 4));
    }

    #[test]
    fn every_message_length_is_the_header_plus_its_body() {
        for (message_len, body_len) in [
            (ALLOCATE_MESSAGE_LEN, ALLOCATE_BODY_LEN),
            (RECALL_MESSAGE_LEN, RECALL_BODY_LEN),
            (REMOTE_REPORT_MESSAGE_LEN, REMOTE_REPORT_BODY_LEN),
            (RECALL_SENT_MESSAGE_LEN, RECALL_SENT_BODY_LEN),
            (CONFIG_UPDATE_MESSAGE_LEN, CONFIG_UPDATE_BODY_LEN),
        ] {
            assert_eq!(message_len, HEADER_LEN + body_len);
            assert!(message_len <= MAX_MESSAGE_LEN);
        }
    }
}
