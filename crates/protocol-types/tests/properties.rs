//! Properties of the codec over sampled valid messages and arbitrary bytes.
//!
//! Set PROPTEST_CASES to widen a soak run.

#![allow(clippy::unwrap_used)]

mod common;

use proptest::prelude::*;
use protocol_types::layout::{BODY_HASH_OFFSET, HEADER_LEN, MAX_MESSAGE_LEN};
use protocol_types::{
    AllocateBody, ApplicationId, AssetAmount, AssetId, BasisPoints, Body, ChainId, Commitment,
    ConfigUpdateBody, ConfigVersion, DeploymentId, EpochId, Flags, Header, LaneId, Message,
    MessageType, PROTOCOL_VERSION, ProbeStatus, RecallBody, RecallSentBody, RemoteReportBody,
    ReportId, SCHEMA_VERSION, Sequence, Timestamp, TransferId, VaultId, decode_message,
    encode_into, encode_message, next_commitment,
};

/// Keeps sampled timestamps far from the integer limit.
const MAX_TIME: u64 = 1 << 40;

fn cases() -> u32 {
    std::env::var("PROPTEST_CASES")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(256)
}

fn config() -> ProptestConfig {
    ProptestConfig {
        cases: cases(),
        ..ProptestConfig::default()
    }
}

fn non_zero_wide() -> impl Strategy<Value = [u8; 32]> {
    prop::array::uniform32(any::<u8>()).prop_map(|mut bytes| {
        if bytes.iter().all(|byte| *byte == 0) {
            bytes[31] = 1;
        }
        bytes
    })
}

/// Observation, publication and expiration in non decreasing order.
fn ordered_times() -> impl Strategy<Value = (u64, u64, u64)> {
    (0..MAX_TIME, 0..MAX_TIME, 0..MAX_TIME).prop_map(|(first, second, third)| {
        let mut sorted = [first, second, third];
        sorted.sort_unstable();
        (sorted[0], sorted[1], sorted[2])
    })
}

fn header_strategy() -> impl Strategy<Value = Header> {
    (
        any::<u32>(),
        any::<u32>(),
        non_zero_wide(),
        non_zero_wide(),
        non_zero_wide(),
        non_zero_wide(),
        any::<u32>(),
        any::<u64>(),
        prop::array::uniform32(any::<u8>()),
        ordered_times(),
    )
        .prop_map(
            |(
                source_chain,
                destination_chain,
                source_application,
                destination_application,
                deployment,
                vault,
                lane,
                sequence,
                previous,
                (observed, published, expires),
            )| {
                let source_chain = source_chain.max(1);
                let destination_chain = if destination_chain.max(1) == source_chain {
                    source_chain.wrapping_add(1).max(1)
                } else {
                    destination_chain.max(1)
                };
                let destination_application = if destination_application == source_application {
                    let mut other = destination_application;
                    other[0] = other[0].wrapping_add(1);
                    other
                } else {
                    destination_application
                };
                Header {
                    protocol_version: PROTOCOL_VERSION,
                    schema_version: SCHEMA_VERSION,
                    flags: Flags::NONE,
                    source_chain: ChainId::new(source_chain),
                    destination_chain: ChainId::new(destination_chain),
                    source_application: ApplicationId::new(source_application),
                    destination_application: ApplicationId::new(destination_application),
                    deployment_id: DeploymentId::new(deployment),
                    vault_id: VaultId::new(vault),
                    lane_id: LaneId::new(lane.max(1)),
                    sequence: Sequence::new(sequence.max(1)),
                    previous_commitment: Commitment::new(previous),
                    observed_at: Timestamp::new(observed),
                    published_at: Timestamp::new(published),
                    expires_at: Timestamp::new(expires),
                }
            },
        )
}

fn allocate_strategy(published: u64) -> impl Strategy<Value = Body> {
    (
        non_zero_wide(),
        non_zero_wide(),
        1u128..=u128::MAX,
        any::<u128>(),
        0..MAX_TIME,
        1u64..=u64::MAX,
        any::<u64>(),
    )
        .prop_map(
            move |(transfer, asset, amount, balance, delay, version, minimum_seed)| {
                let minimum = u128::from(minimum_seed % 100).max(1).min(amount);
                Body::Allocate(AllocateBody {
                    transfer_id: TransferId::new(transfer),
                    asset_id: AssetId::new(asset),
                    amount: AssetAmount::new(amount),
                    expected_source_balance: AssetAmount::new(balance),
                    minimum_destination_amount: AssetAmount::new(minimum),
                    deadline: Timestamp::new(published.saturating_add(delay)),
                    config_version: ConfigVersion::new(version),
                })
            },
        )
}

fn recall_strategy(published: u64) -> impl Strategy<Value = Body> {
    (
        non_zero_wide(),
        non_zero_wide(),
        1u128..=u128::MAX,
        0..MAX_TIME,
        1u64..=u64::MAX,
        any::<u64>(),
    )
        .prop_map(
            move |(transfer, asset, requested, delay, version, minimum_seed)| {
                let minimum = u128::from(minimum_seed % 100).max(1).min(requested);
                Body::Recall(RecallBody {
                    transfer_id: TransferId::new(transfer),
                    asset_id: AssetId::new(asset),
                    requested_amount: AssetAmount::new(requested),
                    minimum_return_amount: AssetAmount::new(minimum),
                    deadline: Timestamp::new(published.saturating_add(delay)),
                    config_version: ConfigVersion::new(version),
                })
            },
        )
}

fn probe_strategy() -> impl Strategy<Value = ProbeStatus> {
    prop_oneof![
        Just(ProbeStatus::NotRequired),
        Just(ProbeStatus::Fresh),
        Just(ProbeStatus::Stale),
        Just(ProbeStatus::Failed),
    ]
}

fn remote_report_strategy(published: u64) -> impl Strategy<Value = Body> {
    (
        non_zero_wide(),
        1u64..=u64::MAX,
        non_zero_wide(),
        any::<u128>(),
        any::<u128>(),
        any::<u128>(),
        prop::array::uniform32(any::<u8>()),
        probe_strategy(),
        0..=published,
        1u64..=u64::MAX,
        non_zero_wide(),
    )
        .prop_map(
            move |(
                report,
                epoch,
                asset,
                principal,
                value,
                unattributed,
                latest,
                probe_status,
                probe_seed,
                version,
                commitment,
            )| {
                let loss = principal / 3;
                let probe_timestamp = if probe_status == ProbeStatus::Fresh {
                    probe_seed.max(1)
                } else {
                    probe_seed
                };
                Body::RemoteReport(RemoteReportBody {
                    report_id: ReportId::new(report),
                    epoch_id: EpochId::new(epoch),
                    asset_id: AssetId::new(asset),
                    remote_principal: AssetAmount::new(principal),
                    reported_value: AssetAmount::new(value),
                    realized_loss: AssetAmount::new(loss),
                    unattributed_balance: AssetAmount::new(unattributed),
                    latest_completed_transfer_id: TransferId::new(latest),
                    probe_status,
                    probe_timestamp: Timestamp::new(probe_timestamp),
                    config_version: ConfigVersion::new(version),
                    remote_state_commitment: Commitment::new(commitment),
                })
            },
        )
}

fn recall_sent_strategy(published: u64) -> impl Strategy<Value = Body> {
    (
        non_zero_wide(),
        non_zero_wide(),
        1u128..=u128::MAX,
        1u128..=u128::MAX,
        non_zero_wide(),
        0..=published,
        1u64..=u64::MAX,
    )
        .prop_map(
            move |(transfer, asset, principal, actual, reference, sent, version)| {
                Body::RecallSent(RecallSentBody {
                    transfer_id: TransferId::new(transfer),
                    asset_id: AssetId::new(asset),
                    principal_sent: AssetAmount::new(principal),
                    actual_amount_sent: AssetAmount::new(actual),
                    realized_loss: AssetAmount::new(principal / 2),
                    destination_reference: Commitment::new(reference),
                    sent_timestamp: Timestamp::new(sent),
                    config_version: ConfigVersion::new(version),
                })
            },
        )
}

fn config_update_strategy(published: u64) -> impl Strategy<Value = Body> {
    (
        1u64..=u64::MAX,
        0u16..=10_000,
        0u16..=10_000,
        0u16..=10_000,
        1u64..=u64::MAX,
        0..MAX_TIME,
        non_zero_wide(),
    )
        .prop_map(
            move |(version, allocation, upward, downward, age, delay, commitment)| {
                Body::ConfigUpdate(ConfigUpdateBody {
                    config_version: ConfigVersion::new(version),
                    previous_config_version: ConfigVersion::new(version.saturating_sub(1)),
                    max_remote_allocation_bps: BasisPoints::new(allocation),
                    max_upward_deviation_bps: BasisPoints::new(upward),
                    max_downward_deviation_bps: BasisPoints::new(downward),
                    max_report_age: age,
                    effective_timestamp: Timestamp::new(published.saturating_add(delay)),
                    config_commitment: Commitment::new(commitment),
                })
            },
        )
}

fn body_strategy(kind: MessageType, published: u64) -> BoxedStrategy<Body> {
    match kind {
        MessageType::Allocate => allocate_strategy(published).boxed(),
        MessageType::Recall => recall_strategy(published).boxed(),
        MessageType::RemoteReport => remote_report_strategy(published).boxed(),
        MessageType::RecallSent => recall_sent_strategy(published).boxed(),
        MessageType::ConfigUpdate => config_update_strategy(published).boxed(),
    }
}

fn kind_strategy() -> impl Strategy<Value = MessageType> {
    prop_oneof![
        Just(MessageType::Allocate),
        Just(MessageType::Recall),
        Just(MessageType::RemoteReport),
        Just(MessageType::RecallSent),
        Just(MessageType::ConfigUpdate),
    ]
}

fn message_strategy() -> impl Strategy<Value = Message> {
    (header_strategy(), kind_strategy()).prop_flat_map(|(header, kind)| {
        body_strategy(kind, header.published_at.get())
            .prop_map(move |body| Message { header, body })
    })
}

fn message_of(kind: MessageType) -> impl Strategy<Value = Message> {
    header_strategy().prop_flat_map(move |header| {
        body_strategy(kind, header.published_at.get())
            .prop_map(move |body| Message { header, body })
    })
}

proptest! {
    #![proptest_config(config())]

    #[test]
    fn every_sampled_message_encodes(subject in message_strategy()) {
        prop_assert!(encode_message(&subject).is_ok());
    }

    #[test]
    fn encoding_then_decoding_returns_the_same_message(subject in message_strategy()) {
        let bytes = encode_message(&subject).unwrap();
        prop_assert_eq!(decode_message(&bytes), Ok(subject));
    }

    #[test]
    fn decoding_then_encoding_returns_the_same_bytes(subject in message_strategy()) {
        let bytes = encode_message(&subject).unwrap();
        let decoded = decode_message(&bytes).unwrap();
        prop_assert_eq!(encode_message(&decoded).unwrap(), bytes);
    }

    #[test]
    fn encoding_is_deterministic(subject in message_strategy()) {
        let first = encode_message(&subject).unwrap();
        let second = encode_message(&subject).unwrap();
        let mut buffer = [0u8; MAX_MESSAGE_LEN];
        let written = encode_into(&subject, &mut buffer).unwrap();
        prop_assert_eq!(&first, &second);
        prop_assert_eq!(buffer.get(..written).unwrap(), first.as_slice());
    }

    #[test]
    fn the_encoded_length_matches_the_message_type(subject in message_strategy()) {
        let bytes = encode_message(&subject).unwrap();
        prop_assert_eq!(bytes.len(), subject.message_type().message_len());
        prop_assert_eq!(bytes.len(), subject.encoded_len());
    }

    #[test]
    fn every_truncation_of_a_valid_message_rejects(subject in message_strategy()) {
        let bytes = encode_message(&subject).unwrap();
        for cut in 0..bytes.len() {
            prop_assert!(decode_message(bytes.get(..cut).unwrap()).is_err(), "cut {}", cut);
        }
    }

    #[test]
    fn appending_bytes_to_a_valid_message_rejects(
        subject in message_strategy(),
        extra in prop::collection::vec(any::<u8>(), 1..8),
    ) {
        let mut bytes = encode_message(&subject).unwrap();
        bytes.extend_from_slice(&extra);
        prop_assert!(decode_message(&bytes).is_err());
    }

    #[test]
    fn prepending_bytes_to_a_valid_message_rejects(
        subject in message_strategy(),
        extra in prop::collection::vec(any::<u8>(), 1..8),
    ) {
        let mut bytes = extra;
        bytes.extend_from_slice(&encode_message(&subject).unwrap());
        prop_assert!(decode_message(&bytes).is_err());
    }

    #[test]
    fn arbitrary_bytes_never_panic(bytes in prop::collection::vec(any::<u8>(), 0..600)) {
        let _ = decode_message(&bytes);
    }

    #[test]
    fn arbitrary_bytes_behind_a_valid_prefix_never_panic(
        subject in message_strategy(),
        tail in prop::collection::vec(any::<u8>(), 0..64),
    ) {
        let mut bytes = encode_message(&subject).unwrap();
        let keep = bytes.len().saturating_sub(tail.len());
        bytes.truncate(keep);
        bytes.extend_from_slice(&tail);
        let _ = decode_message(&bytes);
    }

    #[test]
    fn a_single_byte_change_never_decodes_back_to_the_original(
        subject in message_strategy(),
        position in 0usize..MAX_MESSAGE_LEN,
        mask in 1u8..=u8::MAX,
    ) {
        let bytes = encode_message(&subject).unwrap();
        let position = position % bytes.len();
        let mut edited = bytes.clone();
        *edited.get_mut(position).unwrap() ^= mask;

        if let Ok(decoded) = decode_message(&edited) {
            prop_assert_ne!(&decoded, &subject);
            prop_assert_ne!(decoded.message_id().unwrap(), subject.message_id().unwrap());
        }
    }

    #[test]
    fn a_body_change_is_caught_by_the_body_hash(
        subject in message_strategy(),
        position in 0usize..MAX_MESSAGE_LEN,
        mask in 1u8..=u8::MAX,
    ) {
        let bytes = encode_message(&subject).unwrap();
        let body_len = subject.message_type().body_len();
        let mut edited = bytes.clone();
        *edited.get_mut(HEADER_LEN + position % body_len).unwrap() ^= mask;
        prop_assert_eq!(
            decode_message(&edited),
            Err(protocol_types::DecodeError::BodyHashMismatch)
        );
    }

    #[test]
    fn a_body_hash_change_always_rejects(
        subject in message_strategy(),
        position in 0usize..32,
        mask in 1u8..=u8::MAX,
    ) {
        let mut bytes = encode_message(&subject).unwrap();
        *bytes.get_mut(BODY_HASH_OFFSET + position).unwrap() ^= mask;
        prop_assert_eq!(
            decode_message(&bytes),
            Err(protocol_types::DecodeError::BodyHashMismatch)
        );
    }

    #[test]
    fn the_message_id_is_stable_for_one_message(subject in message_strategy()) {
        let first = subject.message_id().unwrap();
        let bytes = encode_message(&subject).unwrap();
        let second = decode_message(&bytes).unwrap().message_id().unwrap();
        prop_assert_eq!(first, second);
        prop_assert_eq!(first, subject.encode().unwrap().message_id());
    }

    #[test]
    fn two_different_messages_never_share_canonical_bytes(
        first in message_strategy(),
        second in message_strategy(),
    ) {
        let left = encode_message(&first).unwrap();
        let right = encode_message(&second).unwrap();
        if first == second {
            prop_assert_eq!(left, right);
        } else {
            prop_assert_ne!(&left, &right);
            prop_assert_ne!(first.message_id().unwrap(), second.message_id().unwrap());
        }
    }

    #[test]
    fn a_chain_link_changes_with_either_input(
        subject in message_strategy(),
        first in prop::array::uniform32(any::<u8>()),
        second in prop::array::uniform32(any::<u8>()),
    ) {
        let id = subject.message_id().unwrap();
        let left = next_commitment(Commitment::new(first), id);
        let right = next_commitment(Commitment::new(second), id);
        prop_assert_eq!(left, next_commitment(Commitment::new(first), id));
        if first == second {
            prop_assert_eq!(left, right);
        } else {
            prop_assert_ne!(left, right);
        }
    }

    #[test]
    fn a_failed_encoding_leaves_the_buffer_untouched(
        subject in message_strategy(),
        width in 0usize..MAX_MESSAGE_LEN,
    ) {
        let width = width % subject.encoded_len();
        let mut buffer = vec![0xA7u8; width];
        prop_assert!(encode_into(&subject, &mut buffer).is_err());
        prop_assert!(buffer.iter().all(|byte| *byte == 0xA7));
    }
}

proptest! {
    #![proptest_config(config())]

    #[test]
    fn every_allocate_message_round_trips(subject in message_of(MessageType::Allocate)) {
        let bytes = encode_message(&subject).unwrap();
        prop_assert_eq!(bytes.len(), 380);
        prop_assert_eq!(decode_message(&bytes), Ok(subject));
    }

    #[test]
    fn every_recall_message_round_trips(subject in message_of(MessageType::Recall)) {
        let bytes = encode_message(&subject).unwrap();
        prop_assert_eq!(bytes.len(), 364);
        prop_assert_eq!(decode_message(&bytes), Ok(subject));
    }

    #[test]
    fn every_remote_report_message_round_trips(subject in message_of(MessageType::RemoteReport)) {
        let bytes = encode_message(&subject).unwrap();
        prop_assert_eq!(bytes.len(), 476);
        prop_assert_eq!(decode_message(&bytes), Ok(subject));
    }

    #[test]
    fn every_recall_sent_message_round_trips(subject in message_of(MessageType::RecallSent)) {
        let bytes = encode_message(&subject).unwrap();
        prop_assert_eq!(bytes.len(), 412);
        prop_assert_eq!(decode_message(&bytes), Ok(subject));
    }

    #[test]
    fn every_config_update_message_round_trips(subject in message_of(MessageType::ConfigUpdate)) {
        let bytes = encode_message(&subject).unwrap();
        prop_assert_eq!(bytes.len(), 324);
        prop_assert_eq!(decode_message(&bytes), Ok(subject));
    }
}
