//! Publishing what the leg holds, built only from its own state.

#![allow(clippy::unwrap_used, clippy::panic, clippy::arithmetic_side_effects)]

mod common;

use common::{
    ALLOCATE_TRANSFER_ID, DEPLOYMENT_ID, Fixture, LOCAL_APPLICATION_ID, MAX_REPORT_AGE, Pubkey,
    RECALL_TRANSFER_ID, REPORT_LANE_ID, SOURCE_APPLICATION_ID, SOURCE_CHAIN_ID, VAULT_ID,
    expect_error, expect_rejection, last_event,
};
use protocol_types::{Body, ProbeStatus, layout};
use solevm_remote_leg::{RemoteLegError, RemoteReportPublished, ReportRecord, ReportType};

const EPOCH: u64 = 12;

/// Publishes one report and returns the event it emitted.
fn publish(fixture: &mut Fixture, epoch_id: u64) -> RemoteReportPublished {
    let sequence = fixture.outbound_lane().next_sequence;
    let accounts = fixture.outbound_accounts(sequence);
    let instruction = fixture.publish_report_instruction(accounts, epoch_id);
    fixture.publish_and_capture(instruction)
}

/// Decodes the canonical bytes an event carried.
fn decoded(event: &RemoteReportPublished) -> protocol_types::Message {
    protocol_types::decode_message(&event.message_bytes).expect("the report decodes")
}

fn report_body(event: &RemoteReportPublished) -> protocol_types::RemoteReportBody {
    match decoded(event).body {
        Body::RemoteReport(body) => body,
        other => panic!("expected a remote report, found {other:?}"),
    }
}

#[test]
fn an_idle_leg_publishes_a_report_of_zero_principal() {
    let mut fixture = Fixture::ready_to_publish();
    let event = publish(&mut fixture, EPOCH);
    let body = report_body(&event);

    assert_eq!(body.epoch_id.get(), EPOCH);
    assert_eq!(body.remote_principal.get(), 0);
    assert_eq!(body.reported_value.get(), 0);
    assert_eq!(body.realized_loss.get(), 0);
    assert_eq!(body.unattributed_balance.get(), 0);
    assert_eq!(body.latest_completed_transfer_id.as_bytes(), &[0u8; 32]);
}

#[test]
fn a_report_is_exactly_four_hundred_bytes() {
    let mut fixture = Fixture::ready_to_publish();
    let event = publish(&mut fixture, EPOCH);

    assert_eq!(event.message_bytes.len(), 400);
    assert_eq!(event.encoded_len, 400);
    assert_eq!(layout::REMOTE_REPORT_MESSAGE_LEN, 400);
}

#[test]
fn the_header_sends_the_report_back_the_way_the_orders_came() {
    let mut fixture = Fixture::ready_to_publish();
    let event = publish(&mut fixture, EPOCH);
    let header = decoded(&event).header;

    assert_eq!(header.source_chain.get(), common::DESTINATION_CHAIN_ID);
    assert_eq!(header.destination_chain.get(), SOURCE_CHAIN_ID);
    assert_eq!(header.source_application.as_bytes(), &LOCAL_APPLICATION_ID);
    assert_eq!(
        header.destination_application.as_bytes(),
        &SOURCE_APPLICATION_ID
    );
    assert_eq!(header.deployment_id.as_bytes(), &DEPLOYMENT_ID);
    assert_eq!(header.vault_id.as_bytes(), &VAULT_ID);
    assert_eq!(header.lane_id.get(), REPORT_LANE_ID);
    assert_eq!(header.sequence.get(), 1);
    assert_eq!(header.previous_commitment.as_bytes(), &[0u8; 32]);
}

#[test]
fn the_report_expires_after_the_configured_maximum_age() {
    let mut fixture = Fixture::ready_to_publish();
    let event = publish(&mut fixture, EPOCH);
    let header = decoded(&event).header;

    assert_eq!(header.observed_at, header.published_at);
    assert_eq!(
        header.expires_at.get(),
        header.published_at.get() + MAX_REPORT_AGE
    );
}

#[test]
fn the_report_says_no_liquidity_probe_was_run() {
    let mut fixture = Fixture::ready_to_publish();
    let body = report_body(&publish(&mut fixture, EPOCH));

    assert_eq!(body.probe_status, ProbeStatus::NotRequired);
    assert_eq!(body.probe_timestamp.get(), 0);
}

#[test]
fn the_report_carries_the_exact_configuration_version() {
    let mut fixture = Fixture::ready_to_publish();
    let body = report_body(&publish(&mut fixture, EPOCH));

    assert_eq!(body.config_version.get(), fixture.config().config_version);
}

#[test]
fn attributed_custody_counts_as_principal() {
    let mut fixture = Fixture::ready_to_publish();
    fixture.fund_position(ALLOCATE_TRANSFER_ID, 5_000);

    let body = report_body(&publish(&mut fixture, EPOCH));
    assert_eq!(body.remote_principal.get(), 5_000);
    assert_eq!(body.reported_value.get(), 5_000);
}

#[test]
fn deployed_principal_counts_as_principal() {
    let mut fixture = Fixture::ready_to_publish();
    fixture.fund_position(ALLOCATE_TRANSFER_ID, 5_000);
    fixture.deploy(5_000).expect("the deposit lands");

    let body = report_body(&publish(&mut fixture, EPOCH));
    assert_eq!(fixture.position().deployed_principal, 5_000);
    assert_eq!(body.remote_principal.get(), 5_000);
}

#[test]
fn recalled_custody_still_counts_as_principal() {
    let mut fixture = Fixture::ready_to_publish();
    fixture.fund_position(ALLOCATE_TRANSFER_ID, 5_000);
    fixture.deploy(5_000).expect("the deposit lands");
    fixture.accept_recall(RECALL_TRANSFER_ID, 2_000);
    fixture
        .withdraw(RECALL_TRANSFER_ID, 2_000)
        .expect("the withdrawal lands");

    let position = fixture.position();
    assert_eq!(position.recalled_custody, 2_000);

    let body = report_body(&publish(&mut fixture, EPOCH));
    assert_eq!(body.remote_principal.get(), 5_000);
}

#[test]
fn an_unexplained_surplus_is_reported_apart_from_principal() {
    let mut fixture = Fixture::ready_to_publish();
    fixture.fund_position(ALLOCATE_TRANSFER_ID, 5_000);
    fixture.credit(fixture.custody, 700);

    let body = report_body(&publish(&mut fixture, EPOCH));
    assert_eq!(body.remote_principal.get(), 5_000);
    assert_eq!(body.unattributed_balance.get(), 700);
    assert_eq!(body.reported_value.get(), 5_000);
}

#[test]
fn assets_already_sent_leave_the_reported_principal() {
    let mut fixture = Fixture::ready_to_publish();
    fixture.fund_position(ALLOCATE_TRANSFER_ID, 5_000);
    fixture.deploy(5_000).expect("the deposit lands");
    fixture.accept_recall(RECALL_TRANSFER_ID, 5_000);
    fixture
        .withdraw(RECALL_TRANSFER_ID, 5_000)
        .expect("the withdrawal lands");
    fixture
        .send_recall(RECALL_TRANSFER_ID, 5_000)
        .expect("the send lands");

    let body = report_body(&publish(&mut fixture, EPOCH));
    assert_eq!(body.remote_principal.get(), 0);
    assert_eq!(body.reported_value.get(), 0);
}

#[test]
fn a_realized_loss_is_reported_without_being_added_back() {
    let mut fixture = Fixture::ready_to_publish();
    fixture
        .configure_adapter(u64::MAX, 1_000, false)
        .expect("the adapter is configured");
    fixture.fund_position(ALLOCATE_TRANSFER_ID, 10_000);
    fixture.deploy(10_000).expect("the deposit lands");
    fixture.accept_recall(RECALL_TRANSFER_ID, 10_000);
    fixture
        .withdraw(RECALL_TRANSFER_ID, 10_000)
        .expect("the withdrawal lands");
    fixture
        .send_recall(RECALL_TRANSFER_ID, 9_000)
        .expect("the send lands");

    let position = fixture.position();
    assert_eq!(position.cumulative_realized_loss, 1_000);

    let body = report_body(&publish(&mut fixture, EPOCH));
    assert_eq!(body.realized_loss.get(), 1_000);
    assert_eq!(body.remote_principal.get(), 0);
}

#[test]
fn the_latest_completed_transfer_is_reported_exactly() {
    let mut fixture = Fixture::ready_to_publish();
    fixture.fund_position(ALLOCATE_TRANSFER_ID, 5_000);

    let body = report_body(&publish(&mut fixture, EPOCH));
    assert_eq!(
        body.latest_completed_transfer_id.as_bytes(),
        &ALLOCATE_TRANSFER_ID
    );
    assert_eq!(
        fixture.position().latest_completed_transfer_id,
        ALLOCATE_TRANSFER_ID
    );
}

#[test]
fn the_report_commits_to_the_state_it_describes() {
    let mut fixture = Fixture::ready_to_publish();
    fixture.fund_position(ALLOCATE_TRANSFER_ID, 5_000);

    let body = report_body(&publish(&mut fixture, EPOCH));
    assert_ne!(body.remote_state_commitment.as_bytes(), &[0u8; 32]);
}

#[test]
fn a_changed_balance_changes_the_state_commitment() {
    let mut fixture = Fixture::ready_to_publish();
    let first = report_body(&publish(&mut fixture, EPOCH)).remote_state_commitment;

    fixture.fund_position(ALLOCATE_TRANSFER_ID, 5_000);
    let second = report_body(&publish(&mut fixture, EPOCH)).remote_state_commitment;

    assert_ne!(first, second);
}

#[test]
fn a_frozen_leg_still_publishes_its_report() {
    let mut fixture = Fixture::ready_to_publish();
    fixture.fund_position(ALLOCATE_TRANSFER_ID, 5_000);
    let guardian = fixture.guardian.insecure_clone();
    fixture.freeze(&guardian).expect("the leg freezes");

    let body = report_body(&publish(&mut fixture, EPOCH));
    assert_eq!(body.remote_principal.get(), 5_000);
}

#[test]
fn freezing_changes_the_state_commitment() {
    let mut fixture = Fixture::ready_to_publish();
    let before = report_body(&publish(&mut fixture, EPOCH)).remote_state_commitment;

    let guardian = fixture.guardian.insecure_clone();
    fixture.freeze(&guardian).expect("the leg freezes");
    let after = report_body(&publish(&mut fixture, EPOCH)).remote_state_commitment;

    assert_ne!(before, after);
}

#[test]
fn a_frozen_report_creates_no_new_exposure() {
    let mut fixture = Fixture::ready_to_publish();
    fixture.fund_position(ALLOCATE_TRANSFER_ID, 5_000);
    let guardian = fixture.guardian.insecure_clone();
    fixture.freeze(&guardian).expect("the leg freezes");

    let before = fixture.position();
    let custody_before = fixture.token_amount(fixture.custody);
    publish(&mut fixture, EPOCH);
    let after = fixture.position();

    assert_eq!(after.attributed_principal, before.attributed_principal);
    assert_eq!(after.deployed_principal, before.deployed_principal);
    assert_eq!(fixture.token_amount(fixture.custody), custody_before);
}

#[test]
fn publishing_moves_no_tokens() {
    let mut fixture = Fixture::ready_to_publish();
    fixture.fund_position(ALLOCATE_TRANSFER_ID, 5_000);

    let custody = fixture.token_amount(fixture.custody);
    let escrow = fixture.token_amount(fixture.escrow);
    let vault = fixture.token_amount(fixture.adapter_vault);

    publish(&mut fixture, EPOCH);

    assert_eq!(fixture.token_amount(fixture.custody), custody);
    assert_eq!(fixture.token_amount(fixture.escrow), escrow);
    assert_eq!(fixture.token_amount(fixture.adapter_vault), vault);
}

#[test]
fn the_record_stores_the_identifiers_the_event_announced() {
    let mut fixture = Fixture::ready_to_publish();
    let event = publish(&mut fixture, EPOCH);
    let record = fixture.report_record(1);

    assert_eq!(record.state_version, 1);
    assert_eq!(record.report_type, ReportType::RemoteReport);
    assert_eq!(record.lane_id, REPORT_LANE_ID);
    assert_eq!(record.sequence, 1);
    assert_eq!(record.message_id, event.message_id);
    assert_eq!(record.body_hash, event.body_hash);
    assert_eq!(record.previous_commitment, [0u8; 32]);
    assert_eq!(record.next_commitment, event.next_commitment);
    assert_eq!(record.encoded_len, 400);
    assert_eq!(record.epoch_id, EPOCH);
    assert!(record.published_at > 0);
}

#[test]
fn the_record_bump_is_the_canonical_one() {
    let mut fixture = Fixture::ready_to_publish();
    publish(&mut fixture, EPOCH);

    let (_, bump) = ReportRecord::address(&fixture.config, REPORT_LANE_ID, 1);
    assert_eq!(fixture.report_record(1).bump, bump);
}

#[test]
fn the_lane_advances_once_for_one_report() {
    let mut fixture = Fixture::ready_to_publish();
    let event = publish(&mut fixture, EPOCH);
    let lane = fixture.outbound_lane();

    assert_eq!(lane.next_sequence, 2);
    assert_eq!(lane.current_commitment, event.next_commitment);
    assert_eq!(lane.total_reports, 1);
    assert_eq!(lane.total_recall_notifications, 0);
    assert!(lane.last_publication_at > 0);
}

#[test]
fn a_second_report_chains_onto_the_first() {
    let mut fixture = Fixture::ready_to_publish();
    let first = publish(&mut fixture, EPOCH);
    let second = publish(&mut fixture, EPOCH + 1);

    assert_eq!(second.sequence, 2);
    assert_eq!(second.previous_commitment, first.next_commitment);
    assert_ne!(second.next_commitment, first.next_commitment);

    let header = decoded(&second).header;
    assert_eq!(
        header.previous_commitment.as_bytes(),
        &first.next_commitment
    );
    assert_eq!(header.sequence.get(), 2);
}

#[test]
fn a_different_epoch_gives_the_report_a_different_identity() {
    let mut fixture = Fixture::ready_to_publish();
    let first = publish(&mut fixture, EPOCH);
    let second = publish(&mut fixture, EPOCH + 1);

    assert_ne!(first.message_id, second.message_id);
    assert_ne!(first.body_hash, second.body_hash);
}

#[test]
fn an_epoch_of_zero_is_rejected_by_the_shared_rules() {
    let mut fixture = Fixture::ready_to_publish();
    expect_error(
        fixture.publish_report(0),
        RemoteLegError::InvalidOutboundMessage,
    );
}

#[test]
fn a_prefunded_record_address_does_not_block_publication() {
    let mut fixture = Fixture::ready_to_publish();
    let record = fixture.report_record_key(1);
    fixture.prefund(record, 5_000_000);

    publish(&mut fixture, EPOCH);
    assert_eq!(fixture.report_record(1).sequence, 1);
}

#[test]
fn a_record_owned_by_a_stranger_is_rejected() {
    let mut fixture = Fixture::ready_to_publish();
    let record = fixture.report_record_key(1);
    fixture.give_account_to(record, Pubkey::new_unique(), 64);

    expect_error(
        fixture.publish_report(EPOCH),
        RemoteLegError::InvalidReportRecord,
    );
}

#[test]
fn a_record_from_an_earlier_sequence_cannot_be_reused() {
    let mut fixture = Fixture::ready_to_publish();
    publish(&mut fixture, EPOCH);

    // The lane has moved on, so the first record no longer names this slot.
    let accounts = fixture.outbound_accounts(1);
    expect_error(
        fixture.publish_report_with(accounts, EPOCH),
        RemoteLegError::InvalidReportRecord,
    );
}

#[test]
fn a_record_slot_this_program_already_owns_is_rejected() {
    let mut fixture = Fixture::ready_to_publish();
    let record = fixture.report_record_key(1);
    fixture.give_account_to(record, solevm_remote_leg::ID, 64);

    expect_error(
        fixture.publish_report(EPOCH),
        RemoteLegError::ReportAlreadyPublished,
    );
}

#[test]
fn a_record_for_another_sequence_is_rejected() {
    let mut fixture = Fixture::ready_to_publish();
    let mut accounts = fixture.outbound_accounts(1);
    accounts.report_record = fixture.report_record_key(7);

    expect_error(
        fixture.publish_report_with(accounts, EPOCH),
        RemoteLegError::InvalidReportRecord,
    );
}

#[test]
fn a_rejected_report_consumes_no_sequence_and_leaves_no_gap() {
    let mut fixture = Fixture::ready_to_publish();
    let before = fixture.outbound_lane();

    expect_rejection(fixture.publish_report(0));

    let after = fixture.outbound_lane();
    assert_eq!(after.next_sequence, before.next_sequence);
    assert_eq!(after.current_commitment, before.current_commitment);
    assert_eq!(after.total_reports, before.total_reports);
    assert!(!fixture.report_record_exists(1));

    publish(&mut fixture, EPOCH);
    assert_eq!(fixture.report_record(1).sequence, 1);
}

#[test]
fn a_custody_deficit_is_rejected() {
    let mut fixture = Fixture::ready_to_publish();
    fixture.fund_position(ALLOCATE_TRANSFER_ID, 5_000);
    fixture.set_token_amount(fixture.custody, 1_000);

    expect_error(
        fixture.publish_report(EPOCH),
        RemoteLegError::AccountingDeficit,
    );
}

#[test]
fn an_adapter_that_disagrees_about_the_principal_is_rejected() {
    let mut fixture = Fixture::ready_to_publish();
    fixture.fund_position(ALLOCATE_TRANSFER_ID, 5_000);
    fixture.deploy(5_000).expect("the deposit lands");
    fixture.set_adapter_principal(4_000);

    expect_error(
        fixture.publish_report(EPOCH),
        RemoteLegError::InvalidPrincipalDelta,
    );
}

#[test]
fn a_lane_from_another_deployment_is_rejected() {
    let mut fixture = Fixture::ready_to_publish();
    let mut accounts = fixture.outbound_accounts(1);
    accounts.outbound_lane = Pubkey::new_unique();

    expect_rejection(fixture.publish_report_with(accounts, EPOCH));
}

#[test]
fn a_position_from_another_deployment_is_rejected() {
    let mut fixture = Fixture::ready_to_publish();
    let mut accounts = fixture.outbound_accounts(1);
    accounts.remote_position = Pubkey::new_unique();

    expect_rejection(fixture.publish_report_with(accounts, EPOCH));
}

#[test]
fn a_custody_account_the_configuration_does_not_name_is_rejected() {
    let mut fixture = Fixture::ready_to_publish();
    let mut accounts = fixture.outbound_accounts(1);
    accounts.custody_token_account = fixture.escrow;

    expect_error(
        fixture.publish_report_with(accounts, EPOCH),
        RemoteLegError::InvalidCustodyAccount,
    );
}

#[test]
fn an_adapter_account_the_strategy_does_not_name_is_rejected() {
    let mut fixture = Fixture::ready_to_publish();
    let mut accounts = fixture.outbound_accounts(1);
    accounts.adapter_state = Pubkey::new_unique();

    expect_error(
        fixture.publish_report_with(accounts, EPOCH),
        RemoteLegError::InvalidAdapterState,
    );
}

#[test]
fn a_risk_account_from_another_deployment_is_rejected() {
    let mut fixture = Fixture::ready_to_publish();
    let mut accounts = fixture.outbound_accounts(1);
    accounts.risk_config = Pubkey::new_unique();

    expect_rejection(fixture.publish_report_with(accounts, EPOCH));
}

#[test]
fn a_strategy_account_from_another_deployment_is_rejected() {
    let mut fixture = Fixture::ready_to_publish();
    let mut accounts = fixture.outbound_accounts(1);
    accounts.strategy_config = Pubkey::new_unique();

    expect_rejection(fixture.publish_report_with(accounts, EPOCH));
}

#[test]
fn the_reported_value_never_rises_above_the_principal() {
    let mut fixture = Fixture::ready_to_publish();
    fixture.fund_position(ALLOCATE_TRANSFER_ID, 5_000);
    fixture.deploy(5_000).expect("the deposit lands");
    // A generous adapter must not turn into reported profit.
    fixture.credit(fixture.adapter_vault, 50_000);

    let body = report_body(&publish(&mut fixture, EPOCH));
    assert_eq!(body.reported_value.get(), body.remote_principal.get());
    assert_eq!(body.reported_value.get(), 5_000);
}

#[test]
fn the_event_and_the_record_agree_with_the_bytes_that_were_sent() {
    let mut fixture = Fixture::ready_to_publish();
    let event = publish(&mut fixture, EPOCH);

    let rebuilt = protocol_types::decode_message(&event.message_bytes).expect("bytes decode");
    let encoded = rebuilt.encode().expect("the report re-encodes");

    assert_eq!(encoded.as_bytes(), event.message_bytes.as_slice());
    assert_eq!(encoded.message_id().to_bytes(), event.message_id);
    assert_eq!(
        rebuilt.body_hash().expect("the body hashes").to_bytes(),
        event.body_hash
    );
}

#[test]
fn every_publication_emits_its_own_canonical_bytes() {
    let mut fixture = Fixture::ready_to_publish();
    let first = publish(&mut fixture, EPOCH);
    let second = publish(&mut fixture, EPOCH);

    assert_ne!(first.message_bytes, second.message_bytes);
    assert_eq!(first.message_bytes.len(), 400);
    assert_eq!(second.message_bytes.len(), 400);
}

#[test]
fn the_reported_state_matches_the_position_the_leg_holds() {
    let mut fixture = Fixture::ready_to_publish();
    fixture.fund_position(ALLOCATE_TRANSFER_ID, 8_000);
    fixture.deploy(3_000).expect("the deposit lands");
    fixture.credit(fixture.custody, 250);

    let body = report_body(&publish(&mut fixture, EPOCH));
    let position = fixture.position();

    assert_eq!(
        body.remote_principal.get(),
        u128::from(
            position.attributed_principal + position.deployed_principal + position.recalled_custody
        )
    );
    assert_eq!(
        body.unattributed_balance.get(),
        u128::from(position.unattributed_custody)
    );
    assert_eq!(
        body.realized_loss.get(),
        u128::from(position.cumulative_realized_loss)
    );
}

#[test]
fn a_surplus_is_reconciled_into_the_position_as_the_report_is_built() {
    let mut fixture = Fixture::ready_to_publish();
    fixture.credit(fixture.custody, 900);
    assert_eq!(fixture.position().unattributed_custody, 0);

    let body = report_body(&publish(&mut fixture, EPOCH));

    assert_eq!(fixture.position().unattributed_custody, 900);
    assert_eq!(body.unattributed_balance.get(), 900);
}

#[test]
fn the_published_event_names_the_record_it_created() {
    let mut fixture = Fixture::ready_to_publish();
    let event = publish(&mut fixture, EPOCH);

    assert_eq!(event.report_record, fixture.report_record_key(1));
    assert_eq!(event.remote_config, fixture.config);
    assert_eq!(event.lane_id, REPORT_LANE_ID);
}

#[test]
fn a_report_is_readable_from_the_logs_of_its_own_transaction() {
    let mut fixture = Fixture::ready_to_publish();
    let sequence = fixture.outbound_lane().next_sequence;
    let accounts = fixture.outbound_accounts(sequence);
    let instruction = fixture.publish_report_instruction(accounts, EPOCH);

    let payer = fixture.outsider.insecure_clone();
    let result = fixture.send_as(instruction, &payer, &[&payer]);
    assert!(result.is_ok());

    let logs = fixture.last_logs();
    let event: RemoteReportPublished = last_event(&logs);
    assert_eq!(event.encoded_len, 400);
}
