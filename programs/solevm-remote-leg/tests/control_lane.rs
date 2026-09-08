//! Shared inbound control-lane commitment and replay invariants.

#![allow(clippy::unwrap_used, clippy::panic, clippy::arithmetic_side_effects)]

mod common;

use common::messages::{MessageBuilder, next_commitment};
use common::{ALLOCATE_TRANSFER_ID, CONTROL_LANE_ID, Fixture, RECALL_TRANSFER_ID, expect_error};
use solevm_remote_leg::{MessageClass, RemoteLegError, message};

const AMOUNT: u64 = 1_000_000;
const ALLOCATE_ID: [u8; 32] = ALLOCATE_TRANSFER_ID;
const RECALL_ID: [u8; 32] = RECALL_TRANSFER_ID;
const SECOND_ALLOCATE: [u8; 32] = [0x79; 32];

fn message_id(bytes: &[u8]) -> [u8; 32] {
    message::message_id(bytes).expect("id is computed")
}

fn assert_lane(fixture: &Fixture, sequence: u64, commitment: [u8; 32]) {
    let lane = fixture.control_lane();
    assert_eq!(lane.lane_id, CONTROL_LANE_ID);
    assert_eq!(lane.highest_consumed_sequence, sequence);
    assert_eq!(lane.message_commitment, commitment);
}

#[test]
fn allocate_then_recall_share_one_commitment_chain() {
    let mut fixture = Fixture::deployed();
    let allocate_bytes = fixture.allocate_bytes(ALLOCATE_ID, AMOUNT, 1);
    let allocate_id = message_id(&allocate_bytes);
    let allocate_commitment = next_commitment([0u8; 32], allocate_id);

    fixture
        .allocate(ALLOCATE_ID, 1, allocate_bytes)
        .expect("allocate lands");
    fixture.credit(fixture.custody, AMOUNT);
    fixture.attribute(ALLOCATE_ID).expect("attribution lands");

    let recall_bytes = fixture.recall_bytes(RECALL_ID, AMOUNT, 2);
    let recall_id = message_id(&recall_bytes);
    assert_eq!(
        protocol_types::decode_message(&recall_bytes)
            .expect("recall decodes")
            .header
            .previous_commitment
            .to_bytes(),
        allocate_commitment
    );

    fixture
        .recall(RECALL_ID, 2, recall_bytes)
        .expect("recall lands");

    assert_lane(&fixture, 2, next_commitment(allocate_commitment, recall_id));
    assert_eq!(
        fixture.asset_record(1).message_class,
        MessageClass::Allocate
    );
    assert_eq!(fixture.asset_record(2).message_class, MessageClass::Recall);
}

#[test]
fn recall_then_allocate_continue_the_same_chain() {
    let mut fixture = Fixture::deployed();
    fixture.fund_position(ALLOCATE_ID, AMOUNT);

    let recall_bytes = fixture.recall_bytes(RECALL_ID, AMOUNT, 2);
    let recall_id = message_id(&recall_bytes);
    let after_allocate = fixture.control_lane().message_commitment;
    fixture
        .recall(RECALL_ID, 2, recall_bytes)
        .expect("recall lands");
    fixture
        .send_recall(RECALL_ID, AMOUNT)
        .expect("recall completes");

    let allocate_bytes = fixture.allocate_bytes(SECOND_ALLOCATE, 500, 3);
    let allocate_id = message_id(&allocate_bytes);
    let after_recall = next_commitment(after_allocate, recall_id);
    assert_eq!(
        protocol_types::decode_message(&allocate_bytes)
            .expect("allocate decodes")
            .header
            .previous_commitment
            .to_bytes(),
        after_recall
    );

    fixture
        .allocate(SECOND_ALLOCATE, 3, allocate_bytes)
        .expect("later allocate lands");
    assert_lane(&fixture, 3, next_commitment(after_recall, allocate_id));
}

#[test]
fn a_recall_with_a_zero_previous_commitment_is_rejected() {
    let mut fixture = Fixture::deployed();
    fixture.fund_position(ALLOCATE_ID, AMOUNT);
    let lane_before = fixture.raw_data(fixture.control_lane_key());
    let position_before = fixture.raw_data(fixture.position_key());

    let bytes = MessageBuilder::recall()
        .sequence(2)
        .previous_commitment([0u8; 32])
        .transfer_id(RECALL_ID)
        .recall_body(|body| {
            body.requested_amount = protocol_types::AssetAmount::new(u128::from(AMOUNT));
            body.minimum_return_amount = protocol_types::AssetAmount::new(u128::from(AMOUNT));
        })
        .encode();

    expect_error(
        fixture.recall(RECALL_ID, 2, bytes),
        RemoteLegError::InvalidPreviousCommitment,
    );
    assert_eq!(fixture.raw_data(fixture.control_lane_key()), lane_before);
    assert_eq!(fixture.raw_data(fixture.position_key()), position_before);
    assert!(!fixture.transfer_exists(&RECALL_ID));
    assert_eq!(fixture.control_lane().highest_consumed_sequence, 1);
}

#[test]
fn a_duplicate_first_allocate_is_rejected() {
    let mut fixture = Fixture::deployed();
    let bytes = fixture.allocate_bytes(ALLOCATE_ID, AMOUNT, 1);
    fixture
        .allocate(ALLOCATE_ID, 1, bytes.clone())
        .expect("first allocate lands");

    let lane_before = fixture.raw_data(fixture.control_lane_key());
    let position_before = fixture.raw_data(fixture.position_key());
    expect_error(
        fixture.allocate(ALLOCATE_ID, 1, bytes),
        RemoteLegError::InvalidSequence,
    );
    assert_eq!(fixture.raw_data(fixture.control_lane_key()), lane_before);
    assert_eq!(fixture.raw_data(fixture.position_key()), position_before);
    assert_eq!(fixture.control_lane().highest_consumed_sequence, 1);
}

#[test]
fn a_second_class_cannot_reuse_a_consumed_sequence() {
    let mut fixture = Fixture::deployed();
    fixture.fund_position(ALLOCATE_ID, AMOUNT);
    let lane_before = fixture.raw_data(fixture.control_lane_key());

    let bytes = MessageBuilder::recall()
        .sequence(1)
        .previous_commitment([0u8; 32])
        .transfer_id(RECALL_ID)
        .recall_body(|body| {
            body.requested_amount = protocol_types::AssetAmount::new(u128::from(AMOUNT));
            body.minimum_return_amount = protocol_types::AssetAmount::new(u128::from(AMOUNT));
        })
        .encode();

    expect_error(
        fixture.recall(RECALL_ID, 1, bytes),
        RemoteLegError::InvalidSequence,
    );
    assert_eq!(fixture.raw_data(fixture.control_lane_key()), lane_before);
    assert_eq!(
        fixture.asset_record(1).message_class,
        MessageClass::Allocate
    );
    assert!(!fixture.transfer_exists(&RECALL_ID));
}

#[test]
fn a_later_state_failure_does_not_consume_the_lane() {
    let mut fixture = Fixture::deployed();
    let allocate_bytes = fixture.allocate_bytes(ALLOCATE_ID, AMOUNT, 1);
    let allocate_id = message_id(&allocate_bytes);
    fixture
        .allocate(ALLOCATE_ID, 1, allocate_bytes)
        .expect("allocate lands");

    let lane_before = fixture.raw_data(fixture.control_lane_key());
    let recall_bytes = MessageBuilder::recall()
        .sequence(2)
        .previous_commitment(next_commitment([0u8; 32], allocate_id))
        .transfer_id(RECALL_ID)
        .recall_body(|body| {
            body.requested_amount = protocol_types::AssetAmount::new(u128::from(AMOUNT));
            body.minimum_return_amount = protocol_types::AssetAmount::new(u128::from(AMOUNT));
        })
        .encode();

    expect_error(
        fixture.recall(RECALL_ID, 2, recall_bytes.clone()),
        RemoteLegError::UnresolvedCycle,
    );
    assert_eq!(fixture.raw_data(fixture.control_lane_key()), lane_before);
    assert_eq!(fixture.control_lane().highest_consumed_sequence, 1);
    assert_eq!(
        fixture.control_lane().message_commitment,
        next_commitment([0u8; 32], allocate_id)
    );
    assert!(!fixture.record_exists(2));

    expect_error(
        fixture.recall(RECALL_ID, 2, recall_bytes),
        RemoteLegError::UnresolvedCycle,
    );
}

#[test]
fn allocate_recall_allocate_keep_one_commitment_chain() {
    let mut fixture = Fixture::deployed();
    let first = fixture.allocate_bytes(ALLOCATE_ID, AMOUNT, 1);
    let first_id = message_id(&first);
    fixture
        .allocate(ALLOCATE_ID, 1, first)
        .expect("first allocate lands");
    fixture.credit(fixture.custody, AMOUNT);
    fixture.attribute(ALLOCATE_ID).expect("attribution lands");
    let after_first = next_commitment([0u8; 32], first_id);

    let recall = fixture.recall_bytes(RECALL_ID, AMOUNT, 2);
    let recall_id = message_id(&recall);
    fixture.recall(RECALL_ID, 2, recall).expect("recall lands");
    fixture
        .send_recall(RECALL_ID, AMOUNT)
        .expect("recall completes");
    let after_recall = next_commitment(after_first, recall_id);

    let third = fixture.allocate_bytes(SECOND_ALLOCATE, 500, 3);
    let third_id = message_id(&third);
    assert_eq!(
        protocol_types::decode_message(&third)
            .expect("third decodes")
            .header
            .previous_commitment
            .to_bytes(),
        after_recall
    );
    fixture
        .allocate(SECOND_ALLOCATE, 3, third)
        .expect("second allocate lands");

    assert_lane(&fixture, 3, next_commitment(after_recall, third_id));
}

#[test]
fn config_update_shares_the_configured_control_lane() {
    let mut fixture = Fixture::deployed();
    fixture.fund_position(ALLOCATE_ID, AMOUNT);
    let after_allocate = fixture.control_lane().message_commitment;

    let update_id = fixture.accept_next_update();
    let after_update = next_commitment(after_allocate, update_id);
    assert_lane(&fixture, 2, after_update);
    assert_eq!(
        fixture.asset_record(2).message_class,
        MessageClass::ConfigUpdate
    );

    let recall = fixture.recall_bytes(RECALL_ID, AMOUNT, 3);
    let recall_id = message_id(&recall);
    assert_eq!(
        protocol_types::decode_message(&recall)
            .expect("recall decodes")
            .header
            .previous_commitment
            .to_bytes(),
        after_update
    );
    fixture.recall(RECALL_ID, 3, recall).expect("recall lands");
    assert_lane(&fixture, 3, next_commitment(after_update, recall_id));
}

#[test]
fn a_config_update_on_a_different_lane_id_is_rejected() {
    let mut fixture = Fixture::deployed();
    fixture.fund_position(ALLOCATE_ID, AMOUNT);
    let lane_before = fixture.raw_data(fixture.control_lane_key());

    let bytes = MessageBuilder::config_update()
        .sequence(2)
        .previous_commitment(fixture.control_lane().message_commitment)
        .lane(CONTROL_LANE_ID + 1)
        .encode();

    expect_error(fixture.config_update(2, bytes), RemoteLegError::InvalidLane);
    assert_eq!(fixture.raw_data(fixture.control_lane_key()), lane_before);
}
