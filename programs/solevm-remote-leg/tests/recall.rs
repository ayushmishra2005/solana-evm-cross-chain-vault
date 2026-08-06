//! Accepting recalls, unwinding the strategy and sending assets out.

#![allow(clippy::unwrap_used, clippy::panic, clippy::arithmetic_side_effects)]

mod common;

use anchor_lang::solana_program::instruction::AccountMeta;
use common::messages::MessageBuilder;
use common::{
    ALLOCATE_TRANSFER_ID, Fixture, Pubkey, RECALL_TRANSFER_ID, expect_error, expect_rejection,
};
use protocol_types::{AssetAmount, ConfigVersion};
use solevm_remote_leg::{MessageClass, RemoteLegError, TransferKind, TransferStatus};

const ALLOCATE_ID: [u8; 32] = ALLOCATE_TRANSFER_ID;
const ID: [u8; 32] = RECALL_TRANSFER_ID;
const AMOUNT: u64 = 1_000_000;

/// A leg holding attributed custody, with nothing deployed yet.
fn holding_custody() -> Fixture {
    let mut fixture = Fixture::deployed();
    fixture.fund_position(ALLOCATE_ID, AMOUNT);
    fixture
}

/// A leg with all of its principal inside the adapter.
fn fully_deployed() -> Fixture {
    let mut fixture = holding_custody();
    fixture.deploy(AMOUNT).expect("deposit lands");
    fixture
}

fn next_recall(fixture: &Fixture, transfer_id: [u8; 32], amount: u64) -> (u64, Vec<u8>) {
    let lane = fixture.lane(MessageClass::Recall);
    let sequence = lane.highest_consumed_sequence + 1;
    let bytes = MessageBuilder::recall()
        .sequence(sequence)
        .previous_commitment(lane.message_commitment)
        .transfer_id(transfer_id)
        .recall_body(|body| {
            body.requested_amount = AssetAmount::new(u128::from(amount));
            body.minimum_return_amount = AssetAmount::new(u128::from(amount));
        })
        .encode();
    (sequence, bytes)
}

// Accepting the message

#[test]
fn a_valid_recall_reserves_available_custody_without_moving_it() {
    let mut fixture = holding_custody();
    let custody_before = fixture.token_amount(fixture.custody);
    fixture.accept_recall(ID, AMOUNT);

    assert_eq!(fixture.token_amount(fixture.custody), custody_before);

    let position = fixture.position();
    assert_eq!(position.attributed_principal, 0);
    assert_eq!(position.recalled_custody, AMOUNT);
    assert_eq!(position.active_transfer_kind, TransferKind::Recall);

    let record = fixture.transfer(&ID);
    assert_eq!(record.requested_recall_amount, AMOUNT);
    assert_eq!(record.custody_principal_reserved, AMOUNT);
    assert_eq!(record.assets_sent, 0);
}

#[test]
fn a_recall_against_deployed_principal_reserves_nothing_locally() {
    let mut fixture = fully_deployed();
    fixture.accept_recall(ID, AMOUNT);

    let position = fixture.position();
    assert_eq!(position.recalled_custody, 0);
    assert_eq!(position.deployed_principal, AMOUNT);
    assert_eq!(fixture.transfer(&ID).custody_principal_reserved, 0);
}

#[test]
fn a_recall_before_full_attribution_is_rejected() {
    let mut fixture = Fixture::deployed();
    fixture.accept_allocation(ALLOCATE_ID, AMOUNT);
    fixture.credit(fixture.custody, 400);
    fixture.attribute(ALLOCATE_ID).expect("attribution lands");

    let (sequence, bytes) = next_recall(&fixture, ID, 400);
    expect_error(
        fixture.recall(ID, sequence, bytes),
        RemoteLegError::UnresolvedCycle,
    );
}

#[test]
fn a_recall_above_the_remote_principal_is_rejected() {
    let mut fixture = holding_custody();
    let (sequence, bytes) = next_recall(&fixture, ID, AMOUNT + 1);
    expect_error(
        fixture.recall(ID, sequence, bytes),
        RemoteLegError::InsufficientRemotePrincipal,
    );
}

#[test]
fn a_recall_ignores_unattributed_custody_when_it_checks_principal() {
    let mut fixture = holding_custody();
    fixture.credit(fixture.custody, 500_000);
    fixture.reconcile().expect("reconciliation lands");

    let (sequence, bytes) = next_recall(&fixture, ID, AMOUNT + 1);
    expect_error(
        fixture.recall(ID, sequence, bytes),
        RemoteLegError::InsufficientRemotePrincipal,
    );
}

#[test]
fn a_minimum_return_above_the_request_is_rejected_by_the_codec() {
    let bytes = MessageBuilder::recall()
        .recall_body(|body| {
            body.requested_amount = AssetAmount::new(100);
            body.minimum_return_amount = AssetAmount::new(101);
        })
        .build()
        .encode();
    assert!(bytes.is_err(), "the shared codec rejects the body itself");
}

#[test]
fn a_second_recall_while_one_is_open_is_rejected() {
    let mut fixture = holding_custody();
    fixture.accept_recall(ID, 400_000);

    let second = [0x89; 32];
    let (sequence, bytes) = next_recall(&fixture, second, 100);
    expect_error(
        fixture.recall(second, sequence, bytes),
        RemoteLegError::UnresolvedCycle,
    );
}

#[test]
fn a_recall_sequence_gap_is_accepted() {
    let mut fixture = holding_custody();
    let lane = fixture.lane(MessageClass::Recall);
    let bytes = MessageBuilder::recall()
        .sequence(5)
        .previous_commitment(lane.message_commitment)
        .transfer_id(ID)
        .recall_body(|body| {
            body.requested_amount = AssetAmount::new(u128::from(AMOUNT));
            body.minimum_return_amount = AssetAmount::new(u128::from(AMOUNT));
        })
        .encode();

    fixture.recall(ID, 5, bytes).expect("gap is allowed");
    assert_eq!(
        fixture.lane(MessageClass::Recall).highest_consumed_sequence,
        5
    );
}

#[test]
fn an_exact_recall_replay_is_rejected() {
    let mut fixture = holding_custody();
    let (sequence, bytes) = next_recall(&fixture, ID, AMOUNT);
    fixture
        .recall(ID, sequence, bytes.clone())
        .expect("recall lands");

    expect_error(
        fixture.recall(ID, sequence, bytes),
        RemoteLegError::InvalidSequence,
    );
}

#[test]
fn a_recall_body_config_version_out_of_step_is_rejected() {
    let mut fixture = holding_custody();
    let lane = fixture.lane(MessageClass::Recall);
    let bytes = MessageBuilder::recall()
        .sequence(1)
        .previous_commitment(lane.message_commitment)
        .transfer_id(ID)
        .recall_body(|body| body.config_version = ConfigVersion::new(42))
        .encode();

    expect_error(
        fixture.recall(ID, 1, bytes),
        RemoteLegError::InvalidConfigVersion,
    );
}

#[test]
fn an_expired_recall_is_not_accepted() {
    let mut fixture = holding_custody();
    let (sequence, bytes) = next_recall(&fixture, ID, AMOUNT);
    fixture.set_time(common::messages::EXPIRES_AT as i64 + 1);

    expect_error(
        fixture.recall(ID, sequence, bytes),
        RemoteLegError::MessageExpired,
    );
}

#[test]
fn an_accepted_recall_keeps_unwinding_after_its_deadline() {
    let mut fixture = fully_deployed();
    fixture.accept_recall(ID, AMOUNT);
    fixture.set_time(common::messages::EXPIRES_AT as i64 + 10_000);

    fixture.withdraw(ID, AMOUNT).expect("withdrawal lands");
    fixture.send_recall(ID, AMOUNT).expect("send lands");
    assert_eq!(fixture.transfer(&ID).status, TransferStatus::Complete);
}

#[test]
fn a_frozen_leg_rejects_a_new_recall() {
    let mut fixture = holding_custody();
    let guardian = fixture.guardian.insecure_clone();
    fixture.freeze(&guardian).expect("leg freezes");

    let (sequence, bytes) = next_recall(&fixture, ID, AMOUNT);
    expect_error(fixture.recall(ID, sequence, bytes), RemoteLegError::Frozen);
}

#[test]
fn accepting_a_recall_never_claims_assets_were_sent() {
    let mut fixture = holding_custody();
    let escrow_before = fixture.token_amount(fixture.escrow);
    fixture.accept_recall(ID, AMOUNT);

    assert_eq!(fixture.token_amount(fixture.escrow), escrow_before);
    assert_eq!(fixture.transfer(&ID).assets_sent, 0);
    assert_eq!(fixture.transfer(&ID).status, TransferStatus::Pending);
}

// Strategy withdrawal

#[test]
fn a_full_withdrawal_returns_every_deployed_token() {
    let mut fixture = fully_deployed();
    fixture.accept_recall(ID, AMOUNT);
    fixture.withdraw(ID, AMOUNT).expect("withdrawal lands");

    let position = fixture.position();
    assert_eq!(position.deployed_principal, 0);
    assert_eq!(position.recalled_custody, AMOUNT);
    assert_eq!(position.cumulative_realized_loss, 0);
    assert_eq!(fixture.adapter().principal, 0);
    assert_eq!(fixture.token_amount(fixture.custody), AMOUNT);
}

#[test]
fn a_liquidity_bound_returns_only_what_is_available() {
    let mut fixture = fully_deployed();
    fixture
        .configure_adapter(300_000, 0, false)
        .expect("liquidity is capped");
    fixture.accept_recall(ID, AMOUNT);
    fixture.withdraw(ID, AMOUNT).expect("withdrawal lands");

    assert_eq!(fixture.position().recalled_custody, 300_000);
    assert_eq!(fixture.position().deployed_principal, 700_000);
    assert_eq!(fixture.transfer(&ID).strategy_principal_resolved, 300_000);
}

#[test]
fn several_partial_withdrawals_add_up() {
    let mut fixture = fully_deployed();
    fixture
        .configure_adapter(250_000, 0, false)
        .expect("liquidity is capped");
    fixture.accept_recall(ID, AMOUNT);

    for _ in 0..4 {
        fixture.withdraw(ID, AMOUNT).expect("withdrawal lands");
    }

    assert_eq!(fixture.position().deployed_principal, 0);
    assert_eq!(fixture.transfer(&ID).assets_withdrawn, AMOUNT);
    assert_eq!(fixture.adapter().principal, 0);
}

#[test]
fn an_adapter_without_liquidity_rejects_and_changes_nothing() {
    let mut fixture = fully_deployed();
    fixture
        .configure_adapter(0, 0, false)
        .expect("liquidity is removed");
    fixture.accept_recall(ID, AMOUNT);

    let position_before = fixture.raw_data(fixture.position_key());
    let record_before = fixture.raw_data(fixture.transfer_key(&ID));
    expect_rejection(fixture.withdraw(ID, AMOUNT));

    assert_eq!(fixture.raw_data(fixture.position_key()), position_before);
    assert_eq!(fixture.raw_data(fixture.transfer_key(&ID)), record_before);
    assert_eq!(fixture.token_amount(fixture.custody), 0);
}

#[test]
fn the_exact_realized_loss_is_recorded() {
    let mut fixture = fully_deployed();
    fixture
        .configure_adapter(u64::MAX, 250, false)
        .expect("loss is configured");
    fixture.accept_recall(ID, AMOUNT);
    fixture.withdraw(ID, AMOUNT).expect("withdrawal lands");

    let expected_loss = AMOUNT / 40;
    let record = fixture.transfer(&ID);
    assert_eq!(record.realized_loss, expected_loss);
    assert_eq!(record.assets_withdrawn, AMOUNT - expected_loss);
    assert_eq!(record.strategy_principal_resolved, AMOUNT);

    let position = fixture.position();
    assert_eq!(position.cumulative_realized_loss, expected_loss);
    assert_eq!(position.recalled_custody, AMOUNT - expected_loss);
    assert_eq!(position.deployed_principal, 0);
}

#[test]
fn a_total_loss_returns_nothing_and_still_resolves_the_principal() {
    let mut fixture = fully_deployed();
    fixture
        .configure_adapter(u64::MAX, 10_000, false)
        .expect("loss is configured");
    fixture.accept_recall(ID, AMOUNT);
    fixture.withdraw(ID, AMOUNT).expect("withdrawal lands");

    let record = fixture.transfer(&ID);
    assert_eq!(record.realized_loss, AMOUNT);
    assert_eq!(record.assets_withdrawn, 0);
    assert_eq!(record.status, TransferStatus::Complete);
    assert!(!fixture.position().has_active_transfer());
}

#[test]
fn the_realized_loss_never_exceeds_the_resolved_principal() {
    for bps in [0u16, 1, 250, 5_000, 9_999, 10_000] {
        let mut fixture = fully_deployed();
        fixture
            .configure_adapter(u64::MAX, bps, false)
            .expect("loss is configured");
        fixture.accept_recall(ID, AMOUNT);
        fixture.withdraw(ID, AMOUNT).expect("withdrawal lands");

        let record = fixture.transfer(&ID);
        assert!(record.realized_loss <= record.strategy_principal_resolved);
        assert_eq!(
            record.assets_withdrawn + record.realized_loss,
            record.strategy_principal_resolved
        );
    }
}

#[test]
fn a_withdrawal_without_an_active_recall_is_rejected() {
    let mut fixture = fully_deployed();
    expect_error(
        fixture.withdraw(ALLOCATE_ID, AMOUNT),
        RemoteLegError::InvalidTransferKind,
    );
}

#[test]
fn a_frozen_leg_still_unwinds_an_accepted_recall() {
    let mut fixture = fully_deployed();
    fixture.accept_recall(ID, AMOUNT);
    let guardian = fixture.guardian.insecure_clone();
    fixture.freeze(&guardian).expect("leg freezes");

    fixture.withdraw(ID, AMOUNT).expect("withdrawal lands");
    fixture.send_recall(ID, AMOUNT).expect("send lands");
    assert_eq!(fixture.transfer(&ID).status, TransferStatus::Complete);
}

#[test]
fn a_withdrawal_through_wrong_cpi_accounts_is_rejected() {
    let mut fixture = fully_deployed();
    fixture.accept_recall(ID, AMOUNT);

    let mut accounts = fixture.strategy_accounts();
    accounts.adapter_state = Pubkey::new_unique();
    let record = fixture.transfer_key(&ID);
    let instruction = fixture.withdraw_instruction(accounts, record, AMOUNT);
    let payer = fixture.outsider.insecure_clone();
    expect_error(
        fixture.send_as(instruction, &payer, &[&payer]),
        RemoteLegError::InvalidAdapterState,
    );
}

#[test]
fn a_withdrawal_never_resolves_more_than_the_request() {
    let mut fixture = Fixture::deployed();
    fixture.fund_position(ALLOCATE_ID, AMOUNT);
    fixture.deploy(AMOUNT).expect("deposit lands");
    fixture.accept_recall(ID, 400_000);
    fixture.withdraw(ID, u64::MAX).expect("withdrawal lands");

    assert_eq!(fixture.transfer(&ID).strategy_principal_resolved, 400_000);
    assert_eq!(fixture.position().deployed_principal, 600_000);
}

// Outbound send

#[test]
fn a_partial_send_moves_exactly_the_requested_amount() {
    let mut fixture = holding_custody();
    fixture.accept_recall(ID, AMOUNT);
    fixture.send_recall(ID, 400_000).expect("send lands");

    assert_eq!(fixture.token_amount(fixture.escrow), 400_000);
    assert_eq!(fixture.token_amount(fixture.custody), 600_000);
    assert_eq!(fixture.position().recalled_custody, 600_000);
    assert_eq!(fixture.transfer(&ID).assets_sent, 400_000);
    assert_eq!(fixture.transfer(&ID).status, TransferStatus::Pending);
}

#[test]
fn several_partial_sends_add_up_and_complete_the_recall() {
    let mut fixture = holding_custody();
    fixture.accept_recall(ID, AMOUNT);

    for _ in 0..4 {
        fixture.send_recall(ID, 250_000).expect("send lands");
    }

    assert_eq!(fixture.token_amount(fixture.escrow), AMOUNT);
    assert_eq!(fixture.transfer(&ID).assets_sent, AMOUNT);
    assert_eq!(fixture.transfer(&ID).status, TransferStatus::Complete);
    assert!(!fixture.position().has_active_transfer());
}

#[test]
fn a_full_send_completes_the_recall_exactly() {
    let mut fixture = holding_custody();
    fixture.accept_recall(ID, AMOUNT);
    fixture.send_recall(ID, u64::MAX).expect("send lands");

    let record = fixture.transfer(&ID);
    assert_eq!(
        record.assets_sent + record.realized_loss,
        record.requested_recall_amount
    );
    assert_eq!(record.status, TransferStatus::Complete);

    let position = fixture.position();
    assert_eq!(position.recalled_custody, 0);
    assert_eq!(position.latest_completed_transfer_id, ID);
}

#[test]
fn a_recall_with_loss_completes_once_sent_plus_loss_match() {
    let mut fixture = fully_deployed();
    fixture
        .configure_adapter(u64::MAX, 250, false)
        .expect("loss is configured");
    fixture.accept_recall(ID, AMOUNT);
    fixture.withdraw(ID, AMOUNT).expect("withdrawal lands");

    let loss = fixture.transfer(&ID).realized_loss;
    fixture.send_recall(ID, u64::MAX).expect("send lands");

    let record = fixture.transfer(&ID);
    assert_eq!(record.assets_sent, AMOUNT - loss);
    assert_eq!(record.assets_sent + record.realized_loss, AMOUNT);
    assert_eq!(record.status, TransferStatus::Complete);
}

#[test]
fn a_return_below_the_minimum_still_unwinds() {
    let mut fixture = fully_deployed();
    fixture
        .configure_adapter(u64::MAX, 5_000, false)
        .expect("loss is configured");
    fixture.accept_recall(ID, AMOUNT);
    fixture.withdraw(ID, AMOUNT).expect("withdrawal lands");
    fixture.send_recall(ID, u64::MAX).expect("send lands");

    let record = fixture.transfer(&ID);
    assert!(record.assets_sent < record.minimum_amount);
    assert_eq!(record.status, TransferStatus::Complete);
}

#[test]
fn a_caller_cannot_choose_the_destination() {
    let mut fixture = holding_custody();
    fixture.accept_recall(ID, AMOUNT);

    let stranger = Pubkey::new_unique();
    let mint = fixture.mint;
    fixture.write_token_account(stranger, mint, Pubkey::new_unique(), None, None);

    let record = fixture.transfer_key(&ID);
    let instruction = fixture.send_recall_instruction(record, stranger, AMOUNT);
    let payer = fixture.outsider.insecure_clone();
    expect_error(
        fixture.send_as(instruction, &payer, &[&payer]),
        RemoteLegError::InvalidOutboundEscrow,
    );
    assert_eq!(fixture.token_amount(stranger), 0);
}

#[test]
fn a_send_without_an_active_recall_is_rejected() {
    let mut fixture = holding_custody();
    expect_error(
        fixture.send_recall(ALLOCATE_ID, AMOUNT),
        RemoteLegError::InvalidTransferKind,
    );
}

#[test]
fn a_send_without_recalled_custody_is_rejected() {
    let mut fixture = fully_deployed();
    fixture.accept_recall(ID, AMOUNT);
    expect_error(
        fixture.send_recall(ID, AMOUNT),
        RemoteLegError::NoRecalledCustody,
    );
}

#[test]
fn unattributed_custody_is_never_sent_out() {
    let mut fixture = holding_custody();
    fixture.accept_recall(ID, 400_000);
    fixture.credit(fixture.custody, 250_000);

    fixture.send_recall(ID, u64::MAX).expect("send lands");
    assert_eq!(fixture.token_amount(fixture.escrow), 400_000);
    assert_eq!(fixture.position().unattributed_custody, 250_000);
}

#[test]
fn the_latest_completed_transfer_updates_once() {
    let mut fixture = holding_custody();
    fixture.accept_recall(ID, AMOUNT);
    fixture.send_recall(ID, u64::MAX).expect("send lands");

    let first = fixture.position().latest_completion_at;
    expect_error(
        fixture.send_recall(ID, 1),
        RemoteLegError::InvalidTransferStatus,
    );
    assert_eq!(fixture.position().latest_completion_at, first);
    assert_eq!(fixture.position().latest_completed_transfer_id, ID);
}

#[test]
fn a_send_through_another_custody_account_is_rejected() {
    let mut fixture = holding_custody();
    fixture.accept_recall(ID, AMOUNT);

    let stranger = Pubkey::new_unique();
    let mint = fixture.mint;
    let authority = fixture.custody_authority;
    fixture.write_token_account(stranger, mint, authority, None, None);

    let record = fixture.transfer_key(&ID);
    let mut instruction = fixture.send_recall_instruction(record, fixture.escrow, AMOUNT);
    // Custody sits after the configuration, position, record and authority.
    instruction.accounts[4] = AccountMeta::new(stranger, false);

    let payer = fixture.outsider.insecure_clone();
    expect_error(
        fixture.send_as(instruction, &payer, &[&payer]),
        RemoteLegError::InvalidCustodyAccount,
    );
}

#[test]
fn a_mixed_recall_draws_from_custody_and_the_strategy() {
    let mut fixture = Fixture::deployed();
    fixture.fund_position(ALLOCATE_ID, AMOUNT);
    fixture.deploy(600_000).expect("deposit lands");

    fixture.accept_recall(ID, AMOUNT);
    assert_eq!(fixture.transfer(&ID).custody_principal_reserved, 400_000);

    fixture.withdraw(ID, u64::MAX).expect("withdrawal lands");
    assert_eq!(fixture.transfer(&ID).strategy_principal_resolved, 600_000);

    fixture.send_recall(ID, u64::MAX).expect("send lands");
    assert_eq!(fixture.transfer(&ID).status, TransferStatus::Complete);
    assert_eq!(fixture.token_amount(fixture.escrow), AMOUNT);

    let position = fixture.position();
    assert_eq!(position.attributed_principal, 0);
    assert_eq!(position.deployed_principal, 0);
    assert_eq!(position.recalled_custody, 0);
}

#[test]
fn a_new_cycle_may_start_once_the_recall_completes() {
    let mut fixture = holding_custody();
    fixture.accept_recall(ID, AMOUNT);
    fixture.send_recall(ID, u64::MAX).expect("send lands");

    let second = [0x7C; 32];
    fixture.accept_allocation(second, 500);
    assert_eq!(
        fixture.position().active_transfer_kind,
        TransferKind::Allocate
    );
}
