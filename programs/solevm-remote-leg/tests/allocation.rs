//! Accepting allocations and turning observed custody into principal.

#![allow(clippy::unwrap_used, clippy::panic, clippy::arithmetic_side_effects)]

mod common;

use common::messages::MessageBuilder;
use common::{
    ALLOCATE_TRANSFER_ID, Fixture, MAX_REMOTE_PRINCIPAL, Pubkey, RECALL_TRANSFER_ID, expect_error,
};
use protocol_types::{AssetAmount, ConfigVersion};
use solevm_remote_leg::{MessageClass, RemoteLegError, TransferKind, TransferStatus};

const ID: [u8; 32] = ALLOCATE_TRANSFER_ID;
const AMOUNT: u64 = 1_000_000;

/// A valid allocation message for the next sequence of the allocate lane.
fn next_allocate(fixture: &Fixture, transfer_id: [u8; 32], amount: u64) -> (u64, Vec<u8>) {
    let lane = fixture.lane(MessageClass::Allocate);
    let sequence = lane.highest_consumed_sequence + 1;
    let bytes = MessageBuilder::allocate()
        .sequence(sequence)
        .previous_commitment(lane.message_commitment)
        .transfer_id(transfer_id)
        .allocate_body(|body| {
            body.amount = AssetAmount::new(u128::from(amount));
            body.minimum_destination_amount = AssetAmount::new(u128::from(amount));
        })
        .encode();
    (sequence, bytes)
}

// Accepting the message

#[test]
fn a_valid_allocation_opens_an_unresolved_cycle() {
    let mut fixture = Fixture::deployed();
    let (sequence, bytes) = next_allocate(&fixture, ID, AMOUNT);
    fixture
        .allocate(ID, sequence, bytes)
        .expect("allocate lands");

    let record = fixture.transfer(&ID);
    assert_eq!(record.transfer_kind, TransferKind::Allocate);
    assert_eq!(record.message_class, MessageClass::Allocate);
    assert_eq!(record.status, TransferStatus::Pending);
    assert_eq!(record.authorized_amount, AMOUNT);
    assert_eq!(record.attributed_amount, 0);
    assert_eq!(record.message_sequence, sequence);

    let position = fixture.position();
    assert_eq!(position.active_transfer_kind, TransferKind::Allocate);
    assert_eq!(position.active_transfer_id, ID);
    assert_eq!(position.active_transfer_sequence, sequence);
}

#[test]
fn accepting_a_message_alone_never_increases_principal() {
    let mut fixture = Fixture::deployed();
    fixture.accept_allocation(ID, AMOUNT);

    let position = fixture.position();
    assert_eq!(position.attributed_principal, 0);
    assert_eq!(position.deployed_principal, 0);
    assert_eq!(position.unattributed_custody, 0);
    assert_eq!(fixture.token_amount(fixture.custody), 0);
}

#[test]
fn the_allocate_lane_advances_its_commitment() {
    let mut fixture = Fixture::deployed();
    let before = fixture.lane(MessageClass::Allocate).message_commitment;
    fixture.accept_allocation(ID, AMOUNT);

    let lane = fixture.lane(MessageClass::Allocate);
    assert_ne!(lane.message_commitment, before);
    assert_eq!(lane.highest_consumed_sequence, 1);
}

#[test]
fn the_expected_source_balance_is_stored_without_being_verified() {
    let mut fixture = Fixture::deployed();
    let lane = fixture.lane(MessageClass::Allocate);
    let bytes = MessageBuilder::allocate()
        .sequence(1)
        .previous_commitment(lane.message_commitment)
        .transfer_id(ID)
        .allocate_body(|body| {
            body.amount = AssetAmount::new(u128::from(AMOUNT));
            body.minimum_destination_amount = AssetAmount::new(u128::from(AMOUNT));
            body.expected_source_balance = AssetAmount::new(u128::MAX);
        })
        .encode();

    fixture.allocate(ID, 1, bytes).expect("allocate lands");
    assert_eq!(fixture.transfer(&ID).expected_source_balance, u128::MAX);
}

#[test]
fn a_signer_that_is_not_the_verifier_is_rejected() {
    let mut fixture = Fixture::deployed();
    let (sequence, bytes) = next_allocate(&fixture, ID, AMOUNT);

    let mut accounts = fixture.allocate_accounts(ID, sequence);
    accounts.transport_verifier = fixture.administrator_key();
    expect_error(
        fixture.allocate_with(accounts, bytes),
        RemoteLegError::Unauthorized,
    );
}

#[test]
fn a_body_config_version_out_of_step_is_rejected() {
    let mut fixture = Fixture::deployed();
    let lane = fixture.lane(MessageClass::Allocate);
    let bytes = MessageBuilder::allocate()
        .sequence(1)
        .previous_commitment(lane.message_commitment)
        .transfer_id(ID)
        .allocate_body(|body| body.config_version = ConfigVersion::new(99))
        .encode();

    expect_error(
        fixture.allocate(ID, 1, bytes),
        RemoteLegError::InvalidConfigVersion,
    );
}

#[test]
fn a_message_for_another_lane_is_rejected() {
    let mut fixture = Fixture::deployed();
    let lane = fixture.lane(MessageClass::Allocate);
    let bytes = MessageBuilder::allocate()
        .sequence(1)
        .previous_commitment(lane.message_commitment)
        .transfer_id(ID)
        .lane(9)
        .encode();

    expect_error(fixture.allocate(ID, 1, bytes), RemoteLegError::InvalidLane);
}

#[test]
fn a_wrong_previous_commitment_is_rejected() {
    let mut fixture = Fixture::deployed();
    let bytes = MessageBuilder::allocate()
        .sequence(1)
        .previous_commitment([9u8; 32])
        .transfer_id(ID)
        .encode();

    expect_error(
        fixture.allocate(ID, 1, bytes),
        RemoteLegError::InvalidPreviousCommitment,
    );
}

#[test]
fn a_recall_message_is_rejected_on_the_allocate_lane() {
    let mut fixture = Fixture::deployed();
    let bytes = MessageBuilder::recall()
        .sequence(1)
        .transfer_id(ID)
        .encode();
    expect_error(
        fixture.allocate(ID, 1, bytes),
        RemoteLegError::UnsupportedMessageType,
    );
}

// Sequence policy

#[test]
fn a_sequence_gap_is_accepted_on_the_allocate_lane() {
    let mut fixture = Fixture::deployed();
    fixture.fund_position(ID, AMOUNT);

    let second = [0x79; 32];
    let lane = fixture.lane(MessageClass::Allocate);
    let bytes = MessageBuilder::allocate()
        .sequence(7)
        .previous_commitment(lane.message_commitment)
        .transfer_id(second)
        .allocate_body(|body| {
            body.amount = AssetAmount::new(100);
            body.minimum_destination_amount = AssetAmount::new(100);
        })
        .encode();

    fixture.allocate(second, 7, bytes).expect("gap is allowed");
    assert_eq!(
        fixture
            .lane(MessageClass::Allocate)
            .highest_consumed_sequence,
        7
    );
}

#[test]
fn a_sequence_equal_to_the_highest_consumed_is_rejected() {
    let mut fixture = Fixture::deployed();
    fixture.fund_position(ID, AMOUNT);

    let second = [0x79; 32];
    let lane = fixture.lane(MessageClass::Allocate);
    let bytes = MessageBuilder::allocate()
        .sequence(1)
        .previous_commitment(lane.message_commitment)
        .transfer_id(second)
        .encode();

    expect_error(
        fixture.allocate(second, 1, bytes),
        RemoteLegError::InvalidSequence,
    );
}

#[test]
fn a_stale_sequence_becomes_unusable_after_a_newer_one_lands() {
    let mut fixture = Fixture::deployed();
    let lane = fixture.lane(MessageClass::Allocate);
    let stale = MessageBuilder::allocate()
        .sequence(2)
        .previous_commitment(lane.message_commitment)
        .transfer_id([0x7A; 32])
        .encode();

    fixture.fund_position(ID, AMOUNT);
    let second = [0x79; 32];
    let (sequence, bytes) = next_allocate(&fixture, second, 100);
    assert_eq!(sequence, 2);
    fixture.allocate(second, 2, bytes).expect("second lands");

    // Attributing frees the cycle, so only the sequence rule may reject now.
    fixture.credit(fixture.custody, 100);
    fixture.attribute(second).expect("attribution lands");
    expect_error(
        fixture.allocate([0x7A; 32], 2, stale),
        RemoteLegError::InvalidSequence,
    );
}

#[test]
fn an_exact_replay_is_rejected() {
    let mut fixture = Fixture::deployed();
    let (sequence, bytes) = next_allocate(&fixture, ID, AMOUNT);
    fixture
        .allocate(ID, sequence, bytes.clone())
        .expect("allocate lands");

    expect_error(
        fixture.allocate(ID, sequence, bytes),
        RemoteLegError::InvalidSequence,
    );
}

#[test]
fn a_sequence_below_the_watermark_is_rejected() {
    let mut fixture = Fixture::deployed();
    for index in 0u8..4 {
        let id = [0x50 + index; 32];
        fixture.fund_position(id, 100);
    }

    let administrator = fixture.administrator.insecure_clone();
    fixture
        .advance_watermark(&administrator, MessageClass::Allocate, 2)
        .expect("watermark advances");

    let bytes = MessageBuilder::allocate()
        .sequence(1)
        .transfer_id([0x7B; 32])
        .encode();
    expect_error(
        fixture.allocate([0x7B; 32], 1, bytes),
        RemoteLegError::SequenceBelowWatermark,
    );
}

// Transfer identity and the single cycle rule

#[test]
fn a_duplicate_transfer_id_is_rejected() {
    let mut fixture = Fixture::deployed();
    fixture.fund_position(ID, AMOUNT);

    let (sequence, bytes) = next_allocate(&fixture, ID, 100);
    expect_error(
        fixture.allocate(ID, sequence, bytes),
        RemoteLegError::TransferAlreadyExists,
    );
}

#[test]
fn a_second_allocation_while_one_is_unresolved_is_rejected() {
    let mut fixture = Fixture::deployed();
    fixture.accept_allocation(ID, AMOUNT);

    let second = [0x79; 32];
    let (sequence, bytes) = next_allocate(&fixture, second, 100);
    expect_error(
        fixture.allocate(second, sequence, bytes),
        RemoteLegError::UnresolvedCycle,
    );
}

#[test]
fn an_allocation_during_an_open_recall_is_rejected() {
    let mut fixture = Fixture::deployed();
    fixture.fund_position(ID, AMOUNT);
    fixture.accept_recall(RECALL_TRANSFER_ID, AMOUNT);

    let second = [0x79; 32];
    let (sequence, bytes) = next_allocate(&fixture, second, 100);
    expect_error(
        fixture.allocate(second, sequence, bytes),
        RemoteLegError::UnresolvedCycle,
    );
}

// Amount rules

#[test]
fn an_allocation_above_the_permitted_principal_is_rejected() {
    let mut fixture = Fixture::deployed();
    let (sequence, bytes) = next_allocate(&fixture, ID, MAX_REMOTE_PRINCIPAL + 1);
    expect_error(
        fixture.allocate(ID, sequence, bytes),
        RemoteLegError::RemoteAllocationLimitExceeded,
    );
}

#[test]
fn the_ceiling_counts_principal_the_leg_already_holds() {
    let mut fixture = Fixture::deployed();
    fixture.fund_position(ID, MAX_REMOTE_PRINCIPAL);

    let second = [0x79; 32];
    let (sequence, bytes) = next_allocate(&fixture, second, 1);
    expect_error(
        fixture.allocate(second, sequence, bytes),
        RemoteLegError::RemoteAllocationLimitExceeded,
    );
}

#[test]
fn an_allocation_exactly_at_the_ceiling_is_accepted() {
    let mut fixture = Fixture::deployed();
    let (sequence, bytes) = next_allocate(&fixture, ID, MAX_REMOTE_PRINCIPAL);
    fixture
        .allocate(ID, sequence, bytes)
        .expect("the ceiling is inclusive");
}

#[test]
fn a_minimum_above_the_authorized_amount_is_rejected_by_the_codec() {
    let fixture = Fixture::deployed();
    let lane = fixture.lane(MessageClass::Allocate);
    let bytes = MessageBuilder::allocate()
        .sequence(1)
        .previous_commitment(lane.message_commitment)
        .transfer_id(ID)
        .allocate_body(|body| {
            body.amount = AssetAmount::new(100);
            body.minimum_destination_amount = AssetAmount::new(101);
        })
        .build()
        .encode();
    assert!(bytes.is_err(), "the shared codec rejects the body itself");
}

#[test]
fn an_amount_above_the_token_range_is_rejected() {
    let mut fixture = Fixture::deployed();
    let lane = fixture.lane(MessageClass::Allocate);
    let bytes = MessageBuilder::allocate()
        .sequence(1)
        .previous_commitment(lane.message_commitment)
        .transfer_id(ID)
        .allocate_body(|body| {
            body.amount = AssetAmount::new(u128::from(u64::MAX) + 1);
            body.minimum_destination_amount = AssetAmount::new(1);
        })
        .encode();

    expect_error(
        fixture.allocate(ID, 1, bytes),
        RemoteLegError::AmountTooLarge,
    );
}

// Record creation safety

#[test]
fn a_prefunded_transfer_record_address_does_not_block_the_message() {
    let mut fixture = Fixture::deployed();
    let record = fixture.transfer_key(&ID);
    fixture.prefund(record, fixture.empty_account_rent());

    let (sequence, bytes) = next_allocate(&fixture, ID, AMOUNT);
    fixture
        .allocate(ID, sequence, bytes)
        .expect("lamports alone do not block");
    assert!(fixture.transfer_exists(&ID));
}

#[test]
fn a_foreign_owned_transfer_record_address_is_rejected() {
    let mut fixture = Fixture::deployed();
    let record = fixture.transfer_key(&ID);
    fixture.write_owned_account(record, vec![0u8; 8], Pubkey::new_unique());

    let (sequence, bytes) = next_allocate(&fixture, ID, AMOUNT);
    expect_error(
        fixture.allocate(ID, sequence, bytes),
        RemoteLegError::InvalidTransferRecord,
    );
}

#[test]
fn a_transfer_record_at_a_foreign_address_is_rejected() {
    let mut fixture = Fixture::deployed();
    let (sequence, bytes) = next_allocate(&fixture, ID, AMOUNT);

    let mut accounts = fixture.allocate_accounts(ID, sequence);
    accounts.transfer_record = Pubkey::new_unique();
    expect_error(
        fixture.allocate_with(accounts, bytes),
        RemoteLegError::InvalidTransferRecord,
    );
}

#[test]
fn a_rejected_allocation_leaves_every_account_unchanged() {
    let mut fixture = Fixture::deployed();
    let lane_before = fixture.raw_data(fixture.lane_key(MessageClass::Allocate));
    let position_before = fixture.raw_data(fixture.position_key());

    let (sequence, bytes) = next_allocate(&fixture, ID, MAX_REMOTE_PRINCIPAL + 1);
    assert!(fixture.allocate(ID, sequence, bytes).is_err());

    assert_eq!(
        fixture.raw_data(fixture.lane_key(MessageClass::Allocate)),
        lane_before
    );
    assert_eq!(fixture.raw_data(fixture.position_key()), position_before);
    assert!(!fixture.transfer_exists(&ID));
}

#[test]
fn a_frozen_leg_rejects_a_new_allocation() {
    let mut fixture = Fixture::deployed();
    let guardian = fixture.guardian.insecure_clone();
    fixture.freeze(&guardian).expect("leg freezes");

    let (sequence, bytes) = next_allocate(&fixture, ID, AMOUNT);
    expect_error(
        fixture.allocate(ID, sequence, bytes),
        RemoteLegError::Frozen,
    );
}

// Independent asset arrival

#[test]
fn tokens_that_arrive_before_the_message_become_unattributed() {
    let mut fixture = Fixture::deployed();
    fixture.credit(fixture.custody, 400);
    fixture.reconcile().expect("reconciliation lands");

    let position = fixture.position();
    assert_eq!(position.unattributed_custody, 400);
    assert_eq!(position.attributed_principal, 0);
}

#[test]
fn tokens_that_arrived_first_attribute_once_the_message_lands() {
    let mut fixture = Fixture::deployed();
    fixture.credit(fixture.custody, AMOUNT);
    fixture.reconcile().expect("reconciliation lands");
    fixture.accept_allocation(ID, AMOUNT);
    fixture.attribute(ID).expect("attribution lands");

    let position = fixture.position();
    assert_eq!(position.attributed_principal, AMOUNT);
    assert_eq!(position.unattributed_custody, 0);
    assert!(!position.has_active_transfer());
}

#[test]
fn a_message_without_tokens_stays_unresolved() {
    let mut fixture = Fixture::deployed();
    fixture.accept_allocation(ID, AMOUNT);
    expect_error(fixture.attribute(ID), RemoteLegError::NoAttributableAssets);

    assert!(fixture.position().has_active_transfer());
    assert_eq!(fixture.transfer(&ID).status, TransferStatus::Pending);
}

#[test]
fn a_partial_arrival_attributes_exactly_what_arrived() {
    let mut fixture = Fixture::deployed();
    fixture.accept_allocation(ID, AMOUNT);
    fixture.credit(fixture.custody, 300);
    fixture.attribute(ID).expect("attribution lands");

    assert_eq!(fixture.transfer(&ID).attributed_amount, 300);
    assert_eq!(fixture.position().attributed_principal, 300);
    assert!(fixture.position().has_active_transfer());
}

#[test]
fn several_partial_arrivals_add_up() {
    let mut fixture = Fixture::deployed();
    fixture.accept_allocation(ID, AMOUNT);

    let mut total = 0;
    for amount in [250_000u64, 250_000, 500_000] {
        fixture.credit(fixture.custody, amount);
        fixture.attribute(ID).expect("attribution lands");
        total += amount;
        assert_eq!(fixture.transfer(&ID).attributed_amount, total);
    }

    assert_eq!(fixture.transfer(&ID).status, TransferStatus::Complete);
    assert!(!fixture.position().has_active_transfer());
}

#[test]
fn a_full_arrival_completes_the_allocation() {
    let mut fixture = Fixture::deployed();
    fixture.accept_allocation(ID, AMOUNT);
    fixture.credit(fixture.custody, AMOUNT);
    fixture.attribute(ID).expect("attribution lands");

    let record = fixture.transfer(&ID);
    assert_eq!(record.status, TransferStatus::Complete);
    assert_eq!(record.attributed_amount, AMOUNT);
    assert!(record.completed_at > 0);

    let position = fixture.position();
    assert_eq!(position.latest_completed_transfer_id, ID);
    assert!(position.latest_completion_at > 0);
}

#[test]
fn tokens_above_the_authorization_stay_unattributed() {
    let mut fixture = Fixture::deployed();
    fixture.accept_allocation(ID, AMOUNT);
    fixture.credit(fixture.custody, AMOUNT + 750);
    fixture.attribute(ID).expect("attribution lands");

    let position = fixture.position();
    assert_eq!(position.attributed_principal, AMOUNT);
    assert_eq!(position.unattributed_custody, 750);
}

#[test]
fn a_repeated_attribution_cannot_count_the_same_token_twice() {
    let mut fixture = Fixture::deployed();
    fixture.accept_allocation(ID, AMOUNT);
    fixture.credit(fixture.custody, 400);
    fixture.attribute(ID).expect("attribution lands");

    expect_error(fixture.attribute(ID), RemoteLegError::NoAttributableAssets);
    assert_eq!(fixture.transfer(&ID).attributed_amount, 400);
}

#[test]
fn attribution_after_completion_is_rejected() {
    let mut fixture = Fixture::deployed();
    fixture.fund_position(ID, AMOUNT);
    fixture.credit(fixture.custody, 100);

    expect_error(fixture.attribute(ID), RemoteLegError::InvalidTransferStatus);
}

#[test]
fn a_custody_balance_below_the_buckets_is_rejected() {
    let mut fixture = Fixture::deployed();
    fixture.fund_position(ID, AMOUNT);

    // Assets leaving custody without the leg knowing is a deficit.
    let custody = fixture.custody;
    let mint = fixture.mint;
    let authority = fixture.custody_authority;
    fixture.write_token_account(custody, mint, authority, None, None);

    expect_error(fixture.reconcile(), RemoteLegError::AccountingDeficit);
}

#[test]
fn a_frozen_leg_rejects_attribution_but_allows_reconciliation() {
    let mut fixture = Fixture::deployed();
    fixture.accept_allocation(ID, AMOUNT);
    fixture.credit(fixture.custody, AMOUNT);

    let guardian = fixture.guardian.insecure_clone();
    fixture.freeze(&guardian).expect("leg freezes");

    expect_error(fixture.attribute(ID), RemoteLegError::Frozen);
    fixture.reconcile().expect("unwind stays possible");
    assert_eq!(fixture.position().unattributed_custody, AMOUNT);
}

#[test]
fn reconciliation_never_moves_a_token() {
    let mut fixture = Fixture::deployed();
    fixture.credit(fixture.custody, 900);
    let custody_before = fixture.token_amount(fixture.custody);
    let escrow_before = fixture.token_amount(fixture.escrow);

    fixture.reconcile().expect("reconciliation lands");

    assert_eq!(fixture.token_amount(fixture.custody), custody_before);
    assert_eq!(fixture.token_amount(fixture.escrow), escrow_before);
}

#[test]
fn a_reconciliation_with_nothing_new_leaves_the_buckets_alone() {
    let mut fixture = Fixture::deployed();
    fixture.credit(fixture.custody, 900);
    fixture.reconcile().expect("first reconciliation lands");
    let before = fixture.raw_data(fixture.position_key());

    fixture.reconcile().expect("second reconciliation lands");
    assert_eq!(fixture.raw_data(fixture.position_key()), before);
}
