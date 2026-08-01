#![allow(clippy::expect_used, clippy::panic)]

use accounting_model::{
    AccountId, AssetAmount, Authority, Config, ConfigVersion, DepositRequest, EpochId,
    EpochOutcome, Genesis, Operation, Rejection, RequestKey, ShareAmount, State, Timestamp,
    VaultState, apply, check_invariants,
};
use proptest::prelude::*;
use ruint::aliases::U512;

const ADMIN: AccountId = AccountId::new(1);
const GUARDIAN: AccountId = AccountId::new(2);
const USER_BASE: u64 = 10;
const USER_COUNT: u64 = 4;

const EPOCH_DURATION: u64 = 3_600;
const MIN_DEPOSIT: u128 = 1_000_000;
const MIN_REDEEM: u128 = 1_000_000_000_000;
const FUNDING: u128 = 1_000_000_000_000_000;

fn user(index: u8) -> AccountId {
    AccountId::new(USER_BASE.saturating_add(u64::from(index) % USER_COUNT))
}

fn genesis() -> State {
    let accounts = (0..USER_COUNT)
        .map(|offset| {
            (
                AccountId::new(USER_BASE.saturating_add(offset)),
                AssetAmount::new(FUNDING),
            )
        })
        .collect();
    State::new(Genesis {
        config: Config {
            version: ConfigVersion::new(1),
            asset_decimals: 6,
            share_decimals: 18,
            min_deposit_assets: AssetAmount::new(MIN_DEPOSIT),
            min_redeem_shares: ShareAmount::new(MIN_REDEEM),
            epoch_duration: EPOCH_DURATION,
        },
        authority: Authority {
            admin: ADMIN,
            guardian: GUARDIAN,
        },
        accounts,
        unattributed_balance: AssetAmount::new(7),
        opened_at: Timestamp::new(0),
    })
    .expect("valid genesis")
}

// Stepping

fn consumes_a_claim(operation: Operation) -> bool {
    matches!(
        operation,
        Operation::ClaimDeposit { .. }
            | Operation::ClaimRedeem { .. }
            | Operation::ClaimAbortedDeposit { .. }
            | Operation::ClaimAbortedRedeem { .. }
    )
}

/// Applies one operation and checks atomicity, invariants and the exact identities.
fn step(state: &State, operation: Operation) -> Result<State, TestCaseError> {
    let before = state.clone();
    match apply(state, operation) {
        Ok(next) => {
            prop_assert_eq!(&before, state, "successful apply mutated its input");
            if let Err(violation) = check_invariants(&next) {
                return Err(TestCaseError::fail(format!(
                    "{violation} after {operation:?}"
                )));
            }
            assert_exact_backing(&next)?;
            assert_price_never_falls(&next)?;
            if consumes_a_claim(operation) {
                prop_assert!(
                    apply(&next, operation).is_err(),
                    "a claim was consumable twice"
                );
            }
            Ok(next)
        }
        Err(_) => {
            prop_assert_eq!(&before, state, "rejected apply mutated its input");
            Ok(before)
        }
    }
}

// Independent re-derivation of the accounting identities

/// Recomputes reserve and claimable shares from the request records alone.
fn assert_exact_backing(state: &State) -> Result<(), TestCaseError> {
    let mut owed_shares = 0u128;
    let mut owed_assets = 0u128;
    let mut escrow_assets = 0u128;
    let mut escrow_shares = 0u128;

    if let Some(epoch) = state.epoch {
        let totals = state.request_totals(epoch.id).expect("totals");
        escrow_assets = totals.deposit_assets.raw();
        escrow_shares = totals.redeem_shares.raw();
    }

    for (id, outcome) in &state.epochs {
        match outcome {
            EpochOutcome::Finalized(terms) => {
                let owed = state.entitlements(*id, terms).expect("entitlements");
                let totals = state.request_totals(*id).expect("totals");

                let deposit_dust = terms
                    .minted_shares
                    .raw()
                    .checked_sub(owed.deposit_shares.raw());
                let redeem_dust = terms
                    .redeem_assets
                    .raw()
                    .checked_sub(owed.redeem_assets.raw());
                prop_assert_eq!(deposit_dust, Some(terms.deposit_dust.raw()));
                prop_assert_eq!(redeem_dust, Some(terms.redeem_dust.raw()));
                prop_assert!(terms.deposit_dust.raw() <= totals.deposit_count.saturating_sub(1));
                prop_assert!(terms.redeem_dust.raw() <= totals.redeem_count.saturating_sub(1));

                owed_shares = owed_shares
                    .saturating_add(owed.unclaimed_deposit_shares.raw())
                    .saturating_add(terms.deposit_dust.raw());
                owed_assets = owed_assets
                    .saturating_add(owed.unclaimed_redeem_assets.raw())
                    .saturating_add(terms.redeem_dust.raw());
            }
            EpochOutcome::Aborted(aborted) => {
                let totals = state.request_totals(*id).expect("totals");
                prop_assert_eq!(totals.deposit_assets, aborted.refund_assets);
                prop_assert_eq!(totals.redeem_shares, aborted.refund_shares);
                escrow_assets = escrow_assets.saturating_add(totals.unclaimed_deposit_assets.raw());
                escrow_shares = escrow_shares.saturating_add(totals.unclaimed_redeem_shares.raw());
            }
        }
    }

    prop_assert_eq!(owed_shares, state.claimable_deposit_shares.raw());
    prop_assert_eq!(owed_assets, state.buckets.claim_reserve.raw());
    prop_assert_eq!(escrow_assets, state.buckets.pending_deposit_escrow.raw());
    prop_assert_eq!(escrow_shares, state.escrowed_redeem_shares.raw());
    Ok(())
}

/// Cross multiplies the price of consecutive finalized epochs, with no division.
fn assert_price_never_falls(state: &State) -> Result<(), TestCaseError> {
    let mut earlier: Option<(U512, U512)> = None;
    for outcome in state.epochs.values() {
        let EpochOutcome::Finalized(terms) = outcome else {
            continue;
        };
        let current = (
            U512::from(terms.total_assets.raw()) + U512::from(terms.virtual_assets.raw()),
            U512::from(terms.total_supply.raw()) + U512::from(terms.virtual_shares.raw()),
        );
        if let Some(previous) = earlier {
            prop_assert!(
                previous.0 * current.1 <= current.0 * previous.1,
                "settlement lowered the price at epoch {:?}",
                terms.id
            );
        }
        earlier = Some(current);
    }
    Ok(())
}

// Arbitrary sequences

/// An action is resolved against the live state to build a concrete operation.
#[derive(Clone, Copy, Debug)]
enum Action {
    Deposit { account: u8, amount: u128 },
    CancelDeposit { account: u8 },
    RedeemPortion { account: u8, quarters: u8 },
    CancelRedeem { account: u8 },
    Cutoff { early: bool },
    Finalize,
    Abort { actor: u8 },
    ClaimDeposit { account: u8, epoch: u8 },
    ClaimRedeem { account: u8, epoch: u8 },
    ClaimLatestDeposit { account: u8 },
    ClaimLatestRedeem { account: u8 },
    RefundDeposit { account: u8, epoch: u8 },
    RefundRedeem { account: u8, epoch: u8 },
    OpenNext { early: bool },
    Pause { actor: u8 },
    Unpause { actor: u8 },
    Freeze { actor: u8 },
}

fn actor(index: u8) -> AccountId {
    match index % 3 {
        0 => ADMIN,
        1 => GUARDIAN,
        _ => user(index),
    }
}

fn latest_finalized(state: &State) -> EpochId {
    state
        .epochs
        .iter()
        .filter(|(_, outcome)| outcome.finalized().is_some())
        .map(|(id, _)| *id)
        .next_back()
        .unwrap_or(EpochId::GENESIS)
}

fn resolve(state: &State, action: Action) -> Operation {
    match action {
        Action::Deposit { account, amount } => Operation::RequestDeposit {
            account: user(account),
            assets: AssetAmount::new(amount),
        },
        Action::CancelDeposit { account } => Operation::CancelDeposit {
            account: user(account),
        },
        Action::RedeemPortion { account, quarters } => Operation::RequestRedeem {
            account: user(account),
            shares: portion(state, account, quarters),
        },
        Action::CancelRedeem { account } => Operation::CancelRedeem {
            account: user(account),
        },
        Action::Cutoff { early } => {
            let due = state.epoch.map_or(0, |epoch| epoch.cutoff_at.raw());
            Operation::CutoffEpoch {
                now: Timestamp::new(if early { due.saturating_sub(1) } else { due }),
            }
        }
        Action::Finalize => Operation::FinalizeEpoch,
        Action::Abort { actor: index } => Operation::AbortEpoch {
            actor: actor(index),
        },
        Action::ClaimDeposit { account, epoch } => Operation::ClaimDeposit {
            account: user(account),
            epoch: EpochId::new(u64::from(epoch % 5)),
        },
        Action::ClaimRedeem { account, epoch } => Operation::ClaimRedeem {
            account: user(account),
            epoch: EpochId::new(u64::from(epoch % 5)),
        },
        Action::ClaimLatestDeposit { account } => Operation::ClaimDeposit {
            account: user(account),
            epoch: latest_finalized(state),
        },
        Action::ClaimLatestRedeem { account } => Operation::ClaimRedeem {
            account: user(account),
            epoch: latest_finalized(state),
        },
        Action::RefundDeposit { account, epoch } => Operation::ClaimAbortedDeposit {
            account: user(account),
            epoch: EpochId::new(u64::from(epoch % 5)),
        },
        Action::RefundRedeem { account, epoch } => Operation::ClaimAbortedRedeem {
            account: user(account),
            epoch: EpochId::new(u64::from(epoch % 5)),
        },
        Action::OpenNext { early } => {
            let due = state.last_cutoff_at.raw();
            Operation::OpenNextEpoch {
                now: Timestamp::new(if early { due.saturating_sub(1) } else { due }),
            }
        }
        Action::Pause { actor: index } => Operation::Pause {
            actor: actor(index),
        },
        Action::Unpause { actor: index } => Operation::Unpause {
            actor: actor(index),
        },
        Action::Freeze { actor: index } => Operation::Freeze {
            actor: actor(index),
        },
    }
}

fn portion(state: &State, account: u8, quarters: u8) -> ShareAmount {
    let held = state.account(user(account)).shares.raw();
    let parts = u128::from(quarters % 4).saturating_add(1);
    ShareAmount::new(held / 4 * parts)
}

fn amount_strategy() -> impl Strategy<Value = u128> {
    prop_oneof![
        1 => Just(0u128),
        1 => 1u128..MIN_DEPOSIT,
        6 => MIN_DEPOSIT..1_000_000_000u128,
        1 => Just(FUNDING),
        1 => Just(u128::MAX),
    ]
}

fn action_strategy() -> impl Strategy<Value = Action> {
    prop_oneof![
        8 => (any::<u8>(), amount_strategy()).prop_map(|(account, amount)| Action::Deposit { account, amount }),
        3 => any::<u8>().prop_map(|account| Action::CancelDeposit { account }),
        6 => (any::<u8>(), any::<u8>()).prop_map(|(account, quarters)| Action::RedeemPortion { account, quarters }),
        3 => any::<u8>().prop_map(|account| Action::CancelRedeem { account }),
        5 => any::<bool>().prop_map(|early| Action::Cutoff { early }),
        5 => Just(Action::Finalize),
        1 => any::<u8>().prop_map(|actor| Action::Abort { actor }),
        2 => (any::<u8>(), any::<u8>()).prop_map(|(account, epoch)| Action::ClaimDeposit { account, epoch }),
        2 => (any::<u8>(), any::<u8>()).prop_map(|(account, epoch)| Action::ClaimRedeem { account, epoch }),
        5 => any::<u8>().prop_map(|account| Action::ClaimLatestDeposit { account }),
        5 => any::<u8>().prop_map(|account| Action::ClaimLatestRedeem { account }),
        3 => (any::<u8>(), any::<u8>()).prop_map(|(account, epoch)| Action::RefundDeposit { account, epoch }),
        3 => (any::<u8>(), any::<u8>()).prop_map(|(account, epoch)| Action::RefundRedeem { account, epoch }),
        5 => any::<bool>().prop_map(|early| Action::OpenNext { early }),
        1 => any::<u8>().prop_map(|actor| Action::Pause { actor }),
        1 => any::<u8>().prop_map(|actor| Action::Unpause { actor }),
        1 => any::<u8>().prop_map(|actor| Action::Freeze { actor }),
    ]
}

fn drive(actions: &[Action]) -> Result<State, TestCaseError> {
    let mut state = genesis();
    prop_assert!(check_invariants(&state).is_ok());
    for action in actions {
        let operation = resolve(&state, *action);
        state = step(&state, operation)?;
    }
    Ok(state)
}

// Well formed epoch cycles

/// One complete epoch, so settlement and claim paths are reached reliably.
#[derive(Clone, Debug)]
struct Cycle {
    deposits: Vec<(u8, u128)>,
    redeems: Vec<(u8, u8)>,
    cancel_deposits: Vec<u8>,
    cancel_redeems: Vec<u8>,
    reopen_deposits: Vec<(u8, u128)>,
    pause_before_cutoff: bool,
    claimers: Vec<u8>,
    claim_every_user: bool,
}

fn cycle_strategy() -> impl Strategy<Value = Cycle> {
    (
        prop::collection::vec((0u8..4, MIN_DEPOSIT..1_000_000_000u128), 0..4),
        prop::collection::vec((0u8..4, any::<u8>()), 0..4),
        prop::collection::vec(0u8..4, 0..3),
        prop::collection::vec(0u8..4, 0..3),
        prop::collection::vec((0u8..4, MIN_DEPOSIT..1_000_000_000u128), 0..3),
        any::<bool>(),
        prop::collection::vec(0u8..4, 1..6),
        any::<bool>(),
    )
        .prop_map(
            |(
                deposits,
                redeems,
                cancel_deposits,
                cancel_redeems,
                reopen_deposits,
                pause_before_cutoff,
                claimers,
                claim_every_user,
            )| Cycle {
                deposits,
                redeems,
                cancel_deposits,
                cancel_redeems,
                reopen_deposits,
                pause_before_cutoff,
                claimers,
                claim_every_user,
            },
        )
}

fn run_cycles(cycles: &[Cycle]) -> Result<State, TestCaseError> {
    let mut state = genesis();
    prop_assert!(check_invariants(&state).is_ok());
    for cycle in cycles {
        if state.epoch.is_none() {
            let now = state.last_cutoff_at.raw();
            state = step(
                &state,
                Operation::OpenNextEpoch {
                    now: Timestamp::new(now),
                },
            )?;
        }

        for (account, amount) in &cycle.deposits {
            state = step(
                &state,
                Operation::RequestDeposit {
                    account: user(*account),
                    assets: AssetAmount::new(*amount),
                },
            )?;
        }
        for (account, quarters) in &cycle.redeems {
            let shares = portion(&state, *account, *quarters);
            state = step(
                &state,
                Operation::RequestRedeem {
                    account: user(*account),
                    shares,
                },
            )?;
        }
        for account in &cycle.cancel_deposits {
            state = step(
                &state,
                Operation::CancelDeposit {
                    account: user(*account),
                },
            )?;
        }
        for account in &cycle.cancel_redeems {
            state = step(
                &state,
                Operation::CancelRedeem {
                    account: user(*account),
                },
            )?;
        }
        // Reopening after a cancel must be allowed while the epoch is still open.
        for (account, amount) in &cycle.reopen_deposits {
            state = step(
                &state,
                Operation::RequestDeposit {
                    account: user(*account),
                    assets: AssetAmount::new(*amount),
                },
            )?;
        }

        if cycle.pause_before_cutoff {
            state = step(&state, Operation::Pause { actor: GUARDIAN })?;
            state = step(&state, Operation::Unpause { actor: ADMIN })?;
        }

        let due = state.epoch.map_or(0, |epoch| epoch.cutoff_at.raw());
        state = step(
            &state,
            Operation::CutoffEpoch {
                now: Timestamp::new(due),
            },
        )?;
        state = step(&state, Operation::FinalizeEpoch)?;

        let settled = latest_finalized(&state);
        let extra: Vec<u8> = if cycle.claim_every_user {
            (0..4).collect()
        } else {
            Vec::new()
        };
        for account in cycle.claimers.iter().chain(extra.iter()) {
            state = step(
                &state,
                Operation::ClaimDeposit {
                    account: user(*account),
                    epoch: settled,
                },
            )?;
            state = step(
                &state,
                Operation::ClaimRedeem {
                    account: user(*account),
                    epoch: settled,
                },
            )?;
        }
    }
    Ok(state)
}

// Shared post conditions

fn assert_conserved(state: &State) -> Result<(), TestCaseError> {
    let mut assets = state.buckets.pending_deposit_escrow.raw();
    assets = assets.saturating_add(state.buckets.idle_backing.raw());
    assets = assets.saturating_add(state.buckets.claim_reserve.raw());
    assets = assets.saturating_add(state.buckets.unattributed_balance.raw());
    for account in state.accounts.values() {
        assets = assets.saturating_add(account.assets.raw());
    }
    prop_assert_eq!(assets, state.initial_asset_supply.raw());

    let mut shares = state.escrowed_redeem_shares.raw();
    shares = shares.saturating_add(state.claimable_deposit_shares.raw());
    for account in state.accounts.values() {
        shares = shares.saturating_add(account.shares.raw());
    }
    prop_assert_eq!(shares, state.total_share_supply.raw());
    Ok(())
}

fn assert_cancelled_never_settles(state: &State) -> Result<(), TestCaseError> {
    for (key, request) in &state.deposit_requests {
        if !request.cancelled {
            continue;
        }
        prop_assert!(!request.claimed);
        prop_assert!(request.assets.is_zero());
        let settle = apply(
            state,
            Operation::ClaimDeposit {
                account: key.account,
                epoch: key.epoch,
            },
        );
        let refund = apply(
            state,
            Operation::ClaimAbortedDeposit {
                account: key.account,
                epoch: key.epoch,
            },
        );
        prop_assert!(settle.is_err(), "a cancelled deposit settled");
        prop_assert!(refund.is_err(), "a cancelled deposit was refunded");
    }
    for (key, request) in &state.redeem_requests {
        if !request.cancelled {
            continue;
        }
        prop_assert!(!request.claimed);
        prop_assert!(request.shares.is_zero());
        let settle = apply(
            state,
            Operation::ClaimRedeem {
                account: key.account,
                epoch: key.epoch,
            },
        );
        let refund = apply(
            state,
            Operation::ClaimAbortedRedeem {
                account: key.account,
                epoch: key.epoch,
            },
        );
        prop_assert!(settle.is_err(), "a cancelled redemption settled");
        prop_assert!(refund.is_err(), "a cancelled redemption was refunded");
    }
    Ok(())
}

/// No request can take both a settlement and a refund.
fn assert_one_outcome_per_request(state: &State) -> Result<(), TestCaseError> {
    for (key, request) in &state.deposit_requests {
        if !request.is_outstanding() {
            continue;
        }
        let settle = apply(
            state,
            Operation::ClaimDeposit {
                account: key.account,
                epoch: key.epoch,
            },
        );
        let refund = apply(
            state,
            Operation::ClaimAbortedDeposit {
                account: key.account,
                epoch: key.epoch,
            },
        );
        prop_assert!(settle.is_err() || refund.is_err(), "both outcomes offered");
        if let Ok(settled) = settle {
            prop_assert_eq!(
                apply(
                    &settled,
                    Operation::ClaimAbortedDeposit {
                        account: key.account,
                        epoch: key.epoch,
                    },
                )
                .err(),
                Some(Rejection::EpochNotAborted)
            );
        }
        if let Ok(refunded) = refund {
            prop_assert_eq!(
                apply(
                    &refunded,
                    Operation::ClaimDeposit {
                        account: key.account,
                        epoch: key.epoch,
                    },
                )
                .err(),
                Some(Rejection::EpochNotFinalized)
            );
        }
    }
    Ok(())
}

fn assert_claims_survive_emergency(state: &State) -> Result<(), TestCaseError> {
    let mut paused = state.clone();
    paused.vault_state = VaultState::Paused;
    let mut frozen = state.clone();
    frozen.vault_state = VaultState::Frozen;

    for (key, request) in &state.deposit_requests {
        if !request.is_outstanding() {
            continue;
        }
        let operation = match state.epochs.get(&key.epoch) {
            Some(EpochOutcome::Finalized(_)) => Operation::ClaimDeposit {
                account: key.account,
                epoch: key.epoch,
            },
            Some(EpochOutcome::Aborted(_)) => Operation::ClaimAbortedDeposit {
                account: key.account,
                epoch: key.epoch,
            },
            None => continue,
        };
        prop_assert!(apply(&paused, operation).is_ok());
        prop_assert!(apply(&frozen, operation).is_ok());
    }
    for (key, request) in &state.redeem_requests {
        if !request.is_outstanding() {
            continue;
        }
        let operation = match state.epochs.get(&key.epoch) {
            Some(EpochOutcome::Finalized(_)) => Operation::ClaimRedeem {
                account: key.account,
                epoch: key.epoch,
            },
            Some(EpochOutcome::Aborted(_)) => Operation::ClaimAbortedRedeem {
                account: key.account,
                epoch: key.epoch,
            },
            None => continue,
        };
        prop_assert!(apply(&paused, operation).is_ok());
        prop_assert!(apply(&frozen, operation).is_ok());
    }
    Ok(())
}

fn assert_all(state: &State) -> Result<(), TestCaseError> {
    assert_conserved(state)?;
    assert_cancelled_never_settles(state)?;
    assert_one_outcome_per_request(state)?;
    assert_claims_survive_emergency(state)?;
    Ok(())
}

fn settle_deposits(deposits: impl Iterator<Item = (u8, u128)>) -> Result<State, TestCaseError> {
    let mut state = genesis();
    for (account, amount) in deposits {
        state = step(
            &state,
            Operation::RequestDeposit {
                account: user(account),
                assets: AssetAmount::new(amount),
            },
        )?;
    }
    state = step(
        &state,
        Operation::CutoffEpoch {
            now: Timestamp::new(EPOCH_DURATION),
        },
    )?;
    step(&state, Operation::FinalizeEpoch)
}

fn claim_all(
    state: &State,
    accounts: impl Iterator<Item = AccountId>,
) -> Result<State, TestCaseError> {
    let mut current = state.clone();
    for account in accounts {
        current = step(
            &current,
            Operation::ClaimDeposit {
                account,
                epoch: EpochId::GENESIS,
            },
        )?;
    }
    Ok(current)
}

/// Freezes, aborts the live epoch and takes every refund it owes.
fn drain_by_refund(state: &State) -> Result<State, TestCaseError> {
    let mut current = step(state, Operation::Freeze { actor: GUARDIAN })?;
    current = step(&current, Operation::AbortEpoch { actor: GUARDIAN })?;

    let pending: Vec<(AccountId, EpochId)> = current
        .deposit_requests
        .iter()
        .filter(|(key, request)| {
            request.is_outstanding() && current.aborted_terms(key.epoch).is_some()
        })
        .map(|(key, _)| (key.account, key.epoch))
        .collect();
    for (account, epoch) in pending {
        current = step(&current, Operation::ClaimAbortedDeposit { account, epoch })?;
    }

    let pending: Vec<(AccountId, EpochId)> = current
        .redeem_requests
        .iter()
        .filter(|(key, request)| {
            request.is_outstanding() && current.aborted_terms(key.epoch).is_some()
        })
        .map(|(key, _)| (key.account, key.epoch))
        .collect();
    for (account, epoch) in pending {
        current = step(&current, Operation::ClaimAbortedRedeem { account, epoch })?;
    }
    Ok(current)
}

/// Floors the case count, while still letting PROPTEST_CASES raise it for a soak.
fn config() -> ProptestConfig {
    let default = ProptestConfig::default();
    ProptestConfig {
        cases: default.cases.max(384),
        ..default
    }
}

proptest! {
    #![proptest_config(config())]

    #[test]
    fn arbitrary_operation_sequences_hold_every_invariant(
        actions in prop::collection::vec(action_strategy(), 1..60)
    ) {
        let state = drive(&actions)?;
        assert_all(&state)?;
    }

    #[test]
    fn well_formed_epoch_cycles_hold_every_invariant(
        cycles in prop::collection::vec(cycle_strategy(), 1..6)
    ) {
        let state = run_cycles(&cycles)?;
        assert_all(&state)?;
    }

    #[test]
    fn every_finalized_redemption_stays_claimable_to_the_last_holder(
        cycles in prop::collection::vec(cycle_strategy(), 1..6)
    ) {
        let mut state = run_cycles(&cycles)?;
        let outstanding: Vec<(AccountId, EpochId)> = state
            .redeem_requests
            .iter()
            .filter(|(key, request)| {
                request.is_outstanding() && state.finalized_terms(key.epoch).is_some()
            })
            .map(|(key, _)| (key.account, key.epoch))
            .collect();
        for (account, epoch) in outstanding {
            state = step(&state, Operation::ClaimRedeem { account, epoch })?;
        }
        prop_assert_eq!(state.outstanding_redeem_assets(), Ok(AssetAmount::ZERO));
        assert_conserved(&state)?;
    }

    #[test]
    fn frozen_unfinalized_funds_always_have_a_refund_path(
        cycles in prop::collection::vec(cycle_strategy(), 1..5),
        tail in prop::collection::vec((0u8..4, MIN_DEPOSIT..1_000_000_000u128), 1..5),
        redeems in prop::collection::vec((0u8..4, any::<u8>()), 0..4),
    ) {
        let mut state = run_cycles(&cycles)?;
        let now = state.last_cutoff_at.raw();
        state = step(&state, Operation::OpenNextEpoch { now: Timestamp::new(now) })?;
        for (account, amount) in &tail {
            state = step(&state, Operation::RequestDeposit {
                account: user(*account),
                assets: AssetAmount::new(*amount),
            })?;
        }
        for (account, quarters) in &redeems {
            let shares = portion(&state, *account, *quarters);
            state = step(&state, Operation::RequestRedeem {
                account: user(*account),
                shares,
            })?;
        }

        let escrowed = state.buckets.pending_deposit_escrow;
        let escrowed_shares = state.escrowed_redeem_shares;
        let drained = drain_by_refund(&state)?;

        // Everything the frozen epoch held came back to its owner.
        prop_assert_eq!(drained.buckets.pending_deposit_escrow, AssetAmount::ZERO);
        prop_assert_eq!(drained.escrowed_redeem_shares, ShareAmount::ZERO);
        prop_assert_eq!(drained.outstanding_refund_assets(), Ok(AssetAmount::ZERO));
        prop_assert_eq!(drained.outstanding_refund_shares(), Ok(ShareAmount::ZERO));
        prop_assert_eq!(drained.vault_state, VaultState::Frozen);

        let mut returned = AssetAmount::ZERO;
        for account in state.accounts.keys() {
            let before = state.account(*account).assets;
            let after = drained.account(*account).assets;
            prop_assert!(after >= before);
            returned = returned.checked_add(after.checked_sub(before)?)?;
        }
        prop_assert_eq!(returned, escrowed);

        let mut given_back = ShareAmount::ZERO;
        for account in state.accounts.keys() {
            let before = state.account(*account).shares;
            let after = drained.account(*account).shares;
            prop_assert!(after >= before);
            given_back = given_back.checked_add(after.checked_sub(before)?)?;
        }
        prop_assert_eq!(given_back, escrowed_shares);
        assert_conserved(&drained)?;
    }

    #[test]
    fn refunds_stay_available_for_as_long_as_the_vault_is_frozen(
        deposits in prop::collection::vec((0u8..4, MIN_DEPOSIT..1_000_000_000u128), 1..6)
    ) {
        let mut state = genesis();
        for (account, amount) in &deposits {
            state = step(&state, Operation::RequestDeposit {
                account: user(*account),
                assets: AssetAmount::new(*amount),
            })?;
        }
        state = step(&state, Operation::Freeze { actor: ADMIN })?;
        state = step(&state, Operation::AbortEpoch { actor: ADMIN })?;

        for (account, _) in &deposits {
            let key = RequestKey::new(EpochId::GENESIS, user(*account));
            // The refund is offered no matter how many other operations were refused.
            let new_request = apply(&state, Operation::RequestDeposit {
                account: user(*account),
                assets: AssetAmount::new(MIN_DEPOSIT),
            });
            let late_cutoff = apply(&state, Operation::CutoffEpoch {
                now: Timestamp::new(u64::MAX),
            });
            prop_assert!(new_request.is_err(), "a frozen vault accepted a request");
            prop_assert!(late_cutoff.is_err(), "a frozen vault accepted a cutoff");

            if state.deposit_requests.get(&key).is_some_and(DepositRequest::is_outstanding) {
                state = step(&state, Operation::ClaimAbortedDeposit {
                    account: user(*account),
                    epoch: EpochId::GENESIS,
                })?;
            }
        }
        prop_assert_eq!(state.buckets.pending_deposit_escrow, AssetAmount::ZERO);
    }

    #[test]
    fn cancel_and_request_again_never_double_counts(
        rounds in prop::collection::vec((0u8..4, MIN_DEPOSIT..1_000_000_000u128), 1..10)
    ) {
        let mut state = genesis();
        let mut expected: alloc_map::Map = alloc_map::Map::new();
        for (account, amount) in &rounds {
            let id = user(*account);
            state = step(&state, Operation::RequestDeposit {
                account: id,
                assets: AssetAmount::new(*amount),
            })?;
            expected.add(id, *amount);
            state = step(&state, Operation::CancelDeposit { account: id })?;
            expected.clear(id);
        }
        // One final live request per account decides the epoch total.
        for (account, amount) in &rounds {
            let id = user(*account);
            state = step(&state, Operation::RequestDeposit {
                account: id,
                assets: AssetAmount::new(*amount),
            })?;
            expected.add(id, *amount);
        }

        prop_assert_eq!(
            state.buckets.pending_deposit_escrow.raw(),
            expected.total()
        );
        let state = step(&state, Operation::CutoffEpoch { now: Timestamp::new(EPOCH_DURATION) })?;
        let state = step(&state, Operation::FinalizeEpoch)?;
        let terms = state.finalized_terms(EpochId::GENESIS).expect("finalized");
        prop_assert_eq!(terms.deposit_assets.raw(), expected.total());
    }

    #[test]
    fn deposit_order_does_not_change_the_epoch_result(
        deposits in prop::collection::vec((0u8..4, MIN_DEPOSIT..1_000_000_000u128), 1..8)
    ) {
        let forward = settle_deposits(deposits.iter().copied())?;
        let backward = settle_deposits(deposits.iter().rev().copied())?;
        prop_assert_eq!(forward.epochs, backward.epochs);
        prop_assert_eq!(forward.total_share_supply, backward.total_share_supply);
        prop_assert_eq!(forward.buckets, backward.buckets);
    }

    #[test]
    fn claim_order_does_not_change_the_final_state(
        deposits in prop::collection::vec((0u8..4, MIN_DEPOSIT..1_000_000_000u128), 1..8)
    ) {
        let settled = settle_deposits(deposits.iter().copied())?;
        let claimers: Vec<AccountId> = settled
            .deposit_requests
            .iter()
            .filter(|(_, request)| request.is_outstanding())
            .map(|(key, _)| key.account)
            .collect();

        let forward = claim_all(&settled, claimers.iter().copied())?;
        let backward = claim_all(&settled, claimers.iter().rev().copied())?;
        prop_assert_eq!(forward, backward);
    }

    #[test]
    fn the_price_never_falls_through_mixed_settlement(
        cycles in prop::collection::vec(cycle_strategy(), 2..7)
    ) {
        let state = run_cycles(&cycles)?;
        assert_price_never_falls(&state)?;
        let finalized = state
            .epochs
            .values()
            .filter(|outcome| outcome.finalized().is_some())
            .count();
        prop_assert!(finalized >= 2, "the run did not settle enough epochs");
    }
}

/// Small helper so the expected escrow is tracked without extra dependencies.
mod alloc_map {
    use accounting_model::AccountId;

    #[derive(Debug, Default)]
    pub(crate) struct Map {
        entries: Vec<(AccountId, u128)>,
    }

    impl Map {
        pub(crate) fn new() -> Self {
            Self::default()
        }

        pub(crate) fn add(&mut self, account: AccountId, amount: u128) {
            match self.entries.iter_mut().find(|(id, _)| *id == account) {
                Some((_, total)) => *total = total.saturating_add(amount),
                None => self.entries.push((account, amount)),
            }
        }

        pub(crate) fn clear(&mut self, account: AccountId) {
            if let Some((_, total)) = self.entries.iter_mut().find(|(id, _)| *id == account) {
                *total = 0;
            }
        }

        pub(crate) fn total(&self) -> u128 {
            self.entries
                .iter()
                .fold(0u128, |sum, (_, total)| sum.saturating_add(*total))
        }
    }
}
