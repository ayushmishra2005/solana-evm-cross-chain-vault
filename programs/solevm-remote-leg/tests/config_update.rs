//! Processing of canonical configuration messages.

#![allow(clippy::unwrap_used, clippy::panic, clippy::arithmetic_side_effects)]

mod common;

use solana_signer::Signer;
use solevm_remote_leg::{MessageClass, RemoteLegError, STATE_VERSION};

use common::messages::{EFFECTIVE_AT, MessageBuilder, PUBLISHED_AT, next_commitment};
use common::{
    CONFIG_VERSION, CONTROL_LANE_ID, Fixture, START_TIMESTAMP, expect_error, expect_rejection,
};

/// Sends the first valid update and returns its message id.
fn accept_first(fixture: &mut Fixture) -> [u8; 32] {
    let (bytes, id) = MessageBuilder::config_update().encode_with_id();
    fixture.config_update(1, bytes).expect("first update lands");
    id
}

/// Sends the second valid update and returns its message id.
fn accept_second(fixture: &mut Fixture) -> [u8; 32] {
    let commitment = fixture.lane(MessageClass::ConfigUpdate).message_commitment;
    let (bytes, id) = MessageBuilder::config_update()
        .sequence(2)
        .previous_commitment(commitment)
        .config_body(|body| {
            body.previous_config_version = protocol_types::ConfigVersion::new(CONFIG_VERSION + 1);
            body.config_version = protocol_types::ConfigVersion::new(CONFIG_VERSION + 2);
        })
        .encode_with_id();
    fixture
        .config_update(2, bytes)
        .expect("second update lands");
    id
}

#[test]
fn the_first_config_update_is_applied() {
    let mut fixture = Fixture::ready();
    let id = accept_first(&mut fixture);

    let risk = fixture.risk_config();
    assert_eq!(risk.config_version, CONFIG_VERSION + 1);
    assert_eq!(risk.max_remote_allocation_bps, 6_000);
    assert_eq!(risk.max_upward_deviation_bps, 200);
    assert_eq!(risk.max_downward_deviation_bps, 1_000);
    assert_eq!(risk.max_report_age, 3_600);
    assert_eq!(risk.config_commitment, [0xCC; 32]);
    assert_eq!(risk.last_update_at, START_TIMESTAMP);

    assert_eq!(fixture.config().config_version, CONFIG_VERSION + 1);

    let lane = fixture.lane(MessageClass::ConfigUpdate);
    assert_eq!(lane.highest_consumed_sequence, 1);
    assert_eq!(lane.message_commitment, next_commitment([0u8; 32], id));
    assert_eq!(lane.last_accepted_at, START_TIMESTAMP);
}

#[test]
fn a_second_config_update_continues_the_chain() {
    let mut fixture = Fixture::ready();
    let first = accept_first(&mut fixture);
    let second = accept_second(&mut fixture);

    let lane = fixture.lane(MessageClass::ConfigUpdate);
    assert_eq!(lane.highest_consumed_sequence, 2);
    assert_eq!(
        lane.message_commitment,
        next_commitment(next_commitment([0u8; 32], first), second)
    );
    assert_eq!(fixture.risk_config().config_version, CONFIG_VERSION + 2);
}

#[test]
fn the_stored_message_id_matches_the_protocol() {
    let mut fixture = Fixture::ready();
    let id = accept_first(&mut fixture);

    let record = fixture.record(1);
    assert_eq!(record.state_version, STATE_VERSION);
    assert_eq!(record.message_class, MessageClass::ConfigUpdate);
    assert_eq!(record.lane_id, CONTROL_LANE_ID);
    assert_eq!(record.sequence, 1);
    assert_eq!(record.message_id, id);
}

#[test]
fn an_exact_replay_is_rejected() {
    let mut fixture = Fixture::ready();
    let (bytes, _) = MessageBuilder::config_update().encode_with_id();
    fixture
        .config_update(1, bytes.clone())
        .expect("first update lands");

    expect_error(
        fixture.config_update(1, bytes),
        RemoteLegError::InvalidSequence,
    );
}

#[test]
fn the_same_sequence_with_other_bytes_is_rejected() {
    let mut fixture = Fixture::ready();
    accept_first(&mut fixture);

    let bytes = MessageBuilder::config_update()
        .config_body(|body| body.config_commitment = protocol_types::Commitment::new([0xDD; 32]))
        .encode();
    expect_error(
        fixture.config_update(1, bytes),
        RemoteLegError::InvalidSequence,
    );
}

#[test]
fn a_sequence_gap_is_rejected() {
    let mut fixture = Fixture::ready();
    let bytes = MessageBuilder::config_update().sequence(2).encode();
    expect_error(
        fixture.config_update(2, bytes),
        RemoteLegError::InvalidSequence,
    );
}

#[test]
fn a_wrong_previous_commitment_is_rejected() {
    let mut fixture = Fixture::ready();
    accept_first(&mut fixture);

    let bytes = MessageBuilder::config_update()
        .sequence(2)
        .previous_commitment([0x01; 32])
        .encode();
    expect_error(
        fixture.config_update(2, bytes),
        RemoteLegError::InvalidPreviousCommitment,
    );
}

#[test]
fn a_previous_config_version_that_does_not_match_is_rejected() {
    let mut fixture = Fixture::ready();
    let bytes = MessageBuilder::config_update()
        .config_body(|body| {
            body.previous_config_version = protocol_types::ConfigVersion::new(CONFIG_VERSION + 5);
            body.config_version = protocol_types::ConfigVersion::new(CONFIG_VERSION + 6);
        })
        .encode();
    expect_error(
        fixture.config_update(1, bytes),
        RemoteLegError::InvalidConfigVersion,
    );
}

#[test]
fn a_config_version_that_does_not_increase_is_rejected() {
    let mut fixture = Fixture::ready();
    accept_first(&mut fixture);
    let commitment = fixture.lane(MessageClass::ConfigUpdate).message_commitment;

    let bytes = MessageBuilder::config_update()
        .sequence(2)
        .previous_commitment(commitment)
        .config_body(|body| {
            body.previous_config_version = protocol_types::ConfigVersion::new(CONFIG_VERSION);
            body.config_version = protocol_types::ConfigVersion::new(CONFIG_VERSION + 1);
        })
        .encode();
    expect_error(
        fixture.config_update(2, bytes),
        RemoteLegError::InvalidConfigVersion,
    );
}

#[test]
fn an_effective_timestamp_in_the_future_is_rejected() {
    let mut fixture = Fixture::ready();
    let future = START_TIMESTAMP as u64 + 500;
    let bytes = MessageBuilder::config_update()
        .config_body(|body| body.effective_timestamp = protocol_types::Timestamp::new(future))
        .encode();
    expect_error(
        fixture.config_update(1, bytes),
        RemoteLegError::ConfigNotEffective,
    );
}

#[test]
fn an_effective_timestamp_that_has_just_arrived_is_accepted() {
    let mut fixture = Fixture::ready();
    let bytes = MessageBuilder::config_update()
        .config_body(|body| {
            body.effective_timestamp = protocol_types::Timestamp::new(START_TIMESTAMP as u64);
        })
        .encode();
    fixture.config_update(1, bytes).expect("update lands");
}

#[test]
fn an_expired_message_is_rejected() {
    let mut fixture = Fixture::ready();
    let bytes = MessageBuilder::config_update()
        .expires_at(START_TIMESTAMP as u64 - 1)
        .encode();
    expect_error(
        fixture.config_update(1, bytes),
        RemoteLegError::MessageExpired,
    );
}

#[test]
fn a_message_published_in_the_future_is_rejected() {
    let mut fixture = Fixture::ready();
    let future = START_TIMESTAMP as u64 + 1_000;
    let bytes = MessageBuilder::config_update()
        .observed_at(future)
        .published_at(future)
        .config_body(|body| body.effective_timestamp = protocol_types::Timestamp::new(future))
        .encode();
    expect_error(
        fixture.config_update(1, bytes),
        RemoteLegError::InvalidTimestamp,
    );
}

#[test]
fn a_frozen_leg_rejects_config_updates() {
    let mut fixture = Fixture::ready();
    let guardian = fixture.guardian.insecure_clone();
    fixture.freeze(&guardian).expect("leg freezes");

    let bytes = MessageBuilder::config_update().encode();
    expect_error(fixture.config_update(1, bytes), RemoteLegError::Frozen);
}

#[test]
fn a_missing_verifier_signature_is_rejected() {
    let mut fixture = Fixture::ready();
    let bytes = MessageBuilder::config_update().encode();
    let accounts = fixture.update_accounts(1);
    let mut instruction = fixture.config_update_instruction(accounts, bytes);
    for meta in &mut instruction.accounts {
        if meta.pubkey == accounts.transport_verifier {
            meta.is_signer = false;
        }
    }

    let payer = fixture.administrator.insecure_clone();
    expect_rejection(fixture.send_as(instruction, &payer, &[&payer]));
}

#[test]
fn a_verifier_that_is_not_configured_is_rejected() {
    let mut fixture = Fixture::ready();
    let stranger = fixture.outsider.insecure_clone();
    let bytes = MessageBuilder::config_update().encode();

    let mut accounts = fixture.update_accounts(1);
    accounts.transport_verifier = stranger.pubkey();
    expect_error(
        fixture.config_update_with(accounts, bytes, &stranger),
        RemoteLegError::Unauthorized,
    );
}

#[test]
fn the_administrator_may_not_stand_in_for_the_verifier() {
    let mut fixture = Fixture::ready();
    let administrator = fixture.administrator.insecure_clone();
    let bytes = MessageBuilder::config_update().encode();

    let mut accounts = fixture.update_accounts(1);
    accounts.transport_verifier = administrator.pubkey();
    expect_error(
        fixture.config_update_with(accounts, bytes, &administrator),
        RemoteLegError::Unauthorized,
    );
}

#[test]
fn a_risk_version_out_of_step_with_the_configuration_is_rejected() {
    let mut fixture = Fixture::ready();
    let risk = fixture.risk();
    let mut account = fixture.svm.get_account(&risk).expect("risk exists");
    // The stored risk version sits right after the two bumps and three rates.
    account.data[8 + 1 + 1 + 2 + 2 + 2 + 8] = 9;
    fixture.svm.set_account(risk, account).unwrap();

    let bytes = MessageBuilder::config_update().encode();
    expect_error(
        fixture.config_update(1, bytes),
        RemoteLegError::InvalidRiskConfig,
    );
}

#[test]
fn a_rejected_update_leaves_every_account_unchanged() {
    let mut fixture = Fixture::ready();
    let risk_before = fixture.raw_data(fixture.risk());
    let lane_before = fixture.raw_data(fixture.lane_key(MessageClass::ConfigUpdate));
    let config_before = fixture.raw_data(fixture.config);

    let bytes = MessageBuilder::config_update()
        .previous_commitment([0x07; 32])
        .encode();
    expect_error(
        fixture.config_update(1, bytes),
        RemoteLegError::InvalidPreviousCommitment,
    );

    assert_eq!(fixture.raw_data(fixture.risk()), risk_before);
    assert_eq!(
        fixture.raw_data(fixture.lane_key(MessageClass::ConfigUpdate)),
        lane_before
    );
    assert_eq!(fixture.raw_data(fixture.config), config_before);
    assert!(!fixture.record_exists(1));
}

#[test]
fn the_immutable_configuration_fields_survive_an_update() {
    let mut fixture = Fixture::ready();
    let before = fixture.config();
    accept_first(&mut fixture);
    let after = fixture.config();

    assert_eq!(before.administrator, after.administrator);
    assert_eq!(before.emergency_guardian, after.emergency_guardian);
    assert_eq!(before.transport_verifier, after.transport_verifier);
    assert_eq!(before.asset_mint, after.asset_mint);
    assert_eq!(before.token_program, after.token_program);
    assert_eq!(before.custody_authority, after.custody_authority);
    assert_eq!(before.custody_token_account, after.custody_token_account);
    assert_eq!(before.outbound_escrow, after.outbound_escrow);
    assert_eq!(before.source_chain_id, after.source_chain_id);
    assert_eq!(before.destination_chain_id, after.destination_chain_id);
    assert_eq!(before.source_application_id, after.source_application_id);
    assert_eq!(before.local_application_id, after.local_application_id);
    assert_eq!(before.deployment_id, after.deployment_id);
    assert_eq!(before.vault_id, after.vault_id);
    assert_eq!(before.control_lane_id, after.control_lane_id);
    assert_eq!(before.report_lane_id, after.report_lane_id);
    assert_eq!(before.frozen, after.frozen);
    assert_eq!(before.reserved, after.reserved);
    assert_ne!(before.config_version, after.config_version);
}

#[test]
fn the_configuration_account_keeps_its_size() {
    let mut fixture = Fixture::ready();
    let before = fixture.raw_data(fixture.config).len();
    accept_first(&mut fixture);
    assert_eq!(fixture.raw_data(fixture.config).len(), before);
    assert_eq!(before, 492);
}

// Risk parameter bounds

#[test]
fn basis_points_above_ten_thousand_never_reach_the_chain() {
    let setters: [fn(&mut protocol_types::ConfigUpdateBody); 3] = [
        |body| body.max_remote_allocation_bps = protocol_types::BasisPoints::new(10_001),
        |body| body.max_upward_deviation_bps = protocol_types::BasisPoints::new(10_001),
        |body| body.max_downward_deviation_bps = protocol_types::BasisPoints::new(10_001),
    ];

    for set in setters {
        let message = MessageBuilder::config_update().config_body(set).build();
        assert!(
            message.encode().is_err(),
            "the shared codec must refuse to encode out of range basis points"
        );
    }
}

#[test]
fn a_zero_report_age_never_reaches_the_chain() {
    let message = MessageBuilder::config_update()
        .config_body(|body| body.max_report_age = 0)
        .build();
    assert!(message.encode().is_err());
}

#[test]
fn a_zero_config_commitment_never_reaches_the_chain() {
    let message = MessageBuilder::config_update()
        .config_body(|body| body.config_commitment = protocol_types::Commitment::ZERO)
        .build();
    assert!(message.encode().is_err());
}

#[test]
fn out_of_range_basis_points_are_rejected_on_chain() {
    let mut fixture = Fixture::ready();
    let mut bytes = MessageBuilder::config_update().encode();
    patch_body(
        &mut bytes,
        protocol_types::layout::CONFIG_MAX_REMOTE_ALLOCATION_BPS_OFFSET,
        &10_001u16.to_be_bytes(),
    );
    expect_error(
        fixture.config_update(1, bytes),
        RemoteLegError::InvalidMessage,
    );
}

#[test]
fn the_effective_timestamp_must_not_precede_publication() {
    let message = MessageBuilder::config_update()
        .config_body(|body| {
            body.effective_timestamp = protocol_types::Timestamp::new(PUBLISHED_AT - 1);
        })
        .build();
    assert!(message.encode().is_err());
}

#[test]
fn the_sample_effective_timestamp_sits_between_publication_and_now() {
    const { assert!(PUBLISHED_AT <= EFFECTIVE_AT) };
    const { assert!(EFFECTIVE_AT <= START_TIMESTAMP as u64) };
}

/// Overwrites body bytes without fixing the body hash.
fn patch_body(bytes: &mut [u8], offset: usize, value: &[u8]) {
    let start = protocol_types::layout::HEADER_LEN + offset;
    bytes[start..start + value.len()].copy_from_slice(value);
}
