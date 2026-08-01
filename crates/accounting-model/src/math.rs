use ruint::aliases::{U256, U512};

use crate::amount::{AssetAmount, ShareAmount};
use crate::error::Rejection;

/// Pricing basis captured before any settlement changes the vault.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PricingBasis {
    pub total_assets: AssetAmount,
    pub total_supply: ShareAmount,
    pub virtual_assets: AssetAmount,
    pub virtual_shares: ShareAmount,
}

impl PricingBasis {
    fn asset_side(self) -> Result<u128, Rejection> {
        self.total_assets
            .checked_add(self.virtual_assets)
            .map(AssetAmount::raw)
    }

    fn share_side(self) -> Result<u128, Rejection> {
        self.total_supply
            .checked_add(self.virtual_shares)
            .map(ShareAmount::raw)
    }
}

/// Floor of `a * b / denominator` using a wide intermediate value.
pub fn mul_div_floor(a: u128, b: u128, denominator: u128) -> Result<u128, Rejection> {
    if denominator == 0 {
        return Err(Rejection::DivisionByZero);
    }
    let product = U256::from(a)
        .checked_mul(U256::from(b))
        .ok_or(Rejection::ArithmeticOverflow)?;
    let quotient = product
        .checked_div(U256::from(denominator))
        .ok_or(Rejection::DivisionByZero)?;
    u128::try_from(quotient).map_err(|_| Rejection::ArithmeticOverflow)
}

/// Converts assets to shares, rounding down.
pub fn assets_to_shares(
    assets: AssetAmount,
    basis: PricingBasis,
) -> Result<ShareAmount, Rejection> {
    let raw = mul_div_floor(assets.raw(), basis.share_side()?, basis.asset_side()?)?;
    Ok(ShareAmount::new(raw))
}

/// Converts shares to assets, rounding down.
pub fn shares_to_assets(
    shares: ShareAmount,
    basis: PricingBasis,
) -> Result<AssetAmount, Rejection> {
    let raw = mul_div_floor(shares.raw(), basis.asset_side()?, basis.share_side()?)?;
    Ok(AssetAmount::new(raw))
}

/// Compares two prices by cross multiplication, so no division is needed.
///
/// Returns true when the later price is at least the earlier price.
pub(crate) fn price_is_non_decreasing(
    earlier: PricingBasis,
    later: PricingBasis,
) -> Result<bool, Rejection> {
    let (earlier_assets, earlier_shares) = wide_sides(earlier);
    let (later_assets, later_shares) = wide_sides(later);
    let left = earlier_assets
        .checked_mul(later_shares)
        .ok_or(Rejection::ArithmeticOverflow)?;
    let right = later_assets
        .checked_mul(earlier_shares)
        .ok_or(Rejection::ArithmeticOverflow)?;
    Ok(left <= right)
}

/// Both sides of a price widened so the product cannot overflow.
fn wide_sides(basis: PricingBasis) -> (U512, U512) {
    let assets =
        U512::from(basis.total_assets.raw()).saturating_add(U512::from(basis.virtual_assets.raw()));
    let shares =
        U512::from(basis.total_supply.raw()).saturating_add(U512::from(basis.virtual_shares.raw()));
    (assets, shares)
}

/// Virtual share offset for the configured decimal difference.
pub(crate) fn virtual_shares_for(
    asset_decimals: u8,
    share_decimals: u8,
) -> Result<ShareAmount, Rejection> {
    let difference = share_decimals
        .checked_sub(asset_decimals)
        .ok_or(Rejection::InvalidConfiguration)?;
    let offset = 10u128
        .checked_pow(u32::from(difference))
        .ok_or(Rejection::InvalidConfiguration)?;
    Ok(ShareAmount::new(offset))
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;

    fn basis(total_assets: u128, total_supply: u128) -> PricingBasis {
        PricingBasis {
            total_assets: AssetAmount::new(total_assets),
            total_supply: ShareAmount::new(total_supply),
            virtual_assets: AssetAmount::new(1),
            virtual_shares: ShareAmount::new(1_000_000_000_000),
        }
    }

    #[test]
    fn empty_vault_prices_deposits_at_the_virtual_offset() {
        let shares = assets_to_shares(AssetAmount::new(1_000_000), basis(0, 0));
        assert_eq!(shares, Ok(ShareAmount::new(1_000_000_000_000_000_000)));
    }

    #[test]
    fn conversion_rounds_down_in_both_directions() {
        let shares = assets_to_shares(AssetAmount::new(1), basis(2, 0));
        assert_eq!(shares, Ok(ShareAmount::new(333_333_333_333)));
        let assets = shares_to_assets(ShareAmount::new(1), basis(2, 0));
        assert_eq!(assets, Ok(AssetAmount::new(0)));
    }

    #[test]
    fn round_trip_never_returns_more_than_it_started_with() {
        let start = AssetAmount::new(1_234_567);
        let basis = basis(9_999_999, 3_141_592_653_589);
        let shares = assets_to_shares(start, basis).expect("conversion");
        let back = shares_to_assets(shares, basis).expect("conversion");
        assert!(back <= start);
    }

    #[test]
    fn wide_intermediate_avoids_overflow_on_large_inputs() {
        let result = mul_div_floor(u128::MAX, u128::MAX, u128::MAX);
        assert_eq!(result, Ok(u128::MAX));
    }

    #[test]
    fn result_larger_than_the_output_type_is_rejected() {
        let result = mul_div_floor(u128::MAX, u128::MAX, 1);
        assert_eq!(result, Err(Rejection::ArithmeticOverflow));
    }

    #[test]
    fn division_by_zero_is_rejected() {
        assert_eq!(mul_div_floor(1, 1, 0), Err(Rejection::DivisionByZero));
    }

    #[test]
    fn virtual_share_offset_matches_the_decimal_difference() {
        assert_eq!(
            virtual_shares_for(6, 18),
            Ok(ShareAmount::new(1_000_000_000_000))
        );
        assert_eq!(
            virtual_shares_for(18, 6),
            Err(Rejection::InvalidConfiguration)
        );
    }
}
