use accounting_model::{EpochId, EpochPhase, RequestKey, State, VaultState};

use crate::action::{USER_COUNT, account_for};

/// Highest epoch the consumption mask can address. Sixteen epochs times four
/// users times two request kinds fills the mask exactly.
pub const MAX_TRACKED_EPOCHS: u64 = 16;

/// Observable state the harness compares after every operation.
///
/// Only mutable state lives here. Settled epoch terms are immutable once
/// written, so they travel once per scenario instead.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Snapshot {
    pub pending_deposit_escrow: u128,
    pub idle_backing: u128,
    pub claim_reserve: u128,
    pub unattributed_balance: u128,
    pub total_supply: u128,
    pub vault_share_balance: u128,
    pub vault_asset_balance: u128,
    pub status: u8,
    pub epoch_open: bool,
    pub epoch_phase: u8,
    pub epoch_id: u64,
    pub next_epoch_id: u64,
    pub epoch_deposit_assets: u128,
    pub epoch_redeem_shares: u128,
    pub consumed_mask: u128,
    pub actor_assets: [u128; USER_COUNT],
    pub actor_shares: [u128; USER_COUNT],
    pub actor_deposit_assets: [u128; USER_COUNT],
    pub actor_redeem_shares: [u128; USER_COUNT],
}

/// Bit position of one consumption flag inside the mask.
#[must_use]
pub fn consumed_bit(epoch: u64, user: usize, redeem: bool) -> Option<u32> {
    if epoch >= MAX_TRACKED_EPOCHS || user >= USER_COUNT {
        return None;
    }
    let kind = u64::from(redeem);
    let index = epoch * 8 + (user as u64) * 2 + kind;
    u32::try_from(index).ok()
}

/// Id the next epoch will receive.
///
/// The model advances its counter when an epoch settles, so while an epoch is
/// open the stored value still names that open epoch. The vault stores the same
/// meaning one step earlier. Both give the next epoch the same id, so the
/// harness compares the meaning rather than the stored field.
fn next_epoch_id(state: &State) -> u64 {
    state.epoch.map_or_else(
        || state.next_epoch_id.raw(),
        |epoch| epoch.id.raw().saturating_add(1),
    )
}

impl Snapshot {
    /// Reads everything the Solidity side can also observe.
    #[must_use]
    pub fn capture(state: &State) -> Self {
        let mut shot = Self {
            pending_deposit_escrow: state.buckets.pending_deposit_escrow.raw(),
            idle_backing: state.buckets.idle_backing.raw(),
            claim_reserve: state.buckets.claim_reserve.raw(),
            unattributed_balance: state.buckets.unattributed_balance.raw(),
            total_supply: state.total_share_supply.raw(),
            vault_share_balance: state
                .escrowed_redeem_shares
                .raw()
                .saturating_add(state.claimable_deposit_shares.raw()),
            vault_asset_balance: state
                .buckets
                .pending_deposit_escrow
                .raw()
                .saturating_add(state.buckets.idle_backing.raw())
                .saturating_add(state.buckets.claim_reserve.raw())
                .saturating_add(state.buckets.unattributed_balance.raw()),
            status: match state.vault_state {
                VaultState::Active => 0,
                VaultState::Paused => 1,
                VaultState::Frozen => 2,
            },
            epoch_open: state.epoch.is_some(),
            epoch_phase: 0,
            epoch_id: 0,
            next_epoch_id: next_epoch_id(state),
            epoch_deposit_assets: 0,
            epoch_redeem_shares: 0,
            consumed_mask: 0,
            actor_assets: [0; USER_COUNT],
            actor_shares: [0; USER_COUNT],
            actor_deposit_assets: [0; USER_COUNT],
            actor_redeem_shares: [0; USER_COUNT],
        };

        if let Some(epoch) = state.epoch {
            shot.epoch_id = epoch.id.raw();
            shot.epoch_phase = match epoch.phase {
                EpochPhase::Open => 0,
                EpochPhase::CutOff => 1,
            };
            shot.epoch_deposit_assets = epoch.pending_deposit_assets.raw();
            shot.epoch_redeem_shares = epoch.pending_redeem_shares.raw();
        }

        for user in 0..USER_COUNT {
            let account = account_for(user as u8);
            let holding = state.account(account);
            if let Some(slot) = shot.actor_assets.get_mut(user) {
                *slot = holding.assets.raw();
            }
            if let Some(slot) = shot.actor_shares.get_mut(user) {
                *slot = holding.shares.raw();
            }

            // Requests only exist against the epoch that still holds the slot.
            if let Some(epoch) = state.epoch {
                let key = RequestKey::new(epoch.id, account);
                let deposit = state
                    .deposit_requests
                    .get(&key)
                    .map_or(0, |request| request.assets.raw());
                let redeem = state
                    .redeem_requests
                    .get(&key)
                    .map_or(0, |request| request.shares.raw());
                if let Some(slot) = shot.actor_deposit_assets.get_mut(user) {
                    *slot = deposit;
                }
                if let Some(slot) = shot.actor_redeem_shares.get_mut(user) {
                    *slot = redeem;
                }
            }

            for epoch in 0..MAX_TRACKED_EPOCHS {
                let key = RequestKey::new(EpochId::new(epoch), account);
                if state
                    .deposit_requests
                    .get(&key)
                    .is_some_and(|request| request.claimed)
                    && let Some(bit) = consumed_bit(epoch, user, false)
                {
                    shot.consumed_mask |= 1u128 << bit;
                }
                if state
                    .redeem_requests
                    .get(&key)
                    .is_some_and(|request| request.claimed)
                    && let Some(bit) = consumed_bit(epoch, user, true)
                {
                    shot.consumed_mask |= 1u128 << bit;
                }
            }
        }

        shot
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn consumption_bits_never_collide() {
        let mut seen = Vec::new();
        for epoch in 0..8 {
            for user in 0..USER_COUNT {
                for redeem in [false, true] {
                    let bit = consumed_bit(epoch, user, redeem).unwrap_or(u32::MAX);
                    assert!(!seen.contains(&bit));
                    seen.push(bit);
                }
            }
        }
    }

    #[test]
    fn out_of_range_positions_have_no_bit() {
        assert!(consumed_bit(MAX_TRACKED_EPOCHS, 0, false).is_none());
        assert!(consumed_bit(0, USER_COUNT, false).is_none());
    }

    #[test]
    fn bits_stay_inside_the_mask_width() {
        for epoch in 0..16 {
            for user in 0..USER_COUNT {
                let bit = consumed_bit(epoch, user, true).unwrap_or(u32::MAX);
                assert!(bit < 128);
            }
        }
    }
}
