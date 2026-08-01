use crate::amount::{AccountId, AssetAmount, EpochId, ShareAmount, Timestamp};
use crate::error::Rejection;
use crate::math;
use crate::operation::Operation;
use crate::request::RequestKey;
use crate::state::{AbortedTerms, Epoch, EpochOutcome, EpochPhase, EpochTerms, State, VaultState};

/// Applies one operation and returns the next state.
///
/// The input is never modified, so a rejection changes nothing.
pub fn apply(state: &State, operation: Operation) -> Result<State, Rejection> {
    let mut next = state.clone();
    match operation {
        Operation::RequestDeposit { account, assets } => {
            request_deposit(&mut next, account, assets)?;
        }
        Operation::CancelDeposit { account } => cancel_deposit(&mut next, account)?,
        Operation::RequestRedeem { account, shares } => {
            request_redeem(&mut next, account, shares)?;
        }
        Operation::CancelRedeem { account } => cancel_redeem(&mut next, account)?,
        Operation::CutoffEpoch { now } => cutoff_epoch(&mut next, now)?,
        Operation::FinalizeEpoch => finalize_epoch(&mut next)?,
        Operation::AbortEpoch { actor } => abort_epoch(&mut next, actor)?,
        Operation::ClaimDeposit { account, epoch } => claim_deposit(&mut next, account, epoch)?,
        Operation::ClaimRedeem { account, epoch } => claim_redeem(&mut next, account, epoch)?,
        Operation::ClaimAbortedDeposit { account, epoch } => {
            claim_aborted_deposit(&mut next, account, epoch)?;
        }
        Operation::ClaimAbortedRedeem { account, epoch } => {
            claim_aborted_redeem(&mut next, account, epoch)?;
        }
        Operation::OpenNextEpoch { now } => open_next_epoch(&mut next, now)?,
        Operation::Pause { actor } => pause(&mut next, actor)?,
        Operation::Unpause { actor } => unpause(&mut next, actor)?,
        Operation::Freeze { actor } => freeze(&mut next, actor)?,
    }
    Ok(next)
}

fn require_active(state: &State) -> Result<(), Rejection> {
    match state.vault_state {
        VaultState::Active => Ok(()),
        VaultState::Paused | VaultState::Frozen => Err(Rejection::InvalidVaultState),
    }
}

fn require_open_epoch(state: &State) -> Result<Epoch, Rejection> {
    let epoch = state.epoch.ok_or(Rejection::EpochNotOpen)?;
    match epoch.phase {
        EpochPhase::Open => Ok(epoch),
        EpochPhase::CutOff => Err(Rejection::EpochAlreadyCutOff),
    }
}

fn epoch_mut(state: &mut State) -> Result<&mut Epoch, Rejection> {
    state.epoch.as_mut().ok_or(Rejection::EpochNotOpen)
}

fn request_deposit(
    state: &mut State,
    account: AccountId,
    assets: AssetAmount,
) -> Result<(), Rejection> {
    require_active(state)?;
    let epoch = require_open_epoch(state)?;
    if assets.is_zero() {
        return Err(Rejection::ZeroAmount);
    }
    if assets < state.config.min_deposit_assets {
        return Err(Rejection::AmountBelowMinimum);
    }

    let key = RequestKey::new(epoch.id, account);
    if let Some(existing) = state.deposit_requests.get(&key)
        && existing.claimed
    {
        return Err(Rejection::RequestAlreadySettled);
    }

    let remaining = state
        .account(account)
        .assets
        .checked_sub(assets)
        .map_err(|_| Rejection::InsufficientAssetBalance)?;
    let escrow = state.buckets.pending_deposit_escrow.checked_add(assets)?;
    let epoch_total = epoch.pending_deposit_assets.checked_add(assets)?;

    state.account_mut(account).assets = remaining;
    state.buckets.pending_deposit_escrow = escrow;
    epoch_mut(state)?.pending_deposit_assets = epoch_total;
    let entry = state.deposit_requests.entry(key).or_default();
    entry.assets = entry.assets.checked_add(assets)?;
    entry.cancelled = false;
    Ok(())
}

fn cancel_deposit(state: &mut State, account: AccountId) -> Result<(), Rejection> {
    require_cancellation_allowed(state)?;
    let epoch = state.epoch.ok_or(Rejection::EpochNotOpen)?;
    if epoch.phase == EpochPhase::CutOff {
        return Err(Rejection::CancellationAfterCutoff);
    }

    let key = RequestKey::new(epoch.id, account);
    let request = *state
        .deposit_requests
        .get(&key)
        .ok_or(Rejection::RequestNotFound)?;
    if request.claimed {
        return Err(Rejection::RequestAlreadySettled);
    }
    if request.cancelled || request.assets.is_zero() {
        return Err(Rejection::RequestAlreadyCancelled);
    }

    let escrow = state
        .buckets
        .pending_deposit_escrow
        .checked_sub(request.assets)?;
    let epoch_total = epoch.pending_deposit_assets.checked_sub(request.assets)?;
    let returned = state.account(account).assets.checked_add(request.assets)?;
    let cancelled_total = request.cancelled_assets.checked_add(request.assets)?;

    state.buckets.pending_deposit_escrow = escrow;
    epoch_mut(state)?.pending_deposit_assets = epoch_total;
    state.account_mut(account).assets = returned;
    if let Some(entry) = state.deposit_requests.get_mut(&key) {
        entry.assets = AssetAmount::ZERO;
        entry.cancelled_assets = cancelled_total;
        entry.cancelled = true;
    }
    Ok(())
}

fn request_redeem(
    state: &mut State,
    account: AccountId,
    shares: ShareAmount,
) -> Result<(), Rejection> {
    require_active(state)?;
    let epoch = require_open_epoch(state)?;
    if shares.is_zero() {
        return Err(Rejection::ZeroAmount);
    }
    if shares < state.config.min_redeem_shares {
        return Err(Rejection::AmountBelowMinimum);
    }

    let key = RequestKey::new(epoch.id, account);
    if let Some(existing) = state.redeem_requests.get(&key)
        && existing.claimed
    {
        return Err(Rejection::RequestAlreadySettled);
    }

    let remaining = state
        .account(account)
        .shares
        .checked_sub(shares)
        .map_err(|_| Rejection::InsufficientShareBalance)?;
    let escrowed = state.escrowed_redeem_shares.checked_add(shares)?;
    let epoch_total = epoch.pending_redeem_shares.checked_add(shares)?;

    state.account_mut(account).shares = remaining;
    state.escrowed_redeem_shares = escrowed;
    epoch_mut(state)?.pending_redeem_shares = epoch_total;
    let entry = state.redeem_requests.entry(key).or_default();
    entry.shares = entry.shares.checked_add(shares)?;
    entry.cancelled = false;
    Ok(())
}

fn cancel_redeem(state: &mut State, account: AccountId) -> Result<(), Rejection> {
    require_cancellation_allowed(state)?;
    let epoch = state.epoch.ok_or(Rejection::EpochNotOpen)?;
    if epoch.phase == EpochPhase::CutOff {
        return Err(Rejection::CancellationAfterCutoff);
    }

    let key = RequestKey::new(epoch.id, account);
    let request = *state
        .redeem_requests
        .get(&key)
        .ok_or(Rejection::RequestNotFound)?;
    if request.claimed {
        return Err(Rejection::RequestAlreadySettled);
    }
    if request.cancelled || request.shares.is_zero() {
        return Err(Rejection::RequestAlreadyCancelled);
    }

    let escrowed = state.escrowed_redeem_shares.checked_sub(request.shares)?;
    let epoch_total = epoch.pending_redeem_shares.checked_sub(request.shares)?;
    let returned = state.account(account).shares.checked_add(request.shares)?;
    let cancelled_total = request.cancelled_shares.checked_add(request.shares)?;

    state.escrowed_redeem_shares = escrowed;
    epoch_mut(state)?.pending_redeem_shares = epoch_total;
    state.account_mut(account).shares = returned;
    if let Some(entry) = state.redeem_requests.get_mut(&key) {
        entry.shares = ShareAmount::ZERO;
        entry.cancelled_shares = cancelled_total;
        entry.cancelled = true;
    }
    Ok(())
}

/// Cancellation stays available while paused so users are never trapped.
fn require_cancellation_allowed(state: &State) -> Result<(), Rejection> {
    match state.vault_state {
        VaultState::Active | VaultState::Paused => Ok(()),
        VaultState::Frozen => Err(Rejection::InvalidVaultState),
    }
}

fn cutoff_epoch(state: &mut State, now: Timestamp) -> Result<(), Rejection> {
    require_active(state)?;
    let epoch = require_open_epoch(state)?;
    if now < epoch.cutoff_at {
        return Err(Rejection::CutoffNotReached);
    }
    epoch_mut(state)?.phase = EpochPhase::CutOff;
    Ok(())
}

fn finalize_epoch(state: &mut State) -> Result<(), Rejection> {
    require_active(state)?;
    let epoch = state.epoch.ok_or(Rejection::EpochNotOpen)?;
    if epoch.phase != EpochPhase::CutOff {
        return Err(Rejection::EpochNotCutOff);
    }
    if state.epochs.contains_key(&epoch.id) {
        return Err(Rejection::EpochAlreadyFinalized);
    }

    let basis = state.pre_settlement_basis(&epoch);
    let deposit_assets = epoch.pending_deposit_assets;
    let redeem_shares = epoch.pending_redeem_shares;
    let redeem_assets = math::shares_to_assets(redeem_shares, basis)?;
    let minted_shares = math::assets_to_shares(deposit_assets, basis)?;

    // Redemptions draw only on liquidity that existed before this epoch settles.
    let idle_after_redemptions = state
        .buckets
        .idle_backing
        .checked_sub(redeem_assets)
        .map_err(|_| Rejection::InsufficientRedemptionLiquidity)?;
    let idle_backing = idle_after_redemptions.checked_add(deposit_assets)?;
    let pending_deposit_escrow = state
        .buckets
        .pending_deposit_escrow
        .checked_sub(deposit_assets)?;
    let claim_reserve = state.buckets.claim_reserve.checked_add(redeem_assets)?;
    let total_share_supply = state
        .total_share_supply
        .checked_add(minted_shares)?
        .checked_sub(redeem_shares)?;
    let claimable_deposit_shares = state.claimable_deposit_shares.checked_add(minted_shares)?;
    let escrowed_redeem_shares = state.escrowed_redeem_shares.checked_sub(redeem_shares)?;
    let burned_redemption_shares = state.burned_redemption_shares.checked_add(redeem_shares)?;
    let next_epoch_id = epoch.id.next()?;

    let mut terms = EpochTerms {
        id: epoch.id,
        config_version: epoch.config_version,
        cutoff_at: epoch.cutoff_at,
        total_assets: basis.total_assets,
        total_supply: basis.total_supply,
        virtual_assets: basis.virtual_assets,
        virtual_shares: basis.virtual_shares,
        deposit_assets,
        minted_shares,
        redeem_shares,
        redeem_assets,
        deposit_dust: ShareAmount::ZERO,
        redeem_dust: AssetAmount::ZERO,
    };
    // Each claim rounds down, so the aggregate keeps a small remainder.
    let owed = state.entitlements(epoch.id, &terms)?;
    terms.deposit_dust = minted_shares.checked_sub(owed.deposit_shares)?;
    terms.redeem_dust = redeem_assets.checked_sub(owed.redeem_assets)?;

    state.buckets.idle_backing = idle_backing;
    state.buckets.pending_deposit_escrow = pending_deposit_escrow;
    state.buckets.claim_reserve = claim_reserve;
    state.total_share_supply = total_share_supply;
    state.claimable_deposit_shares = claimable_deposit_shares;
    state.escrowed_redeem_shares = escrowed_redeem_shares;
    state.burned_redemption_shares = burned_redemption_shares;
    state
        .epochs
        .insert(epoch.id, EpochOutcome::Finalized(terms));
    state.last_cutoff_at = epoch.cutoff_at;
    state.next_epoch_id = next_epoch_id;
    state.epoch = None;
    Ok(())
}

/// Abandons the current epoch so frozen funds keep a way out.
fn abort_epoch(state: &mut State, actor: AccountId) -> Result<(), Rejection> {
    require_guardian_or_admin(state, actor)?;
    if state.vault_state != VaultState::Frozen {
        return Err(Rejection::InvalidVaultState);
    }
    let epoch = state.epoch.ok_or(Rejection::EpochNotOpen)?;
    if state.epochs.contains_key(&epoch.id) {
        return Err(Rejection::EpochAlreadyFinalized);
    }

    let next_epoch_id = epoch.id.next()?;
    state.epochs.insert(
        epoch.id,
        EpochOutcome::Aborted(AbortedTerms {
            id: epoch.id,
            config_version: epoch.config_version,
            cutoff_at: epoch.cutoff_at,
            refund_assets: epoch.pending_deposit_assets,
            refund_shares: epoch.pending_redeem_shares,
        }),
    );
    state.last_cutoff_at = epoch.cutoff_at;
    state.next_epoch_id = next_epoch_id;
    state.epoch = None;
    Ok(())
}

fn claim_deposit(state: &mut State, account: AccountId, epoch: EpochId) -> Result<(), Rejection> {
    let terms = *state
        .finalized_terms(epoch)
        .ok_or(Rejection::EpochNotFinalized)?;
    let key = RequestKey::new(epoch, account);
    let request = *state
        .deposit_requests
        .get(&key)
        .ok_or(Rejection::RequestNotFound)?;
    if request.cancelled {
        return Err(Rejection::RequestAlreadyCancelled);
    }
    if request.claimed {
        return Err(Rejection::ClaimAlreadyConsumed);
    }
    if request.assets.is_zero() {
        return Err(Rejection::ClaimNotFound);
    }

    let shares = terms.shares_for(request.assets)?;
    let claimable = state.claimable_deposit_shares.checked_sub(shares)?;
    let credited = state.account(account).shares.checked_add(shares)?;

    state.claimable_deposit_shares = claimable;
    state.account_mut(account).shares = credited;
    if let Some(entry) = state.deposit_requests.get_mut(&key) {
        entry.claimed = true;
    }
    Ok(())
}

fn claim_redeem(state: &mut State, account: AccountId, epoch: EpochId) -> Result<(), Rejection> {
    let terms = *state
        .finalized_terms(epoch)
        .ok_or(Rejection::EpochNotFinalized)?;
    let key = RequestKey::new(epoch, account);
    let request = *state
        .redeem_requests
        .get(&key)
        .ok_or(Rejection::RequestNotFound)?;
    if request.cancelled {
        return Err(Rejection::RequestAlreadyCancelled);
    }
    if request.claimed {
        return Err(Rejection::ClaimAlreadyConsumed);
    }
    if request.shares.is_zero() {
        return Err(Rejection::ClaimNotFound);
    }

    let assets = terms.assets_for(request.shares)?;
    let reserve = state.buckets.claim_reserve.checked_sub(assets)?;
    let credited = state.account(account).assets.checked_add(assets)?;

    state.buckets.claim_reserve = reserve;
    state.account_mut(account).assets = credited;
    if let Some(entry) = state.redeem_requests.get_mut(&key) {
        entry.claimed = true;
    }
    Ok(())
}

fn claim_aborted_deposit(
    state: &mut State,
    account: AccountId,
    epoch: EpochId,
) -> Result<(), Rejection> {
    if state.aborted_terms(epoch).is_none() {
        return Err(Rejection::EpochNotAborted);
    }
    let key = RequestKey::new(epoch, account);
    let request = *state
        .deposit_requests
        .get(&key)
        .ok_or(Rejection::RequestNotFound)?;
    if request.cancelled {
        return Err(Rejection::RequestAlreadyCancelled);
    }
    if request.claimed {
        return Err(Rejection::ClaimAlreadyConsumed);
    }
    if request.assets.is_zero() {
        return Err(Rejection::ClaimNotFound);
    }

    let escrow = state
        .buckets
        .pending_deposit_escrow
        .checked_sub(request.assets)?;
    let returned = state.account(account).assets.checked_add(request.assets)?;

    state.buckets.pending_deposit_escrow = escrow;
    state.account_mut(account).assets = returned;
    if let Some(entry) = state.deposit_requests.get_mut(&key) {
        entry.claimed = true;
    }
    Ok(())
}

fn claim_aborted_redeem(
    state: &mut State,
    account: AccountId,
    epoch: EpochId,
) -> Result<(), Rejection> {
    if state.aborted_terms(epoch).is_none() {
        return Err(Rejection::EpochNotAborted);
    }
    let key = RequestKey::new(epoch, account);
    let request = *state
        .redeem_requests
        .get(&key)
        .ok_or(Rejection::RequestNotFound)?;
    if request.cancelled {
        return Err(Rejection::RequestAlreadyCancelled);
    }
    if request.claimed {
        return Err(Rejection::ClaimAlreadyConsumed);
    }
    if request.shares.is_zero() {
        return Err(Rejection::ClaimNotFound);
    }

    let escrowed = state.escrowed_redeem_shares.checked_sub(request.shares)?;
    let returned = state.account(account).shares.checked_add(request.shares)?;

    state.escrowed_redeem_shares = escrowed;
    state.account_mut(account).shares = returned;
    if let Some(entry) = state.redeem_requests.get_mut(&key) {
        entry.claimed = true;
    }
    Ok(())
}

fn open_next_epoch(state: &mut State, now: Timestamp) -> Result<(), Rejection> {
    require_active(state)?;
    if state.epoch.is_some() {
        return Err(Rejection::EpochAlreadyOpen);
    }
    if now < state.last_cutoff_at {
        return Err(Rejection::TimestampNotMonotonic);
    }

    let virtual_shares =
        math::virtual_shares_for(state.config.asset_decimals, state.config.share_decimals)?;
    let cutoff_at = now.checked_add_seconds(state.config.epoch_duration)?;
    state.epoch = Some(Epoch {
        id: state.next_epoch_id,
        opened_at: now,
        cutoff_at,
        config_version: state.config.version,
        asset_decimals: state.config.asset_decimals,
        share_decimals: state.config.share_decimals,
        virtual_assets: AssetAmount::new(1),
        virtual_shares,
        phase: EpochPhase::Open,
        pending_deposit_assets: AssetAmount::ZERO,
        pending_redeem_shares: ShareAmount::ZERO,
    });
    Ok(())
}

fn pause(state: &mut State, actor: AccountId) -> Result<(), Rejection> {
    require_guardian_or_admin(state, actor)?;
    if state.vault_state != VaultState::Active {
        return Err(Rejection::InvalidVaultState);
    }
    state.vault_state = VaultState::Paused;
    Ok(())
}

fn unpause(state: &mut State, actor: AccountId) -> Result<(), Rejection> {
    if actor != state.authority.admin {
        return Err(Rejection::UnauthorizedActor);
    }
    if state.vault_state != VaultState::Paused {
        return Err(Rejection::InvalidVaultState);
    }
    state.vault_state = VaultState::Active;
    Ok(())
}

fn freeze(state: &mut State, actor: AccountId) -> Result<(), Rejection> {
    require_guardian_or_admin(state, actor)?;
    if state.vault_state == VaultState::Frozen {
        return Err(Rejection::InvalidVaultState);
    }
    state.vault_state = VaultState::Frozen;
    Ok(())
}

fn require_guardian_or_admin(state: &State, actor: AccountId) -> Result<(), Rejection> {
    if actor == state.authority.admin || actor == state.authority.guardian {
        Ok(())
    } else {
        Err(Rejection::UnauthorizedActor)
    }
}
