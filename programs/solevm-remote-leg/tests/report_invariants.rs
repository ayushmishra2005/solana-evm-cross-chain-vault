//! The properties outbound publication must keep under every history.
//!
//! Each case drives a real leg through a sequence of financial events and
//! then checks the rule against on chain state rather than against a fixed
//! expected value.

#![allow(clippy::unwrap_used, clippy::panic, clippy::arithmetic_side_effects)]

mod common;

use anchor_lang::prelude::Pubkey;
use common::{ALLOCATE_TRANSFER_ID, Fixture, RECALL_TRANSFER_ID, expect_error, expect_rejection};
use protocol_types::Body;
use solana_signer::Signer;
use solevm_remote_leg::{
    RecallSentPublished, RemoteLegError, RemoteReportPublished, RemoteStateView, ReportType,
    TransferStatus, remote_state,
};

const AMOUNT: u64 = 1_000_000;

/// One financial history a leg can end up in.
#[derive(Clone, Copy, Debug)]
struct History {
    name: &'static str,
    allocated: u64,
    deployed: u64,
    recalled: u64,
    sent: u64,
    surplus: u64,
}

/// The histories every property below is checked against.
const HISTORIES: &[History] = &[
    History {
        name: "an untouched leg",
        allocated: 0,
        deployed: 0,
        recalled: 0,
        sent: 0,
        surplus: 0,
    },
    History {
        name: "custody attributed but not deployed",
        allocated: AMOUNT,
        deployed: 0,
        recalled: 0,
        sent: 0,
        surplus: 0,
    },
    History {
        name: "everything deployed",
        allocated: AMOUNT,
        deployed: AMOUNT,
        recalled: 0,
        sent: 0,
        surplus: 0,
    },
    History {
        name: "part deployed and part held",
        allocated: AMOUNT,
        deployed: 400_000,
        recalled: 0,
        sent: 0,
        surplus: 0,
    },
    History {
        name: "a recall waiting to leave",
        allocated: AMOUNT,
        deployed: AMOUNT,
        recalled: AMOUNT,
        sent: 0,
        surplus: 0,
    },
    History {
        name: "a recall already sent",
        allocated: AMOUNT,
        deployed: AMOUNT,
        recalled: AMOUNT,
        sent: AMOUNT,
        surplus: 0,
    },
    History {
        name: "a surplus nobody attributed",
        allocated: AMOUNT,
        deployed: AMOUNT,
        recalled: 0,
        sent: 0,
        surplus: 250_000,
    },
    History {
        name: "a surplus on a partly deployed leg",
        allocated: AMOUNT,
        deployed: 600_000,
        recalled: 0,
        sent: 0,
        surplus: 75_000,
    },
];

/// Builds a leg that already lived through one history.
fn leg(history: History) -> Fixture {
    let mut fixture = Fixture::ready_to_publish();

    if history.allocated > 0 {
        fixture.fund_position(ALLOCATE_TRANSFER_ID, history.allocated);
    }
    if history.deployed > 0 {
        fixture.deploy(history.deployed).expect("the deposit lands");
    }
    if history.recalled > 0 {
        fixture.accept_recall(RECALL_TRANSFER_ID, history.recalled);
        fixture
            .withdraw(RECALL_TRANSFER_ID, history.recalled)
            .expect("the withdrawal lands");
    }
    if history.sent > 0 {
        fixture
            .send_recall(RECALL_TRANSFER_ID, history.sent)
            .expect("the send lands");
    }
    if history.surplus > 0 {
        let custody = fixture.custody;
        fixture.credit(custody, history.surplus);
    }

    fixture
}

fn publish(fixture: &mut Fixture, epoch_id: u64) -> RemoteReportPublished {
    let sequence = fixture.outbound_lane().next_sequence;
    let accounts = fixture.outbound_accounts(sequence);
    let instruction = fixture.publish_report_instruction(accounts, epoch_id);
    fixture.publish_and_capture(instruction)
}

fn publish_notice(fixture: &mut Fixture) -> RecallSentPublished {
    let sequence = fixture.outbound_lane().next_sequence;
    let accounts = fixture.recall_sent_accounts(RECALL_TRANSFER_ID, sequence);
    let instruction = fixture.publish_recall_sent_instruction(accounts);
    fixture.publish_and_capture(instruction)
}

fn report_body(event: &RemoteReportPublished) -> protocol_types::RemoteReportBody {
    let message = protocol_types::decode_message(&event.message_bytes).expect("the report decodes");
    match message.body {
        Body::RemoteReport(body) => body,
        other => panic!("expected a report, found {other:?}"),
    }
}

fn notice_body(event: &RecallSentPublished) -> protocol_types::RecallSentBody {
    let message = protocol_types::decode_message(&event.message_bytes).expect("the notice decodes");
    match message.body {
        Body::RecallSent(body) => body,
        other => panic!("expected a notice, found {other:?}"),
    }
}

// Reported value

#[test]
fn reported_value_always_equals_the_principal_the_leg_still_holds() {
    for history in HISTORIES {
        let mut fixture = leg(*history);
        let body = report_body(&publish(&mut fixture, 1));
        let position = fixture.position();

        let held =
            position.attributed_principal + position.deployed_principal + position.recalled_custody;
        assert_eq!(
            body.reported_value.get(),
            u128::from(held),
            "{}",
            history.name
        );
        assert_eq!(
            body.remote_principal.get(),
            u128::from(held),
            "{}",
            history.name
        );
    }
}

#[test]
fn reported_value_never_rises_above_remote_principal() {
    for history in HISTORIES {
        let mut fixture = leg(*history);
        let body = report_body(&publish(&mut fixture, 1));
        assert!(
            body.reported_value <= body.remote_principal,
            "{}",
            history.name
        );
    }
}

#[test]
fn a_surplus_nobody_attributed_stays_out_of_reported_value() {
    for history in HISTORIES.iter().filter(|entry| entry.surplus > 0) {
        let mut fixture = leg(*history);
        let body = report_body(&publish(&mut fixture, 1));

        assert_eq!(
            body.unattributed_balance.get(),
            u128::from(history.surplus),
            "{}",
            history.name
        );
        assert_eq!(
            body.reported_value.get(),
            u128::from(history.allocated),
            "{}",
            history.name
        );
    }
}

#[test]
fn assets_already_sent_leave_the_reported_value_behind() {
    let sent = HISTORIES
        .iter()
        .find(|entry| entry.sent > 0)
        .expect("one history sends");
    let mut fixture = leg(*sent);

    let body = report_body(&publish(&mut fixture, 1));
    assert_eq!(body.reported_value.get(), 0);
    assert_eq!(fixture.token_amount(fixture.escrow), sent.sent);
}

// Report contents against live state

#[test]
fn every_report_field_repeats_what_the_accounts_already_say() {
    for history in HISTORIES {
        let mut fixture = leg(*history);
        let body = report_body(&publish(&mut fixture, 9));
        let position = fixture.position();
        let config = fixture.config();

        assert_eq!(
            body.realized_loss.get(),
            u128::from(position.cumulative_realized_loss),
            "{}",
            history.name
        );
        assert_eq!(
            body.unattributed_balance.get(),
            u128::from(position.unattributed_custody),
            "{}",
            history.name
        );
        assert_eq!(
            body.latest_completed_transfer_id.as_bytes(),
            &position.latest_completed_transfer_id,
            "{}",
            history.name
        );
        assert_eq!(
            body.config_version.get(),
            config.config_version,
            "{}",
            history.name
        );
    }
}

#[test]
fn the_committed_state_can_be_rebuilt_from_the_accounts() {
    for history in HISTORIES {
        let mut fixture = leg(*history);
        let body = report_body(&publish(&mut fixture, 3));

        let config = fixture.config();
        let position = fixture.position();
        let expected = remote_state::commitment(&RemoteStateView {
            remote_config: &fixture.config,
            config_version: config.config_version,
            frozen: config.frozen,
            position: &position,
            custody_balance: fixture.token_amount(fixture.custody),
            adapter_principal: fixture.adapter_principal(),
        })
        .expect("the state commits");

        assert_eq!(
            body.remote_state_commitment.as_bytes(),
            &expected,
            "{}",
            history.name
        );
    }
}

#[test]
fn custody_buckets_always_add_up_to_the_tokens_the_leg_can_see() {
    for history in HISTORIES {
        let mut fixture = leg(*history);
        publish(&mut fixture, 1);

        let position = fixture.position();
        let on_chain = position.attributed_principal
            + position.recalled_custody
            + position.unattributed_custody;
        assert_eq!(
            on_chain,
            fixture.token_amount(fixture.custody),
            "{}",
            history.name
        );
        assert_eq!(
            position.deployed_principal,
            fixture.adapter_principal(),
            "{}",
            history.name
        );
    }
}

// Determinism of the encoded message

#[test]
fn the_same_state_at_the_same_sequence_encodes_the_same_bytes() {
    for history in HISTORIES {
        let mut first = leg(*history);
        let mut second = leg(*history);

        let left = publish(&mut first, 5);
        let right = publish(&mut second, 5);

        assert_eq!(left.message_bytes, right.message_bytes, "{}", history.name);
        assert_eq!(left.message_id, right.message_id, "{}", history.name);
        assert_eq!(left.body_hash, right.body_hash, "{}", history.name);
    }
}

#[test]
fn a_different_epoch_gives_the_report_a_different_identity() {
    for history in HISTORIES {
        let mut first = leg(*history);
        let mut second = leg(*history);

        let left = publish(&mut first, 5);
        let right = publish(&mut second, 6);

        assert_ne!(left.message_id, right.message_id, "{}", history.name);
        assert_ne!(left.body_hash, right.body_hash, "{}", history.name);
    }
}

#[test]
fn a_state_change_gives_the_report_a_different_identity() {
    let mut fixture = leg(HISTORIES[2]);
    let first = publish(&mut fixture, 5);

    let custody = fixture.custody;
    fixture.credit(custody, 1);
    let second = publish(&mut fixture, 5);

    assert_ne!(first.body_hash, second.body_hash);
}

// The lane under mixed traffic

#[test]
fn the_lane_keeps_one_order_whatever_the_publication_mix_is() {
    let orders: [&[ReportType]; 4] = [
        &[ReportType::RemoteReport, ReportType::RecallSent],
        &[ReportType::RecallSent, ReportType::RemoteReport],
        &[
            ReportType::RemoteReport,
            ReportType::RecallSent,
            ReportType::RemoteReport,
        ],
        &[
            ReportType::RemoteReport,
            ReportType::RemoteReport,
            ReportType::RecallSent,
        ],
    ];

    for order in orders {
        let mut fixture = leg(HISTORIES[5]);
        let mut commitments = vec![[0u8; 32]];

        for (index, kind) in order.iter().enumerate() {
            let sequence = u64::try_from(index).unwrap() + 1;
            let next = match kind {
                ReportType::RemoteReport => publish(&mut fixture, sequence).next_commitment,
                ReportType::RecallSent => publish_notice(&mut fixture).next_commitment,
                ReportType::TransportSettlement => panic!("these orders publish no settlement"),
            };

            let record = fixture.report_record(sequence);
            assert_eq!(record.sequence, sequence);
            assert_eq!(record.report_type, *kind);
            assert_eq!(record.previous_commitment, *commitments.last().unwrap());
            assert_eq!(record.next_commitment, next);
            commitments.push(next);
        }

        let lane = fixture.outbound_lane();
        assert_eq!(lane.next_sequence, u64::try_from(order.len()).unwrap() + 1);
        assert_eq!(lane.current_commitment, *commitments.last().unwrap());
        assert_eq!(
            lane.total_reports + lane.total_recall_notifications,
            u64::try_from(order.len()).unwrap()
        );
    }
}

#[test]
fn the_commitment_history_can_be_replayed_from_the_records() {
    let mut fixture = leg(HISTORIES[5]);
    publish(&mut fixture, 1);
    publish_notice(&mut fixture);
    publish(&mut fixture, 2);

    let mut running = [0u8; 32];
    for sequence in 1..fixture.outbound_lane().next_sequence {
        let record = fixture.report_record(sequence);
        assert_eq!(record.previous_commitment, running);
        running = record.next_commitment;
    }
    assert_eq!(fixture.outbound_lane().current_commitment, running);
}

#[test]
fn a_refused_publication_never_leaves_a_hole_in_the_lane() {
    let mut fixture = leg(HISTORIES[5]);
    publish(&mut fixture, 1);

    let before = fixture.outbound_lane();
    // A record slot from an old sequence cannot carry the next message.
    expect_error(
        fixture.publish_report_with(fixture.outbound_accounts(1), 2),
        RemoteLegError::InvalidReportRecord,
    );
    // A notice for a transfer that never happened cannot either.
    expect_rejection(fixture.publish_recall_sent(ALLOCATE_TRANSFER_ID));

    assert_eq!(fixture.outbound_lane(), before);

    publish_notice(&mut fixture);
    assert_eq!(fixture.outbound_lane().next_sequence, 3);
    assert!(fixture.report_record_exists(2));
}

#[test]
fn one_sequence_never_holds_two_messages() {
    let mut fixture = leg(HISTORIES[5]);
    publish(&mut fixture, 1);

    let record = fixture.report_record(1);
    // The slot for sequence one is taken, so a notice must move on.
    expect_error(
        fixture.publish_recall_sent_with(fixture.recall_sent_accounts(RECALL_TRANSFER_ID, 1)),
        RemoteLegError::InvalidReportRecord,
    );
    assert_eq!(fixture.report_record(1), record);
}

#[test]
fn a_written_record_is_never_touched_again() {
    let mut fixture = leg(HISTORIES[5]);
    publish(&mut fixture, 1);
    let first = fixture.raw_data(fixture.report_record_key(1));

    publish_notice(&mut fixture);
    publish(&mut fixture, 2);

    assert_eq!(fixture.raw_data(fixture.report_record_key(1)), first);
}

#[test]
fn every_sequence_the_lane_used_has_exactly_one_record() {
    let mut fixture = leg(HISTORIES[5]);
    publish(&mut fixture, 1);
    publish_notice(&mut fixture);
    publish(&mut fixture, 2);

    let lane = fixture.outbound_lane();
    for sequence in 1..lane.next_sequence {
        assert!(fixture.report_record_exists(sequence));
    }
    assert!(!fixture.report_record_exists(lane.next_sequence));
}

// The recall notice against the recall record

/// A leg whose recall lost everything the adapter held.
fn total_loss_leg() -> Fixture {
    let mut fixture = Fixture::ready_to_publish();
    fixture.fund_position(ALLOCATE_TRANSFER_ID, AMOUNT);
    fixture.deploy(AMOUNT).expect("the deposit lands");
    fixture
        .configure_adapter(u64::MAX, 10_000, false)
        .expect("the loss is configured");
    fixture.accept_recall(RECALL_TRANSFER_ID, AMOUNT);
    fixture
        .withdraw(RECALL_TRANSFER_ID, AMOUNT)
        .expect("the withdrawal lands");
    fixture
}

#[test]
fn the_notice_repeats_the_completed_transfer_exactly() {
    for mut fixture in [leg(HISTORIES[5]), total_loss_leg()] {
        let record = fixture.transfer(&RECALL_TRANSFER_ID);
        assert_eq!(record.status, TransferStatus::Complete);

        let body = notice_body(&publish_notice(&mut fixture));
        assert_eq!(body.transfer_id.as_bytes(), &record.transfer_id);
        assert_eq!(
            body.principal_sent.get(),
            u128::from(record.requested_recall_amount)
        );
        assert_eq!(
            body.actual_amount_sent.get(),
            u128::from(record.assets_sent)
        );
        assert_eq!(body.realized_loss.get(), u128::from(record.realized_loss));
        assert_eq!(
            body.sent_timestamp.get(),
            u64::try_from(record.completed_at).unwrap()
        );
        assert_eq!(
            body.destination_reference.as_bytes(),
            &fixture.config().outbound_escrow.to_bytes()
        );
    }
}

#[test]
fn one_recall_publishes_one_notice_and_no_more() {
    let mut fixture = leg(HISTORIES[5]);
    publish_notice(&mut fixture);

    let before = fixture.outbound_lane();
    expect_error(
        fixture.publish_recall_sent(RECALL_TRANSFER_ID),
        RemoteLegError::RecallSentAlreadyPublished,
    );
    assert_eq!(fixture.outbound_lane(), before);
    assert_eq!(before.total_recall_notifications, 1);
}

// What publication must never do

#[test]
fn publication_never_moves_a_single_token() {
    for history in HISTORIES {
        let mut fixture = leg(*history);
        let custody = fixture.token_amount(fixture.custody);
        let escrow = fixture.token_amount(fixture.escrow);
        let vault = fixture.token_amount(fixture.adapter_vault);

        publish(&mut fixture, 1);

        assert_eq!(
            fixture.token_amount(fixture.custody),
            custody,
            "{}",
            history.name
        );
        assert_eq!(
            fixture.token_amount(fixture.escrow),
            escrow,
            "{}",
            history.name
        );
        assert_eq!(
            fixture.token_amount(fixture.adapter_vault),
            vault,
            "{}",
            history.name
        );
    }
}

#[test]
fn publication_never_creates_an_account_for_a_local_user() {
    let mut fixture = leg(HISTORIES[5]);
    let before = fixture.owned_account_count();

    publish(&mut fixture, 1);
    publish_notice(&mut fixture);

    // One report record per message, plus the marker for the notice.
    assert_eq!(fixture.owned_account_count(), before + 3);
}

#[test]
fn a_frozen_leg_reports_without_gaining_any_exposure() {
    let mut fixture = leg(HISTORIES[5]);
    let guardian = fixture.guardian.insecure_clone();
    fixture.freeze(&guardian).expect("the freeze lands");

    let custody = fixture.token_amount(fixture.custody);
    let position = fixture.raw_data(fixture.position_key());

    publish(&mut fixture, 1);
    publish_notice(&mut fixture);

    assert_eq!(fixture.token_amount(fixture.custody), custody);
    assert_eq!(fixture.raw_data(fixture.position_key()), position);
    assert_eq!(fixture.outbound_lane().next_sequence, 3);
}

#[test]
fn a_report_never_lets_a_caller_name_an_amount() {
    // The only thing a caller supplies is the epoch, so two callers with
    // different keys reach the same bytes.
    let mut first = leg(HISTORIES[2]);
    let mut second = leg(HISTORIES[2]);

    let stranger = second.outsider.insecure_clone();
    second.fund(&stranger);
    let sequence = second.outbound_lane().next_sequence;
    let mut accounts = second.outbound_accounts(sequence);
    accounts.payer = stranger.pubkey();
    let instruction = second.publish_report_instruction(accounts, 5);

    let by_admin = publish(&mut first, 5);
    let by_stranger: RemoteReportPublished = second.publish_and_capture_as(instruction, &stranger);

    assert_eq!(by_admin.message_bytes, by_stranger.message_bytes);
}

#[test]
fn the_message_always_fits_the_transport_budget() {
    for history in HISTORIES {
        let mut fixture = leg(*history);
        let report = publish(&mut fixture, 1);
        assert!(report.message_bytes.len() <= 400, "{}", history.name);
    }

    let mut settled = leg(HISTORIES[5]);
    let notice = publish_notice(&mut settled);
    assert!(notice.message_bytes.len() <= 400);
}

#[test]
fn a_report_never_names_an_account_the_leg_does_not_control() {
    let mut fixture = leg(HISTORIES[5]);
    let body = notice_body(&publish_notice(&mut fixture));

    let escrow = Pubkey::new_from_array(
        body.destination_reference
            .as_bytes()
            .as_slice()
            .try_into()
            .unwrap(),
    );
    assert_eq!(escrow, fixture.config().outbound_escrow);
    assert_eq!(escrow, fixture.escrow);
}
