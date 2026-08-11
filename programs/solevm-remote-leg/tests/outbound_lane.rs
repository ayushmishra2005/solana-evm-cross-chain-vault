//! Both outbound message types share one order on one lane.

#![allow(clippy::unwrap_used, clippy::panic, clippy::arithmetic_side_effects)]

mod common;

use common::{
    ALLOCATE_TRANSFER_ID, Fixture, Pubkey, RECALL_TRANSFER_ID, expect_error, expect_rejection,
};
use solevm_remote_leg::{RecallSentPublished, RemoteLegError, RemoteReportPublished, ReportType};

const AMOUNT: u64 = 1_000_000;
const EPOCH: u64 = 12;

/// A leg with one completed recall, ready to announce it.
fn settled() -> Fixture {
    let mut fixture = Fixture::ready_to_publish();
    fixture.fund_position(ALLOCATE_TRANSFER_ID, AMOUNT);
    fixture.deploy(AMOUNT).expect("the deposit lands");
    fixture.accept_recall(RECALL_TRANSFER_ID, AMOUNT);
    fixture
        .withdraw(RECALL_TRANSFER_ID, AMOUNT)
        .expect("the withdrawal lands");
    fixture
        .send_recall(RECALL_TRANSFER_ID, AMOUNT)
        .expect("the send lands");
    fixture
}

fn report(fixture: &mut Fixture, epoch_id: u64) -> RemoteReportPublished {
    let sequence = fixture.outbound_lane().next_sequence;
    let accounts = fixture.outbound_accounts(sequence);
    let instruction = fixture.publish_report_instruction(accounts, epoch_id);
    fixture.publish_and_capture(instruction)
}

fn notice(fixture: &mut Fixture) -> RecallSentPublished {
    let sequence = fixture.outbound_lane().next_sequence;
    let accounts = fixture.recall_sent_accounts(RECALL_TRANSFER_ID, sequence);
    let instruction = fixture.publish_recall_sent_instruction(accounts);
    fixture.publish_and_capture(instruction)
}

#[test]
fn a_report_then_a_notice_chain_in_order() {
    let mut fixture = settled();
    let first = report(&mut fixture, EPOCH);
    let second = notice(&mut fixture);

    assert_eq!(first.sequence, 1);
    assert_eq!(second.sequence, 2);
    assert_eq!(first.previous_commitment, [0u8; 32]);
    assert_eq!(second.previous_commitment, first.next_commitment);
}

#[test]
fn a_notice_then_a_report_chain_in_order() {
    let mut fixture = settled();
    let first = notice(&mut fixture);
    let second = report(&mut fixture, EPOCH);

    assert_eq!(first.sequence, 1);
    assert_eq!(second.sequence, 2);
    assert_eq!(second.previous_commitment, first.next_commitment);
}

#[test]
fn both_message_types_draw_from_one_rising_sequence() {
    let mut fixture = settled();
    let first = report(&mut fixture, EPOCH);
    let second = notice(&mut fixture);
    let third = report(&mut fixture, EPOCH + 1);

    assert_eq!([first.sequence, second.sequence, third.sequence], [1, 2, 3]);

    let lane = fixture.outbound_lane();
    assert_eq!(lane.next_sequence, 4);
    assert_eq!(lane.total_reports, 2);
    assert_eq!(lane.total_recall_notifications, 1);
}

#[test]
fn each_sequence_holds_exactly_one_record_of_a_known_type() {
    let mut fixture = settled();
    report(&mut fixture, EPOCH);
    notice(&mut fixture);
    report(&mut fixture, EPOCH + 1);

    assert_eq!(
        fixture.report_record(1).report_type,
        ReportType::RemoteReport
    );
    assert_eq!(fixture.report_record(2).report_type, ReportType::RecallSent);
    assert_eq!(
        fixture.report_record(3).report_type,
        ReportType::RemoteReport
    );

    for sequence in 1..=3 {
        assert_eq!(fixture.report_record(sequence).sequence, sequence);
    }
}

#[test]
fn every_record_sits_at_its_own_address() {
    let mut fixture = settled();
    report(&mut fixture, EPOCH);
    notice(&mut fixture);

    assert_ne!(fixture.report_record_key(1), fixture.report_record_key(2));
}

#[test]
fn the_commitment_replays_from_the_records_alone() {
    let mut fixture = settled();
    report(&mut fixture, EPOCH);
    notice(&mut fixture);
    report(&mut fixture, EPOCH + 1);

    let mut replayed = [0u8; 32];
    for sequence in 1..=3 {
        let record = fixture.report_record(sequence);
        assert_eq!(record.previous_commitment, replayed);
        replayed = solevm_remote_leg::message::next_commitment(&replayed, &record.message_id);
        assert_eq!(record.next_commitment, replayed);
    }

    assert_eq!(fixture.outbound_lane().current_commitment, replayed);
}

#[test]
fn a_failed_report_leaves_no_gap_in_the_order() {
    let mut fixture = settled();
    report(&mut fixture, EPOCH);

    // A zero epoch never reaches the lane.
    expect_rejection(fixture.publish_report(0));

    let second = notice(&mut fixture);
    assert_eq!(second.sequence, 2);
    assert!(!fixture.report_record_exists(3));
}

#[test]
fn a_failed_notice_leaves_no_gap_in_the_order() {
    let mut fixture = settled();
    notice(&mut fixture);

    expect_error(
        fixture.publish_recall_sent(RECALL_TRANSFER_ID),
        RemoteLegError::RecallSentAlreadyPublished,
    );

    let second = report(&mut fixture, EPOCH);
    assert_eq!(second.sequence, 2);
    assert!(!fixture.report_record_exists(3));
}

#[test]
fn two_messages_can_never_share_one_sequence() {
    let mut fixture = settled();
    report(&mut fixture, EPOCH);

    // The notice must take the next slot, not the one already written.
    let accounts = fixture.recall_sent_accounts(RECALL_TRANSFER_ID, 1);
    expect_error(
        fixture.publish_recall_sent_with(accounts),
        RemoteLegError::InvalidReportRecord,
    );

    let second = notice(&mut fixture);
    assert_eq!(second.sequence, 2);
}

#[test]
fn a_stale_record_address_cannot_overwrite_an_earlier_message() {
    let mut fixture = settled();
    let first = report(&mut fixture, EPOCH);

    let accounts = fixture.outbound_accounts(1);
    expect_error(
        fixture.publish_report_with(accounts, EPOCH),
        RemoteLegError::InvalidReportRecord,
    );

    assert_eq!(fixture.report_record(1).message_id, first.message_id);
}

#[test]
fn a_record_never_changes_after_it_is_written() {
    let mut fixture = settled();
    let first = report(&mut fixture, EPOCH);
    let before = fixture.report_record(1);

    notice(&mut fixture);
    report(&mut fixture, EPOCH + 1);

    let after = fixture.report_record(1);
    assert_eq!(after.message_id, before.message_id);
    assert_eq!(after.message_id, first.message_id);
    assert_eq!(after.next_commitment, before.next_commitment);
    assert_eq!(after.published_at, before.published_at);
    assert_eq!(after.epoch_id, before.epoch_id);
}

#[test]
fn the_lane_only_moves_when_a_message_really_left() {
    let mut fixture = settled();
    let before = fixture.outbound_lane();

    expect_rejection(fixture.publish_report(0));
    let mut accounts = fixture.outbound_accounts(1);
    accounts.adapter_state = Pubkey::new_unique();
    expect_rejection(fixture.publish_report_with(accounts, EPOCH));

    let after = fixture.outbound_lane();
    assert_eq!(after.next_sequence, before.next_sequence);
    assert_eq!(after.current_commitment, before.current_commitment);
    assert_eq!(after.last_publication_at, before.last_publication_at);
    assert_eq!(after.total_reports, before.total_reports);
    assert_eq!(
        after.total_recall_notifications,
        before.total_recall_notifications
    );
}

#[test]
fn one_recall_produces_exactly_one_notice_across_many_reports() {
    let mut fixture = settled();
    report(&mut fixture, EPOCH);
    notice(&mut fixture);
    report(&mut fixture, EPOCH + 1);

    expect_error(
        fixture.publish_recall_sent(RECALL_TRANSFER_ID),
        RemoteLegError::RecallSentAlreadyPublished,
    );

    assert_eq!(fixture.outbound_lane().total_recall_notifications, 1);
}
