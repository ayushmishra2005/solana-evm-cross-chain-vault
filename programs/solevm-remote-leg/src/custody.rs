//! The identity that ties custody tokens to the buckets that explain them.
//!
//! Custody may hold more than the leg has explained, because assets can arrive
//! before the message that authorises them. It may never hold less.

use anchor_lang::prelude::*;

use crate::errors::RemoteLegError;
use crate::strategy::RemotePosition;

/// Moves any unexplained custody surplus into the unattributed bucket.
///
/// It only classifies. No token ever moves here.
pub fn reconcile(position: &mut RemotePosition, custody_amount: u64) -> Result<u64> {
    let accounted = position.accounted_custody()?;
    require_gte!(custody_amount, accounted, RemoteLegError::AccountingDeficit);

    let surplus = custody_amount
        .checked_sub(accounted)
        .ok_or(RemoteLegError::AccountingDeficit)?;
    if surplus > 0 {
        position.unattributed_custody = position
            .unattributed_custody
            .checked_add(surplus)
            .ok_or(RemoteLegError::ArithmeticOverflow)?;
    }
    Ok(surplus)
}

/// Requires the buckets to add up to the custody balance exactly.
pub fn check_identity(position: &RemotePosition, custody_amount: u64) -> Result<()> {
    require_eq!(
        custody_amount,
        position.accounted_custody()?,
        RemoteLegError::AccountingDeficit
    );
    Ok(())
}

/// Requires the adapter to hold exactly the principal the leg deployed.
pub fn check_deployed(position: &RemotePosition, adapter_principal: u64) -> Result<()> {
    require_eq!(
        adapter_principal,
        position.deployed_principal,
        RemoteLegError::InvalidPrincipalDelta
    );
    Ok(())
}

/// Narrows a protocol amount to the token amount this chain uses.
pub fn narrow_amount(amount: u128) -> Result<u64> {
    u64::try_from(amount).map_err(|_| RemoteLegError::AmountTooLarge.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::strategy::{TransferKind, TransferStatus};

    fn position(attributed: u64, recalled: u64, unattributed: u64) -> RemotePosition {
        RemotePosition {
            state_version: crate::state::STATE_VERSION,
            bump: 1,
            attributed_principal: attributed,
            deployed_principal: 0,
            recalled_custody: recalled,
            unattributed_custody: unattributed,
            cumulative_realized_loss: 0,
            active_transfer_id: [0u8; 32],
            active_transfer_kind: TransferKind::None,
            active_transfer_sequence: 0,
            active_transfer_status: TransferStatus::None,
            latest_completed_transfer_id: [0u8; 32],
            latest_completion_at: 0,
            initialized_at: 0,
        }
    }

    #[test]
    fn a_balance_that_matches_the_buckets_has_no_surplus() {
        let mut state = position(10, 5, 3);
        assert_eq!(reconcile(&mut state, 18).expect("the call succeeds"), 0);
        assert_eq!(state.unattributed_custody, 3);
    }

    #[test]
    fn an_unexplained_balance_becomes_unattributed_custody() {
        let mut state = position(10, 5, 3);
        assert_eq!(reconcile(&mut state, 25).expect("the call succeeds"), 7);
        assert_eq!(state.unattributed_custody, 10);
    }

    #[test]
    fn a_surplus_never_becomes_principal() {
        let mut state = position(10, 5, 3);
        reconcile(&mut state, 100).expect("the call succeeds");
        assert_eq!(state.attributed_principal, 10);
        assert_eq!(state.deployed_principal, 0);
        assert_eq!(state.recalled_custody, 5);
    }

    #[test]
    fn a_balance_below_the_buckets_is_rejected() {
        let mut state = position(10, 5, 3);
        assert_eq!(
            reconcile(&mut state, 17).expect_err("the call fails"),
            Error::from(RemoteLegError::AccountingDeficit)
        );
    }

    #[test]
    fn a_rejected_reconciliation_leaves_the_buckets_alone() {
        let mut state = position(10, 5, 3);
        let _ = reconcile(&mut state, 0);
        assert_eq!(state.unattributed_custody, 3);
    }

    #[test]
    fn repeating_a_reconciliation_never_counts_the_same_token_twice() {
        let mut state = position(10, 5, 3);
        assert_eq!(reconcile(&mut state, 30).expect("the call succeeds"), 12);
        for _ in 0..5 {
            assert_eq!(reconcile(&mut state, 30).expect("the call succeeds"), 0);
        }
        assert_eq!(state.unattributed_custody, 15);
        assert_eq!(state.accounted_custody().expect("the call succeeds"), 30);
    }

    #[test]
    fn the_identity_holds_after_reconciliation() {
        let mut state = position(10, 5, 3);
        reconcile(&mut state, 40).expect("the call succeeds");
        assert!(check_identity(&state, 40).is_ok());
        assert_eq!(
            check_identity(&state, 41).expect_err("the call fails"),
            Error::from(RemoteLegError::AccountingDeficit)
        );
    }

    #[test]
    fn the_deployed_principal_must_match_the_adapter() {
        let mut state = position(0, 0, 0);
        state.deployed_principal = 500;
        assert!(check_deployed(&state, 500).is_ok());
        assert_eq!(
            check_deployed(&state, 499).expect_err("the call fails"),
            Error::from(RemoteLegError::InvalidPrincipalDelta)
        );
    }

    #[test]
    fn an_amount_inside_the_token_range_is_kept() {
        assert_eq!(
            narrow_amount(u128::from(u64::MAX)).expect("the call succeeds"),
            u64::MAX
        );
    }

    #[test]
    fn an_amount_above_the_token_range_is_rejected() {
        assert_eq!(
            narrow_amount(u128::from(u64::MAX) + 1).expect_err("the call fails"),
            Error::from(RemoteLegError::AmountTooLarge)
        );
    }
}
