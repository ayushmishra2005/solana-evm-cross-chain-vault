//! Explicit closes and pooled recoveries, each settled exactly once.
//!
//! A close credits only what custody explains and pools the shortfall as
//! impairment. A recovery credits verified unattributed custody against that
//! pool. Both publish their settlement in the instruction that applies them.

#![allow(clippy::unwrap_used, clippy::panic, clippy::arithmetic_side_effects)]

mod common;

use common::messages::MessageBuilder;
use common::{
    ALLOCATE_TRANSFER_ID, Fixture, MAX_REMOTE_PRINCIPAL, RECALL_TRANSFER_ID, START_TIMESTAMP,
    expect_error, last_event,
};
use protocol_types::{AssetAmount, Body, ResolveAction, SettlementKind, TransportSettlementBody};
use solevm_remote_leg::{
    MessageClass, RemoteLegError, RemoteReportPublished, ReportType, SettlementProcessed,
    TransferKind, TransferStatus, TransportSettlementPublished,
};

const ID: [u8; 32] = ALLOCATE_TRANSFER_ID;
const RECOVERY_ID: [u8; 32] = [0x5A; 32];
const OTHER_ID: [u8; 32] = [0x6B; 32];
const AMOUNT: u64 = 1_000_000;
const EPOCH: u64 = 12;
const STALE: u64 = START_TIMESTAMP as u64 - 1;

/// What one settlement published and recorded.
struct Settled {
    published: TransportSettlementPublished,
    body: TransportSettlementBody,
    processed: SettlementProcessed,
}

fn settled(fixture: &Fixture) -> Settled {
    let logs = fixture.last_logs();
    let published: TransportSettlementPublished = last_event(&logs);
    let processed: SettlementProcessed = last_event(&logs);
    let message =
        protocol_types::decode_message(&published.message_bytes).expect("settlement decodes");
    let Body::TransportSettlement(body) = message.body else {
        panic!("expected a transport settlement, found {:?}", message.body);
    };
    Settled {
        published,
        body,
        processed,
    }
}

/// A leg with one pending allocation at sequence one.
fn pending_allocation() -> Fixture {
    let mut fixture = Fixture::ready_to_publish();
    assert_eq!(fixture.accept_allocation(ID, AMOUNT), 1);
    fixture
}

/// A leg whose allocation at sequence one closed with `arrived` credited.
fn closed_short(arrived: u64) -> Fixture {
    let mut fixture = pending_allocation();
    if arrived > 0 {
        fixture.credit(fixture.custody, arrived);
    }
    fixture.close_allocation(ID, 1).expect("close lands");
    fixture
}

fn quarantined_allocation() -> Fixture {
    let mut fixture = Fixture::ready_to_publish();
    let lane = fixture.control_lane();
    let bytes = MessageBuilder::allocate()
        .sequence(lane.highest_consumed_sequence + 1)
        .previous_commitment(lane.message_commitment)
        .transfer_id(ID)
        .allocate_body(|body| {
            body.amount = AssetAmount::new(u128::from(AMOUNT));
            body.minimum_destination_amount = AssetAmount::new(u128::from(AMOUNT));
        })
        .expires_at(STALE)
        .encode();
    fixture
        .allocate(ID, 1, bytes)
        .expect("allocation is consumed");
    assert_eq!(fixture.transfer(&ID).status, TransferStatus::Quarantined);
    fixture
}

fn report_principal(fixture: &mut Fixture) -> u128 {
    let sequence = fixture.outbound_lane().next_sequence;
    let accounts = fixture.outbound_accounts(sequence);
    let instruction = fixture.publish_report_instruction(accounts, EPOCH);
    let event: RemoteReportPublished = fixture.publish_and_capture(instruction);
    let message = protocol_types::decode_message(&event.message_bytes).expect("report decodes");
    let Body::RemoteReport(body) = message.body else {
        panic!("expected a report");
    };
    assert_eq!(body.reported_value, body.remote_principal);
    body.remote_principal.get()
}

/// Custody always equals the buckets that explain it after a settlement.
fn assert_custody_explained(fixture: &Fixture) {
    let position = fixture.position();
    assert_eq!(
        position.attributed_principal + position.recalled_custody + position.unattributed_custody,
        fixture.token_amount(fixture.custody)
    );
}

/// Every closed claim is credited, impaired or recovered, exactly once.
fn assert_ledger_balanced(fixture: &Fixture) {
    let ledger = fixture.transport_ledger().expect("ledger exists");
    assert_eq!(
        ledger.cumulative_closed_claim,
        ledger.cumulative_closed_credit + ledger.impaired_outstanding + ledger.cumulative_recovered
    );
}

fn assert_unconsumed(fixture: &Fixture, highest: u64) {
    assert_eq!(fixture.control_lane().highest_consumed_sequence, highest);
}

// Closing

#[test]
fn closing_a_partly_attributed_allocation_credits_only_what_arrived() {
    let mut fixture = pending_allocation();
    fixture.credit(fixture.custody, 600_000);
    fixture.attribute(ID).expect("attribution lands");
    let lane_before = fixture.outbound_lane();

    fixture.close_allocation(ID, 1).expect("close lands");
    let Settled {
        published,
        body,
        processed,
    } = settled(&fixture);

    assert_eq!(body.transfer_id.to_bytes(), ID);
    assert_eq!(body.kind, SettlementKind::Closed);
    assert_eq!(body.claim_amount.get(), u128::from(AMOUNT));
    assert_eq!(body.credited_amount.get(), 600_000);
    assert_eq!(body.impaired_outstanding.get(), 400_000);
    assert_eq!(body.config_version.get(), fixture.config().config_version);
    assert_eq!(processed.kind, SettlementKind::Closed.to_u8());
    assert_eq!(processed.target_sequence, 1);
    assert_eq!(processed.sequence, 2);
    assert!(!processed.arrived_late);
    assert!(!processed.guardian_cosigned);

    let record = fixture.transfer(&ID);
    assert_eq!(record.status, TransferStatus::Closed);
    assert_eq!(record.attributed_amount, 600_000);
    assert_ne!(record.completed_at, 0);

    let position = fixture.position();
    assert_eq!(position.active_transfer_kind, TransferKind::None);
    assert_eq!(position.active_transfer_status, TransferStatus::None);
    assert_eq!(position.attributed_principal, 600_000);
    assert_eq!(position.latest_completed_transfer_id, [0u8; 32]);

    let ledger = fixture
        .transport_ledger()
        .expect("first close creates the ledger");
    assert_eq!(ledger.impaired_outstanding, 400_000);
    assert_eq!(ledger.cumulative_closed_claim, u128::from(AMOUNT));
    assert_eq!(ledger.cumulative_closed_credit, 600_000);
    assert_eq!(ledger.last_impaired_sequence, 1);
    assert_eq!(ledger.total_closed, 1);
    assert_custody_explained(&fixture);
    assert_ledger_balanced(&fixture);

    assert_eq!(fixture.control_lane().highest_consumed_sequence, 2);
    let consumed = fixture.record(2);
    assert_eq!(consumed.message_class, MessageClass::Resolve);
    assert_eq!(
        processed.lane_commitment,
        fixture.control_lane().message_commitment
    );

    let lane = fixture.outbound_lane();
    assert_eq!(lane.next_sequence, lane_before.next_sequence + 1);
    assert_eq!(lane.current_commitment, published.next_commitment);
    assert_eq!(
        published.previous_commitment,
        lane_before.current_commitment
    );
    assert_eq!(lane.total_reports, lane_before.total_reports);
    let report = fixture.report_record(published.sequence);
    assert_eq!(report.report_type, ReportType::TransportSettlement);
    assert_eq!(report.related_transfer_id, ID);
    assert_eq!(report.message_id, published.message_id);
}

#[test]
fn closing_first_takes_custody_that_arrived_but_was_never_attributed() {
    let fixture = closed_short(700_000);
    let body = settled(&fixture).body;

    assert_eq!(body.credited_amount.get(), 700_000);
    assert_eq!(body.impaired_outstanding.get(), 300_000);
    assert_eq!(fixture.position().attributed_principal, 700_000);
    assert_eq!(fixture.position().unattributed_custody, 0);
    assert_custody_explained(&fixture);
}

#[test]
fn closing_an_allocation_that_never_arrived_impairs_all_of_it() {
    let fixture = closed_short(0);
    let body = settled(&fixture).body;

    assert_eq!(body.credited_amount.get(), 0);
    assert_eq!(body.impaired_outstanding.get(), u128::from(AMOUNT));
    assert_eq!(fixture.position().attributed_principal, 0);
    assert_ledger_balanced(&fixture);
}

#[test]
fn closing_an_allocation_that_fully_arrived_impairs_nothing() {
    let fixture = closed_short(AMOUNT);
    let body = settled(&fixture).body;

    assert_eq!(body.credited_amount.get(), u128::from(AMOUNT));
    assert_eq!(body.impaired_outstanding.get(), 0);
    let ledger = fixture.transport_ledger().unwrap();
    assert_eq!(ledger.last_impaired_sequence, 0);
    assert_eq!(fixture.transfer(&ID).status, TransferStatus::Closed);
}

#[test]
fn custody_beyond_the_allocation_stays_unattributed_when_it_closes() {
    let fixture = closed_short(AMOUNT + 250);
    assert_eq!(
        settled(&fixture).body.credited_amount.get(),
        u128::from(AMOUNT)
    );
    assert_eq!(fixture.position().unattributed_custody, 250);
    assert_custody_explained(&fixture);
}

/// The vault asked to close before it applied the completion, so the
/// completion settles here, once and in full.
#[test]
fn closing_a_completed_allocation_settles_it_in_full_once() {
    let mut fixture = Fixture::ready_to_publish();
    fixture.fund_position(ID, AMOUNT);
    let completed_at = fixture.transfer(&ID).completed_at;
    assert_eq!(fixture.position().latest_completed_transfer_id, ID);

    fixture.close_allocation(ID, 1).expect("close lands");
    let body = settled(&fixture).body;

    assert_eq!(body.credited_amount.get(), u128::from(AMOUNT));
    assert_eq!(body.impaired_outstanding.get(), 0);
    let record = fixture.transfer(&ID);
    assert_eq!(record.status, TransferStatus::Closed);
    assert_eq!(record.completed_at, completed_at);
    assert_eq!(fixture.position().attributed_principal, AMOUNT);

    expect_error(
        fixture.close_allocation(ID, 1),
        RemoteLegError::TransferNotClosable,
    );
    assert_unconsumed(&fixture, 2);
}

/// A quarantined allocation was never admitted, so nothing is credited to it;
/// what arrived for it stays unattributed for a recovery.
#[test]
fn closing_a_quarantined_allocation_credits_nothing() {
    let mut fixture = quarantined_allocation();
    fixture.credit(fixture.custody, 400_000);

    fixture.close_allocation(ID, 1).expect("close lands");
    let body = settled(&fixture).body;

    assert_eq!(body.credited_amount.get(), 0);
    assert_eq!(body.impaired_outstanding.get(), u128::from(AMOUNT));
    assert_eq!(fixture.transfer(&ID).status, TransferStatus::Closed);
    let position = fixture.position();
    assert_eq!(position.active_transfer_kind, TransferKind::None);
    assert_eq!(position.attributed_principal, 0);
}

#[test]
fn a_closed_allocation_frees_the_leg_and_its_id_stays_spent() {
    let mut fixture = closed_short(600_000);

    let sequence = fixture.control_lane().highest_consumed_sequence + 1;
    let bytes = fixture.allocate_bytes(ID, AMOUNT, sequence);
    expect_error(
        fixture.allocate(ID, sequence, bytes),
        RemoteLegError::TransferAlreadyExists,
    );

    fixture.accept_allocation(OTHER_ID, AMOUNT);
    assert_eq!(fixture.transfer(&OTHER_ID).status, TransferStatus::Pending);
}

#[test]
fn a_close_naming_another_message_reverts_unconsumed() {
    let mut fixture = pending_allocation();
    let sequence = 2;
    let bytes = fixture.settlement_bytes(ID, 1, sequence, ResolveAction::Close);
    let mut wrong = fixture.settlement_bytes(ID, 1, sequence, ResolveAction::Close);
    common::messages::patch_body(
        &mut wrong,
        protocol_types::layout::RESOLVE_TARGET_SEQUENCE_OFFSET,
        &0u64.to_be_bytes(),
    );
    expect_error(
        fixture.settle(ID, sequence, wrong),
        RemoteLegError::InvalidMessage,
    );

    let decoy = fixture.settlement_bytes(OTHER_ID, 1, sequence, ResolveAction::Close);
    assert!(fixture.settle(OTHER_ID, sequence, decoy).is_err());
    assert_unconsumed(&fixture, 1);
    fixture
        .settle(ID, sequence, bytes)
        .expect("the right close still lands");
}

#[test]
fn a_close_naming_another_sequence_of_the_same_transfer_reverts_unconsumed() {
    let mut fixture = pending_allocation();
    fixture.admit(ID, 1).expect("a stray admission is consumed");
    assert_unconsumed(&fixture, 2);

    expect_error(
        fixture.close_allocation(ID, 2),
        RemoteLegError::InvalidTransferRecord,
    );
    assert_unconsumed(&fixture, 2);
    fixture.close_allocation(ID, 1).expect("close lands");
}

#[test]
fn closing_a_recall_reverts_unconsumed() {
    let mut fixture = Fixture::ready_to_publish();
    fixture.fund_position(ID, AMOUNT);
    let recall = fixture.accept_recall(RECALL_TRANSFER_ID, 500_000);

    expect_error(
        fixture.close_allocation(RECALL_TRANSFER_ID, recall),
        RemoteLegError::InvalidTransferKind,
    );
    assert_unconsumed(&fixture, recall);
}

#[test]
fn an_admission_on_the_settlement_path_reverts_unconsumed() {
    let mut fixture = quarantined_allocation();
    let bytes = fixture.settlement_bytes(ID, 1, 2, ResolveAction::Admit);
    expect_error(
        fixture.settle(ID, 2, bytes),
        RemoteLegError::WrongResolveInstruction,
    );
    assert_unconsumed(&fixture, 1);
    assert_eq!(fixture.transfer(&ID).status, TransferStatus::Quarantined);
}

#[test]
fn a_close_cannot_be_replayed() {
    let mut fixture = pending_allocation();
    let bytes = fixture.settlement_bytes(ID, 1, 2, ResolveAction::Close);
    fixture.settle(ID, 2, bytes.clone()).expect("close lands");
    let ledger = fixture.transport_ledger();

    expect_error(
        fixture.settle(ID, 2, bytes),
        RemoteLegError::InvalidSequence,
    );
    assert_eq!(fixture.transport_ledger(), ledger);
}

/// The vault holds its cycle open until the settlement, so a late close still
/// binds instead of being dropped.
#[test]
fn a_late_close_still_settles_and_says_it_was_late() {
    let mut fixture = pending_allocation();
    let lane = fixture.control_lane();
    let config_version = fixture.config().config_version;
    let bytes = MessageBuilder::resolve()
        .sequence(2)
        .previous_commitment(lane.message_commitment)
        .resolve_body(|body| {
            body.transfer_id = protocol_types::TransferId::new(ID);
            body.target_sequence = protocol_types::Sequence::new(1);
            body.action = ResolveAction::Close;
            body.config_version = protocol_types::ConfigVersion::new(config_version);
        })
        .expires_at(STALE)
        .encode();

    fixture.settle(ID, 2, bytes).expect("late close lands");
    let Settled {
        body, processed, ..
    } = settled(&fixture);
    assert!(processed.arrived_late);
    assert_eq!(body.impaired_outstanding.get(), u128::from(AMOUNT));
}

#[test]
fn a_frozen_leg_closes_only_with_the_guardian() {
    let mut fixture = pending_allocation();
    let administrator = fixture.administrator.insecure_clone();
    fixture.freeze(&administrator).expect("freeze lands");

    expect_error(fixture.close_allocation(ID, 1), RemoteLegError::Frozen);
    assert_unconsumed(&fixture, 1);

    let outsider = fixture.outsider.insecure_clone();
    let bytes = fixture.settlement_bytes(ID, 1, 2, ResolveAction::Close);
    expect_error(
        fixture.settle_cosigned(ID, 2, bytes.clone(), &outsider),
        RemoteLegError::Unauthorized,
    );

    let guardian = fixture.guardian.insecure_clone();
    fixture
        .settle_cosigned(ID, 2, bytes, &guardian)
        .expect("guardian close lands");
    assert!(settled(&fixture).processed.guardian_cosigned);
    assert_eq!(fixture.transfer(&ID).status, TransferStatus::Closed);
}

// Reports around a close

/// The vault counts an allocation only when it settles, so a report never
/// counts part of an open allocation early.
#[test]
fn reports_leave_an_open_allocations_partial_attribution_out() {
    let mut fixture = Fixture::ready_to_publish();
    fixture.fund_position(OTHER_ID, 2_000_000);
    fixture.accept_allocation(ID, AMOUNT);
    fixture.credit(fixture.custody, 600_000);
    fixture.attribute(ID).expect("attribution lands");
    assert_eq!(fixture.position().attributed_principal, 2_600_000);

    assert_eq!(report_principal(&mut fixture), 2_000_000);

    fixture.close_allocation(ID, 2).expect("close lands");
    assert_eq!(report_principal(&mut fixture), 2_600_000);
}

#[test]
fn a_completed_allocation_is_reported_in_full_again() {
    let mut fixture = Fixture::ready_to_publish();
    fixture.accept_allocation(ID, AMOUNT);
    fixture.credit(fixture.custody, 600_000);
    fixture.attribute(ID).expect("attribution lands");
    assert_eq!(report_principal(&mut fixture), 0);

    fixture.credit(fixture.custody, 400_000);
    fixture.attribute(ID).expect("attribution completes");
    assert_eq!(report_principal(&mut fixture), u128::from(AMOUNT));
}

#[test]
fn a_report_during_an_open_allocation_requires_its_record() {
    let mut fixture = pending_allocation();
    let sequence = fixture.outbound_lane().next_sequence;

    let mut accounts = fixture.outbound_accounts(sequence);
    accounts.active_transfer_record = None;
    expect_error(
        fixture.publish_report_with(accounts, EPOCH),
        RemoteLegError::MissingActiveTransferRecord,
    );

    let mut accounts = fixture.outbound_accounts(sequence);
    accounts.active_transfer_record = Some(fixture.position_key());
    assert!(fixture.publish_report_with(accounts, EPOCH).is_err());
    assert_eq!(fixture.outbound_lane().next_sequence, sequence);
}

#[test]
fn a_settlement_shares_one_commitment_chain_with_reports() {
    let mut fixture = pending_allocation();
    report_principal(&mut fixture);
    fixture.close_allocation(ID, 1).expect("close lands");
    report_principal(&mut fixture);

    let mut previous = [0u8; 32];
    let types = [
        ReportType::RemoteReport,
        ReportType::TransportSettlement,
        ReportType::RemoteReport,
    ];
    for (index, kind) in types.iter().enumerate() {
        let record = fixture.report_record(index as u64 + 1);
        assert_eq!(record.report_type, *kind);
        assert_eq!(record.previous_commitment, previous);
        previous = record.next_commitment;
    }
    let lane = fixture.outbound_lane();
    assert_eq!(lane.current_commitment, previous);
    assert_eq!(lane.total_reports, 2);
}

// Recovering

#[test]
fn a_recovery_credits_late_custody_against_the_pool() {
    let mut fixture = closed_short(600_000);
    fixture.credit(fixture.custody, 300_000);

    fixture
        .recover_impairment(RECOVERY_ID, 1)
        .expect("recovery lands");
    let Settled {
        body, processed, ..
    } = settled(&fixture);

    assert_eq!(body.transfer_id.to_bytes(), RECOVERY_ID);
    assert_eq!(body.kind, SettlementKind::Recovered);
    assert_eq!(body.claim_amount.get(), 400_000);
    assert_eq!(body.credited_amount.get(), 300_000);
    assert_eq!(body.impaired_outstanding.get(), 100_000);
    assert_eq!(processed.target_sequence, 1);

    let position = fixture.position();
    assert_eq!(position.attributed_principal, 900_000);
    assert_eq!(position.unattributed_custody, 0);
    assert_custody_explained(&fixture);

    let recovery = fixture.transfer(&RECOVERY_ID);
    assert_eq!(recovery.transfer_kind, TransferKind::Recover);
    assert_eq!(recovery.message_class, MessageClass::Resolve);
    assert_eq!(recovery.status, TransferStatus::Complete);
    assert_eq!(recovery.authorized_amount, 300_000);
    assert_eq!(recovery.attributed_amount, 300_000);
    assert_eq!(recovery.message_sequence, 3);
    assert_eq!(fixture.transfer(&ID).status, TransferStatus::Closed);
    assert_eq!(fixture.transfer(&ID).attributed_amount, 600_000);

    let ledger = fixture.transport_ledger().unwrap();
    assert_eq!(ledger.impaired_outstanding, 100_000);
    assert_eq!(ledger.cumulative_recovered, 300_000);
    assert_eq!(ledger.total_recoveries, 1);
    assert_ledger_balanced(&fixture);
}

#[test]
fn a_recovery_never_credits_more_than_the_pool() {
    let mut fixture = closed_short(600_000);
    fixture.credit(fixture.custody, AMOUNT);

    fixture
        .recover_impairment(RECOVERY_ID, 1)
        .expect("recovery lands");
    let body = settled(&fixture).body;
    assert_eq!(body.credited_amount.get(), 400_000);
    assert_eq!(body.impaired_outstanding.get(), 0);
    assert_eq!(fixture.position().unattributed_custody, 600_000);
    assert_custody_explained(&fixture);
}

/// The ceiling counts deployed principal too, not only attributed principal.
#[test]
fn a_recovery_respects_the_ceiling_over_everything_the_leg_holds() {
    let mut fixture = Fixture::ready_to_publish();
    let held = MAX_REMOTE_PRINCIPAL - 300_000;
    fixture.fund_position(OTHER_ID, held);
    fixture.deploy(held - 100_000).expect("deployment lands");
    let position = fixture.position();
    assert_eq!(position.deployed_principal, held - 100_000);
    assert_eq!(position.attributed_principal, 100_000);

    let sequence = fixture.accept_allocation(ID, AMOUNT);
    fixture.close_allocation(ID, sequence).expect("close lands");
    fixture.credit(fixture.custody, AMOUNT);

    fixture
        .recover_impairment(RECOVERY_ID, sequence)
        .expect("recovery lands");
    let body = settled(&fixture).body;
    assert_eq!(body.credited_amount.get(), 300_000);
    assert_eq!(body.impaired_outstanding.get(), 700_000);
    assert_eq!(
        fixture.position().committed_principal().unwrap(),
        MAX_REMOTE_PRINCIPAL
    );
    assert_custody_explained(&fixture);
}

/// Nothing available still settles, so the vault's cycle always ends.
#[test]
fn a_recovery_with_nothing_available_still_settles() {
    let mut fixture = closed_short(600_000);
    fixture
        .recover_impairment(RECOVERY_ID, 1)
        .expect("recovery lands");
    let body = settled(&fixture).body;

    assert_eq!(body.claim_amount.get(), 400_000);
    assert_eq!(body.credited_amount.get(), 0);
    assert_eq!(body.impaired_outstanding.get(), 400_000);
    assert_eq!(fixture.transfer(&RECOVERY_ID).authorized_amount, 0);
    assert_ledger_balanced(&fixture);
}

#[test]
fn a_recovery_must_name_the_last_impaired_allocation() {
    let mut fixture = closed_short(600_000);
    fixture.credit(fixture.custody, 100_000);
    let sequence = fixture.control_lane().highest_consumed_sequence;
    expect_error(
        fixture.recover_impairment(RECOVERY_ID, 2),
        RemoteLegError::RecoveryStateMismatch,
    );
    assert_unconsumed(&fixture, sequence);
}

#[test]
fn a_recovery_without_impairment_reverts_unconsumed() {
    let mut fixture = Fixture::ready_to_publish();
    fixture.fund_position(OTHER_ID, AMOUNT);
    expect_error(
        fixture.recover_impairment(RECOVERY_ID, 1),
        RemoteLegError::RecoveryStateMismatch,
    );
    assert_unconsumed(&fixture, 1);

    let mut fixture = closed_short(AMOUNT);
    expect_error(
        fixture.recover_impairment(RECOVERY_ID, 1),
        RemoteLegError::RecoveryStateMismatch,
    );
    assert_unconsumed(&fixture, 2);
}

#[test]
fn a_recovery_never_overlaps_an_open_cycle() {
    let mut fixture = closed_short(600_000);
    let open = fixture.accept_allocation(OTHER_ID, AMOUNT);
    fixture.credit(fixture.custody, 100_000);

    expect_error(
        fixture.recover_impairment(RECOVERY_ID, 1),
        RemoteLegError::UnresolvedCycle,
    );
    assert_unconsumed(&fixture, open);
}

#[test]
fn a_recovery_id_that_names_an_existing_transfer_reverts_unconsumed() {
    let mut fixture = closed_short(600_000);
    fixture.credit(fixture.custody, 100_000);
    expect_error(
        fixture.recover_impairment(ID, 1),
        RemoteLegError::TransferAlreadyExists,
    );
    assert_unconsumed(&fixture, 2);
}

#[test]
fn a_frozen_leg_recovers_only_with_the_guardian() {
    let mut fixture = closed_short(600_000);
    fixture.credit(fixture.custody, 100_000);
    let administrator = fixture.administrator.insecure_clone();
    fixture.freeze(&administrator).expect("freeze lands");

    expect_error(
        fixture.recover_impairment(RECOVERY_ID, 1),
        RemoteLegError::Frozen,
    );
    assert_unconsumed(&fixture, 2);

    let guardian = fixture.guardian.insecure_clone();
    let bytes = fixture.settlement_bytes(RECOVERY_ID, 1, 3, ResolveAction::Recover);
    fixture
        .settle_cosigned(RECOVERY_ID, 3, bytes, &guardian)
        .expect("guardian recovery lands");
    assert_eq!(settled(&fixture).body.credited_amount.get(), 100_000);
}

#[test]
fn the_ledger_balances_across_closes_and_recoveries() {
    let mut fixture = closed_short(250_000);
    fixture.credit(fixture.custody, 100_000);
    fixture
        .recover_impairment(RECOVERY_ID, 1)
        .expect("recovery lands");

    let second = fixture.accept_allocation(OTHER_ID, 500_000);
    fixture
        .close_allocation(OTHER_ID, second)
        .expect("close lands");
    assert_eq!(settled(&fixture).body.impaired_outstanding.get(), 1_150_000);
    fixture.credit(fixture.custody, 2_000_000);
    fixture
        .recover_impairment([0x5B; 32], second)
        .expect("second recovery lands");

    let ledger = fixture.transport_ledger().unwrap();
    assert_eq!(ledger.cumulative_closed_claim, 1_500_000);
    assert_eq!(ledger.cumulative_closed_credit, 250_000);
    assert_eq!(ledger.cumulative_recovered, 1_250_000);
    assert_eq!(ledger.impaired_outstanding, 0);
    assert_eq!(ledger.total_closed, 2);
    assert_eq!(ledger.total_recoveries, 2);
    assert_eq!(ledger.last_impaired_sequence, second);
    assert_eq!(fixture.position().attributed_principal, 1_500_000);
    assert_eq!(fixture.position().unattributed_custody, 850_000);
    assert_ledger_balanced(&fixture);
    assert_custody_explained(&fixture);
}
