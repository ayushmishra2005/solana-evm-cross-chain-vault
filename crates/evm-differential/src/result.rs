use accounting_model::Rejection;

use crate::action::ActionKind;

/// Stable numeric outcome shared with the Solidity harness.
///
/// The two implementations name their errors differently, so the harness
/// compares these codes instead of any message or selector.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum ResultCode {
    Success = 0,
    InvalidVaultState = 1,
    Unauthorized = 2,
    EpochNotOpen = 3,
    EpochAlreadyOpen = 4,
    EpochAlreadyCutOff = 5,
    EpochNotCutOff = 6,
    EpochAlreadySettled = 7,
    CutoffNotReached = 8,
    CancellationAfterCutoff = 9,
    EpochNotFinalized = 10,
    EpochNotAborted = 11,
    RequestNotFound = 12,
    RequestNotActive = 13,
    ClaimAlreadyConsumed = 14,
    RefundAlreadyConsumed = 15,
    ZeroAmount = 16,
    AmountBelowMinimum = 17,
    InsufficientAssetBalance = 18,
    InsufficientShareBalance = 19,
    InsufficientLiquidity = 20,
    TimestampNotMonotonic = 21,
    InvalidConfiguration = 22,
    ArithmeticFailure = 23,
}

impl ResultCode {
    #[must_use]
    pub const fn raw(self) -> u8 {
        self as u8
    }

    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Success => "Success",
            Self::InvalidVaultState => "InvalidVaultState",
            Self::Unauthorized => "Unauthorized",
            Self::EpochNotOpen => "EpochNotOpen",
            Self::EpochAlreadyOpen => "EpochAlreadyOpen",
            Self::EpochAlreadyCutOff => "EpochAlreadyCutOff",
            Self::EpochNotCutOff => "EpochNotCutOff",
            Self::EpochAlreadySettled => "EpochAlreadySettled",
            Self::CutoffNotReached => "CutoffNotReached",
            Self::CancellationAfterCutoff => "CancellationAfterCutoff",
            Self::EpochNotFinalized => "EpochNotFinalized",
            Self::EpochNotAborted => "EpochNotAborted",
            Self::RequestNotFound => "RequestNotFound",
            Self::RequestNotActive => "RequestNotActive",
            Self::ClaimAlreadyConsumed => "ClaimAlreadyConsumed",
            Self::RefundAlreadyConsumed => "RefundAlreadyConsumed",
            Self::ZeroAmount => "ZeroAmount",
            Self::AmountBelowMinimum => "AmountBelowMinimum",
            Self::InsufficientAssetBalance => "InsufficientAssetBalance",
            Self::InsufficientShareBalance => "InsufficientShareBalance",
            Self::InsufficientLiquidity => "InsufficientLiquidity",
            Self::TimestampNotMonotonic => "TimestampNotMonotonic",
            Self::InvalidConfiguration => "InvalidConfiguration",
            Self::ArithmeticFailure => "ArithmeticFailure",
        }
    }
}

/// Maps a model rejection onto the shared code.
///
/// The operation matters in two places. Solidity separates a spent claim from a
/// spent refund, and it cannot tell a missing cancellation target from an
/// already cancelled one, so both collapse to `RequestNotActive`.
#[must_use]
pub fn code_for(kind: ActionKind, rejection: Rejection) -> ResultCode {
    match rejection {
        Rejection::InvalidVaultState => ResultCode::InvalidVaultState,
        Rejection::UnauthorizedActor => ResultCode::Unauthorized,
        Rejection::EpochNotOpen => ResultCode::EpochNotOpen,
        Rejection::EpochAlreadyOpen => ResultCode::EpochAlreadyOpen,
        Rejection::EpochAlreadyCutOff => ResultCode::EpochAlreadyCutOff,
        Rejection::EpochNotCutOff => ResultCode::EpochNotCutOff,
        Rejection::EpochAlreadyFinalized => ResultCode::EpochAlreadySettled,
        Rejection::CutoffNotReached => ResultCode::CutoffNotReached,
        Rejection::CancellationAfterCutoff => ResultCode::CancellationAfterCutoff,
        Rejection::EpochNotFinalized => ResultCode::EpochNotFinalized,
        Rejection::EpochNotAborted => ResultCode::EpochNotAborted,
        Rejection::RequestNotFound | Rejection::ClaimNotFound => {
            if kind.is_cancellation() {
                ResultCode::RequestNotActive
            } else {
                ResultCode::RequestNotFound
            }
        }
        Rejection::RequestAlreadyCancelled => ResultCode::RequestNotActive,
        Rejection::RequestAlreadySettled | Rejection::ClaimAlreadyConsumed => {
            if kind.is_refund() {
                ResultCode::RefundAlreadyConsumed
            } else {
                ResultCode::ClaimAlreadyConsumed
            }
        }
        Rejection::ZeroAmount => ResultCode::ZeroAmount,
        Rejection::AmountBelowMinimum => ResultCode::AmountBelowMinimum,
        Rejection::InsufficientAssetBalance => ResultCode::InsufficientAssetBalance,
        Rejection::InsufficientShareBalance => ResultCode::InsufficientShareBalance,
        Rejection::InsufficientRedemptionLiquidity => ResultCode::InsufficientLiquidity,
        Rejection::TimestampNotMonotonic => ResultCode::TimestampNotMonotonic,
        Rejection::InvalidConfiguration | Rejection::DuplicateAccount => {
            ResultCode::InvalidConfiguration
        }
        Rejection::ArithmeticOverflow
        | Rejection::ArithmeticUnderflow
        | Rejection::DivisionByZero => ResultCode::ArithmeticFailure,
        _ => ResultCode::ArithmeticFailure,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every rejection the model can produce, so a new variant breaks the build.
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
    fn every_model_rejection_maps_to_a_named_code() {
        for rejection in ALL {
            for kind in ActionKind::ALL {
                let code = code_for(kind, rejection);
                assert_ne!(code, ResultCode::Success);
                assert!(!code.name().is_empty());
            }
        }
    }

    #[test]
    fn a_spent_refund_and_a_spent_claim_use_different_codes() {
        assert_eq!(
            code_for(ActionKind::RefundDeposit, Rejection::ClaimAlreadyConsumed),
            ResultCode::RefundAlreadyConsumed
        );
        assert_eq!(
            code_for(ActionKind::ClaimDeposit, Rejection::ClaimAlreadyConsumed),
            ResultCode::ClaimAlreadyConsumed
        );
    }

    #[test]
    fn a_cancellation_reports_one_code_for_every_missing_target() {
        assert_eq!(
            code_for(ActionKind::CancelDeposit, Rejection::RequestNotFound),
            ResultCode::RequestNotActive
        );
        assert_eq!(
            code_for(
                ActionKind::CancelDeposit,
                Rejection::RequestAlreadyCancelled
            ),
            ResultCode::RequestNotActive
        );
        assert_eq!(
            code_for(ActionKind::CancelRedeem, Rejection::RequestNotFound),
            ResultCode::RequestNotActive
        );
    }

    #[test]
    fn a_missing_claim_target_stays_distinct_from_a_cancelled_one() {
        assert_eq!(
            code_for(ActionKind::ClaimDeposit, Rejection::RequestNotFound),
            ResultCode::RequestNotFound
        );
        assert_eq!(
            code_for(ActionKind::ClaimDeposit, Rejection::RequestAlreadyCancelled),
            ResultCode::RequestNotActive
        );
    }

    #[test]
    fn codes_keep_their_wire_numbers() {
        assert_eq!(ResultCode::Success.raw(), 0);
        assert_eq!(ResultCode::InvalidVaultState.raw(), 1);
        assert_eq!(ResultCode::RefundAlreadyConsumed.raw(), 15);
        assert_eq!(ResultCode::ArithmeticFailure.raw(), 23);
    }
}
