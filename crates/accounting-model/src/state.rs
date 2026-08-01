use alloc::collections::BTreeMap;
use alloc::vec::Vec;

use crate::amount::{AccountId, AssetAmount, ConfigVersion, EpochId, ShareAmount, Timestamp};
use crate::error::Rejection;
use crate::math::{self, PricingBasis};
use crate::request::{DepositRequest, RedeemRequest, RequestKey, RequestState};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VaultState {
    Active,
    Paused,
    Frozen,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EpochPhase {
    Open,
    CutOff,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Config {
    pub version: ConfigVersion,
    pub asset_decimals: u8,
    pub share_decimals: u8,
    pub min_deposit_assets: AssetAmount,
    pub min_redeem_shares: ShareAmount,
    pub epoch_duration: u64,
}

impl Config {
    fn validate(&self) -> Result<(), Rejection> {
        if self.share_decimals < self.asset_decimals || self.share_decimals > 38 {
            return Err(Rejection::InvalidConfiguration);
        }
        if self.epoch_duration == 0 {
            return Err(Rejection::InvalidConfiguration);
        }
        if self.min_deposit_assets.is_zero() || self.min_redeem_shares.is_zero() {
            return Err(Rejection::InvalidConfiguration);
        }
        Ok(())
    }
}

/// Administrative roles. Role transfer is out of scope for this milestone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Authority {
    pub admin: AccountId,
    pub guardian: AccountId,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Account {
    pub assets: AssetAmount,
    pub shares: ShareAmount,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Buckets {
    pub pending_deposit_escrow: AssetAmount,
    pub idle_backing: AssetAmount,
    pub claim_reserve: AssetAmount,
    pub unattributed_balance: AssetAmount,
}

impl Buckets {
    fn total(&self) -> Result<AssetAmount, Rejection> {
        self.pending_deposit_escrow
            .checked_add(self.idle_backing)?
            .checked_add(self.claim_reserve)?
            .checked_add(self.unattributed_balance)
    }

    /// Buckets that never back the share price.
    fn excluded_from_nav(&self) -> Result<AssetAmount, Rejection> {
        self.pending_deposit_escrow
            .checked_add(self.claim_reserve)?
            .checked_add(self.unattributed_balance)
    }
}

/// The single epoch that is open or cut off but not yet settled.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Epoch {
    pub id: EpochId,
    pub opened_at: Timestamp,
    pub cutoff_at: Timestamp,
    pub config_version: ConfigVersion,
    pub asset_decimals: u8,
    pub share_decimals: u8,
    pub virtual_assets: AssetAmount,
    pub virtual_shares: ShareAmount,
    pub phase: EpochPhase,
    pub pending_deposit_assets: AssetAmount,
    pub pending_redeem_shares: ShareAmount,
}

/// Immutable settlement terms of a finalized epoch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EpochTerms {
    pub id: EpochId,
    pub config_version: ConfigVersion,
    pub cutoff_at: Timestamp,
    pub total_assets: AssetAmount,
    pub total_supply: ShareAmount,
    pub virtual_assets: AssetAmount,
    pub virtual_shares: ShareAmount,
    pub deposit_assets: AssetAmount,
    pub minted_shares: ShareAmount,
    pub redeem_shares: ShareAmount,
    pub redeem_assets: AssetAmount,
    /// Minted shares that no single claim can reach, because each claim rounds down.
    pub deposit_dust: ShareAmount,
    /// Reserved assets that no single claim can reach, for the same reason.
    pub redeem_dust: AssetAmount,
}

impl EpochTerms {
    #[must_use]
    pub const fn basis(&self) -> PricingBasis {
        PricingBasis {
            total_assets: self.total_assets,
            total_supply: self.total_supply,
            virtual_assets: self.virtual_assets,
            virtual_shares: self.virtual_shares,
        }
    }

    /// Shares owed for a settled deposit amount.
    pub fn shares_for(&self, assets: AssetAmount) -> Result<ShareAmount, Rejection> {
        math::assets_to_shares(assets, self.basis())
    }

    /// Assets owed for a settled redemption amount.
    pub fn assets_for(&self, shares: ShareAmount) -> Result<AssetAmount, Rejection> {
        math::shares_to_assets(shares, self.basis())
    }
}

/// Immutable record of an epoch that was abandoned while frozen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AbortedTerms {
    pub id: EpochId,
    pub config_version: ConfigVersion,
    pub cutoff_at: Timestamp,
    pub refund_assets: AssetAmount,
    pub refund_shares: ShareAmount,
}

/// How an epoch left the active slot. An epoch has exactly one outcome.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EpochOutcome {
    Finalized(EpochTerms),
    Aborted(AbortedTerms),
}

impl EpochOutcome {
    #[must_use]
    pub const fn finalized(&self) -> Option<&EpochTerms> {
        match self {
            Self::Finalized(terms) => Some(terms),
            Self::Aborted(_) => None,
        }
    }

    #[must_use]
    pub const fn aborted(&self) -> Option<&AbortedTerms> {
        match self {
            Self::Aborted(terms) => Some(terms),
            Self::Finalized(_) => None,
        }
    }
}

/// Genesis inputs for a fresh model instance.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Genesis {
    pub config: Config,
    pub authority: Authority,
    pub accounts: Vec<(AccountId, AssetAmount)>,
    pub unattributed_balance: AssetAmount,
    pub opened_at: Timestamp,
}

/// Amounts requested in one epoch, derived from the request records.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RequestTotals {
    pub deposit_assets: AssetAmount,
    pub redeem_shares: ShareAmount,
    pub unclaimed_deposit_assets: AssetAmount,
    pub unclaimed_redeem_shares: ShareAmount,
    pub deposit_count: u128,
    pub redeem_count: u128,
}

/// Entitlements a finalized epoch owes, derived from the request records.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Entitlements {
    pub deposit_shares: ShareAmount,
    pub unclaimed_deposit_shares: ShareAmount,
    pub redeem_assets: AssetAmount,
    pub unclaimed_redeem_assets: AssetAmount,
}

/// Complete canonical accounting state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct State {
    pub config: Config,
    pub authority: Authority,
    pub vault_state: VaultState,
    pub epoch: Option<Epoch>,
    pub next_epoch_id: EpochId,
    pub last_cutoff_at: Timestamp,
    pub epochs: BTreeMap<EpochId, EpochOutcome>,
    pub accounts: BTreeMap<AccountId, Account>,
    pub deposit_requests: BTreeMap<RequestKey, DepositRequest>,
    pub redeem_requests: BTreeMap<RequestKey, RedeemRequest>,
    pub buckets: Buckets,
    pub total_share_supply: ShareAmount,
    pub escrowed_redeem_shares: ShareAmount,
    pub claimable_deposit_shares: ShareAmount,
    pub burned_redemption_shares: ShareAmount,
    pub initial_asset_supply: AssetAmount,
}

impl State {
    pub fn new(genesis: Genesis) -> Result<Self, Rejection> {
        genesis.config.validate()?;
        let virtual_shares =
            math::virtual_shares_for(genesis.config.asset_decimals, genesis.config.share_decimals)?;
        let virtual_assets = AssetAmount::new(1);

        let mut accounts = BTreeMap::new();
        let mut supply = genesis.unattributed_balance;
        for (id, assets) in genesis.accounts {
            let account = Account {
                assets,
                shares: ShareAmount::ZERO,
            };
            if accounts.insert(id, account).is_some() {
                return Err(Rejection::DuplicateAccount);
            }
            supply = supply.checked_add(assets)?;
        }

        let cutoff_at = genesis
            .opened_at
            .checked_add_seconds(genesis.config.epoch_duration)?;

        Ok(Self {
            config: genesis.config,
            authority: genesis.authority,
            vault_state: VaultState::Active,
            epoch: Some(Epoch {
                id: EpochId::GENESIS,
                opened_at: genesis.opened_at,
                cutoff_at,
                config_version: genesis.config.version,
                asset_decimals: genesis.config.asset_decimals,
                share_decimals: genesis.config.share_decimals,
                virtual_assets,
                virtual_shares,
                phase: EpochPhase::Open,
                pending_deposit_assets: AssetAmount::ZERO,
                pending_redeem_shares: ShareAmount::ZERO,
            }),
            next_epoch_id: EpochId::GENESIS.next()?,
            last_cutoff_at: genesis.opened_at,
            epochs: BTreeMap::new(),
            accounts,
            deposit_requests: BTreeMap::new(),
            redeem_requests: BTreeMap::new(),
            buckets: Buckets {
                unattributed_balance: genesis.unattributed_balance,
                ..Buckets::default()
            },
            total_share_supply: ShareAmount::ZERO,
            escrowed_redeem_shares: ShareAmount::ZERO,
            claimable_deposit_shares: ShareAmount::ZERO,
            burned_redemption_shares: ShareAmount::ZERO,
            initial_asset_supply: supply,
        })
    }

    /// Assets that back the share price. Escrow and reserves are excluded.
    #[must_use]
    pub const fn managed_nav(&self) -> AssetAmount {
        self.buckets.idle_backing
    }

    #[must_use]
    pub fn account(&self, id: AccountId) -> Account {
        self.accounts.get(&id).copied().unwrap_or_default()
    }

    #[must_use]
    pub fn finalized_terms(&self, id: EpochId) -> Option<&EpochTerms> {
        self.epochs.get(&id).and_then(EpochOutcome::finalized)
    }

    #[must_use]
    pub fn aborted_terms(&self, id: EpochId) -> Option<&AbortedTerms> {
        self.epochs.get(&id).and_then(EpochOutcome::aborted)
    }

    pub(crate) fn account_mut(&mut self, id: AccountId) -> &mut Account {
        self.accounts.entry(id).or_default()
    }

    pub(crate) fn total_bucket_assets(&self) -> Result<AssetAmount, Rejection> {
        self.buckets.total()
    }

    pub(crate) fn excluded_from_nav(&self) -> Result<AssetAmount, Rejection> {
        self.buckets.excluded_from_nav()
    }

    pub(crate) fn total_account_assets(&self) -> Result<AssetAmount, Rejection> {
        let mut total = AssetAmount::ZERO;
        for account in self.accounts.values() {
            total = total.checked_add(account.assets)?;
        }
        Ok(total)
    }

    pub(crate) fn total_account_shares(&self) -> Result<ShareAmount, Rejection> {
        let mut total = ShareAmount::ZERO;
        for account in self.accounts.values() {
            total = total.checked_add(account.shares)?;
        }
        Ok(total)
    }

    /// Pricing basis taken before any settlement of the current epoch.
    pub(crate) fn pre_settlement_basis(&self, epoch: &Epoch) -> PricingBasis {
        PricingBasis {
            total_assets: self.managed_nav(),
            total_supply: self.total_share_supply,
            virtual_assets: epoch.virtual_assets,
            virtual_shares: epoch.virtual_shares,
        }
    }

    /// Sums the request records that belong to one epoch.
    pub fn request_totals(&self, epoch: EpochId) -> Result<RequestTotals, Rejection> {
        let mut totals = RequestTotals::default();
        for (_, request) in self.deposit_requests.range(RequestKey::epoch_range(epoch)) {
            if !request.is_active() {
                continue;
            }
            totals.deposit_assets = totals.deposit_assets.checked_add(request.assets)?;
            totals.deposit_count = totals.deposit_count.saturating_add(1);
            if !request.claimed {
                totals.unclaimed_deposit_assets = totals
                    .unclaimed_deposit_assets
                    .checked_add(request.assets)?;
            }
        }
        for (_, request) in self.redeem_requests.range(RequestKey::epoch_range(epoch)) {
            if !request.is_active() {
                continue;
            }
            totals.redeem_shares = totals.redeem_shares.checked_add(request.shares)?;
            totals.redeem_count = totals.redeem_count.saturating_add(1);
            if !request.claimed {
                totals.unclaimed_redeem_shares =
                    totals.unclaimed_redeem_shares.checked_add(request.shares)?;
            }
        }
        Ok(totals)
    }

    /// Prices every request of one epoch against the given terms.
    pub fn entitlements(
        &self,
        epoch: EpochId,
        terms: &EpochTerms,
    ) -> Result<Entitlements, Rejection> {
        let mut owed = Entitlements::default();
        for (_, request) in self.deposit_requests.range(RequestKey::epoch_range(epoch)) {
            if !request.is_active() {
                continue;
            }
            let shares = terms.shares_for(request.assets)?;
            owed.deposit_shares = owed.deposit_shares.checked_add(shares)?;
            if !request.claimed {
                owed.unclaimed_deposit_shares =
                    owed.unclaimed_deposit_shares.checked_add(shares)?;
            }
        }
        for (_, request) in self.redeem_requests.range(RequestKey::epoch_range(epoch)) {
            if !request.is_active() {
                continue;
            }
            let assets = terms.assets_for(request.shares)?;
            owed.redeem_assets = owed.redeem_assets.checked_add(assets)?;
            if !request.claimed {
                owed.unclaimed_redeem_assets = owed.unclaimed_redeem_assets.checked_add(assets)?;
            }
        }
        Ok(owed)
    }

    #[must_use]
    pub fn deposit_request_state(&self, key: RequestKey) -> Option<RequestState> {
        let request = self.deposit_requests.get(&key)?;
        Some(self.lifecycle(key.epoch, request.cancelled, request.claimed))
    }

    #[must_use]
    pub fn redeem_request_state(&self, key: RequestKey) -> Option<RequestState> {
        let request = self.redeem_requests.get(&key)?;
        Some(self.lifecycle(key.epoch, request.cancelled, request.claimed))
    }

    fn lifecycle(&self, epoch: EpochId, cancelled: bool, claimed: bool) -> RequestState {
        if cancelled {
            return RequestState::Cancelled;
        }
        match self.epochs.get(&epoch) {
            Some(EpochOutcome::Finalized(_)) if claimed => return RequestState::Claimed,
            Some(EpochOutcome::Finalized(_)) => return RequestState::Claimable,
            Some(EpochOutcome::Aborted(_)) if claimed => return RequestState::Refunded,
            Some(EpochOutcome::Aborted(_)) => return RequestState::Refundable,
            None => {}
        }
        match self.epoch {
            Some(current) if current.id == epoch && current.phase == EpochPhase::CutOff => {
                RequestState::Locked
            }
            _ => RequestState::Pending,
        }
    }

    /// Assets still owed to holders of finalized redemption claims.
    pub fn outstanding_redeem_assets(&self) -> Result<AssetAmount, Rejection> {
        let mut total = AssetAmount::ZERO;
        for (id, outcome) in &self.epochs {
            let Some(terms) = outcome.finalized() else {
                continue;
            };
            total = total.checked_add(self.entitlements(*id, terms)?.unclaimed_redeem_assets)?;
        }
        Ok(total)
    }

    /// Shares still owed to holders of finalized deposit claims.
    pub fn outstanding_deposit_shares(&self) -> Result<ShareAmount, Rejection> {
        let mut total = ShareAmount::ZERO;
        for (id, outcome) in &self.epochs {
            let Some(terms) = outcome.finalized() else {
                continue;
            };
            total = total.checked_add(self.entitlements(*id, terms)?.unclaimed_deposit_shares)?;
        }
        Ok(total)
    }

    /// Assets still refundable from epochs that were aborted.
    pub fn outstanding_refund_assets(&self) -> Result<AssetAmount, Rejection> {
        let mut total = AssetAmount::ZERO;
        for (id, outcome) in &self.epochs {
            if outcome.aborted().is_none() {
                continue;
            }
            total = total.checked_add(self.request_totals(*id)?.unclaimed_deposit_assets)?;
        }
        Ok(total)
    }

    /// Shares still refundable from epochs that were aborted.
    pub fn outstanding_refund_shares(&self) -> Result<ShareAmount, Rejection> {
        let mut total = ShareAmount::ZERO;
        for (id, outcome) in &self.epochs {
            if outcome.aborted().is_none() {
                continue;
            }
            total = total.checked_add(self.request_totals(*id)?.unclaimed_redeem_shares)?;
        }
        Ok(total)
    }
}
