use std::collections::BTreeMap;

use accounting_model::{
    AssetAmount, Authority, Config, ConfigVersion, EpochId, EpochOutcome, EpochPhase, Genesis,
    Rejection, RequestKey, ShareAmount, State, Timestamp, VaultState, apply,
};

use crate::action::{ADMIN_SLOT, Action, ActionKind, GUARDIAN_SLOT, USER_COUNT, account_for};
use crate::result::{ResultCode, code_for};
use crate::snapshot::Snapshot;

/// Outcome marker shared with Solidity, matching its `EpochOutcome` enum.
pub const OUTCOME_FINALIZED: u8 = 1;
pub const OUTCOME_ABORTED: u8 = 2;

pub const ASSET_DECIMALS: u8 = 6;
pub const SHARE_DECIMALS: u8 = 18;

/// What one epoch owes a single user. Immutable once the epoch leaves the slot.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EpochActorTerms {
    pub deposit_assets: u128,
    pub redeem_shares: u128,
    pub claim_shares: u128,
    pub claim_assets: u128,
}

/// Settled terms of one epoch plus the step from which they are observable.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EpochRecord {
    pub epoch_id: u64,
    pub outcome: u8,
    pub settled_at_step: u32,
    pub cutoff_at: u64,
    pub total_assets: u128,
    pub total_supply: u128,
    pub deposit_assets: u128,
    pub minted_shares: u128,
    pub redeem_shares: u128,
    pub redeem_assets: u128,
    pub deposit_dust: u128,
    pub redeem_dust: u128,
    pub actors: [EpochActorTerms; USER_COUNT],
}

/// One operation together with the result the model produced.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RecordedAction {
    pub action: Action,
    pub result: ResultCode,
    pub return_value: u128,
}

/// A complete trace with expected results and expected states.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Scenario {
    pub index: u32,
    pub seed: u64,
    pub family: u8,
    pub start_timestamp: u64,
    pub epoch_duration: u64,
    pub min_deposit: u128,
    pub min_redeem: u128,
    pub config_version: u32,
    pub initial_assets: [u128; USER_COUNT],
    pub initial_snapshot: Snapshot,
    pub actions: Vec<RecordedAction>,
    pub snapshots: Vec<Snapshot>,
    pub epochs: Vec<EpochRecord>,
}

impl Scenario {
    /// Settings this scenario started from, so a replay can rebuild genesis.
    #[must_use]
    pub const fn setup(&self) -> Setup {
        Setup {
            index: self.index,
            seed: self.seed,
            family: self.family,
            start_timestamp: self.start_timestamp,
            epoch_duration: self.epoch_duration,
            min_deposit: self.min_deposit,
            min_redeem: self.min_redeem,
            config_version: self.config_version,
            initial_assets: self.initial_assets,
        }
    }

    /// Counts operations the model accepted.
    #[must_use]
    pub fn successes(&self) -> usize {
        self.actions
            .iter()
            .filter(|entry| entry.result == ResultCode::Success)
            .count()
    }

    #[must_use]
    pub fn rejections(&self) -> usize {
        self.actions.len().saturating_sub(self.successes())
    }

    #[must_use]
    pub fn has_finalized_epoch(&self) -> bool {
        self.epochs
            .iter()
            .any(|epoch| epoch.outcome == OUTCOME_FINALIZED)
    }

    #[must_use]
    pub fn has_aborted_epoch(&self) -> bool {
        self.epochs
            .iter()
            .any(|epoch| epoch.outcome == OUTCOME_ABORTED)
    }

    #[must_use]
    pub fn counts(&self, kind: ActionKind) -> usize {
        self.actions
            .iter()
            .filter(|entry| entry.action.kind == kind && entry.result == ResultCode::Success)
            .count()
    }
}

/// Reasons a scenario could not be produced.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BuildError {
    Genesis(Rejection),
    Pricing(Rejection),
}

impl core::fmt::Display for BuildError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Genesis(reason) => write!(formatter, "genesis rejected: {reason}"),
            Self::Pricing(reason) => write!(formatter, "pricing failed: {reason}"),
        }
    }
}

impl core::error::Error for BuildError {}

/// Settings a scenario starts from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Setup {
    pub index: u32,
    pub seed: u64,
    pub family: u8,
    pub start_timestamp: u64,
    pub epoch_duration: u64,
    pub min_deposit: u128,
    pub min_redeem: u128,
    pub config_version: u32,
    pub initial_assets: [u128; USER_COUNT],
}

/// Runs actions against the model and records what the harness must reproduce.
#[derive(Debug)]
pub struct Builder {
    state: State,
    scenario: Scenario,
    settled_steps: BTreeMap<u64, u32>,
    now: u64,
}

impl Builder {
    pub fn new(setup: Setup) -> Result<Self, BuildError> {
        let mut accounts = Vec::with_capacity(USER_COUNT);
        for user in 0..USER_COUNT {
            let balance = setup.initial_assets.get(user).copied().unwrap_or(0);
            accounts.push((account_for(user as u8), AssetAmount::new(balance)));
        }

        let genesis = Genesis {
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
        };

        let state = State::new(genesis).map_err(BuildError::Genesis)?;
        let initial_snapshot = Snapshot::capture(&state);

        Ok(Self {
            state,
            scenario: Scenario {
                index: setup.index,
                seed: setup.seed,
                family: setup.family,
                start_timestamp: setup.start_timestamp,
                epoch_duration: setup.epoch_duration,
                min_deposit: setup.min_deposit,
                min_redeem: setup.min_redeem,
                config_version: setup.config_version,
                initial_assets: setup.initial_assets,
                initial_snapshot,
                actions: Vec::new(),
                snapshots: Vec::new(),
                epochs: Vec::new(),
            },
            settled_steps: BTreeMap::new(),
            now: setup.start_timestamp,
        })
    }

    #[must_use]
    pub const fn state(&self) -> &State {
        &self.state
    }

    #[must_use]
    pub const fn now(&self) -> u64 {
        self.now
    }

    #[must_use]
    pub fn steps(&self) -> usize {
        self.scenario.actions.len()
    }

    /// Moves the clock forward. Time never runs backwards inside a scenario.
    pub fn advance_to(&mut self, timestamp: u64) {
        self.now = self.now.max(timestamp);
    }

    pub fn advance_by(&mut self, seconds: u64) {
        self.now = self.now.saturating_add(seconds);
    }

    #[must_use]
    pub fn user_assets(&self, user: usize) -> u128 {
        self.state.account(account_for(user as u8)).assets.raw()
    }

    #[must_use]
    pub fn user_shares(&self, user: usize) -> u128 {
        self.state.account(account_for(user as u8)).shares.raw()
    }

    #[must_use]
    pub fn epoch_open(&self) -> bool {
        self.state.epoch.is_some()
    }

    #[must_use]
    pub fn epoch_is_taking_requests(&self) -> bool {
        self.state
            .epoch
            .is_some_and(|epoch| epoch.phase == EpochPhase::Open)
    }

    #[must_use]
    pub fn current_epoch_id(&self) -> u64 {
        self.state.epoch.map_or(0, |epoch| epoch.id.raw())
    }

    #[must_use]
    pub fn current_cutoff_at(&self) -> u64 {
        self.state.epoch.map_or(0, |epoch| epoch.cutoff_at.raw())
    }

    #[must_use]
    pub fn is_active(&self) -> bool {
        self.state.vault_state == VaultState::Active
    }

    #[must_use]
    pub fn is_frozen(&self) -> bool {
        self.state.vault_state == VaultState::Frozen
    }

    #[must_use]
    pub fn active_deposit(&self, user: usize, epoch: u64) -> u128 {
        let key = RequestKey::new(EpochId::new(epoch), account_for(user as u8));
        self.state
            .deposit_requests
            .get(&key)
            .map_or(0, |request| request.assets.raw())
    }

    #[must_use]
    pub fn active_redeem(&self, user: usize, epoch: u64) -> u128 {
        let key = RequestKey::new(EpochId::new(epoch), account_for(user as u8));
        self.state
            .redeem_requests
            .get(&key)
            .map_or(0, |request| request.shares.raw())
    }

    /// Epochs that finalized, in settlement order.
    #[must_use]
    pub fn finalized_epochs(&self) -> Vec<u64> {
        self.state
            .epochs
            .iter()
            .filter(|(_, outcome)| outcome.finalized().is_some())
            .map(|(id, _)| id.raw())
            .collect()
    }

    #[must_use]
    pub fn aborted_epochs(&self) -> Vec<u64> {
        self.state
            .epochs
            .iter()
            .filter(|(_, outcome)| outcome.aborted().is_some())
            .map(|(id, _)| id.raw())
            .collect()
    }

    /// Applies one operation at the current clock and records the outcome.
    pub fn exec(&mut self, kind: ActionKind, actor: u8, amount: u128, epoch: u64) {
        let action = Action::new(kind, actor, amount, epoch, self.now);
        let expected_return = self.return_value(action);

        let (result, return_value) = match apply(&self.state, action.to_operation()) {
            Ok(next) => {
                self.state = next;
                (ResultCode::Success, expected_return)
            }
            Err(reason) => (code_for(kind, reason), 0),
        };

        self.scenario.actions.push(RecordedAction {
            action,
            result,
            return_value,
        });
        self.scenario.snapshots.push(Snapshot::capture(&self.state));
        self.note_settlements();
    }

    /// Convenience wrapper for operations that carry no epoch or amount.
    pub fn exec_simple(&mut self, kind: ActionKind, actor: u8) {
        self.exec(kind, actor, 0, 0);
    }

    /// Value the call returns when it succeeds. Read before the state moves.
    fn return_value(&self, action: Action) -> u128 {
        let account = account_for(action.actor);
        let key = RequestKey::new(EpochId::new(action.epoch), account);
        match action.kind {
            ActionKind::ClaimDeposit => self
                .state
                .finalized_terms(EpochId::new(action.epoch))
                .and_then(|terms| {
                    let request = self.state.deposit_requests.get(&key)?;
                    terms.shares_for(request.assets).ok()
                })
                .map_or(0, ShareAmount::raw),
            ActionKind::ClaimRedeem => self
                .state
                .finalized_terms(EpochId::new(action.epoch))
                .and_then(|terms| {
                    let request = self.state.redeem_requests.get(&key)?;
                    terms.assets_for(request.shares).ok()
                })
                .map_or(0, AssetAmount::raw),
            ActionKind::RefundDeposit => self
                .state
                .deposit_requests
                .get(&key)
                .map_or(0, |request| request.assets.raw()),
            ActionKind::RefundRedeem => self
                .state
                .redeem_requests
                .get(&key)
                .map_or(0, |request| request.shares.raw()),
            _ => 0,
        }
    }

    /// Remembers the step at which an epoch left the slot.
    fn note_settlements(&mut self) {
        let step = u32::try_from(self.scenario.actions.len().saturating_sub(1)).unwrap_or(u32::MAX);
        for id in self.state.epochs.keys() {
            self.settled_steps.entry(id.raw()).or_insert(step);
        }
    }

    /// Freezes the immutable epoch terms and returns the finished scenario.
    pub fn finish(mut self) -> Result<Scenario, BuildError> {
        let mut records = Vec::new();
        for (id, outcome) in &self.state.epochs {
            let settled_at_step = self.settled_steps.get(&id.raw()).copied().unwrap_or(0);
            let mut record = EpochRecord {
                epoch_id: id.raw(),
                outcome: OUTCOME_FINALIZED,
                settled_at_step,
                cutoff_at: 0,
                total_assets: 0,
                total_supply: 0,
                deposit_assets: 0,
                minted_shares: 0,
                redeem_shares: 0,
                redeem_assets: 0,
                deposit_dust: 0,
                redeem_dust: 0,
                actors: [EpochActorTerms::default(); USER_COUNT],
            };

            match outcome {
                EpochOutcome::Finalized(terms) => {
                    record.cutoff_at = terms.cutoff_at.raw();
                    record.total_assets = terms.total_assets.raw();
                    record.total_supply = terms.total_supply.raw();
                    record.deposit_assets = terms.deposit_assets.raw();
                    record.minted_shares = terms.minted_shares.raw();
                    record.redeem_shares = terms.redeem_shares.raw();
                    record.redeem_assets = terms.redeem_assets.raw();
                    record.deposit_dust = terms.deposit_dust.raw();
                    record.redeem_dust = terms.redeem_dust.raw();
                }
                EpochOutcome::Aborted(terms) => {
                    record.outcome = OUTCOME_ABORTED;
                    record.cutoff_at = terms.cutoff_at.raw();
                    record.deposit_assets = terms.refund_assets.raw();
                    record.redeem_shares = terms.refund_shares.raw();
                }
            }

            for user in 0..USER_COUNT {
                let account = account_for(user as u8);
                let key = RequestKey::new(*id, account);
                let deposit = self
                    .state
                    .deposit_requests
                    .get(&key)
                    .map_or(AssetAmount::ZERO, |request| request.assets);
                let redeem = self
                    .state
                    .redeem_requests
                    .get(&key)
                    .map_or(ShareAmount::ZERO, |request| request.shares);

                let mut entry = EpochActorTerms {
                    deposit_assets: deposit.raw(),
                    redeem_shares: redeem.raw(),
                    claim_shares: 0,
                    claim_assets: 0,
                };
                if let EpochOutcome::Finalized(terms) = outcome {
                    entry.claim_shares = terms
                        .shares_for(deposit)
                        .map_err(BuildError::Pricing)?
                        .raw();
                    entry.claim_assets =
                        terms.assets_for(redeem).map_err(BuildError::Pricing)?.raw();
                }
                if let Some(slot) = record.actors.get_mut(user) {
                    *slot = entry;
                }
            }

            records.push(record);
        }

        self.scenario.epochs = records;
        Ok(self.scenario)
    }
}
