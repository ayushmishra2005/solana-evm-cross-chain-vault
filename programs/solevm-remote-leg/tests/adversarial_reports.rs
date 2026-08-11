//! Hostile attempts against the two publication instructions.
//!
//! Every case here swaps, corrupts or squats an account and expects the
//! publication to reject without touching state.

#![allow(clippy::unwrap_used, clippy::panic, clippy::arithmetic_side_effects)]

mod common;

use common::{
    ALLOCATE_TRANSFER_ID, Fixture, Pubkey, RECALL_TRANSFER_ID, expect_error, expect_rejection,
};
use solevm_remote_leg::{RemoteLegError, TransferStatus};

const AMOUNT: u64 = 1_000_000;
const EPOCH: u64 = 7;

/// A leg with an open outbound lane and money in the adapter.
fn funded() -> Fixture {
    let mut fixture = Fixture::ready_to_publish();
    fixture.fund_position(ALLOCATE_TRANSFER_ID, AMOUNT);
    fixture.deploy(AMOUNT).expect("the deposit lands");
    fixture
}

/// A leg holding one completed recall.
fn settled() -> Fixture {
    let mut fixture = funded();
    fixture.accept_recall(RECALL_TRANSFER_ID, AMOUNT);
    fixture
        .withdraw(RECALL_TRANSFER_ID, AMOUNT)
        .expect("the withdrawal lands");
    fixture
        .send_recall(RECALL_TRANSFER_ID, AMOUNT)
        .expect("the send lands");
    fixture
}

/// Everything a publication could disturb.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Snapshot {
    lane: Vec<u8>,
    position: Vec<u8>,
    config: Vec<u8>,
    custody: u64,
    escrow: u64,
    adapter_vault: u64,
}

fn snapshot(fixture: &Fixture) -> Snapshot {
    Snapshot {
        lane: fixture.raw_data(fixture.outbound_lane_key()),
        position: fixture.raw_data(fixture.position_key()),
        config: fixture.raw_data(fixture.config),
        custody: fixture.token_amount(fixture.custody),
        escrow: fixture.token_amount(fixture.escrow),
        adapter_vault: fixture.token_amount(fixture.adapter_vault),
    }
}

/// Runs one hostile report attempt and proves nothing moved.
#[track_caller]
fn report_rejects(
    fixture: &mut Fixture,
    edit: impl FnOnce(&mut Fixture, &mut common::OutboundAccounts),
) {
    let sequence = fixture.outbound_lane().next_sequence;
    let mut accounts = fixture.outbound_accounts(sequence);
    edit(fixture, &mut accounts);

    let before = snapshot(fixture);
    expect_rejection(fixture.publish_report_with(accounts, EPOCH));
    assert_eq!(snapshot(fixture), before);
}

/// Runs one hostile notice attempt and proves nothing moved.
#[track_caller]
fn notice_rejects(
    fixture: &mut Fixture,
    edit: impl FnOnce(&mut Fixture, &mut common::RecallSentAccounts),
) {
    let sequence = fixture.outbound_lane().next_sequence;
    let mut accounts = fixture.recall_sent_accounts(RECALL_TRANSFER_ID, sequence);
    edit(fixture, &mut accounts);

    let before = snapshot(fixture);
    expect_rejection(fixture.publish_recall_sent_with(accounts));
    assert_eq!(snapshot(fixture), before);
}

// Substituted accounts on the report path

#[test]
fn a_report_against_another_vaults_configuration_rejects() {
    let mut fixture = funded();
    let other = fixture.add_second_vault([0x5C; 32]);
    report_rejects(&mut fixture, |_, accounts| {
        accounts.remote_config = other;
    });
}

#[test]
fn a_report_with_a_foreign_risk_configuration_rejects() {
    let mut fixture = funded();
    let stranger = Pubkey::new_unique();
    report_rejects(&mut fixture, |_, accounts| {
        accounts.risk_config = stranger;
    });
}

#[test]
fn a_report_with_a_foreign_strategy_configuration_rejects() {
    let mut fixture = funded();
    let stranger = Pubkey::new_unique();
    report_rejects(&mut fixture, |_, accounts| {
        accounts.strategy_config = stranger;
    });
}

#[test]
fn a_report_with_a_foreign_position_rejects() {
    let mut fixture = funded();
    let stranger = Pubkey::new_unique();
    report_rejects(&mut fixture, |_, accounts| {
        accounts.remote_position = stranger;
    });
}

#[test]
fn a_report_with_another_lane_account_rejects() {
    let mut fixture = funded();
    let stranger = fixture.risk();
    report_rejects(&mut fixture, |_, accounts| {
        accounts.outbound_lane = stranger;
    });
}

#[test]
fn a_report_with_a_substituted_custody_account_rejects() {
    let mut fixture = funded();
    let decoy = Pubkey::new_unique();
    let owner = fixture.custody_authority;
    let mint = fixture.mint;
    fixture.write_token_account(decoy, mint, owner, None, None);
    fixture.set_token_amount(decoy, 10_000_000);

    report_rejects(&mut fixture, |_, accounts| {
        accounts.custody_token_account = decoy;
    });
}

#[test]
fn a_report_against_an_unrelated_adapter_account_rejects() {
    let mut fixture = funded();
    let stranger = Pubkey::new_unique();
    report_rejects(&mut fixture, |_, accounts| {
        accounts.adapter_state = stranger;
    });
}

#[test]
fn a_report_writing_into_a_record_of_another_sequence_rejects() {
    let mut fixture = funded();
    report_rejects(&mut fixture, |fixture, accounts| {
        accounts.report_record = fixture.report_record_key(9);
    });
}

#[test]
fn a_report_pointing_the_record_at_live_state_rejects() {
    let mut fixture = funded();
    report_rejects(&mut fixture, |fixture, accounts| {
        accounts.report_record = fixture.position_key();
    });
}

#[test]
fn a_report_reusing_the_lane_as_its_own_record_rejects() {
    let mut fixture = funded();
    report_rejects(&mut fixture, |fixture, accounts| {
        accounts.report_record = fixture.outbound_lane_key();
    });
}

// Corrupted state on the report path

#[test]
fn a_report_over_a_corrupted_lane_version_rejects() {
    let mut fixture = funded();
    let lane = fixture.outbound_lane_key();
    fixture.overwrite_byte(lane, 8, 9);

    expect_error(
        fixture.publish_report(EPOCH),
        RemoteLegError::InvalidStateVersion,
    );
}

#[test]
fn a_report_over_a_corrupted_lane_bump_rejects() {
    let mut fixture = funded();
    let lane = fixture.outbound_lane_key();
    fixture.overwrite_byte(lane, 9, 1);

    expect_rejection(fixture.publish_report(EPOCH));
}

#[test]
fn a_report_over_a_lane_bound_to_another_route_rejects() {
    let mut fixture = funded();
    let lane = fixture.outbound_lane_key();
    // The lane identifier sits right after the version and the bump.
    fixture.overwrite_byte(lane, 10, 0xFF);

    expect_error(
        fixture.publish_report(EPOCH),
        RemoteLegError::InvalidOutboundLane,
    );
}

#[test]
fn a_report_over_a_lane_owned_by_a_stranger_rejects() {
    let mut fixture = funded();
    let lane = fixture.outbound_lane_key();
    fixture.reassign_account(lane, Pubkey::new_unique());

    expect_rejection(fixture.publish_report(EPOCH));
}

#[test]
fn a_report_against_a_record_slot_a_stranger_owns_rejects() {
    let mut fixture = funded();
    let sequence = fixture.outbound_lane().next_sequence;
    let record = fixture.report_record_key(sequence);
    fixture.give_account_to(record, Pubkey::new_unique(), 64);

    let before = snapshot(&fixture);
    expect_error(
        fixture.publish_report(EPOCH),
        RemoteLegError::InvalidReportRecord,
    );
    assert_eq!(snapshot(&fixture), before);
}

#[test]
fn a_report_whose_custody_fell_short_rejects() {
    let mut fixture = Fixture::ready_to_publish();
    fixture.fund_position(ALLOCATE_TRANSFER_ID, AMOUNT);
    let custody = fixture.custody;
    fixture.set_token_amount(custody, 0);

    expect_rejection(fixture.publish_report(EPOCH));
}

#[test]
fn a_report_whose_adapter_drifted_from_the_position_rejects() {
    let mut fixture = funded();
    fixture.set_adapter_principal(AMOUNT + 1);

    expect_rejection(fixture.publish_report(EPOCH));
}

// Substituted accounts on the notice path

#[test]
fn a_notice_against_another_vaults_configuration_rejects() {
    let mut fixture = settled();
    let other = fixture.add_second_vault([0x5D; 32]);
    notice_rejects(&mut fixture, |_, accounts| {
        accounts.remote_config = other;
    });
}

#[test]
fn a_notice_with_a_foreign_risk_configuration_rejects() {
    let mut fixture = settled();
    let stranger = Pubkey::new_unique();
    notice_rejects(&mut fixture, |_, accounts| {
        accounts.risk_config = stranger;
    });
}

#[test]
fn a_notice_naming_the_allocation_record_rejects() {
    let mut fixture = settled();
    notice_rejects(&mut fixture, |fixture, accounts| {
        accounts.transfer_record = fixture.transfer_key(&ALLOCATE_TRANSFER_ID);
    });
}

#[test]
fn a_notice_with_a_foreign_transfer_record_rejects() {
    let mut fixture = settled();
    let stranger = Pubkey::new_unique();
    notice_rejects(&mut fixture, |_, accounts| {
        accounts.transfer_record = stranger;
    });
}

#[test]
fn a_notice_with_another_lane_account_rejects() {
    let mut fixture = settled();
    let stranger = fixture.risk();
    notice_rejects(&mut fixture, |_, accounts| {
        accounts.outbound_lane = stranger;
    });
}

#[test]
fn a_notice_writing_into_a_record_of_another_sequence_rejects() {
    let mut fixture = settled();
    notice_rejects(&mut fixture, |fixture, accounts| {
        accounts.report_record = fixture.report_record_key(4);
    });
}

#[test]
fn a_notice_with_a_marker_for_another_transfer_rejects() {
    let mut fixture = settled();
    notice_rejects(&mut fixture, |fixture, accounts| {
        accounts.recall_sent_marker = fixture.recall_marker_key(&ALLOCATE_TRANSFER_ID);
    });
}

#[test]
fn a_notice_pointing_the_marker_at_live_state_rejects() {
    let mut fixture = settled();
    notice_rejects(&mut fixture, |fixture, accounts| {
        accounts.recall_sent_marker = fixture.outbound_lane_key();
    });
}

#[test]
fn a_notice_against_a_marker_slot_a_stranger_owns_rejects() {
    let mut fixture = settled();
    let marker = fixture.recall_marker_key(&RECALL_TRANSFER_ID);
    fixture.give_account_to(marker, Pubkey::new_unique(), 64);

    let before = snapshot(&fixture);
    expect_error(
        fixture.publish_recall_sent(RECALL_TRANSFER_ID),
        RemoteLegError::InvalidRecallIdentity,
    );
    assert_eq!(snapshot(&fixture), before);
}

#[test]
fn a_notice_for_a_transfer_that_is_still_open_rejects() {
    let mut fixture = funded();
    fixture.accept_recall(RECALL_TRANSFER_ID, AMOUNT);

    assert_eq!(
        fixture.transfer(&RECALL_TRANSFER_ID).status,
        TransferStatus::Pending
    );
    expect_error(
        fixture.publish_recall_sent(RECALL_TRANSFER_ID),
        RemoteLegError::RecallNotComplete,
    );
}

// Squatted slots still let honest work through

#[test]
fn a_prefunded_record_slot_does_not_block_a_report() {
    let mut fixture = funded();
    let sequence = fixture.outbound_lane().next_sequence;
    let record = fixture.report_record_key(sequence);
    fixture.prefund(record, 5_000_000);

    fixture.publish_report(EPOCH).expect("the report lands");
    assert_eq!(fixture.report_record(sequence).sequence, sequence);
}

#[test]
fn a_prefunded_marker_slot_does_not_block_a_notice() {
    let mut fixture = settled();
    let marker = fixture.recall_marker_key(&RECALL_TRANSFER_ID);
    fixture.prefund(marker, 5_000_000);

    fixture
        .publish_recall_sent(RECALL_TRANSFER_ID)
        .expect("the notice lands");
    assert_eq!(
        fixture.recall_marker(&RECALL_TRANSFER_ID).transfer_id,
        RECALL_TRANSFER_ID
    );
}

#[test]
fn a_prefunded_future_record_slot_does_not_block_the_next_report() {
    let mut fixture = funded();
    fixture.prefund(fixture.report_record_key(2), 5_000_000);

    fixture.publish_report(EPOCH).expect("the first lands");
    fixture.publish_report(EPOCH).expect("the second lands");
    assert_eq!(fixture.outbound_lane().next_sequence, 3);
}

// A frozen leg keeps reporting and gains no new exposure

#[test]
fn a_frozen_leg_still_publishes_and_moves_nothing() {
    let mut fixture = funded();
    let guardian = fixture.guardian.insecure_clone();
    fixture.freeze(&guardian).expect("the freeze lands");

    let custody = fixture.token_amount(fixture.custody);
    let escrow = fixture.token_amount(fixture.escrow);
    let vault = fixture.token_amount(fixture.adapter_vault);

    fixture.publish_report(EPOCH).expect("the report lands");

    assert_eq!(fixture.token_amount(fixture.custody), custody);
    assert_eq!(fixture.token_amount(fixture.escrow), escrow);
    assert_eq!(fixture.token_amount(fixture.adapter_vault), vault);
}

#[test]
fn a_frozen_leg_cannot_take_on_new_assets() {
    let mut fixture = funded();
    let guardian = fixture.guardian.insecure_clone();
    fixture.freeze(&guardian).expect("the freeze lands");

    fixture.publish_report(EPOCH).expect("the report lands");

    // Reporting never reopens the inbound path.
    expect_rejection(fixture.allocate(
        [0x9E; 32],
        3,
        fixture.allocate_bytes([0x9E; 32], AMOUNT, 3),
    ));
}

#[test]
fn a_frozen_leg_still_publishes_a_finished_recall() {
    let mut fixture = settled();
    let guardian = fixture.guardian.insecure_clone();
    fixture.freeze(&guardian).expect("the freeze lands");

    fixture
        .publish_recall_sent(RECALL_TRANSFER_ID)
        .expect("the notice lands");
    assert_eq!(fixture.outbound_lane().total_recall_notifications, 1);
}
