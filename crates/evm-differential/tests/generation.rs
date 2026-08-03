#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

//! Properties every scenario must hold before Solidity ever sees it.

use std::collections::BTreeSet;

use accounting_model::{
    AssetAmount, Authority, Config, ConfigVersion, Genesis, ShareAmount, State, Timestamp, apply,
    check_invariants,
};

use evm_differential::abi::{encode_bundle, encode_scenario};
use evm_differential::action::{ADMIN_SLOT, ActionKind, GUARDIAN_SLOT, USER_COUNT, account_for};
use evm_differential::generator::FAMILY_COUNT;
use evm_differential::result::{ResultCode, code_for};
use evm_differential::scenario::{
    ASSET_DECIMALS, OUTCOME_ABORTED, OUTCOME_FINALIZED, SHARE_DECIMALS, Scenario,
};
use evm_differential::snapshot::Snapshot;
use evm_differential::{RunConfig, build_run};

/// Matches the caps the vault enforces per epoch.
const MAX_REQUESTS_PER_EPOCH: usize = 32;

fn standard() -> RunConfig {
    RunConfig {
        seed: 1,
        cases: 64,
        steps: 4,
        only: None,
    }
}

fn run(config: RunConfig) -> Vec<Scenario> {
    build_run(config).unwrap_or_else(|reason| panic!("generation failed: {reason}"))
}

#[test]
fn the_same_inputs_produce_the_same_bytes() {
    let first = encode_bundle(1, &run(standard()));
    let second = encode_bundle(1, &run(standard()));
    assert_eq!(first, second);
}

#[test]
fn different_seeds_produce_different_bytes() {
    let base = standard();
    let other = RunConfig { seed: 2, ..base };
    assert_ne!(
        encode_bundle(base.seed, &run(base)),
        encode_bundle(other.seed, &run(other))
    );
}

#[test]
fn encoding_one_scenario_is_stable() {
    let scenarios = run(standard());
    for scenario in &scenarios {
        assert_eq!(encode_scenario(scenario), encode_scenario(scenario));
    }
}

#[test]
fn every_family_appears_in_a_standard_run() {
    let scenarios = run(standard());
    for family in 0..FAMILY_COUNT {
        assert!(
            scenarios.iter().any(|scenario| scenario.family == family),
            "family {family} never ran"
        );
    }
}

#[test]
fn every_scenario_mixes_accepted_and_refused_operations() {
    for scenario in run(standard()) {
        assert!(
            scenario.successes() > 0,
            "scenario {} accepted nothing",
            scenario.index
        );
        assert!(
            scenario.rejections() > 0,
            "scenario {} refused nothing",
            scenario.index
        );
    }
}

#[test]
fn scenario_length_stays_in_the_intended_band() {
    for scenario in run(standard()) {
        let steps = scenario.actions.len();
        assert!(
            (10..=80).contains(&steps),
            "scenario {} ran {steps} steps",
            scenario.index
        );
    }
}

/// The vault refuses more than thirty two priced controllers per epoch, so no
/// scenario may reach that many.
#[test]
fn no_epoch_collects_more_requests_than_the_vault_allows() {
    for scenario in run(standard()) {
        for epoch in &scenario.epochs {
            let deposits = epoch
                .actors
                .iter()
                .filter(|actor| actor.deposit_assets > 0)
                .count();
            let redemptions = epoch
                .actors
                .iter()
                .filter(|actor| actor.redeem_shares > 0)
                .count();
            assert!(deposits <= MAX_REQUESTS_PER_EPOCH);
            assert!(redemptions <= MAX_REQUESTS_PER_EPOCH);
        }
    }
}

#[test]
fn timestamps_never_move_backwards() {
    for scenario in run(standard()) {
        let mut previous = scenario.start_timestamp;
        for entry in &scenario.actions {
            assert!(
                entry.action.timestamp >= previous,
                "scenario {} moved time backwards",
                scenario.index
            );
            previous = entry.action.timestamp;
        }
    }
}

#[test]
fn every_scenario_records_one_snapshot_per_operation() {
    for scenario in run(standard()) {
        assert_eq!(scenario.actions.len(), scenario.snapshots.len());
    }
}

/// Replays each trace through the model on its own and checks the recording.
#[test]
fn a_replay_reproduces_every_recorded_result_and_state() {
    for scenario in run(standard()) {
        let setup = scenario.setup();
        let mut accounts = Vec::new();
        for user in 0..USER_COUNT {
            accounts.push((
                account_for(u8::try_from(user).unwrap()),
                AssetAmount::new(setup.initial_assets[user]),
            ));
        }

        let mut state = State::new(Genesis {
            config: Config {
                version: ConfigVersion::new(setup.config_version),
                asset_decimals: ASSET_DECIMALS,
                share_decimals: SHARE_DECIMALS,
                min_deposit_assets: AssetAmount::new(setup.min_deposit),
                min_redeem_shares: ShareAmount::new(setup.min_redeem),
                epoch_duration: setup.epoch_duration,
            },
            authority: Authority {
                admin: account_for(ADMIN_SLOT),
                guardian: account_for(GUARDIAN_SLOT),
            },
            accounts,
            unattributed_balance: AssetAmount::ZERO,
            opened_at: Timestamp::new(setup.start_timestamp),
        })
        .expect("valid genesis");

        assert_eq!(Snapshot::capture(&state), scenario.initial_snapshot);

        for (step, entry) in scenario.actions.iter().enumerate() {
            let expected = match apply(&state, entry.action.to_operation()) {
                Ok(next) => {
                    state = next;
                    ResultCode::Success
                }
                Err(reason) => code_for(entry.action.kind, reason),
            };
            assert_eq!(
                expected, entry.result,
                "scenario {} step {step} result",
                scenario.index
            );
            assert_eq!(
                Snapshot::capture(&state),
                scenario.snapshots[step],
                "scenario {} step {step} state",
                scenario.index
            );
            check_invariants(&state).unwrap_or_else(|violation| {
                panic!("scenario {} step {step}: {violation}", scenario.index)
            });
        }
    }
}

/// A refused operation must leave the model exactly where it was.
#[test]
fn a_refused_operation_never_changes_the_state() {
    for scenario in run(standard()) {
        for (step, entry) in scenario.actions.iter().enumerate() {
            if entry.result == ResultCode::Success {
                continue;
            }
            let before = if step == 0 {
                scenario.initial_snapshot
            } else {
                scenario.snapshots[step - 1]
            };
            assert_eq!(
                before, scenario.snapshots[step],
                "scenario {} step {step} moved on a refusal",
                scenario.index
            );
        }
    }
}

/// A successful deposit must be affordable at the moment it is sent.
#[test]
fn accepted_requests_are_always_covered_by_the_actor() {
    for scenario in run(standard()) {
        for (step, entry) in scenario.actions.iter().enumerate() {
            if entry.result != ResultCode::Success {
                continue;
            }
            let user = usize::from(entry.action.actor);
            if user >= USER_COUNT {
                continue;
            }
            let before = if step == 0 {
                scenario.initial_snapshot
            } else {
                scenario.snapshots[step - 1]
            };
            match entry.action.kind {
                ActionKind::RequestDeposit => {
                    assert!(
                        before.actor_assets[user] >= entry.action.amount,
                        "scenario {} step {step} overspent assets",
                        scenario.index
                    );
                }
                ActionKind::RequestRedeem => {
                    assert!(
                        before.actor_shares[user] >= entry.action.amount,
                        "scenario {} step {step} overspent shares",
                        scenario.index
                    );
                }
                _ => {}
            }
        }
    }
}

/// Claims and refunds must hand back a non zero amount at least sometimes,
/// otherwise the comparison would be checking nothing.
#[test]
fn claims_and_refunds_move_real_amounts() {
    let scenarios = run(standard());
    let mut deposit_claim = 0usize;
    let mut redeem_claim = 0usize;
    let mut deposit_refund = 0usize;
    let mut redeem_refund = 0usize;

    for scenario in &scenarios {
        for entry in &scenario.actions {
            if entry.result != ResultCode::Success || entry.return_value == 0 {
                continue;
            }
            match entry.action.kind {
                ActionKind::ClaimDeposit => deposit_claim += 1,
                ActionKind::ClaimRedeem => redeem_claim += 1,
                ActionKind::RefundDeposit => deposit_refund += 1,
                ActionKind::RefundRedeem => redeem_refund += 1,
                _ => {}
            }
        }
    }

    assert!(deposit_claim > 0, "no deposit claim paid out");
    assert!(redeem_claim > 0, "no redemption claim paid out");
    assert!(deposit_refund > 0, "no deposit refund paid out");
    assert!(redeem_refund > 0, "no redemption refund paid out");
}

/// Reachability. A generator that never reaches these states proves little.
#[test]
fn a_standard_run_reaches_every_interesting_state() {
    let scenarios = run(standard());
    let total = scenarios.len();

    let finalized = scenarios.iter().filter(|s| s.has_finalized_epoch()).count();
    let aborted = scenarios.iter().filter(|s| s.has_aborted_epoch()).count();
    let deposit_claims = scenarios
        .iter()
        .filter(|s| s.counts(ActionKind::ClaimDeposit) > 0)
        .count();
    let redeem_claims = scenarios
        .iter()
        .filter(|s| s.counts(ActionKind::ClaimRedeem) > 0)
        .count();
    let deposit_refunds = scenarios
        .iter()
        .filter(|s| s.counts(ActionKind::RefundDeposit) > 0)
        .count();
    let redeem_refunds = scenarios
        .iter()
        .filter(|s| s.counts(ActionKind::RefundRedeem) > 0)
        .count();
    let cancellations = scenarios
        .iter()
        .filter(|s| s.counts(ActionKind::CancelDeposit) + s.counts(ActionKind::CancelRedeem) > 0)
        .count();
    let pauses = scenarios
        .iter()
        .filter(|s| s.counts(ActionKind::Pause) > 0)
        .count();
    let freezes = scenarios
        .iter()
        .filter(|s| s.counts(ActionKind::Freeze) > 0)
        .count();

    assert_eq!(finalized, total, "every scenario should finalize an epoch");
    assert!(
        deposit_claims * 100 / total >= 90,
        "deposit claims too rare"
    );
    assert!(
        redeem_claims * 100 / total >= 30,
        "redemption claims too rare"
    );
    assert!(cancellations * 100 / total >= 30, "cancellations too rare");
    assert!(pauses * 100 / total >= 10, "pauses too rare");
    assert!(freezes * 100 / total >= 10, "freezes too rare");
    assert!(aborted * 100 / total >= 10, "aborts too rare");
    assert!(
        deposit_refunds * 100 / total >= 10,
        "deposit refunds too rare"
    );
    assert!(
        redeem_refunds * 100 / total >= 10,
        "redemption refunds too rare"
    );
}

/// Multi epoch traces must really settle three or more epochs.
#[test]
fn some_scenarios_settle_at_least_three_epochs() {
    let scenarios = run(standard());
    let deep = scenarios
        .iter()
        .filter(|scenario| scenario.epochs.len() >= 3)
        .count();
    assert!(deep > 0, "no scenario settled three epochs");
}

/// Price must actually move, otherwise rounding is never exercised.
#[test]
fn some_epochs_settle_at_a_price_other_than_the_first_one() {
    let scenarios = run(standard());
    let moved = scenarios.iter().any(|scenario| {
        scenario
            .epochs
            .iter()
            .any(|epoch| epoch.outcome == OUTCOME_FINALIZED && epoch.total_supply > 0)
    });
    assert!(moved, "every epoch settled against an empty vault");
}

/// Rounding leftovers must appear somewhere, or the dust checks are idle.
#[test]
fn some_epoch_keeps_rounding_dust() {
    let scenarios = run(RunConfig {
        cases: 128,
        ..standard()
    });
    let dusty = scenarios.iter().any(|scenario| {
        scenario
            .epochs
            .iter()
            .any(|epoch| epoch.deposit_dust > 0 || epoch.redeem_dust > 0)
    });
    assert!(dusty, "no epoch produced dust");
}

/// Aborted epochs must leave the priced fields alone.
#[test]
fn an_aborted_epoch_prices_nothing() {
    for scenario in run(standard()) {
        for epoch in &scenario.epochs {
            if epoch.outcome != OUTCOME_ABORTED {
                continue;
            }
            assert_eq!(epoch.minted_shares, 0);
            assert_eq!(epoch.redeem_assets, 0);
            assert_eq!(epoch.deposit_dust, 0);
            assert_eq!(epoch.redeem_dust, 0);
            for actor in &epoch.actors {
                assert_eq!(actor.claim_shares, 0);
                assert_eq!(actor.claim_assets, 0);
            }
        }
    }
}

/// The rejection traces must exercise a wide range of refusal reasons.
#[test]
fn a_standard_run_covers_many_distinct_refusals() {
    let scenarios = run(standard());
    let mut seen = BTreeSet::new();
    for scenario in &scenarios {
        for entry in &scenario.actions {
            if entry.result != ResultCode::Success {
                seen.insert(entry.result);
            }
        }
    }

    for wanted in [
        ResultCode::InvalidVaultState,
        ResultCode::Unauthorized,
        ResultCode::EpochNotOpen,
        ResultCode::EpochAlreadyOpen,
        ResultCode::EpochAlreadyCutOff,
        ResultCode::EpochNotCutOff,
        ResultCode::CutoffNotReached,
        ResultCode::CancellationAfterCutoff,
        ResultCode::EpochNotFinalized,
        ResultCode::EpochNotAborted,
        ResultCode::RequestNotFound,
        ResultCode::RequestNotActive,
        ResultCode::ClaimAlreadyConsumed,
        ResultCode::RefundAlreadyConsumed,
        ResultCode::ZeroAmount,
        ResultCode::AmountBelowMinimum,
        ResultCode::InsufficientAssetBalance,
        ResultCode::InsufficientShareBalance,
    ] {
        assert!(seen.contains(&wanted), "{} never happened", wanted.name());
    }
}

/// A soak run must stay healthy as well.
#[test]
fn a_larger_run_still_builds() {
    let scenarios = run(RunConfig {
        seed: 4_242,
        cases: 256,
        steps: 5,
        only: None,
    });
    assert_eq!(scenarios.len(), 256);
    assert!(
        scenarios
            .iter()
            .all(|scenario| !scenario.actions.is_empty())
    );
}
