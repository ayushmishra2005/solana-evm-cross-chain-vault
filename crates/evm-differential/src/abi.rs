use alloy_primitives::{Bytes, U256};
use alloy_sol_types::{SolValue, sol};

use crate::action::USER_COUNT;
use crate::scenario::{EpochRecord, Scenario};
use crate::snapshot::Snapshot;

sol! {
    /// Mutable state compared after every operation.
    struct AbiSnapshot {
        uint256 pendingDepositEscrow;
        uint256 idleBacking;
        uint256 claimReserve;
        uint256 unattributedBalance;
        uint256 totalSupply;
        uint256 vaultShareBalance;
        uint256 vaultAssetBalance;
        uint8 status;
        bool epochOpen;
        uint8 epochPhase;
        uint64 epochId;
        uint64 nextEpochId;
        uint256 epochDepositAssets;
        uint256 epochRedeemShares;
        uint256 consumedMask;
        uint256[4] actorAssets;
        uint256[4] actorShares;
        uint256[4] actorDepositAssets;
        uint256[4] actorRedeemShares;
    }

    /// One operation with the outcome the model produced.
    struct AbiAction {
        uint8 kind;
        uint8 actor;
        uint256 amount;
        uint64 epochId;
        uint64 timestamp;
        uint8 expectedResult;
        uint256 expectedReturn;
    }

    /// What one settled epoch owes a single user.
    struct AbiEpochActor {
        uint256 depositAssets;
        uint256 redeemShares;
        uint256 claimShares;
        uint256 claimAssets;
    }

    /// Immutable terms of one settled epoch.
    struct AbiEpoch {
        uint64 epochId;
        uint8 outcome;
        uint32 settledAtStep;
        uint64 cutoffAt;
        uint256 totalAssets;
        uint256 totalSupply;
        uint256 depositAssets;
        uint256 mintedShares;
        uint256 redeemShares;
        uint256 redeemAssets;
        uint256 depositDust;
        uint256 redeemDust;
        AbiEpochActor[4] actors;
    }

    /// A complete trace the Solidity harness replays.
    struct AbiScenario {
        uint32 index;
        uint64 seed;
        uint8 family;
        uint64 startTimestamp;
        uint64 epochDuration;
        uint256 minDeposit;
        uint256 minRedeem;
        uint32 configVersion;
        uint256[4] initialAssets;
        AbiSnapshot initialSnapshot;
        AbiAction[] actions;
        AbiSnapshot[] snapshots;
        AbiEpoch[] epochs;
    }
}

fn word(value: u128) -> U256 {
    U256::from(value)
}

fn words(values: &[u128; USER_COUNT]) -> [U256; 4] {
    let mut out = [U256::ZERO; 4];
    for (slot, value) in out.iter_mut().zip(values.iter()) {
        *slot = word(*value);
    }
    out
}

impl From<&Snapshot> for AbiSnapshot {
    fn from(shot: &Snapshot) -> Self {
        Self {
            pendingDepositEscrow: word(shot.pending_deposit_escrow),
            idleBacking: word(shot.idle_backing),
            claimReserve: word(shot.claim_reserve),
            unattributedBalance: word(shot.unattributed_balance),
            totalSupply: word(shot.total_supply),
            vaultShareBalance: word(shot.vault_share_balance),
            vaultAssetBalance: word(shot.vault_asset_balance),
            status: shot.status,
            epochOpen: shot.epoch_open,
            epochPhase: shot.epoch_phase,
            epochId: shot.epoch_id,
            nextEpochId: shot.next_epoch_id,
            epochDepositAssets: word(shot.epoch_deposit_assets),
            epochRedeemShares: word(shot.epoch_redeem_shares),
            consumedMask: word(shot.consumed_mask),
            actorAssets: words(&shot.actor_assets),
            actorShares: words(&shot.actor_shares),
            actorDepositAssets: words(&shot.actor_deposit_assets),
            actorRedeemShares: words(&shot.actor_redeem_shares),
        }
    }
}

impl From<&EpochRecord> for AbiEpoch {
    fn from(record: &EpochRecord) -> Self {
        let mut actors = core::array::from_fn(|_| AbiEpochActor {
            depositAssets: U256::ZERO,
            redeemShares: U256::ZERO,
            claimShares: U256::ZERO,
            claimAssets: U256::ZERO,
        });
        for (slot, entry) in actors.iter_mut().zip(record.actors.iter()) {
            *slot = AbiEpochActor {
                depositAssets: word(entry.deposit_assets),
                redeemShares: word(entry.redeem_shares),
                claimShares: word(entry.claim_shares),
                claimAssets: word(entry.claim_assets),
            };
        }

        Self {
            epochId: record.epoch_id,
            outcome: record.outcome,
            settledAtStep: record.settled_at_step,
            cutoffAt: record.cutoff_at,
            totalAssets: word(record.total_assets),
            totalSupply: word(record.total_supply),
            depositAssets: word(record.deposit_assets),
            mintedShares: word(record.minted_shares),
            redeemShares: word(record.redeem_shares),
            redeemAssets: word(record.redeem_assets),
            depositDust: word(record.deposit_dust),
            redeemDust: word(record.redeem_dust),
            actors,
        }
    }
}

impl From<&Scenario> for AbiScenario {
    fn from(scenario: &Scenario) -> Self {
        Self {
            index: scenario.index,
            seed: scenario.seed,
            family: scenario.family,
            startTimestamp: scenario.start_timestamp,
            epochDuration: scenario.epoch_duration,
            minDeposit: word(scenario.min_deposit),
            minRedeem: word(scenario.min_redeem),
            configVersion: scenario.config_version,
            initialAssets: words(&scenario.initial_assets),
            initialSnapshot: AbiSnapshot::from(&scenario.initial_snapshot),
            actions: scenario
                .actions
                .iter()
                .map(|entry| AbiAction {
                    kind: entry.action.kind.raw(),
                    actor: entry.action.actor,
                    amount: word(entry.action.amount),
                    epochId: entry.action.epoch,
                    timestamp: entry.action.timestamp,
                    expectedResult: entry.result.raw(),
                    expectedReturn: word(entry.return_value),
                })
                .collect(),
            snapshots: scenario.snapshots.iter().map(AbiSnapshot::from).collect(),
            epochs: scenario.epochs.iter().map(AbiEpoch::from).collect(),
        }
    }
}

/// Encodes one scenario so Solidity can decode it on its own.
#[must_use]
pub fn encode_scenario(scenario: &Scenario) -> Vec<u8> {
    AbiScenario::from(scenario).abi_encode()
}

/// Encodes the whole run as `abi.encode(uint64, uint32, bytes[])`.
#[must_use]
pub fn encode_bundle(seed: u64, scenarios: &[Scenario]) -> Vec<u8> {
    let parts: Vec<Bytes> = scenarios
        .iter()
        .map(|scenario| Bytes::from(encode_scenario(scenario)))
        .collect();
    let count = u32::try_from(scenarios.len()).unwrap_or(u32::MAX);
    (seed, count, parts).abi_encode_params()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generator::{generate, setup_for};
    use crate::rng::Rng;

    fn sample() -> Scenario {
        let mut rng = Rng::new(11);
        let setup = setup_for(0, 11, &mut rng);
        generate(setup, &mut rng, 3).unwrap_or_else(|reason| panic!("{reason}"))
    }

    #[test]
    fn encoding_a_scenario_twice_gives_the_same_bytes() {
        let scenario = sample();
        assert_eq!(encode_scenario(&scenario), encode_scenario(&scenario));
    }

    #[test]
    fn a_bundle_grows_with_its_scenarios() {
        let scenario = sample();
        let one = encode_bundle(11, std::slice::from_ref(&scenario));
        let two = encode_bundle(11, &[scenario.clone(), scenario]);
        assert!(two.len() > one.len());
    }

    #[test]
    fn an_encoded_scenario_round_trips_in_rust() {
        let scenario = sample();
        let raw = encode_scenario(&scenario);
        let decoded = AbiScenario::abi_decode(&raw).unwrap_or_else(|reason| panic!("{reason}"));
        assert_eq!(decoded.index, scenario.index);
        assert_eq!(decoded.actions.len(), scenario.actions.len());
        assert_eq!(decoded.snapshots.len(), scenario.snapshots.len());
        assert_eq!(decoded.epochs.len(), scenario.epochs.len());
    }

    #[test]
    fn a_bundle_round_trips_in_rust() {
        let scenario = sample();
        let raw = encode_bundle(77, std::slice::from_ref(&scenario));
        let (seed, count, parts) = <(u64, u32, Vec<Bytes>)>::abi_decode_params(&raw)
            .unwrap_or_else(|reason| panic!("{reason}"));
        assert_eq!(seed, 77);
        assert_eq!(count, 1);
        assert_eq!(parts.len(), 1);
    }
}
