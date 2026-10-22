//! Resolutions that admit a quarantined transfer on its original terms.
//!
//! A resolution is consumed in order whatever it finds. It changes state only
//! when it names the exact quarantined cycle and that cycle's terms still hold.

#![allow(clippy::unwrap_used, clippy::panic, clippy::arithmetic_side_effects)]

mod common;

use common::messages::{MessageBuilder, patch_body};
use common::{
    ALLOCATE_TRANSFER_ID, Fixture, MAX_REMOTE_PRINCIPAL, RECALL_TRANSFER_ID, START_TIMESTAMP,
    expect_error, last_event,
};
use protocol_types::{AssetAmount, layout};
use solana_signer::Signer;
use solevm_remote_leg::{
    MessageClass, RecallCustodyReserved, RemoteLegError, ResolveOutcome, ResolveProcessed,
    TransferKind, TransferStatus,
};

const ID: [u8; 32] = ALLOCATE_TRANSFER_ID;
const RECALL_ID: [u8; 32] = RECALL_TRANSFER_ID;
const AMOUNT: u64 = 1_000_000;
const STALE: u64 = START_TIMESTAMP as u64 - 1;

fn stale_allocation(fixture: &Fixture, transfer_id: [u8; 32], amount: u64) -> Vec<u8> {
    let lane = fixture.control_lane();
    MessageBuilder::allocate()
        .sequence(lane.highest_consumed_sequence + 1)
        .previous_commitment(lane.message_commitment)
        .transfer_id(transfer_id)
        .allocate_body(|body| {
            body.amount = AssetAmount::new(u128::from(amount));
            body.minimum_destination_amount = AssetAmount::new(u128::from(amount));
        })
        .expires_at(STALE)
        .encode()
}

fn stale_recall(fixture: &Fixture, transfer_id: [u8; 32], amount: u64) -> Vec<u8> {
    let lane = fixture.control_lane();
    MessageBuilder::recall()
        .sequence(lane.highest_consumed_sequence + 1)
        .previous_commitment(lane.message_commitment)
        .transfer_id(transfer_id)
        .recall_body(|body| {
            body.requested_amount = AssetAmount::new(u128::from(amount));
            body.minimum_return_amount = AssetAmount::new(u128::from(amount));
        })
        .expires_at(STALE)
        .encode()
}

/// A leg holding one quarantined allocation at sequence one.
fn quarantined_allocation() -> Fixture {
    let mut fixture = Fixture::deployed();
    let bytes = stale_allocation(&fixture, ID, AMOUNT);
    fixture
        .allocate(ID, 1, bytes)
        .expect("allocation is consumed");
    fixture
}

fn processed(fixture: &Fixture) -> ResolveProcessed {
    last_event(&fixture.last_logs())
}

// Admission

#[test]
fn an_admitted_allocation_attributes_on_its_original_terms() {
    let mut fixture = quarantined_allocation();
    fixture.credit(fixture.custody, AMOUNT);

    fixture.admit(ID, 1).expect("resolution lands");
    let event = processed(&fixture);
    assert_eq!(event.outcome, ResolveOutcome::Admitted);
    assert_eq!(event.transfer_kind, TransferKind::Allocate);
    assert_eq!(event.transfer_id, ID);
    assert_eq!(event.target_sequence, 1);
    assert_eq!(event.sequence, 2);
    assert!(!event.guardian_cosigned);
    assert_eq!(fixture.transfer(&ID).status, TransferStatus::Pending);
    assert_eq!(
        fixture.position().active_transfer_status,
        TransferStatus::Pending
    );
    assert_eq!(fixture.position().attributed_principal, 0);

    fixture.attribute(ID).expect("attribution lands");
    let record = fixture.transfer(&ID);
    assert_eq!(record.status, TransferStatus::Complete);
    assert_eq!(record.attributed_amount, AMOUNT);
    assert_eq!(record.authorized_amount, AMOUNT);
    let position = fixture.position();
    assert_eq!(position.attributed_principal, AMOUNT);
    assert_eq!(position.latest_completed_transfer_id, ID);
    assert!(!position.has_active_transfer());
}

#[test]
fn an_admitted_recall_reserves_custody_like_a_fresh_one() {
    let mut fixture = Fixture::deployed();
    fixture.fund_position(ID, AMOUNT);
    let sequence = fixture.control_lane().highest_consumed_sequence + 1;
    let bytes = stale_recall(&fixture, RECALL_ID, AMOUNT);
    fixture
        .recall(RECALL_ID, sequence, bytes)
        .expect("recall is consumed");
    assert_eq!(fixture.position().recalled_custody, 0);

    fixture
        .admit(RECALL_ID, sequence)
        .expect("resolution lands");
    assert_eq!(processed(&fixture).outcome, ResolveOutcome::Admitted);
    let reserved: RecallCustodyReserved = last_event(&fixture.last_logs());
    assert_eq!(reserved.reserved_amount, AMOUNT);
    assert_eq!(reserved.remaining_principal, 0);
    let position = fixture.position();
    assert_eq!(position.attributed_principal, 0);
    assert_eq!(position.recalled_custody, AMOUNT);
    assert_eq!(
        fixture.transfer(&RECALL_ID).custody_principal_reserved,
        AMOUNT
    );

    fixture.send_recall(RECALL_ID, AMOUNT).expect("send lands");
    assert_eq!(
        fixture.transfer(&RECALL_ID).status,
        TransferStatus::Complete
    );
    assert_eq!(fixture.token_amount(fixture.escrow), AMOUNT);
}

// Resolutions that change nothing

#[test]
fn a_resolution_naming_another_transfer_changes_nothing() {
    let mut fixture = quarantined_allocation();
    let other = [0x79; 32];
    let bytes = fixture.resolve_bytes(other, 1, 2);
    fixture.resolve(other, 2, bytes).expect("resolution lands");

    let event = processed(&fixture);
    assert_eq!(event.outcome, ResolveOutcome::NotQuarantined);
    assert_eq!(event.transfer_kind, TransferKind::None);
    assert_eq!(fixture.control_lane().highest_consumed_sequence, 2);
    assert_eq!(fixture.record(2).message_class, MessageClass::Resolve);
    assert_eq!(fixture.transfer(&ID).status, TransferStatus::Quarantined);
    assert_eq!(
        fixture.position().active_transfer_status,
        TransferStatus::Quarantined
    );
}

#[test]
fn a_resolution_naming_the_wrong_sequence_changes_nothing() {
    let mut fixture = quarantined_allocation();
    let other = fixture.resolve_bytes([0x79; 32], 1, 2);
    fixture
        .resolve([0x79; 32], 2, other)
        .expect("first resolution lands");

    let bytes = fixture.resolve_bytes(ID, 2, 3);
    fixture.resolve(ID, 3, bytes).expect("resolution lands");
    assert_eq!(processed(&fixture).outcome, ResolveOutcome::NotQuarantined);
    assert_eq!(fixture.transfer(&ID).status, TransferStatus::Quarantined);
}

#[test]
fn a_resolution_for_a_transfer_that_is_already_running_changes_nothing() {
    let mut fixture = Fixture::deployed();
    let sequence = fixture.accept_allocation(ID, AMOUNT);
    let position_before = fixture.raw_data(fixture.position_key());
    let record_before = fixture.raw_data(fixture.transfer_key(&ID));

    fixture.admit(ID, sequence).expect("resolution lands");
    assert_eq!(processed(&fixture).outcome, ResolveOutcome::NotQuarantined);
    assert_eq!(fixture.raw_data(fixture.position_key()), position_before);
    assert_eq!(fixture.raw_data(fixture.transfer_key(&ID)), record_before);
}

#[test]
fn an_expired_resolution_is_consumed_and_a_fresh_one_still_admits() {
    let mut fixture = quarantined_allocation();
    let lane = fixture.control_lane();
    let bytes = MessageBuilder::resolve()
        .sequence(2)
        .previous_commitment(lane.message_commitment)
        .expires_at(STALE)
        .encode();
    fixture
        .resolve(ID, 2, bytes)
        .expect("resolution is consumed");
    assert_eq!(processed(&fixture).outcome, ResolveOutcome::Expired);
    assert_eq!(fixture.transfer(&ID).status, TransferStatus::Quarantined);

    fixture.admit(ID, 1).expect("fresh resolution lands");
    assert_eq!(processed(&fixture).outcome, ResolveOutcome::Admitted);
}

#[test]
fn an_allocation_still_over_the_ceiling_stays_quarantined() {
    let mut fixture = Fixture::deployed();
    let bytes = fixture.allocate_bytes(ID, MAX_REMOTE_PRINCIPAL + 1, 1);
    fixture
        .allocate(ID, 1, bytes)
        .expect("allocation is consumed");
    let record_before = fixture.raw_data(fixture.transfer_key(&ID));

    fixture.admit(ID, 1).expect("resolution lands");
    assert_eq!(processed(&fixture).outcome, ResolveOutcome::AllocationLimit);
    assert_eq!(fixture.raw_data(fixture.transfer_key(&ID)), record_before);
    assert_eq!(
        fixture.position().active_transfer_status,
        TransferStatus::Quarantined
    );
}

#[test]
fn a_recall_for_more_than_the_accepted_principal_stays_quarantined() {
    let mut fixture = Fixture::deployed();
    fixture.fund_position(ID, AMOUNT);
    let sequence = fixture.control_lane().highest_consumed_sequence + 1;
    let bytes = stale_recall(&fixture, RECALL_ID, AMOUNT + 1);
    fixture
        .recall(RECALL_ID, sequence, bytes)
        .expect("recall is consumed");

    fixture
        .admit(RECALL_ID, sequence)
        .expect("resolution lands");
    assert_eq!(
        processed(&fixture).outcome,
        ResolveOutcome::InsufficientPrincipal
    );
    let position = fixture.position();
    assert_eq!(position.attributed_principal, AMOUNT);
    assert_eq!(position.recalled_custody, 0);
    assert_eq!(position.active_transfer_status, TransferStatus::Quarantined);
}

// Authority on a frozen leg

#[test]
fn a_frozen_leg_admits_only_with_the_guardian() {
    let mut fixture = Fixture::deployed();
    let administrator = fixture.administrator.insecure_clone();
    fixture.freeze(&administrator).expect("freeze lands");
    let bytes = fixture.allocate_bytes(ID, AMOUNT, 1);
    fixture
        .allocate(ID, 1, bytes)
        .expect("allocation is consumed");

    expect_error(fixture.admit(ID, 1), RemoteLegError::Frozen);
    assert_eq!(fixture.control_lane().highest_consumed_sequence, 1);

    let outsider = fixture.outsider.insecure_clone();
    expect_error(
        fixture.admit_cosigned(ID, 1, &outsider),
        RemoteLegError::Unauthorized,
    );

    let guardian = fixture.guardian.insecure_clone();
    fixture
        .admit_cosigned(ID, 1, &guardian)
        .expect("guardian resolution lands");
    let event = processed(&fixture);
    assert_eq!(event.outcome, ResolveOutcome::Admitted);
    assert!(event.guardian_cosigned);
    assert_eq!(fixture.transfer(&ID).status, TransferStatus::Pending);
}

#[test]
fn a_frozen_leg_consumes_a_resolution_that_changes_nothing_without_the_guardian() {
    let mut fixture = Fixture::deployed();
    let administrator = fixture.administrator.insecure_clone();
    fixture.freeze(&administrator).expect("freeze lands");
    let bytes = fixture.allocate_bytes(ID, AMOUNT, 1);
    fixture
        .allocate(ID, 1, bytes)
        .expect("allocation is consumed");

    let other = [0x79; 32];
    let bytes = fixture.resolve_bytes(other, 1, 2);
    fixture.resolve(other, 2, bytes).expect("resolution lands");
    assert_eq!(processed(&fixture).outcome, ResolveOutcome::NotQuarantined);
    assert_eq!(fixture.transfer(&ID).status, TransferStatus::Quarantined);
}

/// Tokens sent to a leg that froze in flight come home only with the guardian.
#[test]
fn a_frozen_leg_returns_late_allocated_tokens_through_the_guardian() {
    let mut fixture = Fixture::deployed();
    let administrator = fixture.administrator.insecure_clone();
    let guardian = fixture.guardian.insecure_clone();
    fixture.freeze(&administrator).expect("freeze lands");

    let bytes = fixture.allocate_bytes(ID, AMOUNT, 1);
    fixture
        .allocate(ID, 1, bytes)
        .expect("allocation is consumed");
    fixture.credit(fixture.custody, AMOUNT);

    fixture
        .admit_cosigned(ID, 1, &guardian)
        .expect("guardian admits");
    expect_error(fixture.attribute(ID), RemoteLegError::Frozen);
    fixture
        .attribute_cosigned(ID, &guardian)
        .expect("guardian attributes");
    assert_eq!(fixture.transfer(&ID).status, TransferStatus::Complete);
    assert_eq!(fixture.position().attributed_principal, AMOUNT);

    let sequence = fixture.control_lane().highest_consumed_sequence + 1;
    let bytes = fixture.recall_bytes(RECALL_ID, AMOUNT, sequence);
    expect_error(
        fixture.recall(RECALL_ID, sequence, bytes.clone()),
        RemoteLegError::Frozen,
    );
    fixture
        .recall_cosigned(RECALL_ID, sequence, bytes, &guardian)
        .expect("guardian recalls");
    fixture.send_recall(RECALL_ID, AMOUNT).expect("send lands");

    assert_eq!(
        fixture.transfer(&RECALL_ID).status,
        TransferStatus::Complete
    );
    assert_eq!(fixture.token_amount(fixture.escrow), AMOUNT);
    assert_eq!(fixture.token_amount(fixture.custody), 0);
    let position = fixture.position();
    assert_eq!(position.accepted_principal().unwrap(), 0);
    assert_eq!(position.accounted_custody().unwrap(), 0);
    assert!(!position.has_active_transfer());
}

// Integrity

#[test]
fn a_resolution_cannot_be_replayed() {
    let mut fixture = quarantined_allocation();
    let bytes = fixture.resolve_bytes(ID, 1, 2);
    fixture
        .resolve(ID, 2, bytes.clone())
        .expect("resolution lands");

    expect_error(
        fixture.resolve(ID, 2, bytes),
        RemoteLegError::InvalidSequence,
    );
    assert_eq!(fixture.control_lane().highest_consumed_sequence, 2);
}

#[test]
fn a_resolution_with_a_broken_commitment_reverts() {
    let mut fixture = quarantined_allocation();
    let bytes = MessageBuilder::resolve()
        .sequence(2)
        .previous_commitment([0xEE; 32])
        .encode();

    expect_error(
        fixture.resolve(ID, 2, bytes),
        RemoteLegError::InvalidPreviousCommitment,
    );
    assert_eq!(fixture.transfer(&ID).status, TransferStatus::Quarantined);
}

#[test]
fn a_resolution_from_an_outsider_reverts() {
    let mut fixture = quarantined_allocation();
    let bytes = fixture.resolve_bytes(ID, 1, 2);
    let outsider = fixture.outsider.insecure_clone();
    let mut instruction = fixture.resolve_instruction(ID, 2, bytes, None);
    instruction.accounts[0].pubkey = outsider.pubkey();
    let administrator = fixture.administrator.insecure_clone();

    expect_error(
        fixture.send_as(instruction, &administrator, &[&administrator, &outsider]),
        RemoteLegError::Unauthorized,
    );
    assert_eq!(fixture.transfer(&ID).status, TransferStatus::Quarantined);
}

#[test]
fn a_resolution_bound_to_another_record_reverts() {
    let mut fixture = quarantined_allocation();
    let bytes = fixture.resolve_bytes(ID, 1, 2);
    let mut instruction = fixture.resolve_instruction(ID, 2, bytes, None);
    let transfer_record = fixture.transfer_key(&ID);
    let decoy = fixture.transfer_key(&[0x79; 32]);
    for meta in &mut instruction.accounts {
        if meta.pubkey == transfer_record {
            meta.pubkey = decoy;
        }
    }
    let administrator = fixture.administrator.insecure_clone();
    let verifier = fixture.verifier.insecure_clone();

    expect_error(
        fixture.send_as(instruction, &administrator, &[&administrator, &verifier]),
        RemoteLegError::InvalidTransferRecord,
    );
    assert_eq!(fixture.transfer(&ID).status, TransferStatus::Quarantined);
}

#[test]
fn a_resolution_with_an_unknown_action_reverts() {
    let mut fixture = quarantined_allocation();
    let mut bytes = fixture.resolve_bytes(ID, 1, 2);
    patch_body(&mut bytes, layout::RESOLVE_ACTION_OFFSET, &[4]);

    expect_error(
        fixture.resolve(ID, 2, bytes),
        RemoteLegError::InvalidMessage,
    );
    assert_eq!(fixture.control_lane().highest_consumed_sequence, 1);
}

/// A close or recovery must never be consumed as a no-op on the admission
/// path, or the vault would wait for a settlement that never comes.
#[test]
fn a_close_or_recovery_on_the_admission_path_reverts_unconsumed() {
    for action in [2u8, 3] {
        let mut fixture = quarantined_allocation();
        let mut bytes = fixture.resolve_bytes(ID, 1, 2);
        patch_body(&mut bytes, layout::RESOLVE_ACTION_OFFSET, &[action]);

        expect_error(
            fixture.resolve(ID, 2, bytes),
            RemoteLegError::WrongResolveInstruction,
        );
        assert_eq!(fixture.control_lane().highest_consumed_sequence, 1);
        assert_eq!(fixture.transfer(&ID).status, TransferStatus::Quarantined);
    }
}

#[test]
fn a_resolution_delivered_as_an_allocation_reverts() {
    let mut fixture = quarantined_allocation();
    let bytes = fixture.resolve_bytes(ID, 1, 2);

    expect_error(
        fixture.allocate([0x79; 32], 2, bytes),
        RemoteLegError::UnsupportedMessageType,
    );
    assert_eq!(fixture.control_lane().highest_consumed_sequence, 1);
}
