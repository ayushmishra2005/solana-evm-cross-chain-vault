#![allow(clippy::expect_used, clippy::panic)]

mod common;

use accounting_model::{
    AssetAmount, CycleKind, Operation, Rejection, TransferId, VaultState, apply, check_invariants,
};
use common::{ADMIN, ALICE, funded_with_shares, run};

fn tid(byte: u8) -> TransferId {
    TransferId::new([byte; 32])
}

fn funded() -> accounting_model::State {
    funded_with_shares(ALICE, 100_000_000)
}

fn allocate(id: u8, amount: u128) -> Operation {
    Operation::BeginAllocate {
        actor: ADMIN,
        transfer_id: tid(id),
        amount: AssetAmount::new(amount),
    }
}

fn complete(id: u8, reported: u128) -> Operation {
    Operation::CompleteAllocation {
        transfer_id: tid(id),
        reported_value: AssetAmount::new(reported),
    }
}

fn recall(id: u8, amount: u128) -> Operation {
    Operation::BeginRecall {
        actor: ADMIN,
        transfer_id: tid(id),
        amount: AssetAmount::new(amount),
    }
}

fn recall_sent(id: u8, principal: u128, actual: u128, loss: u128) -> Operation {
    Operation::AcceptRecallSent {
        transfer_id: tid(id),
        principal: AssetAmount::new(principal),
        actual: AssetAmount::new(actual),
        loss: AssetAmount::new(loss),
    }
}

fn observe(amount: u128) -> Operation {
    Operation::ObserveInbound {
        amount: AssetAmount::new(amount),
    }
}

#[test]
fn allocation_moves_idle_to_outbound_without_changing_nav() {
    let state = funded();
    let nav = state.managed_nav();
    let state = run(&state, &[allocate(1, 40_000_000)]);
    check_invariants(&state).expect("invariants");
    assert_eq!(state.managed_nav(), nav);
    assert_eq!(state.buckets.idle_backing, AssetAmount::new(60_000_000));
    assert_eq!(
        state.buckets.outbound_in_transit,
        AssetAmount::new(40_000_000)
    );
    assert_eq!(state.active_cycle, CycleKind::Allocate);
}

#[test]
fn a_second_unresolved_cycle_is_rejected() {
    let state = run(&funded(), &[allocate(1, 10_000_000)]);
    let err = apply(&state, allocate(2, 10_000_000)).expect_err("second cycle");
    assert_eq!(err, Rejection::ActiveTransferExists);
}

#[test]
fn completing_allocation_caps_reported_value_to_principal() {
    let state = run(
        &funded(),
        &[allocate(1, 40_000_000), complete(1, 99_000_000)],
    );
    check_invariants(&state).expect("invariants");
    assert_eq!(state.remote_principal_cap, AssetAmount::new(40_000_000));
    assert_eq!(
        state.buckets.accepted_remote_value,
        AssetAmount::new(40_000_000)
    );
    assert!(state.buckets.outbound_in_transit.is_zero());
    assert_eq!(state.managed_nav(), AssetAmount::new(100_000_000));
}

#[test]
fn a_haircut_report_lowers_nav_without_creating_value() {
    let state = run(
        &funded(),
        &[allocate(1, 40_000_000), complete(1, 30_000_000)],
    );
    check_invariants(&state).expect("invariants");
    assert_eq!(
        state.buckets.accepted_remote_value,
        AssetAmount::new(30_000_000)
    );
    assert_eq!(state.managed_nav(), AssetAmount::new(90_000_000));
}

#[test]
fn recall_request_does_not_change_nav() {
    let state = run(
        &funded(),
        &[allocate(1, 40_000_000), complete(1, 40_000_000)],
    );
    let nav = state.managed_nav();
    let state = run(&state, &[recall(2, 40_000_000)]);
    assert_eq!(state.managed_nav(), nav);
    assert_eq!(state.active_cycle, CycleKind::Recall);
}

#[test]
fn recall_sent_before_assets_recognizes_exact_loss() {
    let state = run(
        &funded(),
        &[
            allocate(1, 40_000_000),
            complete(1, 40_000_000),
            recall(2, 40_000_000),
            recall_sent(2, 40_000_000, 35_000_000, 5_000_000),
        ],
    );
    check_invariants(&state).expect("invariants");
    assert_eq!(state.managed_nav(), AssetAmount::new(95_000_000));
    assert!(state.remote_principal_cap.is_zero());
    assert_eq!(
        state.buckets.inbound_in_transit,
        AssetAmount::new(35_000_000)
    );
    assert_eq!(state.recognized_loss, AssetAmount::new(5_000_000));
    assert_eq!(state.buckets.idle_backing, AssetAmount::new(60_000_000));
}

#[test]
fn assets_before_recall_sent_do_not_double_count() {
    let state = run(
        &funded(),
        &[
            allocate(1, 40_000_000),
            complete(1, 40_000_000),
            recall(2, 40_000_000),
            observe(35_000_000),
        ],
    );
    assert_eq!(state.managed_nav(), AssetAmount::new(100_000_000));
    assert_eq!(
        state.buckets.unattributed_inbound,
        AssetAmount::new(35_000_000)
    );
    let state = run(&state, &[recall_sent(2, 40_000_000, 35_000_000, 5_000_000)]);
    check_invariants(&state).expect("invariants");
    assert_eq!(state.managed_nav(), AssetAmount::new(95_000_000));
    assert!(state.buckets.inbound_in_transit.is_zero());
    assert!(state.buckets.unattributed_inbound.is_zero());
    assert_eq!(state.buckets.idle_backing, AssetAmount::new(95_000_000));
}

#[test]
fn arrival_after_recall_sent_preserves_nav() {
    let state = run(
        &funded(),
        &[
            allocate(1, 40_000_000),
            complete(1, 40_000_000),
            recall(2, 40_000_000),
            recall_sent(2, 40_000_000, 35_000_000, 5_000_000),
        ],
    );
    let nav = state.managed_nav();
    let state = run(&state, &[observe(35_000_000)]);
    check_invariants(&state).expect("invariants");
    assert_eq!(state.managed_nav(), nav);
    assert!(state.buckets.inbound_in_transit.is_zero());
    assert_eq!(state.buckets.idle_backing, AssetAmount::new(95_000_000));
}

#[test]
fn duplicate_assets_stay_unattributed() {
    let state = run(
        &funded(),
        &[
            allocate(1, 40_000_000),
            complete(1, 40_000_000),
            recall(2, 40_000_000),
            recall_sent(2, 40_000_000, 35_000_000, 5_000_000),
            observe(35_000_000),
            observe(35_000_000),
        ],
    );
    check_invariants(&state).expect("invariants");
    assert_eq!(state.managed_nav(), AssetAmount::new(95_000_000));
    assert_eq!(
        state.buckets.unattributed_balance,
        AssetAmount::new(35_000_500)
    );
}

#[test]
fn total_loss_recall_needs_no_arrival() {
    let state = run(
        &funded(),
        &[
            allocate(1, 40_000_000),
            complete(1, 40_000_000),
            recall(2, 40_000_000),
            recall_sent(2, 40_000_000, 0, 40_000_000),
        ],
    );
    check_invariants(&state).expect("invariants");
    assert_eq!(state.managed_nav(), AssetAmount::new(60_000_000));
    assert!(state.buckets.inbound_in_transit.is_zero());
    assert_eq!(state.recognized_loss, AssetAmount::new(40_000_000));
    assert_eq!(state.active_cycle, CycleKind::None);
}

#[test]
fn freeze_blocks_allocation_and_allows_recall() {
    let state = run(
        &funded(),
        &[allocate(1, 40_000_000), complete(1, 40_000_000)],
    );
    let frozen = run(&state, &[Operation::Freeze { actor: ADMIN }]);
    assert_eq!(frozen.vault_state, VaultState::Frozen);
    let err = apply(&frozen, allocate(2, 1_000_000)).expect_err("allocate frozen");
    assert_eq!(err, Rejection::InvalidVaultState);
    let state = run(&frozen, &[recall(3, 40_000_000)]);
    assert_eq!(state.active_cycle, CycleKind::Recall);
}

#[test]
fn a_reused_transfer_id_is_rejected() {
    let state = run(
        &funded(),
        &[allocate(1, 10_000_000), complete(1, 10_000_000)],
    );
    let err = apply(&state, allocate(1, 10_000_000)).expect_err("reuse");
    assert_eq!(err, Rejection::TransferIdReused);
}

/// A Solana report: the leg's principal and the loss realized since the last
/// loss the vault acknowledged.
fn report(value: u128, newly_realized_loss: u128) -> Operation {
    Operation::ApplyRemoteValuation {
        reported_value: AssetAmount::new(value),
        newly_realized_loss: AssetAmount::new(newly_realized_loss),
    }
}

/// 80 of 100 deployed; the expected NAV is always 100 less realized loss.
fn deployed() -> accounting_model::State {
    run(
        &funded(),
        &[allocate(1, 80_000_000), complete(1, 80_000_000)],
    )
}

fn assert_nav(state: &accounting_model::State, loss: u128, step: &str) {
    assert_eq!(
        state.managed_nav(),
        AssetAmount::new(100_000_000 - loss),
        "{step}"
    );
}

#[test]
fn a_report_after_the_send_keeps_nav_until_the_notice() {
    let state = run(&deployed(), &[recall(2, 40_000_000), report(40_000_000, 0)]);
    assert_nav(&state, 0, "report before notice");
    let state = run(&state, &[observe(40_000_000)]);
    assert_nav(&state, 0, "assets before notice");
    let state = run(&state, &[recall_sent(2, 40_000_000, 40_000_000, 0)]);
    assert_nav(&state, 0, "notice");
    assert_eq!(
        state.buckets.accepted_remote_value,
        AssetAmount::new(40_000_000)
    );
    let state = run(&state, &[report(40_000_000, 0)]);
    assert_nav(&state, 0, "next report");
}

#[test]
fn a_partial_send_report_keeps_nav() {
    let state = run(&deployed(), &[recall(2, 40_000_000), report(55_000_000, 0)]);
    assert_nav(&state, 0, "partial send");
    let state = run(&state, &[recall_sent(2, 40_000_000, 40_000_000, 0)]);
    assert_nav(&state, 0, "notice");
}

#[test]
fn a_lossy_withdrawal_is_counted_once() {
    let state = run(
        &deployed(),
        &[recall(2, 40_000_000), report(70_000_000, 10_000_000)],
    );
    assert_nav(&state, 10_000_000, "loss before send");
    let state = run(&state, &[report(40_000_000, 0)]);
    assert_nav(&state, 10_000_000, "loss after send");
    let state = run(
        &state,
        &[recall_sent(2, 40_000_000, 30_000_000, 10_000_000)],
    );
    assert_nav(&state, 10_000_000, "notice");
    assert_eq!(state.recognized_loss, AssetAmount::new(10_000_000));
    let state = run(&state, &[observe(30_000_000), report(40_000_000, 0)]);
    assert_nav(&state, 10_000_000, "settled");
}

#[test]
fn a_total_loss_during_a_recall_is_counted_once() {
    let state = run(
        &deployed(),
        &[recall(2, 40_000_000), report(40_000_000, 40_000_000)],
    );
    assert_nav(&state, 40_000_000, "report");
    let state = run(&state, &[recall_sent(2, 40_000_000, 0, 40_000_000)]);
    assert_nav(&state, 40_000_000, "notice");
}

#[test]
fn a_settled_loss_is_not_deducted_again_by_the_next_recall() {
    let state = run(
        &deployed(),
        &[
            recall(2, 40_000_000),
            recall_sent(2, 40_000_000, 30_000_000, 10_000_000),
            recall(3, 20_000_000),
            report(20_000_000, 0),
        ],
    );
    assert_nav(&state, 10_000_000, "report in second recall");
    let state = run(&state, &[recall_sent(3, 20_000_000, 20_000_000, 0)]);
    assert_nav(&state, 10_000_000, "second notice");
}

#[test]
fn a_lock_during_a_recall_prices_the_epoch_net_of_loss() {
    let state = run(
        &deployed(),
        &[recall(2, 40_000_000), report(40_000_000, 10_000_000)],
    );
    let epoch = state.epoch.expect("open epoch");
    let state = run(
        &state,
        &[
            Operation::CutoffEpoch {
                now: epoch.cutoff_at,
            },
            Operation::LockRemoteValuation,
            Operation::FinalizeEpoch,
        ],
    );
    match state.epochs.get(&epoch.id) {
        Some(accounting_model::EpochOutcome::Finalized(terms)) => {
            assert_eq!(terms.total_assets, AssetAmount::new(90_000_000));
        }
        other => panic!("epoch not finalized: {other:?}"),
    }
    let state = run(
        &state,
        &[recall_sent(2, 40_000_000, 30_000_000, 10_000_000)],
    );
    assert_nav(&state, 10_000_000, "notice after finalize");
}
