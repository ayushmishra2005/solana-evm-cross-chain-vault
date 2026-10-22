//! The watermark may not step over an unresolved transfer.

#![allow(clippy::unwrap_used, clippy::panic, clippy::arithmetic_side_effects)]

mod common;

use anchor_lang::error::ErrorCode;
use common::{
    ALLOCATE_TRANSFER_ID, Fixture, Pubkey, RECALL_TRANSFER_ID, expect_anchor_error, expect_error,
};
use solana_signer::Signer;
use solevm_remote_leg::RemoteLegError;

const AMOUNT: u64 = 100;

/// Opens an allocation and lifts the shared lane above it.
///
/// Lifting the lane isolates the obligation guard from the bounds and lag
/// rules, which would otherwise reject first.
fn open_allocation(fixture: &mut Fixture) -> u64 {
    // Leave the starting watermark at 1 so the open transfer sits above it.
    fixture.accept_updates(2);
    let sequence = fixture.accept_allocation(ALLOCATE_TRANSFER_ID, AMOUNT);
    fixture.set_lane_highest(sequence + 8);
    sequence
}

/// Opens a recall the same way, funding the position first.
fn open_recall(fixture: &mut Fixture) -> u64 {
    fixture.fund_position([0x70; 32], AMOUNT);
    let sequence = fixture.accept_recall(RECALL_TRANSFER_ID, AMOUNT);
    fixture.set_lane_highest(sequence + 8);
    sequence
}

#[test]
fn an_open_transfer_always_sits_at_the_top_of_its_lane() {
    let mut fixture = Fixture::deployed();
    fixture.accept_allocation(ALLOCATE_TRANSFER_ID, AMOUNT);

    let position = fixture.position();
    let lane = fixture.control_lane();
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
        fixture.advance_watermark(&administrator, sequence + 1),
        RemoteLegError::FinancialObligationBlocksWatermark,
    );
    assert_eq!(fixture.control_lane().minimum_acceptable_sequence, 1);
}

#[test]
fn the_watermark_may_reach_an_open_allocation() {
    let mut fixture = Fixture::deployed();
    let sequence = open_allocation(&mut fixture);

    let administrator = fixture.administrator.insecure_clone();
    fixture
        .advance_watermark(&administrator, sequence)
        .expect("reaching the open sequence is allowed");
    assert_eq!(fixture.control_lane().minimum_acceptable_sequence, sequence);
}

#[test]
fn the_watermark_may_not_pass_an_open_recall() {
    let mut fixture = Fixture::deployed();
    let sequence = open_recall(&mut fixture);

    let administrator = fixture.administrator.insecure_clone();
    expect_error(
        fixture.advance_watermark(&administrator, sequence + 1),
        RemoteLegError::FinancialObligationBlocksWatermark,
    );
}

#[test]
fn the_watermark_may_reach_an_open_recall() {
    let mut fixture = Fixture::deployed();
    let sequence = open_recall(&mut fixture);

    let administrator = fixture.administrator.insecure_clone();
    fixture
        .advance_watermark(&administrator, sequence)
        .expect("reaching the open sequence is allowed");
}

#[test]
fn the_watermark_moves_once_the_allocation_completes() {
    let mut fixture = Fixture::deployed();
    let sequence = open_allocation(&mut fixture);

    let administrator = fixture.administrator.insecure_clone();
    expect_error(
        fixture.advance_watermark(&administrator, sequence + 1),
        RemoteLegError::FinancialObligationBlocksWatermark,
    );

    fixture.credit(fixture.custody, AMOUNT);
    fixture
        .attribute(ALLOCATE_TRANSFER_ID)
        .expect("attribution lands");
    fixture
        .advance_watermark(&administrator, sequence + 1)
        .expect("a closed cycle no longer blocks");
}

#[test]
fn the_watermark_moves_once_the_recall_completes() {
    let mut fixture = Fixture::deployed();
    let sequence = open_recall(&mut fixture);

    let administrator = fixture.administrator.insecure_clone();
    expect_error(
        fixture.advance_watermark(&administrator, sequence + 1),
        RemoteLegError::FinancialObligationBlocksWatermark,
    );

    fixture
        .send_recall(RECALL_TRANSFER_ID, AMOUNT)
        .expect("send lands");
    fixture
        .advance_watermark(&administrator, sequence + 1)
        .expect("a closed cycle no longer blocks");
}

#[test]
fn an_open_allocation_blocks_the_shared_control_lane() {
    let mut fixture = Fixture::deployed();
    let sequence = open_allocation(&mut fixture);

    let administrator = fixture.administrator.insecure_clone();
    expect_error(
        fixture.advance_watermark(&administrator, sequence + 1),
        RemoteLegError::FinancialObligationBlocksWatermark,
    );
}

#[test]
fn the_watermark_may_move_below_an_open_transfer() {
    let mut fixture = Fixture::deployed();
    fixture.accept_updates(4);
    let sequence = open_allocation(&mut fixture);

    let administrator = fixture.administrator.insecure_clone();
    fixture
        .advance_watermark(&administrator, 2)
        .expect("a watermark below the open sequence is still allowed");
    expect_error(
        fixture.advance_watermark(&administrator, sequence + 1),
        RemoteLegError::FinancialObligationBlocksWatermark,
    );
}

#[test]
fn a_foreign_position_cannot_authorize_a_watermark() {
    let mut fixture = Fixture::deployed();
    let sequence = open_allocation(&mut fixture);

    let administrator = fixture.administrator.insecure_clone();
    let instruction = fixture.watermark_instruction(
        administrator.pubkey(),
        fixture.control_lane_key(),
        Pubkey::new_unique(),
        sequence,
    );
    expect_anchor_error(
        fixture.send(instruction, &[&administrator]),
        ErrorCode::AccountNotInitialized,
    );
}

#[test]
fn the_placeholder_for_an_absent_position_cannot_skip_the_obligation() {
    let mut fixture = Fixture::deployed();
    let sequence = open_allocation(&mut fixture);

    // Anchor reads the program id in an optional slot as an absent account.
    let administrator = fixture.administrator.insecure_clone();
    let instruction = fixture.watermark_instruction(
        administrator.pubkey(),
        fixture.control_lane_key(),
        solevm_remote_leg::ID,
        sequence + 1,
    );
    expect_anchor_error(
        fixture.send(instruction, &[&administrator]),
        ErrorCode::AccountOwnedByWrongProgram,
    );
    assert_eq!(fixture.control_lane().minimum_acceptable_sequence, 1);
}

#[test]
fn a_watermark_without_the_position_is_rejected() {
    let mut fixture = Fixture::deployed();
    let sequence = open_allocation(&mut fixture);

    let administrator = fixture.administrator.insecure_clone();
    let mut instruction = fixture.watermark_instruction(
        administrator.pubkey(),
        fixture.control_lane_key(),
        fixture.position_key(),
        sequence + 1,
    );
    instruction.accounts.pop();
    expect_anchor_error(
        fixture.send(instruction, &[&administrator]),
        ErrorCode::AccountNotEnoughKeys,
    );
    assert_eq!(fixture.control_lane().minimum_acceptable_sequence, 1);
}

#[test]
fn a_record_left_by_a_finished_transfer_closes_and_stays_unreplayable() {
    let mut fixture = Fixture::deployed();
    let sequence = fixture.accept_allocation(ALLOCATE_TRANSFER_ID, AMOUNT);
    fixture.credit(fixture.custody, AMOUNT);
    fixture
        .attribute(ALLOCATE_TRANSFER_ID)
        .expect("attribution lands");

    fixture.set_lane_highest(sequence + 8);
    let administrator = fixture.administrator.insecure_clone();
    fixture
        .advance_watermark(&administrator, sequence + 1)
        .expect("watermark advances");

    let record = fixture.asset_record_key(sequence);
    let outsider = fixture.outsider.insecure_clone();
    let instruction = fixture.close_record_instruction(
        administrator.pubkey(),
        fixture.control_lane_key(),
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
