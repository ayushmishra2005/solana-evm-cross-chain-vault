use core::fmt;

/// Reason a transition was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum Rejection {
    InvalidVaultState,
    EpochNotOpen,
    EpochAlreadyOpen,
    CutoffNotReached,
    EpochAlreadyCutOff,
    EpochNotCutOff,
    EpochAlreadyFinalized,
    EpochNotFinalized,
    EpochNotAborted,
    RequestNotFound,
    RequestAlreadyCancelled,
    RequestAlreadySettled,
    ClaimNotFound,
    ClaimAlreadyConsumed,
    CancellationAfterCutoff,
    ZeroAmount,
    AmountBelowMinimum,
    InsufficientAssetBalance,
    InsufficientShareBalance,
    InsufficientRedemptionLiquidity,
    ArithmeticOverflow,
    ArithmeticUnderflow,
    DivisionByZero,
    UnauthorizedActor,
    InvalidConfiguration,
    DuplicateAccount,
    TimestampNotMonotonic,
}

impl Rejection {
    #[must_use]
    pub const fn message(self) -> &'static str {
        match self {
            Self::InvalidVaultState => "operation is not allowed in the current vault state",
            Self::EpochNotOpen => "no epoch is open",
            Self::EpochAlreadyOpen => "an epoch is already open",
            Self::CutoffNotReached => "cutoff timestamp has not been reached",
            Self::EpochAlreadyCutOff => "epoch is already cut off",
            Self::EpochNotCutOff => "epoch has not been cut off",
            Self::EpochAlreadyFinalized => "epoch already has a settled outcome",
            Self::EpochNotFinalized => "epoch is not finalized",
            Self::EpochNotAborted => "epoch is not aborted",
            Self::RequestNotFound => "request does not exist",
            Self::RequestAlreadyCancelled => "request is already cancelled",
            Self::RequestAlreadySettled => "request is already settled",
            Self::ClaimNotFound => "claim does not exist",
            Self::ClaimAlreadyConsumed => "claim is already consumed",
            Self::CancellationAfterCutoff => "cancellation is not allowed after cutoff",
            Self::ZeroAmount => "amount must be greater than zero",
            Self::AmountBelowMinimum => "amount is below the configured minimum",
            Self::InsufficientAssetBalance => "asset balance is too low",
            Self::InsufficientShareBalance => "share balance is too low",
            Self::InsufficientRedemptionLiquidity => "redemption liquidity is too low",
            Self::ArithmeticOverflow => "arithmetic overflow",
            Self::ArithmeticUnderflow => "arithmetic underflow",
            Self::DivisionByZero => "division by zero",
            Self::UnauthorizedActor => "actor is not allowed to perform this operation",
            Self::InvalidConfiguration => "configuration is invalid",
            Self::DuplicateAccount => "an account appears more than once",
            Self::TimestampNotMonotonic => "timestamp moves backwards",
        }
    }
}

impl fmt::Display for Rejection {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.message())
    }
}

impl core::error::Error for Rejection {}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::ToString;

    const ALL: [Rejection; 27] = [
        Rejection::InvalidVaultState,
        Rejection::EpochNotOpen,
        Rejection::EpochAlreadyOpen,
        Rejection::CutoffNotReached,
        Rejection::EpochAlreadyCutOff,
        Rejection::EpochNotCutOff,
        Rejection::EpochAlreadyFinalized,
        Rejection::EpochNotFinalized,
        Rejection::EpochNotAborted,
        Rejection::RequestNotFound,
        Rejection::RequestAlreadyCancelled,
        Rejection::RequestAlreadySettled,
        Rejection::ClaimNotFound,
        Rejection::ClaimAlreadyConsumed,
        Rejection::CancellationAfterCutoff,
        Rejection::ZeroAmount,
        Rejection::AmountBelowMinimum,
        Rejection::InsufficientAssetBalance,
        Rejection::InsufficientShareBalance,
        Rejection::InsufficientRedemptionLiquidity,
        Rejection::ArithmeticOverflow,
        Rejection::ArithmeticUnderflow,
        Rejection::DivisionByZero,
        Rejection::UnauthorizedActor,
        Rejection::InvalidConfiguration,
        Rejection::DuplicateAccount,
        Rejection::TimestampNotMonotonic,
    ];

    #[test]
    fn every_rejection_renders_a_distinct_message() {
        for (index, rejection) in ALL.iter().enumerate() {
            let text = rejection.to_string();
            assert!(!text.is_empty());
            for other in ALL.iter().skip(index.saturating_add(1)) {
                assert_ne!(text, other.to_string());
            }
        }
    }
}
