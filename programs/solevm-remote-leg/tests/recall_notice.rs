//! Publishing the one terminal notice of a finished recall.

#![allow(clippy::unwrap_used, clippy::panic, clippy::arithmetic_side_effects)]

mod common;

use common::{
    ALLOCATE_TRANSFER_ID, DEPLOYMENT_ID, Fixture, LOCAL_APPLICATION_ID, Pubkey, RECALL_TRANSFER_ID,
    REPORT_LANE_ID, SOURCE_APPLICATION_ID, SOURCE_CHAIN_ID, VAULT_ID, expect_error,
    expect_rejection,
};
use protocol_types::{Body, layout};
use solevm_remote_leg::{
    RecallSentMarker, RecallSentPublished, RemoteLegError, ReportType, TransferStatus,
};

const AMOUNT: u64 = 1_000_000;

/// Drives one recall from arrival to a completed send.
fn complete_recall(fixture: &mut Fixture, amount: u64, sent: u64) {
    fixture.fund_position(ALLOCATE_TRANSFER_ID, amount);
    fixture.deploy(amount).expect("the deposit lands");
    fixture.accept_recall(RECALL_TRANSFER_ID, amount);
    fixture
        .withdraw(RECALL_TRANSFER_ID, amount)
        .expect("the withdrawal lands");
    if sent > 0 {
        fixture
            .send_recall(RECALL_TRANSFER_ID, sent)
            .expect("the send lands");
    }
}

/// A leg holding one completed recall with no loss.
fn settled() -> Fixture {
    let mut fixture = Fixture::ready_to_publish();
    complete_recall(&mut fixture, AMOUNT, AMOUNT);
    fixture
}

fn publish(fixture: &mut Fixture) -> RecallSentPublished {
    let sequence = fixture.outbound_lane().next_sequence;
    let accounts = fixture.recall_sent_accounts(RECALL_TRANSFER_ID, sequence);
    let instruction = fixture.publish_recall_sent_instruction(accounts);
    fixture.publish_and_capture(instruction)
}

fn notice_body(event: &RecallSentPublished) -> protocol_types::RecallSentBody {
    let message = protocol_types::decode_message(&event.message_bytes).expect("the notice decodes");
    match message.body {
        Body::RecallSent(body) => body,
        other => panic!("expected a recall notice, found {other:?}"),
    }
}

#[test]
fn a_completed_recall_publishes_its_notice() {
    let mut fixture = settled();
    let body = notice_body(&publish(&mut fixture));

    assert_eq!(body.transfer_id.as_bytes(), &RECALL_TRANSFER_ID);
    assert_eq!(body.principal_sent.get(), u128::from(AMOUNT));
    assert_eq!(body.actual_amount_sent.get(), u128::from(AMOUNT));
    assert_eq!(body.realized_loss.get(), 0);
}

#[test]
fn the_notice_stays_inside_the_transport_budget() {
    let mut fixture = settled();
    let event = publish(&mut fixture);

    assert_eq!(event.message_bytes.len(), layout::RECALL_SENT_MESSAGE_LEN);
    assert_eq!(
        usize::from(event.encoded_len),
        layout::RECALL_SENT_MESSAGE_LEN
    );
    assert!(event.message_bytes.len() <= 400);
}

#[test]
fn the_header_sends_the_notice_back_the_way_the_orders_came() {
    let mut fixture = settled();
    let event = publish(&mut fixture);
    let header = protocol_types::decode_message(&event.message_bytes)
        .expect("the notice decodes")
        .header;

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
}

#[test]
fn the_notice_points_at_the_configured_escrow() {
    let mut fixture = settled();
    let body = notice_body(&publish(&mut fixture));

    assert_eq!(
        body.destination_reference.as_bytes(),
        &fixture.config().outbound_escrow.to_bytes()
    );
}

#[test]
fn the_send_time_is_the_time_the_recall_completed() {
    let mut fixture = settled();
    let completed_at = fixture.transfer(&RECALL_TRANSFER_ID).completed_at;
    let body = notice_body(&publish(&mut fixture));

    assert_eq!(
        body.sent_timestamp.get(),
        u64::try_from(completed_at).unwrap()
    );
}

#[test]
fn the_notice_carries_the_current_configuration_version() {
    let mut fixture = settled();
    let body = notice_body(&publish(&mut fixture));

    assert_eq!(body.config_version.get(), fixture.config().config_version);
}

#[test]
fn a_recall_that_lost_value_reports_both_numbers() {
    let mut fixture = Fixture::ready_to_publish();
    fixture
        .configure_adapter(u64::MAX, 1_000, false)
        .expect("the adapter is configured");
    complete_recall(&mut fixture, AMOUNT, AMOUNT - AMOUNT / 10);

    let record = fixture.transfer(&RECALL_TRANSFER_ID);
    assert_eq!(record.status, TransferStatus::Complete);

    let body = notice_body(&publish(&mut fixture));
    assert_eq!(body.principal_sent.get(), u128::from(AMOUNT));
    assert_eq!(
        body.actual_amount_sent.get(),
        u128::from(record.assets_sent)
    );
    assert_eq!(body.realized_loss.get(), u128::from(record.realized_loss));
    assert_eq!(
        body.actual_amount_sent.get() + body.realized_loss.get(),
        body.principal_sent.get()
    );
}

#[test]
fn a_total_loss_publishes_a_notice_that_sent_nothing() {
    let mut fixture = Fixture::ready_to_publish();
    fixture
        .configure_adapter(u64::MAX, 10_000, false)
        .expect("the adapter is configured");
    complete_recall(&mut fixture, AMOUNT, 0);

    let record = fixture.transfer(&RECALL_TRANSFER_ID);
    assert_eq!(record.status, TransferStatus::Complete);
    assert_eq!(record.assets_sent, 0);
    assert_eq!(record.realized_loss, AMOUNT);

    let body = notice_body(&publish(&mut fixture));
    assert_eq!(body.actual_amount_sent.get(), 0);
    assert_eq!(body.realized_loss.get(), u128::from(AMOUNT));
    assert_eq!(body.principal_sent.get(), u128::from(AMOUNT));
}

#[test]
fn a_pending_recall_has_nothing_to_announce() {
    let mut fixture = Fixture::ready_to_publish();
    fixture.fund_position(ALLOCATE_TRANSFER_ID, AMOUNT);
    fixture.deploy(AMOUNT).expect("the deposit lands");
    fixture.accept_recall(RECALL_TRANSFER_ID, AMOUNT);

    expect_error(
        fixture.publish_recall_sent(RECALL_TRANSFER_ID),
        RemoteLegError::RecallNotComplete,
    );
}

#[test]
fn a_partly_sent_recall_has_nothing_to_announce() {
    let mut fixture = Fixture::ready_to_publish();
    fixture.fund_position(ALLOCATE_TRANSFER_ID, AMOUNT);
    fixture.deploy(AMOUNT).expect("the deposit lands");
    fixture.accept_recall(RECALL_TRANSFER_ID, AMOUNT);
    fixture
        .withdraw(RECALL_TRANSFER_ID, AMOUNT)
        .expect("the withdrawal lands");
    fixture
        .send_recall(RECALL_TRANSFER_ID, AMOUNT / 2)
        .expect("the partial send lands");

    assert_eq!(
        fixture.transfer(&RECALL_TRANSFER_ID).status,
        TransferStatus::Pending
    );
    expect_error(
        fixture.publish_recall_sent(RECALL_TRANSFER_ID),
        RemoteLegError::RecallNotComplete,
    );
}

#[test]
fn an_allocation_is_never_announced_as_a_recall() {
    let mut fixture = Fixture::ready_to_publish();
    fixture.fund_position(ALLOCATE_TRANSFER_ID, AMOUNT);

    expect_error(
        fixture.publish_recall_sent(ALLOCATE_TRANSFER_ID),
        RemoteLegError::InvalidRecallIdentity,
    );
}

#[test]
fn the_notice_publishes_only_once() {
    let mut fixture = settled();
    publish(&mut fixture);

    expect_error(
        fixture.publish_recall_sent(RECALL_TRANSFER_ID),
        RemoteLegError::RecallSentAlreadyPublished,
    );
}

#[test]
fn a_refused_repeat_consumes_no_sequence() {
    let mut fixture = settled();
    publish(&mut fixture);
    let before = fixture.outbound_lane();

    expect_error(
        fixture.publish_recall_sent(RECALL_TRANSFER_ID),
        RemoteLegError::RecallSentAlreadyPublished,
    );

    let after = fixture.outbound_lane();
    assert_eq!(after.next_sequence, before.next_sequence);
    assert_eq!(after.current_commitment, before.current_commitment);
    assert_eq!(
        after.total_recall_notifications,
        before.total_recall_notifications
    );
}

#[test]
fn the_marker_records_the_notice_that_was_sent() {
    let mut fixture = settled();
    let event = publish(&mut fixture);
    let marker = fixture.recall_marker(&RECALL_TRANSFER_ID);

    assert_eq!(marker.state_version, 1);
    assert_eq!(marker.transfer_id, RECALL_TRANSFER_ID);
    assert_eq!(marker.report_sequence, event.sequence);
    assert_eq!(marker.message_id, event.message_id);
    assert!(marker.published_at > 0);

    let (_, bump) = RecallSentMarker::address(&fixture.config, &RECALL_TRANSFER_ID);
    assert_eq!(marker.bump, bump);
}

#[test]
fn the_record_stores_the_recall_it_announced() {
    let mut fixture = settled();
    let event = publish(&mut fixture);
    let record = fixture.report_record(event.sequence);

    assert_eq!(record.report_type, ReportType::RecallSent);
    assert_eq!(record.related_transfer_id, RECALL_TRANSFER_ID);
    assert_eq!(record.epoch_id, 0);
    assert_eq!(record.message_id, event.message_id);
    assert_eq!(record.body_hash, event.body_hash);
    assert_eq!(record.next_commitment, event.next_commitment);
}

#[test]
fn a_prefunded_marker_address_does_not_block_the_notice() {
    let mut fixture = settled();
    let marker = fixture.recall_marker_key(&RECALL_TRANSFER_ID);
    fixture.prefund(marker, 5_000_000);

    publish(&mut fixture);
    assert_eq!(
        fixture.recall_marker(&RECALL_TRANSFER_ID).transfer_id,
        RECALL_TRANSFER_ID
    );
}

#[test]
fn a_marker_owned_by_a_stranger_is_rejected() {
    let mut fixture = settled();
    let marker = fixture.recall_marker_key(&RECALL_TRANSFER_ID);
    fixture.give_account_to(marker, Pubkey::new_unique(), 64);

    expect_error(
        fixture.publish_recall_sent(RECALL_TRANSFER_ID),
        RemoteLegError::InvalidRecallIdentity,
    );
}

#[test]
fn a_marker_for_another_transfer_is_rejected() {
    let mut fixture = settled();
    let sequence = fixture.outbound_lane().next_sequence;
    let mut accounts = fixture.recall_sent_accounts(RECALL_TRANSFER_ID, sequence);
    accounts.recall_sent_marker = fixture.recall_marker_key(&ALLOCATE_TRANSFER_ID);

    expect_error(
        fixture.publish_recall_sent_with(accounts),
        RemoteLegError::InvalidRecallIdentity,
    );
}

#[test]
fn a_transfer_record_from_another_deployment_is_rejected() {
    let mut fixture = settled();
    let sequence = fixture.outbound_lane().next_sequence;
    let mut accounts = fixture.recall_sent_accounts(RECALL_TRANSFER_ID, sequence);
    accounts.transfer_record = Pubkey::new_unique();

    expect_rejection(fixture.publish_recall_sent_with(accounts));
}

#[test]
fn a_lane_from_another_deployment_is_rejected() {
    let mut fixture = settled();
    let sequence = fixture.outbound_lane().next_sequence;
    let mut accounts = fixture.recall_sent_accounts(RECALL_TRANSFER_ID, sequence);
    accounts.outbound_lane = Pubkey::new_unique();

    expect_rejection(fixture.publish_recall_sent_with(accounts));
}

#[test]
fn a_record_for_the_wrong_sequence_is_rejected() {
    let mut fixture = settled();
    let sequence = fixture.outbound_lane().next_sequence;
    let mut accounts = fixture.recall_sent_accounts(RECALL_TRANSFER_ID, sequence);
    accounts.report_record = fixture.report_record_key(sequence + 5);

    expect_error(
        fixture.publish_recall_sent_with(accounts),
        RemoteLegError::InvalidReportRecord,
    );
}

#[test]
fn a_frozen_leg_still_announces_a_finished_recall() {
    let mut fixture = settled();
    let guardian = fixture.guardian.insecure_clone();
    fixture.freeze(&guardian).expect("the leg freezes");

    let body = notice_body(&publish(&mut fixture));
    assert_eq!(body.actual_amount_sent.get(), u128::from(AMOUNT));
}

#[test]
fn announcing_a_recall_moves_no_tokens() {
    let mut fixture = settled();
    let custody = fixture.token_amount(fixture.custody);
    let escrow = fixture.token_amount(fixture.escrow);

    publish(&mut fixture);

    assert_eq!(fixture.token_amount(fixture.custody), custody);
    assert_eq!(fixture.token_amount(fixture.escrow), escrow);
}

#[test]
fn announcing_a_recall_changes_no_transfer_state() {
    let mut fixture = settled();
    let before = fixture.transfer(&RECALL_TRANSFER_ID);
    let position_before = fixture.position();

    publish(&mut fixture);

    let after = fixture.transfer(&RECALL_TRANSFER_ID);
    assert_eq!(after.assets_sent, before.assets_sent);
    assert_eq!(after.realized_loss, before.realized_loss);
    assert_eq!(after.status, before.status);
    assert_eq!(after.completed_at, before.completed_at);
    assert_eq!(
        fixture.position().cumulative_realized_loss,
        position_before.cumulative_realized_loss
    );
}

#[test]
fn the_notice_says_only_what_the_leg_can_prove() {
    let mut fixture = settled();
    let body = notice_body(&publish(&mut fixture));

    // The escrow is the leg's own account, not a hub receipt.
    assert_eq!(
        body.destination_reference.as_bytes(),
        &fixture.escrow.to_bytes()
    );
    assert_eq!(
        u64::try_from(body.actual_amount_sent.get()).unwrap(),
        fixture.token_amount(fixture.escrow)
    );
}

#[test]
fn the_notice_re_encodes_to_the_bytes_that_were_emitted() {
    let mut fixture = settled();
    let event = publish(&mut fixture);

    let rebuilt = protocol_types::decode_message(&event.message_bytes).expect("the notice decodes");
    let encoded = rebuilt.encode().expect("the notice re-encodes");

    assert_eq!(encoded.as_bytes(), event.message_bytes.as_slice());
    assert_eq!(encoded.message_id().to_bytes(), event.message_id);
}

#[test]
fn the_lane_counts_the_notice_on_its_own_tally() {
    let mut fixture = settled();
    publish(&mut fixture);
    let lane = fixture.outbound_lane();

    assert_eq!(lane.total_recall_notifications, 1);
    assert_eq!(lane.total_reports, 0);
    assert_eq!(lane.next_sequence, 2);
}
