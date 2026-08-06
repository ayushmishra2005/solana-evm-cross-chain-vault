//! The watermark may not step over an unresolved transfer.

#![allow(clippy::unwrap_used, clippy::panic, clippy::arithmetic_side_effects)]

mod common;

use common::{ALLOCATE_TRANSFER_ID, Fixture, RECALL_TRANSFER_ID, expect_error};
use solana_signer::Signer;
use solevm_remote_leg::{MessageClass, RemoteLegError};

const AMOUNT: u64 = 100;
/// Leaves room under the open transfer, so the watermark can move at all.
const OPEN_SEQUENCE: u64 = 10;

/// Opens an allocation and lifts the lane above it.
///
/// Lifting the lane isolates the obligation guard from the bounds and lag
/// rules, which would otherwise reject first.
fn open_allocation(fixture: &mut Fixture) -> u64 {
    let sequence = fixture.accept_allocation_at(ALLOCATE_TRANSFER_ID, AMOUNT, Some(OPEN_SEQUENCE));
    fixture.set_lane_highest(MessageClass::Allocate, sequence + 8);
    sequence
}

/// Opens a recall the same way, funding the position first.
fn open_recall(fixture: &mut Fixture) -> u64 {
    fixture.fund_position([0x70; 32], AMOUNT);
    let sequence = fixture.accept_recall_at(RECALL_TRANSFER_ID, AMOUNT, Some(OPEN_SEQUENCE));
    fixture.set_lane_highest(MessageClass::Recall, sequence + 8);
    sequence
}

#[test]
fn an_open_transfer_always_sits_at_the_top_of_its_lane() {
    let mut fixture = Fixture::deployed();
    fixture.accept_allocation(ALLOCATE_TRANSFER_ID, AMOUNT);

    let position = fixture.position();
    let lane = fixture.lane(MessageClass::Allocate);
    assert_eq!(
        position.active_transfer_sequence,
        lane.highest_consumed_sequence
    );
}

#[test]
fn the_watermark_may_not_pass_an_open_allocation() {
    let mut fixture = Fixture::deployed();
    let sequence = open_allocation(&mut fixture);

    let administrator = fixture.administrator.insecure_clone();
    expect_error(
        fixture.advance_watermark(&administrator, MessageClass::Allocate, sequence + 1),
        RemoteLegError::FinancialObligationBlocksWatermark,
    );
    assert_eq!(
        fixture
            .lane(MessageClass::Allocate)
            .minimum_acceptable_sequence,
        1
    );
}

#[test]
fn the_watermark_may_reach_an_open_allocation() {
    let mut fixture = Fixture::deployed();
    let sequence = open_allocation(&mut fixture);

    let administrator = fixture.administrator.insecure_clone();
    fixture
        .advance_watermark(&administrator, MessageClass::Allocate, sequence)
        .expect("reaching the open sequence is allowed");
    assert_eq!(
        fixture
            .lane(MessageClass::Allocate)
            .minimum_acceptable_sequence,
        sequence
    );
}

#[test]
fn the_watermark_may_not_pass_an_open_recall() {
    let mut fixture = Fixture::deployed();
    let sequence = open_recall(&mut fixture);

    let administrator = fixture.administrator.insecure_clone();
    expect_error(
        fixture.advance_watermark(&administrator, MessageClass::Recall, sequence + 1),
        RemoteLegError::FinancialObligationBlocksWatermark,
    );
}

#[test]
fn the_watermark_may_reach_an_open_recall() {
    let mut fixture = Fixture::deployed();
    let sequence = open_recall(&mut fixture);

    let administrator = fixture.administrator.insecure_clone();
    fixture
        .advance_watermark(&administrator, MessageClass::Recall, sequence)
        .expect("reaching the open sequence is allowed");
}

#[test]
fn the_watermark_moves_once_the_allocation_completes() {
    let mut fixture = Fixture::deployed();
    let sequence = open_allocation(&mut fixture);

    let administrator = fixture.administrator.insecure_clone();
    expect_error(
        fixture.advance_watermark(&administrator, MessageClass::Allocate, sequence + 1),
        RemoteLegError::FinancialObligationBlocksWatermark,
    );

    fixture.credit(fixture.custody, AMOUNT);
    fixture
        .attribute(ALLOCATE_TRANSFER_ID)
        .expect("attribution lands");
    fixture
        .advance_watermark(&administrator, MessageClass::Allocate, sequence + 1)
        .expect("a closed cycle no longer blocks");
}

#[test]
fn the_watermark_moves_once_the_recall_completes() {
    let mut fixture = Fixture::deployed();
    let sequence = open_recall(&mut fixture);

    let administrator = fixture.administrator.insecure_clone();
    expect_error(
        fixture.advance_watermark(&administrator, MessageClass::Recall, sequence + 1),
        RemoteLegError::FinancialObligationBlocksWatermark,
    );

    fixture
        .send_recall(RECALL_TRANSFER_ID, AMOUNT)
        .expect("send lands");
    fixture
        .advance_watermark(&administrator, MessageClass::Recall, sequence + 1)
        .expect("a closed cycle no longer blocks");
}

#[test]
fn an_allocation_never_blocks_the_recall_lane() {
    let mut fixture = Fixture::deployed();
    let sequence = open_allocation(&mut fixture);
    fixture.set_lane_highest(MessageClass::Recall, sequence + 8);

    let administrator = fixture.administrator.insecure_clone();
    fixture
        .advance_watermark(&administrator, MessageClass::Recall, sequence + 1)
        .expect("the other lane is free");
}

#[test]
fn the_config_lane_keeps_its_earlier_behavior() {
    let mut fixture = Fixture::deployed();
    fixture.accept_updates(4);
    open_allocation(&mut fixture);

    let administrator = fixture.administrator.insecure_clone();
    fixture
        .advance_watermark(&administrator, MessageClass::ConfigUpdate, 2)
        .expect("an open allocation does not touch the config lane");
}

#[test]
fn an_asset_lane_requires_the_position() {
    let mut fixture = Fixture::deployed();
    let sequence = open_allocation(&mut fixture);

    let administrator = fixture.administrator.insecure_clone();
    let instruction = fixture.watermark_instruction(
        administrator.pubkey(),
        fixture.lane_key(MessageClass::Allocate),
        None,
        MessageClass::Allocate,
        sequence,
    );
    expect_error(
        fixture.send(instruction, &[&administrator]),
        RemoteLegError::InvalidRemotePosition,
    );
}

#[test]
fn a_record_left_by_a_finished_transfer_closes_and_stays_unreplayable() {
    let mut fixture = Fixture::deployed();
    let sequence = fixture.accept_allocation_at(ALLOCATE_TRANSFER_ID, AMOUNT, Some(OPEN_SEQUENCE));
    fixture.credit(fixture.custody, AMOUNT);
    fixture
        .attribute(ALLOCATE_TRANSFER_ID)
        .expect("attribution lands");

    fixture.set_lane_highest(MessageClass::Allocate, sequence + 8);
    let administrator = fixture.administrator.insecure_clone();
    fixture
        .advance_watermark(&administrator, MessageClass::Allocate, sequence + 1)
        .expect("watermark advances");

    let record = fixture.asset_record_key(MessageClass::Allocate, sequence);
    let outsider = fixture.outsider.insecure_clone();
    let instruction = fixture.close_record_instruction(
        administrator.pubkey(),
        fixture.lane_key(MessageClass::Allocate),
        record,
    );
    fixture
        .send_as(instruction, &outsider, &[&outsider])
        .expect("record closes");
    assert!(fixture.svm.get_account(&record).is_none_or(
        |account| account.data.is_empty() || account.owner != solevm_remote_leg::ID
    ));

    // The watermark keeps the closed sequence out of the lane.
    let replay = [0x99; 32];
    let bytes = fixture.allocate_bytes(replay, AMOUNT, sequence);
    expect_error(
        fixture.allocate(replay, sequence, bytes),
        RemoteLegError::SequenceBelowWatermark,
    );
}
