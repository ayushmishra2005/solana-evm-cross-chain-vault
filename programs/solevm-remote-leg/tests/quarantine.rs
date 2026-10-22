//! Authentic, in order messages that may not run are consumed without effect.
//!
//! Every integrity rule still rejects first. Only a message that is genuine
//! and next on the lane can ever be held back.

#![allow(clippy::unwrap_used, clippy::panic, clippy::arithmetic_side_effects)]

mod common;

use common::messages::MessageBuilder;
use common::{
    ALLOCATE_TRANSFER_ID, Fixture, RECALL_TRANSFER_ID, SOURCE_CHAIN_ID, START_TIMESTAMP,
    expect_error,
};
use protocol_types::AssetAmount;
use solana_signer::Signer;
use solevm_remote_leg::{QuarantineReason, RemoteLegError, TransferKind};

const ID: [u8; 32] = ALLOCATE_TRANSFER_ID;
const AMOUNT: u64 = 1_000_000;
/// Already past when every fixture starts, yet after publication.
const STALE: u64 = START_TIMESTAMP as u64 - 1;

/// The next allocation on the lane, before any field is bent.
fn allocation(fixture: &Fixture, transfer_id: [u8; 32], amount: u64) -> MessageBuilder {
    let lane = fixture.control_lane();
    MessageBuilder::allocate()
        .sequence(lane.highest_consumed_sequence + 1)
        .previous_commitment(lane.message_commitment)
        .transfer_id(transfer_id)
        .allocate_body(|body| {
            body.amount = AssetAmount::new(u128::from(amount));
            body.minimum_destination_amount = AssetAmount::new(u128::from(amount));
        })
}

/// A leg holding one quarantined allocation at sequence one.
fn quarantined() -> Fixture {
    let mut fixture = Fixture::deployed();
    let bytes = allocation(&fixture, ID, AMOUNT).expires_at(STALE).encode();
    fixture
        .allocate(ID, 1, bytes)
        .expect("allocation is consumed");
    fixture
}

#[test]
fn an_expired_allocation_is_held_on_its_original_terms() {
    let fixture = quarantined();
    fixture.expect_quarantined(ID, 1, TransferKind::Allocate, QuarantineReason::Expired);

    let record = fixture.transfer(&ID);
    assert_eq!(record.authorized_amount, AMOUNT);
    assert_eq!(record.minimum_amount, AMOUNT);
    assert_eq!(record.expected_source_balance, 5_000_000);
}

#[test]
fn tokens_for_a_quarantined_allocation_stay_unattributed() {
    let mut fixture = quarantined();
    fixture.credit(fixture.custody, AMOUNT);
    fixture.reconcile().expect("reconciliation lands");

    expect_error(fixture.attribute(ID), RemoteLegError::InvalidTransferStatus);
    let position = fixture.position();
    assert_eq!(position.unattributed_custody, AMOUNT);
    assert_eq!(position.attributed_principal, 0);
    assert_eq!(position.accepted_principal().unwrap(), 0);
    assert_eq!(fixture.transfer(&ID).attributed_amount, 0);
}

#[test]
fn a_quarantined_cycle_blocks_the_next_transfer() {
    let mut fixture = quarantined();
    let second = [0x79; 32];
    let bytes = allocation(&fixture, second, AMOUNT).encode();

    expect_error(
        fixture.allocate(second, 2, bytes),
        RemoteLegError::UnresolvedCycle,
    );
    assert_eq!(fixture.control_lane().highest_consumed_sequence, 1);
    assert!(!fixture.transfer_exists(&second));
}

#[test]
fn a_quarantined_cycle_holds_the_watermark_at_its_sequence() {
    let mut fixture = quarantined();
    let other = [0x79; 32];
    for sequence in 2..=4 {
        let bytes = fixture.resolve_bytes(other, 1, sequence);
        fixture
            .resolve(other, sequence, bytes)
            .expect("a no-op resolution lands");
    }

    let administrator = fixture.administrator.insecure_clone();
    expect_error(
        fixture.advance_watermark(&administrator, 2),
        RemoteLegError::FinancialObligationBlocksWatermark,
    );
    assert!(fixture.record_exists(1));
    assert_eq!(
        fixture.position().active_transfer_status,
        solevm_remote_leg::TransferStatus::Quarantined
    );
}

#[test]
fn an_expired_allocation_with_a_broken_commitment_still_reverts() {
    let mut fixture = Fixture::deployed();
    let bytes = allocation(&fixture, ID, AMOUNT)
        .previous_commitment([0xEE; 32])
        .expires_at(STALE)
        .encode();

    expect_error(
        fixture.allocate(ID, 1, bytes),
        RemoteLegError::InvalidPreviousCommitment,
    );
    assert_eq!(fixture.control_lane().highest_consumed_sequence, 0);
    assert!(!fixture.transfer_exists(&ID));
}

#[test]
fn an_expired_allocation_out_of_sequence_still_reverts() {
    let mut fixture = Fixture::deployed();
    let bytes = allocation(&fixture, ID, AMOUNT)
        .sequence(2)
        .expires_at(STALE)
        .encode();

    expect_error(
        fixture.allocate(ID, 2, bytes),
        RemoteLegError::InvalidSequence,
    );
    assert!(!fixture.transfer_exists(&ID));
}

#[test]
fn an_expired_allocation_from_another_chain_still_reverts() {
    let mut fixture = Fixture::deployed();
    let bytes = allocation(&fixture, ID, AMOUNT)
        .source_chain(SOURCE_CHAIN_ID + 1)
        .expires_at(STALE)
        .encode();

    expect_error(
        fixture.allocate(ID, 1, bytes),
        RemoteLegError::InvalidSourceDomain,
    );
    assert!(!fixture.transfer_exists(&ID));
}

#[test]
fn an_expired_allocation_from_an_outsider_still_reverts() {
    let mut fixture = Fixture::deployed();
    let bytes = allocation(&fixture, ID, AMOUNT).expires_at(STALE).encode();
    let mut accounts = fixture.allocate_accounts(ID, 1);
    accounts.transport_verifier = fixture.outsider.pubkey();

    expect_error(
        fixture.allocate_with(accounts, bytes),
        RemoteLegError::Unauthorized,
    );
    assert_eq!(fixture.control_lane().highest_consumed_sequence, 0);
}

#[test]
fn an_expired_allocation_to_a_frozen_leg_reports_its_expiry() {
    let mut fixture = Fixture::deployed();
    let administrator = fixture.administrator.insecure_clone();
    fixture.freeze(&administrator).expect("freeze lands");

    let bytes = allocation(&fixture, ID, AMOUNT).expires_at(STALE).encode();
    fixture
        .allocate(ID, 1, bytes)
        .expect("allocation is consumed");
    fixture.expect_quarantined(ID, 1, TransferKind::Allocate, QuarantineReason::Expired);
}

#[test]
fn an_expired_recall_on_a_frozen_leg_still_needs_the_guardian() {
    let mut fixture = Fixture::deployed();
    fixture.fund_position(ID, AMOUNT);
    let administrator = fixture.administrator.insecure_clone();
    fixture.freeze(&administrator).expect("freeze lands");

    let lane = fixture.control_lane();
    let sequence = lane.highest_consumed_sequence + 1;
    let bytes = MessageBuilder::recall()
        .sequence(sequence)
        .previous_commitment(lane.message_commitment)
        .recall_body(|body| {
            body.requested_amount = AssetAmount::new(u128::from(AMOUNT));
            body.minimum_return_amount = AssetAmount::new(u128::from(AMOUNT));
        })
        .expires_at(STALE)
        .encode();

    expect_error(
        fixture.recall(RECALL_TRANSFER_ID, sequence, bytes.clone()),
        RemoteLegError::Frozen,
    );
    assert_eq!(
        fixture.control_lane().highest_consumed_sequence,
        sequence - 1
    );

    let guardian = fixture.guardian.insecure_clone();
    fixture
        .recall_cosigned(RECALL_TRANSFER_ID, sequence, bytes, &guardian)
        .expect("recall is consumed");
    fixture.expect_quarantined(
        RECALL_TRANSFER_ID,
        sequence,
        TransferKind::Recall,
        QuarantineReason::Expired,
    );
    assert_eq!(fixture.position().attributed_principal, AMOUNT);
    assert_eq!(fixture.position().recalled_custody, 0);
}
