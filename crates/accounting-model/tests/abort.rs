#![allow(clippy::expect_used, clippy::panic)]

mod common;

use accounting_model::{
    AssetAmount, EpochId, Operation, Rejection, RequestKey, RequestState, ShareAmount, State,
    VaultState, Violation, apply, check_invariants,
};
use common::{
    ADMIN, ALICE, BOB, CAROL, EPOCH_DURATION, FUNDING, GUARDIAN, OUTSIDER, claim_deposit,
    claim_redeem, cutoff, deposit, funded_with_shares, genesis, open_next, redeem, refund_deposit,
    refund_redeem, run, settle_epoch,
};

fn freeze(state: &State) -> State {
    run(state, &[Operation::Freeze { actor: GUARDIAN }])
}

fn abort() -> Operation {
    Operation::AbortEpoch { actor: GUARDIAN }
}

/// A frozen genesis epoch holding one deposit and nothing else.
fn frozen_with_deposit() -> State {
    let state = run(&genesis(), &[deposit(ALICE, 40_000_000)]);
    freeze(&state)
}

/// A frozen second epoch holding both a deposit and a redemption.
fn frozen_with_both() -> State {
    let state = funded_with_shares(ALICE, 20_000_000);
    let held = state.account(ALICE).shares.raw();
    let state = run(&state, &[redeem(ALICE, held / 2), deposit(BOB, 9_000_000)]);
    freeze(&state)
}

// Abort admission

#[test]
fn an_open_epoch_can_be_aborted_while_frozen() {
    let state = run(&frozen_with_deposit(), &[abort()]);
    assert!(state.epoch.is_none());
    assert!(state.aborted_terms(EpochId::GENESIS).is_some());
    assert_eq!(state.vault_state, VaultState::Frozen);
}

#[test]
fn a_cut_off_epoch_can_be_aborted_while_frozen() {
    let state = run(
        &genesis(),
        &[deposit(ALICE, 12_000_000), cutoff(EPOCH_DURATION)],
    );
    let state = run(&freeze(&state), &[abort()]);
    assert!(state.epoch.is_none());
    let terms = state.aborted_terms(EpochId::GENESIS).expect("aborted");
    assert_eq!(terms.refund_assets, AssetAmount::new(12_000_000));
}

#[test]
fn abort_fails_while_active() {
    let state = run(&genesis(), &[deposit(ALICE, 5_000_000)]);
    assert_eq!(apply(&state, abort()), Err(Rejection::InvalidVaultState));
}

#[test]
fn abort_fails_while_paused() {
    let state = run(
        &genesis(),
        &[deposit(ALICE, 5_000_000), Operation::Pause { actor: ADMIN }],
    );
    assert_eq!(apply(&state, abort()), Err(Rejection::InvalidVaultState));
}

#[test]
fn abort_fails_without_an_active_epoch() {
    let state = settle_epoch(&genesis(), EPOCH_DURATION);
    let state = freeze(&state);
    assert_eq!(apply(&state, abort()), Err(Rejection::EpochNotOpen));
}

#[test]
fn an_unauthorized_actor_cannot_abort() {
    let state = frozen_with_deposit();
    assert_eq!(
        apply(&state, Operation::AbortEpoch { actor: OUTSIDER }),
        Err(Rejection::UnauthorizedActor)
    );
    assert!(apply(&state, Operation::AbortEpoch { actor: ADMIN }).is_ok());
}

#[test]
fn a_rejected_abort_leaves_the_state_unchanged() {
    let active = run(&genesis(), &[deposit(ALICE, 5_000_000)]);
    let before = active.clone();
    assert!(apply(&active, abort()).is_err());
    assert_eq!(before, active);

    let frozen = frozen_with_deposit();
    let before = frozen.clone();
    assert!(apply(&frozen, Operation::AbortEpoch { actor: OUTSIDER }).is_err());
    assert_eq!(before, frozen);
}

// Abort behaviour

#[test]
fn abort_preserves_escrow_and_request_records() {
    let before = frozen_with_both();
    let escrow = before.buckets.pending_deposit_escrow;
    let escrowed = before.escrowed_redeem_shares;
    let deposits = before.deposit_requests.clone();
    let redeems = before.redeem_requests.clone();

    let after = run(&before, &[abort()]);
    assert_eq!(after.buckets.pending_deposit_escrow, escrow);
    assert_eq!(after.escrowed_redeem_shares, escrowed);
    assert_eq!(after.deposit_requests, deposits);
    assert_eq!(after.redeem_requests, redeems);
}

#[test]
fn abort_leaves_unrelated_balances_unchanged() {
    let before = frozen_with_both();
    let after = run(&before, &[abort()]);
    assert_eq!(after.accounts, before.accounts);
    assert_eq!(after.buckets, before.buckets);
    assert_eq!(after.total_share_supply, before.total_share_supply);
    assert_eq!(
        after.claimable_deposit_shares,
        before.claimable_deposit_shares
    );
    assert_eq!(
        after.burned_redemption_shares,
        before.burned_redemption_shares
    );
    assert_eq!(after.managed_nav(), before.managed_nav());
}

#[test]
fn abort_does_not_settle_any_request() {
    let state = run(&frozen_with_both(), &[abort()]);
    let aborted = EpochId::new(1);
    for (key, request) in &state.deposit_requests {
        if key.epoch == aborted {
            assert!(!request.claimed);
        }
    }
    for (key, request) in &state.redeem_requests {
        if key.epoch == aborted {
            assert!(!request.claimed);
        }
    }
    assert_eq!(
        state.deposit_request_state(RequestKey::new(aborted, BOB)),
        Some(RequestState::Refundable)
    );
    assert_eq!(
        state.redeem_request_state(RequestKey::new(aborted, ALICE)),
        Some(RequestState::Refundable)
    );
}

#[test]
fn an_aborted_epoch_can_never_finalize() {
    let mut state = run(&frozen_with_deposit(), &[abort()]);
    assert!(apply(&state, Operation::FinalizeEpoch).is_err());

    // The rule holds even without the freeze that led to the abort.
    state.vault_state = VaultState::Active;
    assert_eq!(check_invariants(&state), Ok(()));
    assert_eq!(
        apply(&state, Operation::FinalizeEpoch),
        Err(Rejection::EpochNotOpen)
    );
    assert!(state.finalized_terms(EpochId::GENESIS).is_none());
    assert!(state.aborted_terms(EpochId::GENESIS).is_some());
}

#[test]
fn a_finalized_epoch_can_never_abort() {
    let state = run(&genesis(), &[deposit(ALICE, 10_000_000)]);
    let state = settle_epoch(&state, EPOCH_DURATION);
    let state = freeze(&state);
    assert_eq!(apply(&state, abort()), Err(Rejection::EpochNotOpen));
    assert!(state.aborted_terms(EpochId::GENESIS).is_none());
}

// Refunds

#[test]
fn a_deposit_is_refunded_in_full_after_abort() {
    let state = run(&frozen_with_deposit(), &[abort()]);
    assert_eq!(
        state.account(ALICE).assets,
        AssetAmount::new(FUNDING.saturating_sub(40_000_000))
    );

    let state = run(&state, &[refund_deposit(ALICE, 0)]);
    assert_eq!(state.account(ALICE).assets, AssetAmount::new(FUNDING));
    assert_eq!(state.buckets.pending_deposit_escrow, AssetAmount::ZERO);
    assert_eq!(
        state.deposit_request_state(RequestKey::new(EpochId::GENESIS, ALICE)),
        Some(RequestState::Refunded)
    );
}

#[test]
fn escrowed_shares_are_refunded_in_full_after_abort() {
    let state = funded_with_shares(ALICE, 20_000_000);
    let held = state.account(ALICE).shares;
    let state = run(&state, &[redeem(ALICE, held.raw())]);
    let state = run(&freeze(&state), &[abort()]);
    assert_eq!(state.account(ALICE).shares, ShareAmount::ZERO);

    let state = run(&state, &[refund_redeem(ALICE, 1)]);
    assert_eq!(state.account(ALICE).shares, held);
    assert_eq!(state.escrowed_redeem_shares, ShareAmount::ZERO);
}

#[test]
fn refunds_stay_available_while_the_vault_is_frozen() {
    let state = run(&frozen_with_both(), &[abort()]);
    assert_eq!(state.vault_state, VaultState::Frozen);
    // An aborted epoch owes no settlement, only refunds.
    assert_eq!(state.outstanding_redeem_assets(), Ok(AssetAmount::ZERO));
    let state = run(&state, &[refund_deposit(BOB, 1), refund_redeem(ALICE, 1)]);
    assert_eq!(state.buckets.pending_deposit_escrow, AssetAmount::ZERO);
    assert_eq!(state.escrowed_redeem_shares, ShareAmount::ZERO);
}

#[test]
fn a_deposit_refund_cannot_be_claimed_twice() {
    let state = run(&frozen_with_deposit(), &[abort(), refund_deposit(ALICE, 0)]);
    assert_eq!(
        apply(&state, refund_deposit(ALICE, 0)),
        Err(Rejection::ClaimAlreadyConsumed)
    );
}

#[test]
fn a_redemption_refund_cannot_be_claimed_twice() {
    let state = run(&frozen_with_both(), &[abort(), refund_redeem(ALICE, 1)]);
    assert_eq!(
        apply(&state, refund_redeem(ALICE, 1)),
        Err(Rejection::ClaimAlreadyConsumed)
    );
}

#[test]
fn an_aborted_request_cannot_settle() {
    let state = run(&frozen_with_both(), &[abort()]);
    assert_eq!(
        apply(&state, claim_deposit(BOB, 1)),
        Err(Rejection::EpochNotFinalized)
    );
    assert_eq!(
        apply(&state, claim_redeem(ALICE, 1)),
        Err(Rejection::EpochNotFinalized)
    );
}

#[test]
fn a_refund_cannot_be_taken_from_a_finalized_epoch() {
    let state = run(&genesis(), &[deposit(ALICE, 10_000_000)]);
    let state = settle_epoch(&state, EPOCH_DURATION);
    assert_eq!(
        apply(&state, refund_deposit(ALICE, 0)),
        Err(Rejection::EpochNotAborted)
    );
}

#[test]
fn a_settled_request_cannot_also_be_refunded() {
    let state = run(&genesis(), &[deposit(ALICE, 10_000_000)]);
    let state = settle_epoch(&state, EPOCH_DURATION);
    let state = run(&state, &[claim_deposit(ALICE, 0)]);
    assert_eq!(
        apply(&state, refund_deposit(ALICE, 0)),
        Err(Rejection::EpochNotAborted)
    );
}

#[test]
fn a_cancelled_request_is_not_refundable_after_abort() {
    let state = run(
        &genesis(),
        &[
            deposit(ALICE, 8_000_000),
            Operation::CancelDeposit { account: ALICE },
            deposit(BOB, 3_000_000),
        ],
    );
    let state = run(&freeze(&state), &[abort()]);
    assert_eq!(
        apply(&state, refund_deposit(ALICE, 0)),
        Err(Rejection::RequestAlreadyCancelled)
    );
    let terms = state.aborted_terms(EpochId::GENESIS).expect("aborted");
    assert_eq!(terms.refund_assets, AssetAmount::new(3_000_000));
}

#[test]
fn a_cancelled_redemption_is_not_refundable_after_abort() {
    let state = funded_with_shares(ALICE, 20_000_000);
    let held = state.account(ALICE).shares.raw();
    let state = run(
        &state,
        &[
            redeem(ALICE, held),
            Operation::CancelRedeem { account: ALICE },
            deposit(BOB, 3_000_000),
        ],
    );
    let state = run(&freeze(&state), &[abort()]);
    assert_eq!(
        apply(&state, refund_redeem(ALICE, 1)),
        Err(Rejection::RequestAlreadyCancelled)
    );
    let terms = state.aborted_terms(EpochId::new(1)).expect("aborted");
    assert_eq!(terms.refund_shares, ShareAmount::ZERO);
}

#[test]
fn an_account_without_a_request_has_nothing_to_refund() {
    let state = run(&frozen_with_deposit(), &[abort()]);
    assert_eq!(
        apply(&state, refund_deposit(CAROL, 0)),
        Err(Rejection::RequestNotFound)
    );
}

// Finalized claims survive a later freeze and abort

#[test]
fn an_earlier_deposit_claim_still_works_after_a_later_abort() {
    let state = run(&genesis(), &[deposit(ALICE, 10_000_000)]);
    let state = settle_epoch(&state, EPOCH_DURATION);
    let state = run(
        &state,
        &[open_next(EPOCH_DURATION), deposit(BOB, 4_000_000)],
    );
    let state = run(&freeze(&state), &[abort()]);

    let state = run(&state, &[claim_deposit(ALICE, 0)]);
    assert_eq!(
        state.account(ALICE).shares,
        ShareAmount::new(10_000_000_000_000_000_000)
    );
}

#[test]
fn an_earlier_redemption_claim_still_works_after_a_later_abort() {
    let state = funded_with_shares(ALICE, 10_000_000);
    let held = state.account(ALICE).shares.raw();
    let state = run(&state, &[redeem(ALICE, held)]);
    let state = settle_epoch(&state, EPOCH_DURATION.saturating_mul(2));
    let state = run(
        &state,
        &[
            open_next(EPOCH_DURATION.saturating_mul(2)),
            deposit(BOB, 6_000_000),
        ],
    );
    let state = run(&freeze(&state), &[abort()]);

    let state = run(&state, &[claim_redeem(ALICE, 1)]);
    assert_eq!(state.account(ALICE).assets, AssetAmount::new(FUNDING));
}

#[test]
fn refunds_and_finalized_claims_coexist_in_one_state() {
    let state = run(&genesis(), &[deposit(ALICE, 10_000_000)]);
    let state = settle_epoch(&state, EPOCH_DURATION);
    let state = run(
        &state,
        &[open_next(EPOCH_DURATION), deposit(BOB, 4_000_000)],
    );
    let state = run(&freeze(&state), &[abort()]);

    // The finalized epoch owes a settlement while the aborted one owes a refund.
    assert_eq!(
        state.outstanding_refund_assets(),
        Ok(AssetAmount::new(4_000_000))
    );
    assert!(
        !state
            .outstanding_deposit_shares()
            .expect("shares")
            .is_zero()
    );

    let state = run(&state, &[claim_deposit(ALICE, 0), refund_deposit(BOB, 1)]);
    assert_eq!(state.buckets.pending_deposit_escrow, AssetAmount::ZERO);
    assert_eq!(state.claimable_deposit_shares, ShareAmount::ZERO);
    assert_eq!(state.account(BOB).assets, AssetAmount::new(FUNDING));
    assert_eq!(state.outstanding_refund_assets(), Ok(AssetAmount::ZERO));
    assert_eq!(state.outstanding_refund_shares(), Ok(ShareAmount::ZERO));
    assert_eq!(check_invariants(&state), Ok(()));
}

// Frozen funds always have a way out

#[test]
fn no_frozen_deposit_is_stranded_after_abort() {
    let state = run(
        &genesis(),
        &[
            deposit(ALICE, 10_000_001),
            deposit(BOB, 7_000_003),
            deposit(CAROL, 1_000_007),
        ],
    );
    let state = run(&freeze(&state), &[abort()]);
    let state = run(
        &state,
        &[
            refund_deposit(ALICE, 0),
            refund_deposit(BOB, 0),
            refund_deposit(CAROL, 0),
        ],
    );
    assert_eq!(state.buckets.pending_deposit_escrow, AssetAmount::ZERO);
    for account in [ALICE, BOB, CAROL] {
        assert_eq!(state.account(account).assets, AssetAmount::new(FUNDING));
    }
}

#[test]
fn abort_refunds_can_be_taken_in_any_order() {
    let base = run(
        &genesis(),
        &[
            deposit(ALICE, 10_000_001),
            deposit(BOB, 7_000_003),
            deposit(CAROL, 1_000_007),
        ],
    );
    let base = run(&freeze(&base), &[abort()]);
    let forward = run(
        &base,
        &[
            refund_deposit(ALICE, 0),
            refund_deposit(BOB, 0),
            refund_deposit(CAROL, 0),
        ],
    );
    let reverse = run(
        &base,
        &[
            refund_deposit(CAROL, 0),
            refund_deposit(BOB, 0),
            refund_deposit(ALICE, 0),
        ],
    );
    assert_eq!(forward, reverse);
}

// Tampering with abort records

#[test]
fn rewriting_an_aborted_refund_total_is_detected() {
    let mut state = run(&frozen_with_both(), &[abort()]);
    match state
        .epochs
        .get_mut(&EpochId::new(1))
        .expect("epoch has an outcome")
    {
        accounting_model::EpochOutcome::Aborted(terms) => {
            terms.refund_assets = AssetAmount::new(terms.refund_assets.raw() + 1);
        }
        accounting_model::EpochOutcome::Finalized(_) => panic!("epoch was finalized"),
    }
    assert_eq!(
        check_invariants(&state),
        Err(Violation::AbortedRefundMismatch)
    );
}

#[test]
fn refunding_without_releasing_escrow_is_detected() {
    let mut state = run(&frozen_with_both(), &[abort()]);
    state
        .deposit_requests
        .get_mut(&RequestKey::new(EpochId::new(1), BOB))
        .expect("request")
        .claimed = true;
    assert_eq!(
        check_invariants(&state),
        Err(Violation::EscrowAggregateMismatch)
    );
}
