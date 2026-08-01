use core::fmt;

use crate::amount::{AssetAmount, ShareAmount};
use crate::error::Rejection;
use crate::math;
use crate::state::{EpochTerms, State};

/// A broken accounting rule.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Violation {
    AssetConservation,
    ShareConservation,
    NavCompositionMismatch,
    EscrowAggregateMismatch,
    BurnedSharesMismatch,
    ClaimReserveNotExplained,
    ClaimableSharesNotExplained,
    RequestLifecycleInvalid,
    EpochAggregateMismatch,
    EpochTermsInconsistent,
    SettledEpochStillCurrent,
    AbortedRefundMismatch,
    DustOutOfBounds,
    PriceDecreased,
    ArithmeticFailure(Rejection),
}

impl fmt::Display for Violation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AssetConservation => formatter.write_str("assets are not conserved"),
            Self::ShareConservation => formatter.write_str("shares are not conserved"),
            Self::NavCompositionMismatch => {
                formatter.write_str("nav plus excluded buckets does not equal tracked assets")
            }
            Self::EscrowAggregateMismatch => {
                formatter.write_str("escrow does not match the requests behind it")
            }
            Self::BurnedSharesMismatch => {
                formatter.write_str("burned shares do not match finalized terms")
            }
            Self::ClaimReserveNotExplained => {
                formatter.write_str("claim reserve does not equal entitlement plus dust")
            }
            Self::ClaimableSharesNotExplained => {
                formatter.write_str("claimable shares do not equal entitlement plus dust")
            }
            Self::RequestLifecycleInvalid => {
                formatter.write_str("a request holds an impossible combination of flags")
            }
            Self::EpochAggregateMismatch => {
                formatter.write_str("an epoch aggregate does not match its requests")
            }
            Self::EpochTermsInconsistent => {
                formatter.write_str("finalized terms do not match their own price")
            }
            Self::SettledEpochStillCurrent => {
                formatter.write_str("an epoch with an outcome is still the current epoch")
            }
            Self::AbortedRefundMismatch => {
                formatter.write_str("aborted refunds do not match the requests behind them")
            }
            Self::DustOutOfBounds => {
                formatter.write_str("measured dust is negative or above its bound")
            }
            Self::PriceDecreased => formatter.write_str("settlement lowered the share price"),
            Self::ArithmeticFailure(reason) => {
                formatter.write_str("arithmetic failure while checking invariants: ")?;
                formatter.write_str(reason.message())
            }
        }
    }
}

impl core::error::Error for Violation {}

impl From<Rejection> for Violation {
    fn from(value: Rejection) -> Self {
        Self::ArithmeticFailure(value)
    }
}

/// Checks every state level accounting rule.
pub fn check_invariants(state: &State) -> Result<(), Violation> {
    check_asset_conservation(state)?;
    check_nav_composition(state)?;
    check_share_conservation(state)?;
    check_burned_shares(state)?;
    check_request_lifecycle(state)?;
    check_escrow_backing(state)?;
    check_claim_backing(state)?;
    check_finalized_terms(state)?;
    check_price_never_falls(state)?;
    Ok(())
}

fn check_asset_conservation(state: &State) -> Result<(), Violation> {
    let held = state.total_account_assets()?;
    let bucketed = state.total_bucket_assets()?;
    if held.checked_add(bucketed)? == state.initial_asset_supply {
        Ok(())
    } else {
        Err(Violation::AssetConservation)
    }
}

fn check_nav_composition(state: &State) -> Result<(), Violation> {
    let composed = state
        .managed_nav()
        .checked_add(state.excluded_from_nav()?)?;
    if composed == state.total_bucket_assets()? {
        Ok(())
    } else {
        Err(Violation::NavCompositionMismatch)
    }
}

fn check_share_conservation(state: &State) -> Result<(), Violation> {
    let accounted = state
        .total_account_shares()?
        .checked_add(state.escrowed_redeem_shares)?
        .checked_add(state.claimable_deposit_shares)?;
    if accounted == state.total_share_supply {
        Ok(())
    } else {
        Err(Violation::ShareConservation)
    }
}

fn check_burned_shares(state: &State) -> Result<(), Violation> {
    let mut burned = ShareAmount::ZERO;
    for outcome in state.epochs.values() {
        if let Some(terms) = outcome.finalized() {
            burned = burned.checked_add(terms.redeem_shares)?;
        }
    }
    if burned == state.burned_redemption_shares {
        Ok(())
    } else {
        Err(Violation::BurnedSharesMismatch)
    }
}

/// A cancelled request is empty, and no request holds two outcomes at once.
fn check_request_lifecycle(state: &State) -> Result<(), Violation> {
    for request in state.deposit_requests.values() {
        if !request.flags_agree() {
            return Err(Violation::RequestLifecycleInvalid);
        }
    }
    for request in state.redeem_requests.values() {
        if !request.flags_agree() {
            return Err(Violation::RequestLifecycleInvalid);
        }
    }
    Ok(())
}

/// Escrow holds the current epoch plus every refund an aborted epoch still owes.
fn check_escrow_backing(state: &State) -> Result<(), Violation> {
    let mut assets = AssetAmount::ZERO;
    let mut shares = ShareAmount::ZERO;

    if let Some(epoch) = state.epoch {
        if state.epochs.contains_key(&epoch.id) {
            return Err(Violation::SettledEpochStillCurrent);
        }
        let totals = state.request_totals(epoch.id)?;
        if totals.deposit_assets != epoch.pending_deposit_assets
            || totals.redeem_shares != epoch.pending_redeem_shares
        {
            return Err(Violation::EpochAggregateMismatch);
        }
        assets = totals.deposit_assets;
        shares = totals.redeem_shares;
    }

    for (id, outcome) in &state.epochs {
        let Some(aborted) = outcome.aborted() else {
            continue;
        };
        let totals = state.request_totals(*id)?;
        if totals.deposit_assets != aborted.refund_assets
            || totals.redeem_shares != aborted.refund_shares
        {
            return Err(Violation::AbortedRefundMismatch);
        }
        assets = assets.checked_add(totals.unclaimed_deposit_assets)?;
        shares = shares.checked_add(totals.unclaimed_redeem_shares)?;
    }

    if assets != state.buckets.pending_deposit_escrow || shares != state.escrowed_redeem_shares {
        return Err(Violation::EscrowAggregateMismatch);
    }
    Ok(())
}

/// Every unit of reserve and claimable shares is either owed or measured dust.
fn check_claim_backing(state: &State) -> Result<(), Violation> {
    let mut owed_shares = ShareAmount::ZERO;
    let mut owed_assets = AssetAmount::ZERO;

    for (id, outcome) in &state.epochs {
        let Some(terms) = outcome.finalized() else {
            continue;
        };
        let owed = state.entitlements(*id, terms)?;
        let totals = state.request_totals(*id)?;
        check_dust(
            terms.minted_shares.raw(),
            owed.deposit_shares.raw(),
            terms.deposit_dust.raw(),
            totals.deposit_count,
        )?;
        check_dust(
            terms.redeem_assets.raw(),
            owed.redeem_assets.raw(),
            terms.redeem_dust.raw(),
            totals.redeem_count,
        )?;
        owed_shares = owed_shares
            .checked_add(owed.unclaimed_deposit_shares)?
            .checked_add(terms.deposit_dust)?;
        owed_assets = owed_assets
            .checked_add(owed.unclaimed_redeem_assets)?
            .checked_add(terms.redeem_dust)?;
    }

    if owed_shares != state.claimable_deposit_shares {
        return Err(Violation::ClaimableSharesNotExplained);
    }
    if owed_assets != state.buckets.claim_reserve {
        return Err(Violation::ClaimReserveNotExplained);
    }
    Ok(())
}

/// Dust is the aggregate minus the sum of the parts, and one claim can lose at most one unit.
fn check_dust(aggregate: u128, parts: u128, recorded: u128, count: u128) -> Result<(), Violation> {
    if parts > aggregate {
        return Err(Violation::DustOutOfBounds);
    }
    let measured = aggregate
        .checked_sub(parts)
        .ok_or(Violation::DustOutOfBounds)?;
    if measured != recorded || measured > count.saturating_sub(1) {
        return Err(Violation::DustOutOfBounds);
    }
    Ok(())
}

fn check_finalized_terms(state: &State) -> Result<(), Violation> {
    for (id, outcome) in &state.epochs {
        let Some(terms) = outcome.finalized() else {
            continue;
        };
        if *id != terms.id {
            return Err(Violation::EpochTermsInconsistent);
        }
        if terms.shares_for(terms.deposit_assets)? != terms.minted_shares {
            return Err(Violation::EpochTermsInconsistent);
        }
        if terms.assets_for(terms.redeem_shares)? != terms.redeem_assets {
            return Err(Violation::EpochTermsInconsistent);
        }
        if terms.redeem_assets > terms.total_assets {
            return Err(Violation::EpochTermsInconsistent);
        }
        let totals = state.request_totals(*id)?;
        if totals.deposit_assets != terms.deposit_assets
            || totals.redeem_shares != terms.redeem_shares
        {
            return Err(Violation::EpochAggregateMismatch);
        }
    }
    Ok(())
}

/// Settlement rounds in favour of the vault, so the price can only rise.
fn check_price_never_falls(state: &State) -> Result<(), Violation> {
    let mut earlier: Option<&EpochTerms> = None;
    for outcome in state.epochs.values() {
        let Some(terms) = outcome.finalized() else {
            continue;
        };
        if let Some(previous) = earlier
            && !math::price_is_non_decreasing(previous.basis(), terms.basis())?
        {
            return Err(Violation::PriceDecreased);
        }
        earlier = Some(terms);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::ToString;

    const ALL: [Violation; 14] = [
        Violation::AssetConservation,
        Violation::ShareConservation,
        Violation::NavCompositionMismatch,
        Violation::EscrowAggregateMismatch,
        Violation::BurnedSharesMismatch,
        Violation::ClaimReserveNotExplained,
        Violation::ClaimableSharesNotExplained,
        Violation::RequestLifecycleInvalid,
        Violation::EpochAggregateMismatch,
        Violation::EpochTermsInconsistent,
        Violation::SettledEpochStillCurrent,
        Violation::AbortedRefundMismatch,
        Violation::DustOutOfBounds,
        Violation::PriceDecreased,
    ];

    #[test]
    fn every_violation_renders_a_distinct_message() {
        for (index, violation) in ALL.iter().enumerate() {
            let text = violation.to_string();
            assert!(!text.is_empty());
            for other in ALL.iter().skip(index.saturating_add(1)) {
                assert_ne!(text, other.to_string());
            }
        }
    }

    #[test]
    fn an_arithmetic_failure_reports_its_cause() {
        let violation = Violation::from(Rejection::ArithmeticOverflow);
        assert!(violation.to_string().contains("arithmetic overflow"));
    }

    #[test]
    fn dust_above_the_claim_count_is_refused() {
        assert_eq!(check_dust(10, 7, 3, 2), Err(Violation::DustOutOfBounds));
        assert_eq!(check_dust(10, 8, 2, 3), Ok(()));
    }

    #[test]
    fn dust_that_does_not_match_the_record_is_refused() {
        assert_eq!(check_dust(10, 8, 1, 5), Err(Violation::DustOutOfBounds));
    }

    #[test]
    fn parts_above_the_aggregate_are_refused() {
        assert_eq!(check_dust(10, 11, 0, 5), Err(Violation::DustOutOfBounds));
    }
}
