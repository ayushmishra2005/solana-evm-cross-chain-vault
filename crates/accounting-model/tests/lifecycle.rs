#![allow(clippy::expect_used, clippy::panic)]

mod common;

use accounting_model::{
    Account, AssetAmount, EpochId, EpochOutcome, Genesis, Operation, RedeemRequest, Rejection,
    RequestKey, RequestState, ShareAmount, State, Timestamp, VaultState, Violation, apply,
    check_invariants,
};
use common::{
    ADMIN, ALICE, BOB, CAROL, EPOCH_DURATION, FUNDING, GUARDIAN, MIN_DEPOSIT, MIN_REDEEM,
    authority, claim_deposit, claim_redeem, config, cutoff, deposit, funded_with_shares, genesis,
    genesis_funding_only, open_next, redeem, run, settle_epoch,
};

// Deposit and redemption lifecycle

#[test]
fn first_deposit_receives_shares_at_the_virtual_offset_rate() {
    let state = run(&genesis(), &[deposit(ALICE, 10_000_000)]);
    assert_eq!(state.managed_nav(), AssetAmount::ZERO);
    assert_eq!(
        state.buckets.pending_deposit_escrow,
        AssetAmount::new(10_000_000)
    );

    let state = settle_epoch(&state, EPOCH_DURATION);
    assert_eq!(state.managed_nav(), AssetAmount::new(10_000_000));

    let state = run(&state, &[claim_deposit(ALICE, 0)]);
    assert_eq!(
        state.account(ALICE).shares,
        ShareAmount::new(10_000_000_000_000_000_000)
    );
}

#[test]
fn multiple_deposits_in_one_epoch_share_the_same_price() {
    let state = run(
        &genesis(),
        &[deposit(ALICE, 4_000_000), deposit(BOB, 6_000_000)],
    );
    let state = settle_epoch(&state, EPOCH_DURATION);
    let state = run(&state, &[claim_deposit(ALICE, 0), claim_deposit(BOB, 0)]);
    let alice = state.account(ALICE).shares.raw();
    let bob = state.account(BOB).shares.raw();
    assert_eq!(alice.saturating_mul(3), bob.saturating_mul(2));
}

#[test]
fn repeated_deposits_from_one_account_aggregate_within_an_epoch() {
    let state = run(
        &genesis(),
        &[deposit(ALICE, 2_000_000), deposit(ALICE, 3_000_000)],
    );
    let epoch = state.epoch.expect("epoch is open");
    assert_eq!(epoch.pending_deposit_assets, AssetAmount::new(5_000_000));
}

#[test]
fn multiple_redemptions_in_one_epoch_are_all_funded() {
    let state = run(
        &genesis(),
        &[deposit(ALICE, 10_000_000), deposit(BOB, 10_000_000)],
    );
    let state = settle_epoch(&state, EPOCH_DURATION);
    let state = run(
        &state,
        &[
            claim_deposit(ALICE, 0),
            claim_deposit(BOB, 0),
            open_next(EPOCH_DURATION),
        ],
    );

    let alice_shares = state.account(ALICE).shares.raw();
    let bob_shares = state.account(BOB).shares.raw();
    let state = run(
        &state,
        &[redeem(ALICE, alice_shares), redeem(BOB, bob_shares)],
    );
    let state = settle_epoch(&state, EPOCH_DURATION.saturating_mul(2));
    let state = run(&state, &[claim_redeem(ALICE, 1), claim_redeem(BOB, 1)]);

    assert_eq!(state.account(ALICE).assets, AssetAmount::new(FUNDING));
    assert_eq!(state.account(BOB).assets, AssetAmount::new(FUNDING));
    assert_eq!(state.total_share_supply, ShareAmount::ZERO);
}

// Cancellation and re-request

#[test]
fn deposit_cancellation_before_cutoff_returns_the_exact_amount() {
    let state = run(
        &genesis(),
        &[
            deposit(ALICE, 7_000_000),
            Operation::CancelDeposit { account: ALICE },
        ],
    );
    assert_eq!(state.account(ALICE).assets, AssetAmount::new(FUNDING));
    assert_eq!(state.buckets.pending_deposit_escrow, AssetAmount::ZERO);
    assert_eq!(
        state.deposit_request_state(RequestKey::new(EpochId::GENESIS, ALICE)),
        Some(RequestState::Cancelled)
    );
}

#[test]
fn redemption_cancellation_before_cutoff_returns_the_exact_shares() {
    let state = funded_with_shares(ALICE, 10_000_000);
    let held = state.account(ALICE).shares;
    let state = run(
        &state,
        &[
            redeem(ALICE, held.raw()),
            Operation::CancelRedeem { account: ALICE },
        ],
    );
    assert_eq!(state.account(ALICE).shares, held);
    assert_eq!(state.escrowed_redeem_shares, ShareAmount::ZERO);
}

#[test]
fn deposit_cancellation_after_cutoff_is_rejected() {
    let state = run(&genesis(), &[deposit(ALICE, 5_000_000)]);
    let state = run(&state, &[cutoff(EPOCH_DURATION)]);
    assert_eq!(
        apply(&state, Operation::CancelDeposit { account: ALICE }),
        Err(Rejection::CancellationAfterCutoff)
    );
}

#[test]
fn redemption_cancellation_after_cutoff_is_rejected() {
    let state = funded_with_shares(ALICE, 10_000_000);
    let held = state.account(ALICE).shares.raw();
    let state = run(
        &state,
        &[
            redeem(ALICE, held),
            cutoff(EPOCH_DURATION.saturating_mul(2)),
        ],
    );
    assert_eq!(
        apply(&state, Operation::CancelRedeem { account: ALICE }),
        Err(Rejection::CancellationAfterCutoff)
    );
}

#[test]
fn a_cancelled_deposit_never_settles() {
    let state = run(
        &genesis(),
        &[
            deposit(ALICE, 5_000_000),
            Operation::CancelDeposit { account: ALICE },
            deposit(BOB, 5_000_000),
        ],
    );
    let state = settle_epoch(&state, EPOCH_DURATION);
    let terms = state
        .finalized_terms(EpochId::GENESIS)
        .expect("epoch is finalized");
    assert_eq!(terms.deposit_assets, AssetAmount::new(5_000_000));
    assert_eq!(
        apply(&state, claim_deposit(ALICE, 0)),
        Err(Rejection::RequestAlreadyCancelled)
    );
}

#[test]
fn a_deposit_can_be_cancelled_and_requested_again_before_cutoff() {
    let state = run(
        &genesis(),
        &[
            deposit(ALICE, 7_000_000),
            Operation::CancelDeposit { account: ALICE },
            deposit(ALICE, 3_000_000),
        ],
    );
    assert_eq!(
        state.buckets.pending_deposit_escrow,
        AssetAmount::new(3_000_000)
    );
    assert_eq!(
        state.account(ALICE).assets,
        AssetAmount::new(FUNDING.saturating_sub(3_000_000))
    );
    assert_eq!(
        state.deposit_request_state(RequestKey::new(EpochId::GENESIS, ALICE)),
        Some(RequestState::Pending)
    );
}

#[test]
fn a_redemption_can_be_cancelled_and_requested_again_before_cutoff() {
    let state = funded_with_shares(ALICE, 10_000_000);
    let held = state.account(ALICE).shares.raw();
    let half = held / 2;
    let state = run(
        &state,
        &[
            redeem(ALICE, held),
            Operation::CancelRedeem { account: ALICE },
            redeem(ALICE, half),
        ],
    );
    assert_eq!(state.escrowed_redeem_shares, ShareAmount::new(half));
    assert_eq!(
        state.account(ALICE).shares,
        ShareAmount::new(held.saturating_sub(half))
    );
}

#[test]
fn a_reopened_deposit_settles_only_the_new_amount() {
    let state = run(
        &genesis(),
        &[
            deposit(ALICE, 9_000_000),
            Operation::CancelDeposit { account: ALICE },
            deposit(ALICE, 2_000_000),
        ],
    );
    let state = settle_epoch(&state, EPOCH_DURATION);
    let terms = *state
        .finalized_terms(EpochId::GENESIS)
        .expect("epoch is finalized");
    assert_eq!(terms.deposit_assets, AssetAmount::new(2_000_000));

    let state = run(&state, &[claim_deposit(ALICE, 0)]);
    assert_eq!(
        state.account(ALICE).shares,
        ShareAmount::new(2_000_000_000_000_000_000)
    );
}

#[test]
fn a_reopened_redemption_settles_only_the_new_shares() {
    let state = funded_with_shares(ALICE, 10_000_000);
    let held = state.account(ALICE).shares.raw();
    let quarter = held / 4;
    let state = run(
        &state,
        &[
            redeem(ALICE, held),
            Operation::CancelRedeem { account: ALICE },
            redeem(ALICE, quarter),
        ],
    );
    let state = settle_epoch(&state, EPOCH_DURATION.saturating_mul(2));
    let terms = *state.finalized_terms(EpochId::new(1)).expect("finalized");
    assert_eq!(terms.redeem_shares, ShareAmount::new(quarter));
    assert_eq!(terms.redeem_assets, AssetAmount::new(2_500_000));
}

#[test]
fn repeated_cancel_and_request_cycles_keep_escrow_exact() {
    let mut state = genesis();
    for round in 1..=4u128 {
        state = run(
            &state,
            &[
                deposit(ALICE, MIN_DEPOSIT.saturating_mul(round)),
                Operation::CancelDeposit { account: ALICE },
            ],
        );
        assert_eq!(state.buckets.pending_deposit_escrow, AssetAmount::ZERO);
        assert_eq!(state.account(ALICE).assets, AssetAmount::new(FUNDING));
    }
    let state = run(&state, &[deposit(ALICE, 6_000_000)]);
    assert_eq!(
        state.buckets.pending_deposit_escrow,
        AssetAmount::new(6_000_000)
    );
    let epoch = state.epoch.expect("epoch is open");
    assert_eq!(epoch.pending_deposit_assets, AssetAmount::new(6_000_000));
}

#[test]
fn a_reopened_request_can_be_cancelled_again() {
    let state = run(
        &genesis(),
        &[
            deposit(ALICE, 4_000_000),
            Operation::CancelDeposit { account: ALICE },
            deposit(ALICE, 5_000_000),
            Operation::CancelDeposit { account: ALICE },
        ],
    );
    assert_eq!(state.account(ALICE).assets, AssetAmount::new(FUNDING));
    assert_eq!(state.buckets.pending_deposit_escrow, AssetAmount::ZERO);
    assert_eq!(
        state.deposit_request_state(RequestKey::new(EpochId::GENESIS, ALICE)),
        Some(RequestState::Cancelled)
    );
}

#[test]
fn cancelling_twice_without_a_new_request_is_rejected() {
    let state = run(
        &genesis(),
        &[
            deposit(ALICE, 4_000_000),
            Operation::CancelDeposit { account: ALICE },
        ],
    );
    assert_eq!(
        apply(&state, Operation::CancelDeposit { account: ALICE }),
        Err(Rejection::RequestAlreadyCancelled)
    );
}

#[test]
fn a_cancelled_amount_never_reaches_finalized_terms() {
    let state = run(
        &genesis(),
        &[
            deposit(ALICE, 100_000_000),
            Operation::CancelDeposit { account: ALICE },
            deposit(ALICE, 1_000_000),
            deposit(BOB, 2_000_000),
            Operation::CancelDeposit { account: BOB },
        ],
    );
    let state = settle_epoch(&state, EPOCH_DURATION);
    let terms = state.finalized_terms(EpochId::GENESIS).expect("finalized");
    assert_eq!(terms.deposit_assets, AssetAmount::new(1_000_000));
    assert_eq!(
        state.deposit_requests[&RequestKey::new(EpochId::GENESIS, ALICE)].cancelled_assets,
        AssetAmount::new(100_000_000)
    );
}

// Claims

#[test]
fn a_deposit_claim_cannot_be_repeated() {
    let state = run(&genesis(), &[deposit(ALICE, 8_000_000)]);
    let state = settle_epoch(&state, EPOCH_DURATION);
    let state = run(&state, &[claim_deposit(ALICE, 0)]);
    assert_eq!(
        apply(&state, claim_deposit(ALICE, 0)),
        Err(Rejection::ClaimAlreadyConsumed)
    );
}

#[test]
fn a_redemption_claim_cannot_be_repeated() {
    let state = funded_with_shares(ALICE, 10_000_000);
    let held = state.account(ALICE).shares.raw();
    let state = run(&state, &[redeem(ALICE, held)]);
    let state = settle_epoch(&state, EPOCH_DURATION.saturating_mul(2));
    let state = run(&state, &[claim_redeem(ALICE, 1)]);
    assert_eq!(
        apply(&state, claim_redeem(ALICE, 1)),
        Err(Rejection::ClaimAlreadyConsumed)
    );
}

#[test]
fn claims_work_while_paused() {
    let state = run(&genesis(), &[deposit(ALICE, 9_000_000)]);
    let state = settle_epoch(&state, EPOCH_DURATION);
    let state = run(&state, &[Operation::Pause { actor: GUARDIAN }]);
    assert_eq!(state.vault_state, VaultState::Paused);
    let state = run(&state, &[claim_deposit(ALICE, 0)]);
    assert!(!state.account(ALICE).shares.is_zero());
}

#[test]
fn claims_work_while_frozen() {
    let state = run(&genesis(), &[deposit(ALICE, 9_000_000)]);
    let state = settle_epoch(&state, EPOCH_DURATION);
    let state = run(&state, &[Operation::Freeze { actor: GUARDIAN }]);
    assert_eq!(state.vault_state, VaultState::Frozen);
    let state = run(&state, &[claim_deposit(ALICE, 0)]);
    assert!(!state.account(ALICE).shares.is_zero());
}

// Pricing

#[test]
fn redemption_liquidity_always_covers_the_obligation_under_the_public_api() {
    let state = funded_with_shares(ALICE, 10_000_000);
    let held = state.account(ALICE).shares.raw();
    let settled = settle_epoch(
        &run(&state, &[redeem(ALICE, held)]),
        EPOCH_DURATION.saturating_mul(2),
    );
    let terms = settled.finalized_terms(EpochId::new(1)).expect("finalized");
    assert!(terms.redeem_assets <= terms.total_assets);
    assert!(terms.redeem_assets <= settled.initial_asset_supply);
}

#[test]
fn mixed_settlement_never_owes_more_than_the_backing() {
    let mut state = settle_epoch(&everyone_deposits(&genesis(), 0), EPOCH_DURATION);
    for round in 1..=4u64 {
        state = claim_round(&state, round.saturating_sub(1));
        state = run(&state, &[open_next(EPOCH_DURATION.saturating_mul(round))]);

        let alice = state.account(ALICE).shares.raw();
        let bob = state.account(BOB).shares.raw();
        state = run(&state, &[redeem(ALICE, alice / 3), redeem(BOB, bob / 7)]);
        state = everyone_deposits(&state, round);
        state = settle_epoch(&state, EPOCH_DURATION.saturating_mul(round + 1));

        let terms = state.finalized_terms(EpochId::new(round)).expect("terms");
        assert!(terms.redeem_assets <= terms.total_assets);
    }
}

#[test]
fn same_epoch_deposits_cannot_fund_same_epoch_redemptions() {
    let state = funded_with_shares(ALICE, 10_000_000);
    let held = state.account(ALICE).shares.raw();
    let state = run(&state, &[redeem(ALICE, held), deposit(BOB, 50_000_000)]);
    let state = settle_epoch(&state, EPOCH_DURATION.saturating_mul(2));

    let terms = state.finalized_terms(EpochId::new(1)).expect("finalized");
    // Only the pre-existing backing priced and funded the redemption.
    assert_eq!(terms.total_assets, AssetAmount::new(10_000_000));
    assert_eq!(terms.redeem_assets, AssetAmount::new(10_000_000));
    assert_eq!(state.buckets.claim_reserve, AssetAmount::new(10_000_000));
    assert_eq!(state.managed_nav(), AssetAmount::new(50_000_000));
}

/// Every user deposits a slightly different odd amount.
fn everyone_deposits(state: &State, round: u64) -> State {
    let bump = u128::from(round).saturating_mul(1_013);
    run(
        state,
        &[
            deposit(ALICE, 7_000_003 + bump),
            deposit(BOB, 13_000_009 + bump),
            deposit(CAROL, 1_000_007 + bump),
        ],
    )
}

fn claim_round(state: &State, epoch: u64) -> State {
    run(
        state,
        &[
            claim_deposit(ALICE, epoch),
            claim_deposit(BOB, epoch),
            claim_deposit(CAROL, epoch),
        ],
    )
}

#[test]
fn the_price_never_falls_across_settlements() {
    let mut state = settle_epoch(&everyone_deposits(&genesis(), 0), EPOCH_DURATION);
    let mut previous = (0u128, 1u128);
    for round in 1..=5u64 {
        state = claim_round(&state, round.saturating_sub(1));
        state = run(&state, &[open_next(EPOCH_DURATION.saturating_mul(round))]);

        let alice = state.account(ALICE).shares.raw();
        state = run(&state, &[redeem(ALICE, alice / 5 + 1)]);
        state = everyone_deposits(&state, round);
        state = settle_epoch(&state, EPOCH_DURATION.saturating_mul(round + 1));

        let terms = state.finalized_terms(EpochId::new(round)).expect("terms");
        let current = (
            terms.total_assets.raw().saturating_add(1),
            terms
                .total_supply
                .raw()
                .saturating_add(terms.virtual_shares.raw()),
        );
        // Cross multiply so the comparison needs no division.
        assert!(
            previous.0.saturating_mul(current.1) <= current.0.saturating_mul(previous.1),
            "price fell at epoch {round}"
        );
        previous = current;
    }
}

// Dust

/// Reserve and claimable shares recomputed straight from the request records.
fn recompute_backing(state: &State) -> (u128, u128) {
    let mut shares = 0u128;
    let mut assets = 0u128;
    for (id, outcome) in &state.epochs {
        let EpochOutcome::Finalized(terms) = outcome else {
            continue;
        };
        let owed = state.entitlements(*id, terms).expect("entitlements");
        shares = shares
            .saturating_add(owed.unclaimed_deposit_shares.raw())
            .saturating_add(terms.deposit_dust.raw());
        assets = assets
            .saturating_add(owed.unclaimed_redeem_assets.raw())
            .saturating_add(terms.redeem_dust.raw());
    }
    (shares, assets)
}

/// Settles a mixed epoch that leaves a non zero rounding remainder.
fn state_with_dust() -> State {
    let state = run(&genesis(), &[deposit(ALICE, 10_000_000)]);
    let state = settle_epoch(&state, EPOCH_DURATION);
    let state = run(
        &state,
        &[claim_deposit(ALICE, 0), open_next(EPOCH_DURATION)],
    );

    // An odd redemption makes the price ratio stop being a whole number.
    let state = run(&state, &[redeem(ALICE, 3_000_000_000_001)]);
    let state = settle_epoch(&state, EPOCH_DURATION.saturating_mul(2));
    let state = run(
        &state,
        &[
            claim_redeem(ALICE, 1),
            open_next(EPOCH_DURATION.saturating_mul(2)),
        ],
    );

    let state = run(
        &state,
        &[
            deposit(ALICE, 1_000_003),
            deposit(BOB, 2_000_007),
            deposit(CAROL, 3_000_011),
        ],
    );
    settle_epoch(&state, EPOCH_DURATION.saturating_mul(3))
}

#[test]
fn a_single_claimant_leaves_no_dust() {
    let state = run(&genesis(), &[deposit(ALICE, 3_141_593)]);
    let state = settle_epoch(&state, EPOCH_DURATION);
    let terms = state.finalized_terms(EpochId::GENESIS).expect("finalized");
    assert_eq!(terms.deposit_dust, ShareAmount::ZERO);
    assert_eq!(terms.redeem_dust, AssetAmount::ZERO);
}

#[test]
fn many_claimants_leave_measured_dust() {
    let state = state_with_dust();
    let terms = state.finalized_terms(EpochId::new(2)).expect("finalized");
    assert!(
        !terms.deposit_dust.is_zero(),
        "expected a rounding remainder"
    );
    assert!(
        terms.deposit_dust.raw() <= 2,
        "dust exceeds the claim count"
    );
}

#[test]
fn the_claimable_share_identity_holds_before_any_claim() {
    let state = state_with_dust();
    let (shares, assets) = recompute_backing(&state);
    assert_eq!(shares, state.claimable_deposit_shares.raw());
    assert_eq!(assets, state.buckets.claim_reserve.raw());
}

#[test]
fn the_identity_holds_after_a_partial_set_of_claims() {
    let state = state_with_dust();
    let state = run(&state, &[claim_deposit(BOB, 2)]);
    let (shares, assets) = recompute_backing(&state);
    assert_eq!(shares, state.claimable_deposit_shares.raw());
    assert_eq!(assets, state.buckets.claim_reserve.raw());
}

#[test]
fn dust_stays_measurable_after_every_user_claims() {
    let state = state_with_dust();
    let dust = state
        .finalized_terms(EpochId::new(2))
        .expect("finalized")
        .deposit_dust;
    let state = run(
        &state,
        &[
            claim_deposit(ALICE, 2),
            claim_deposit(BOB, 2),
            claim_deposit(CAROL, 2),
        ],
    );
    assert_eq!(state.outstanding_deposit_shares(), Ok(ShareAmount::ZERO));
    assert_eq!(state.claimable_deposit_shares, dust);
}

#[test]
fn claim_order_does_not_change_the_measured_dust() {
    let base = state_with_dust();
    let forward = run(
        &base,
        &[
            claim_deposit(ALICE, 2),
            claim_deposit(BOB, 2),
            claim_deposit(CAROL, 2),
        ],
    );
    let reverse = run(
        &base,
        &[
            claim_deposit(CAROL, 2),
            claim_deposit(BOB, 2),
            claim_deposit(ALICE, 2),
        ],
    );
    assert_eq!(forward, reverse);
}

#[test]
fn repeated_epochs_accumulate_only_measured_dust() {
    let mut state = state_with_dust();
    for round in 3..=7u64 {
        state = claim_round(&state, round.saturating_sub(1));
        state = run(&state, &[open_next(EPOCH_DURATION.saturating_mul(round))]);

        let alice = state.account(ALICE).shares.raw();
        state = run(&state, &[redeem(ALICE, alice / 3 + 7)]);
        state = everyone_deposits(&state, round);
        state = settle_epoch(&state, EPOCH_DURATION.saturating_mul(round + 1));
        state = run(&state, &[claim_redeem(ALICE, round)]);

        let (shares, assets) = recompute_backing(&state);
        assert_eq!(shares, state.claimable_deposit_shares.raw());
        assert_eq!(assets, state.buckets.claim_reserve.raw());
    }

    let recorded: u128 = state
        .epochs
        .values()
        .filter_map(EpochOutcome::finalized)
        .map(|terms| {
            terms
                .deposit_dust
                .raw()
                .saturating_add(terms.redeem_dust.raw())
        })
        .sum();
    assert!(recorded > 0, "the run produced no rounding remainder");
}

// Tampering

fn finalized_mut(state: &mut State, epoch: u64) -> &mut accounting_model::EpochTerms {
    match state
        .epochs
        .get_mut(&EpochId::new(epoch))
        .expect("epoch has an outcome")
    {
        EpochOutcome::Finalized(terms) => terms,
        EpochOutcome::Aborted(_) => panic!("epoch was aborted"),
    }
}

#[test]
fn moving_a_unit_into_the_claim_reserve_is_detected() {
    let mut state = state_with_dust();
    state.buckets.claim_reserve = AssetAmount::new(state.buckets.claim_reserve.raw() + 1);
    state.buckets.idle_backing = AssetAmount::new(state.buckets.idle_backing.raw() - 1);
    assert_eq!(
        check_invariants(&state),
        Err(Violation::ClaimReserveNotExplained)
    );
}

#[test]
fn inflating_claimable_shares_is_detected() {
    let mut state = state_with_dust();
    state.claimable_deposit_shares = ShareAmount::new(state.claimable_deposit_shares.raw() + 1);
    state.total_share_supply = ShareAmount::new(state.total_share_supply.raw() + 1);
    assert_eq!(
        check_invariants(&state),
        Err(Violation::ClaimableSharesNotExplained)
    );
}

#[test]
fn rewriting_recorded_dust_is_detected() {
    let mut state = state_with_dust();
    let terms = finalized_mut(&mut state, 2);
    terms.deposit_dust = ShareAmount::new(terms.deposit_dust.raw() + 1);
    assert_eq!(check_invariants(&state), Err(Violation::DustOutOfBounds));
}

#[test]
fn rewriting_a_finalized_request_amount_is_detected() {
    let mut state = state_with_dust();
    let key = RequestKey::new(EpochId::new(2), BOB);
    let request = state.deposit_requests.get_mut(&key).expect("request");
    request.assets = AssetAmount::new(request.assets.raw() + 1_000);
    assert!(check_invariants(&state).is_err());
}

#[test]
fn rewriting_a_finalized_aggregate_total_is_detected() {
    let mut state = state_with_dust();
    let terms = finalized_mut(&mut state, 2);
    terms.deposit_assets = AssetAmount::new(terms.deposit_assets.raw() + 1);
    assert_eq!(
        check_invariants(&state),
        Err(Violation::EpochTermsInconsistent)
    );
}

#[test]
fn rewriting_a_finalized_price_is_detected() {
    let mut state = state_with_dust();
    let terms = finalized_mut(&mut state, 2);
    terms.total_assets = AssetAmount::new(terms.total_assets.raw() + 5);
    assert!(check_invariants(&state).is_err());
}

#[test]
fn marking_a_claim_consumed_without_moving_value_is_detected() {
    let mut state = state_with_dust();
    let key = RequestKey::new(EpochId::new(2), BOB);
    state
        .deposit_requests
        .get_mut(&key)
        .expect("request")
        .claimed = true;
    assert_eq!(
        check_invariants(&state),
        Err(Violation::ClaimableSharesNotExplained)
    );
}

#[test]
fn hiding_a_settled_request_behind_the_cancelled_flag_is_detected() {
    let mut state = state_with_dust();
    let key = RequestKey::new(EpochId::new(2), BOB);
    let request = state.deposit_requests.get_mut(&key).expect("request");
    request.cancelled = true;
    request.assets = AssetAmount::ZERO;
    assert!(check_invariants(&state).is_err());
}

#[test]
fn a_request_that_is_both_cancelled_and_claimed_is_detected() {
    let mut state = state_with_dust();
    let key = RequestKey::new(EpochId::new(2), BOB);
    let request = state.deposit_requests.get_mut(&key).expect("request");
    request.cancelled = true;
    request.claimed = true;
    assert_eq!(
        check_invariants(&state),
        Err(Violation::RequestLifecycleInvalid)
    );
}

#[test]
fn a_missing_asset_is_detected() {
    let mut state = state_with_dust();
    state.buckets.idle_backing = AssetAmount::new(state.buckets.idle_backing.raw() - 1);
    assert_eq!(check_invariants(&state), Err(Violation::AssetConservation));
}

#[test]
fn an_inflated_share_supply_is_detected() {
    let mut state = state_with_dust();
    state.total_share_supply = ShareAmount::new(state.total_share_supply.raw() + 1);
    assert_eq!(check_invariants(&state), Err(Violation::ShareConservation));
}

#[test]
fn a_wrong_burned_share_total_is_detected() {
    let mut state = state_with_dust();
    state.burned_redemption_shares = ShareAmount::new(state.burned_redemption_shares.raw() + 1);
    assert_eq!(
        check_invariants(&state),
        Err(Violation::BurnedSharesMismatch)
    );
}

#[test]
fn a_settled_epoch_left_in_the_current_slot_is_detected() {
    let mut state = run(&genesis(), &[deposit(ALICE, 10_000_000)]);
    state = settle_epoch(&state, EPOCH_DURATION);
    state = run(&state, &[open_next(EPOCH_DURATION)]);
    if let Some(epoch) = state.epoch.as_mut() {
        epoch.id = EpochId::GENESIS;
    }
    assert_eq!(
        check_invariants(&state),
        Err(Violation::SettledEpochStillCurrent)
    );
}

#[test]
fn a_current_epoch_aggregate_that_drifts_is_detected() {
    let mut state = run(&genesis(), &[deposit(ALICE, 10_000_000)]);
    if let Some(epoch) = state.epoch.as_mut() {
        epoch.pending_deposit_assets = AssetAmount::new(epoch.pending_deposit_assets.raw() + 1);
    }
    assert_eq!(
        check_invariants(&state),
        Err(Violation::EpochAggregateMismatch)
    );
}

#[test]
fn a_finalized_epoch_stored_under_the_wrong_id_is_detected() {
    let mut state = state_with_dust();
    finalized_mut(&mut state, 2).id = EpochId::new(9);
    assert_eq!(
        check_invariants(&state),
        Err(Violation::EpochTermsInconsistent)
    );
}

#[test]
fn a_finalized_redemption_total_that_drifts_is_detected() {
    let mut state = state_with_dust();
    let shift = 1_000_000_000_000u128;
    let terms = finalized_mut(&mut state, 1);
    terms.redeem_shares = ShareAmount::new(terms.redeem_shares.raw() + shift);
    state.burned_redemption_shares = ShareAmount::new(state.burned_redemption_shares.raw() + shift);
    assert_eq!(
        check_invariants(&state),
        Err(Violation::EpochTermsInconsistent)
    );
}

#[test]
fn an_extra_request_slipped_into_a_finalized_epoch_is_detected() {
    let mut state = state_with_dust();
    // One share prices to nothing, so only the aggregate check can see it.
    state.redeem_requests.insert(
        RequestKey::new(EpochId::new(1), CAROL),
        RedeemRequest {
            shares: ShareAmount::new(1),
            ..RedeemRequest::default()
        },
    );
    assert_eq!(
        check_invariants(&state),
        Err(Violation::EpochAggregateMismatch)
    );
}

#[test]
fn a_price_that_falls_between_epochs_is_detected() {
    let state = run(&genesis(), &[deposit(ALICE, 10_000_000)]);
    let state = settle_epoch(&state, EPOCH_DURATION);
    let state = run(&state, &[open_next(EPOCH_DURATION)]);
    let mut state = settle_epoch(&state, EPOCH_DURATION.saturating_mul(2));
    assert_eq!(check_invariants(&state), Ok(()));

    // The empty second epoch carries no requests, so only its price changes.
    let terms = finalized_mut(&mut state, 1);
    terms.total_assets = AssetAmount::new(terms.total_assets.raw() - 5);
    assert_eq!(check_invariants(&state), Err(Violation::PriceDecreased));
}

#[test]
fn a_cancelled_request_that_still_holds_an_amount_is_detected() {
    let mut state = run(&genesis(), &[deposit(ALICE, 5_000_000)]);
    let key = RequestKey::new(EpochId::GENESIS, ALICE);
    state
        .deposit_requests
        .get_mut(&key)
        .expect("request")
        .cancelled = true;
    assert_eq!(
        check_invariants(&state),
        Err(Violation::RequestLifecycleInvalid)
    );
}

#[test]
fn a_cancelled_redemption_that_still_holds_shares_is_detected() {
    let state = funded_with_shares(ALICE, 10_000_000);
    let held = state.account(ALICE).shares.raw();
    let mut state = run(&state, &[redeem(ALICE, held)]);
    let key = RequestKey::new(EpochId::new(1), ALICE);
    state
        .redeem_requests
        .get_mut(&key)
        .expect("request")
        .cancelled = true;
    assert_eq!(
        check_invariants(&state),
        Err(Violation::RequestLifecycleInvalid)
    );
}

// Emergency states

#[test]
fn requests_fail_while_paused() {
    let state = run(&genesis(), &[Operation::Pause { actor: ADMIN }]);
    assert_eq!(
        apply(&state, deposit(ALICE, 5_000_000)),
        Err(Rejection::InvalidVaultState)
    );
    assert_eq!(
        apply(&state, redeem(ALICE, MIN_REDEEM)),
        Err(Rejection::InvalidVaultState)
    );
}

#[test]
fn requests_fail_while_frozen() {
    let state = run(&genesis(), &[Operation::Freeze { actor: ADMIN }]);
    assert_eq!(
        apply(&state, deposit(ALICE, 5_000_000)),
        Err(Rejection::InvalidVaultState)
    );
    assert_eq!(
        apply(&state, redeem(ALICE, MIN_REDEEM)),
        Err(Rejection::InvalidVaultState)
    );
}

#[test]
fn cancellation_fails_while_frozen_but_works_while_paused() {
    let state = run(&genesis(), &[deposit(ALICE, 5_000_000)]);
    let frozen = run(&state, &[Operation::Freeze { actor: ADMIN }]);
    assert_eq!(
        apply(&frozen, Operation::CancelDeposit { account: ALICE }),
        Err(Rejection::InvalidVaultState)
    );
    let paused = run(&state, &[Operation::Pause { actor: ADMIN }]);
    let cancelled = run(&paused, &[Operation::CancelDeposit { account: ALICE }]);
    assert_eq!(cancelled.account(ALICE).assets, AssetAmount::new(FUNDING));
}

#[test]
fn cutoff_fails_while_frozen() {
    let state = run(&genesis(), &[Operation::Freeze { actor: ADMIN }]);
    assert_eq!(
        apply(&state, cutoff(EPOCH_DURATION)),
        Err(Rejection::InvalidVaultState)
    );
}

#[test]
fn finalization_fails_while_frozen() {
    let state = run(&genesis(), &[cutoff(EPOCH_DURATION)]);
    let state = run(&state, &[Operation::Freeze { actor: ADMIN }]);
    assert_eq!(
        apply(&state, Operation::FinalizeEpoch),
        Err(Rejection::InvalidVaultState)
    );
}

#[test]
fn only_the_admin_can_unpause() {
    let state = run(&genesis(), &[Operation::Pause { actor: GUARDIAN }]);
    assert_eq!(
        apply(&state, Operation::Unpause { actor: GUARDIAN }),
        Err(Rejection::UnauthorizedActor)
    );
    let state = run(&state, &[Operation::Unpause { actor: ADMIN }]);
    assert_eq!(state.vault_state, VaultState::Active);
}

#[test]
fn a_frozen_vault_cannot_be_unpaused() {
    let state = run(&genesis(), &[Operation::Freeze { actor: ADMIN }]);
    assert_eq!(
        apply(&state, Operation::Unpause { actor: ADMIN }),
        Err(Rejection::InvalidVaultState)
    );
}

// Admission rules

#[test]
fn cutoff_before_the_fixed_timestamp_is_rejected() {
    assert_eq!(
        apply(&genesis(), cutoff(EPOCH_DURATION.saturating_sub(1))),
        Err(Rejection::CutoffNotReached)
    );
}

#[test]
fn zero_value_requests_are_rejected() {
    let state = genesis();
    assert_eq!(apply(&state, deposit(ALICE, 0)), Err(Rejection::ZeroAmount));
    assert_eq!(apply(&state, redeem(ALICE, 0)), Err(Rejection::ZeroAmount));
}

#[test]
fn minimum_deposit_is_enforced() {
    let state = genesis();
    assert_eq!(
        apply(&state, deposit(ALICE, MIN_DEPOSIT.saturating_sub(1))),
        Err(Rejection::AmountBelowMinimum)
    );
    assert!(apply(&state, deposit(ALICE, MIN_DEPOSIT)).is_ok());
}

#[test]
fn minimum_redemption_is_enforced() {
    let state = funded_with_shares(ALICE, 10_000_000);
    assert_eq!(
        apply(&state, redeem(ALICE, MIN_REDEEM.saturating_sub(1))),
        Err(Rejection::AmountBelowMinimum)
    );
    assert!(apply(&state, redeem(ALICE, MIN_REDEEM)).is_ok());
}

#[test]
fn a_deposit_larger_than_the_balance_is_rejected() {
    assert_eq!(
        apply(&genesis(), deposit(ALICE, FUNDING.saturating_add(1))),
        Err(Rejection::InsufficientAssetBalance)
    );
}

#[test]
fn a_redemption_larger_than_the_share_balance_is_rejected() {
    assert_eq!(
        apply(&genesis(), redeem(ALICE, MIN_REDEEM)),
        Err(Rejection::InsufficientShareBalance)
    );
}

// Genesis

#[test]
fn duplicate_account_ids_with_equal_balances_are_rejected() {
    let outcome = State::new(Genesis {
        config: config(),
        authority: authority(),
        accounts: vec![
            (ALICE, AssetAmount::new(100)),
            (BOB, AssetAmount::new(50)),
            (ALICE, AssetAmount::new(100)),
        ],
        unattributed_balance: AssetAmount::ZERO,
        opened_at: Timestamp::new(0),
    });
    assert_eq!(outcome.err(), Some(Rejection::DuplicateAccount));
}

#[test]
fn duplicate_account_ids_with_different_balances_are_rejected() {
    let outcome = State::new(Genesis {
        config: config(),
        authority: authority(),
        accounts: vec![(ALICE, AssetAmount::new(100)), (ALICE, AssetAmount::new(7))],
        unattributed_balance: AssetAmount::ZERO,
        opened_at: Timestamp::new(0),
    });
    assert_eq!(outcome.err(), Some(Rejection::DuplicateAccount));
}

#[test]
fn unique_account_ids_are_accepted_and_conserve_the_initial_supply() {
    let state = State::new(Genesis {
        config: config(),
        authority: authority(),
        accounts: vec![
            (ALICE, AssetAmount::new(100)),
            (BOB, AssetAmount::new(50)),
            (CAROL, AssetAmount::new(1)),
        ],
        unattributed_balance: AssetAmount::new(9),
        opened_at: Timestamp::new(0),
    })
    .expect("valid genesis");
    assert_eq!(state.initial_asset_supply, AssetAmount::new(160));
    assert_eq!(check_invariants(&state), Ok(()));
}

#[test]
fn a_request_reports_each_lifecycle_state_in_turn() {
    let key = RequestKey::new(EpochId::GENESIS, ALICE);
    let state = run(&genesis(), &[deposit(ALICE, 5_000_000)]);
    assert_eq!(
        state.deposit_request_state(key),
        Some(RequestState::Pending)
    );

    let state = run(&state, &[cutoff(EPOCH_DURATION)]);
    assert_eq!(state.deposit_request_state(key), Some(RequestState::Locked));

    let state = run(&state, &[Operation::FinalizeEpoch]);
    assert_eq!(
        state.deposit_request_state(key),
        Some(RequestState::Claimable)
    );

    let state = run(&state, &[claim_deposit(ALICE, 0)]);
    assert_eq!(
        state.deposit_request_state(key),
        Some(RequestState::Claimed)
    );
    assert_eq!(
        state.deposit_request_state(RequestKey::new(EpochId::GENESIS, BOB)),
        None
    );
}

#[test]
fn a_zero_epoch_duration_is_rejected_at_genesis() {
    let mut invalid = config();
    invalid.epoch_duration = 0;
    let outcome = State::new(Genesis {
        config: invalid,
        authority: authority(),
        accounts: Vec::new(),
        unattributed_balance: AssetAmount::ZERO,
        opened_at: Timestamp::new(0),
    });
    assert_eq!(outcome.err(), Some(Rejection::InvalidConfiguration));
}

#[test]
fn a_zero_minimum_request_size_is_rejected_at_genesis() {
    let mut invalid = config();
    invalid.min_deposit_assets = AssetAmount::ZERO;
    let outcome = State::new(Genesis {
        config: invalid,
        authority: authority(),
        accounts: Vec::new(),
        unattributed_balance: AssetAmount::ZERO,
        opened_at: Timestamp::new(0),
    });
    assert_eq!(outcome.err(), Some(Rejection::InvalidConfiguration));
}

#[test]
fn invalid_configuration_is_rejected_at_genesis() {
    let mut invalid = config();
    invalid.share_decimals = 4;
    let outcome = State::new(Genesis {
        config: invalid,
        authority: authority(),
        accounts: Vec::new(),
        unattributed_balance: AssetAmount::ZERO,
        opened_at: Timestamp::new(0),
    });
    assert_eq!(outcome.err(), Some(Rejection::InvalidConfiguration));
}

// Arithmetic and atomicity

#[test]
fn arithmetic_at_the_upper_bound_is_rejected_without_panicking() {
    let state = genesis_funding_only(ALICE, u128::MAX);
    let state = run(&state, &[deposit(ALICE, u128::MAX), cutoff(EPOCH_DURATION)]);
    // Minting against the whole asset range overflows the share side and is refused.
    assert_eq!(
        apply(&state, Operation::FinalizeEpoch),
        Err(Rejection::ArithmeticOverflow)
    );
}

#[test]
fn a_share_balance_at_the_upper_bound_does_not_panic() {
    let mut state = genesis();
    state.accounts.insert(
        ALICE,
        Account {
            assets: AssetAmount::new(FUNDING),
            shares: ShareAmount::new(u128::MAX),
        },
    );
    state.total_share_supply = ShareAmount::new(u128::MAX);
    assert_eq!(check_invariants(&state), Ok(()));
    assert!(apply(&state, redeem(ALICE, u128::MAX)).is_ok());
}

#[test]
fn failed_transitions_leave_byte_for_byte_equal_state() {
    let state = run(&genesis(), &[deposit(ALICE, 5_000_000)]);
    let rejected = [
        deposit(ALICE, 0),
        deposit(BOB, FUNDING.saturating_add(1)),
        redeem(CAROL, MIN_REDEEM),
        Operation::CancelDeposit { account: BOB },
        Operation::CancelRedeem { account: ALICE },
        Operation::FinalizeEpoch,
        Operation::AbortEpoch { actor: ADMIN },
        open_next(EPOCH_DURATION),
        cutoff(0),
        Operation::Unpause { actor: ADMIN },
        Operation::Pause { actor: ALICE },
        claim_deposit(ALICE, 0),
        Operation::ClaimAbortedDeposit {
            account: ALICE,
            epoch: EpochId::GENESIS,
        },
    ];
    for operation in rejected {
        let before = state.clone();
        assert!(apply(&state, operation).is_err(), "{operation:?}");
        assert_eq!(before, state, "{operation:?}");
    }
}

// Ordering

#[test]
fn claim_order_produces_the_same_final_state() {
    let base = run(
        &genesis(),
        &[
            deposit(ALICE, 3_333_333),
            deposit(BOB, 7_777_777),
            deposit(CAROL, 1_111_111),
        ],
    );
    let base = settle_epoch(&base, EPOCH_DURATION);
    let forward = run(
        &base,
        &[
            claim_deposit(ALICE, 0),
            claim_deposit(BOB, 0),
            claim_deposit(CAROL, 0),
        ],
    );
    let reverse = run(
        &base,
        &[
            claim_deposit(CAROL, 0),
            claim_deposit(BOB, 0),
            claim_deposit(ALICE, 0),
        ],
    );
    assert_eq!(forward, reverse);
}

#[test]
fn request_insertion_order_produces_the_same_epoch_result() {
    let forward = run(
        &genesis(),
        &[
            deposit(ALICE, 3_333_333),
            deposit(BOB, 7_777_777),
            deposit(CAROL, 1_111_111),
        ],
    );
    let reverse = run(
        &genesis(),
        &[
            deposit(CAROL, 1_111_111),
            deposit(BOB, 7_777_777),
            deposit(ALICE, 3_333_333),
        ],
    );
    assert_eq!(
        settle_epoch(&forward, EPOCH_DURATION),
        settle_epoch(&reverse, EPOCH_DURATION)
    );
}

#[test]
fn finalized_claim_amounts_never_change() {
    let state = run(
        &genesis(),
        &[deposit(ALICE, 10_000_000), deposit(BOB, 10_000_000)],
    );
    let state = settle_epoch(&state, EPOCH_DURATION);
    let terms = *state
        .finalized_terms(EpochId::GENESIS)
        .expect("epoch is finalized");
    let entitlement = terms
        .shares_for(AssetAmount::new(10_000_000))
        .expect("conversion");

    let state = run(
        &state,
        &[
            claim_deposit(ALICE, 0),
            open_next(EPOCH_DURATION),
            deposit(CAROL, 900_000_000),
        ],
    );
    let state = settle_epoch(&state, EPOCH_DURATION.saturating_mul(2));

    let after = *state
        .finalized_terms(EpochId::GENESIS)
        .expect("epoch is finalized");
    assert_eq!(terms, after);
    let state = run(&state, &[claim_deposit(BOB, 0)]);
    assert_eq!(state.account(BOB).shares, entitlement);
}

// Epoch slot

#[test]
fn only_one_epoch_may_be_unsettled_at_a_time() {
    assert_eq!(
        apply(&genesis(), open_next(EPOCH_DURATION)),
        Err(Rejection::EpochAlreadyOpen)
    );
}

#[test]
fn finalizing_again_is_rejected_because_no_epoch_remains_open() {
    let state = settle_epoch(&genesis(), EPOCH_DURATION);
    assert_eq!(
        apply(&state, Operation::FinalizeEpoch),
        Err(Rejection::EpochNotOpen)
    );
    assert!(state.finalized_terms(EpochId::GENESIS).is_some());
}

#[test]
fn the_unattributed_balance_stays_outside_nav() {
    let state = run(&genesis(), &[deposit(ALICE, 10_000_000)]);
    let state = settle_epoch(&state, EPOCH_DURATION);
    assert_eq!(state.buckets.unattributed_balance, AssetAmount::new(500));
    assert_eq!(state.managed_nav(), AssetAmount::new(10_000_000));
}
