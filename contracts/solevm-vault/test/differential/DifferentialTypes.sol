// SPDX-License-Identifier: Apache-2.0
pragma solidity 0.8.30;

/// Mutable state compared after every operation.
///
/// The layout mirrors the Rust generator field for field. Changing one side
/// without the other breaks decoding straight away.
struct Snapshot {
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
struct Action {
    uint8 kind;
    uint8 actor;
    uint256 amount;
    uint64 epochId;
    uint64 timestamp;
    uint8 expectedResult;
    uint256 expectedReturn;
}

/// What one settled epoch owes a single user.
struct EpochActor {
    uint256 depositAssets;
    uint256 redeemShares;
    uint256 claimShares;
    uint256 claimAssets;
}

/// Immutable terms of one settled epoch.
struct EpochTerms {
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
    EpochActor[4] actors;
}

/// A complete trace the harness replays against the vault.
struct Scenario {
    uint32 index;
    uint64 seed;
    uint8 family;
    uint64 startTimestamp;
    uint64 epochDuration;
    uint256 minDeposit;
    uint256 minRedeem;
    uint32 configVersion;
    uint256[4] initialAssets;
    Snapshot initialSnapshot;
    Action[] actions;
    Snapshot[] snapshots;
    EpochTerms[] epochs;
}

/// Operation codes shared with the generator.
library Op {
    uint8 internal constant REQUEST_DEPOSIT = 0;
    uint8 internal constant CANCEL_DEPOSIT = 1;
    uint8 internal constant REQUEST_REDEEM = 2;
    uint8 internal constant CANCEL_REDEEM = 3;
    uint8 internal constant CUTOFF_EPOCH = 4;
    uint8 internal constant FINALIZE_EPOCH = 5;
    uint8 internal constant OPEN_NEXT_EPOCH = 6;
    uint8 internal constant CLAIM_DEPOSIT = 7;
    uint8 internal constant CLAIM_REDEEM = 8;
    uint8 internal constant REFUND_DEPOSIT = 9;
    uint8 internal constant REFUND_REDEEM = 10;
    uint8 internal constant PAUSE = 11;
    uint8 internal constant UNPAUSE = 12;
    uint8 internal constant FREEZE = 13;
    uint8 internal constant ABORT_EPOCH = 14;

    function name(uint8 kind) internal pure returns (string memory) {
        if (kind == REQUEST_DEPOSIT) return "RequestDeposit";
        if (kind == CANCEL_DEPOSIT) return "CancelDeposit";
        if (kind == REQUEST_REDEEM) return "RequestRedeem";
        if (kind == CANCEL_REDEEM) return "CancelRedeem";
        if (kind == CUTOFF_EPOCH) return "CutoffEpoch";
        if (kind == FINALIZE_EPOCH) return "FinalizeEpoch";
        if (kind == OPEN_NEXT_EPOCH) return "OpenNextEpoch";
        if (kind == CLAIM_DEPOSIT) return "ClaimDeposit";
        if (kind == CLAIM_REDEEM) return "ClaimRedeem";
        if (kind == REFUND_DEPOSIT) return "RefundDeposit";
        if (kind == REFUND_REDEEM) return "RefundRedeem";
        if (kind == PAUSE) return "Pause";
        if (kind == UNPAUSE) return "Unpause";
        if (kind == FREEZE) return "Freeze";
        if (kind == ABORT_EPOCH) return "AbortEpoch";
        return "Unknown";
    }
}

/// Result codes shared with the generator.
library Code {
    uint8 internal constant SUCCESS = 0;
    uint8 internal constant INVALID_VAULT_STATE = 1;
    uint8 internal constant UNAUTHORIZED = 2;
    uint8 internal constant EPOCH_NOT_OPEN = 3;
    uint8 internal constant EPOCH_ALREADY_OPEN = 4;
    uint8 internal constant EPOCH_ALREADY_CUT_OFF = 5;
    uint8 internal constant EPOCH_NOT_CUT_OFF = 6;
    uint8 internal constant EPOCH_ALREADY_SETTLED = 7;
    uint8 internal constant CUTOFF_NOT_REACHED = 8;
    uint8 internal constant CANCELLATION_AFTER_CUTOFF = 9;
    uint8 internal constant EPOCH_NOT_FINALIZED = 10;
    uint8 internal constant EPOCH_NOT_ABORTED = 11;
    uint8 internal constant REQUEST_NOT_FOUND = 12;
    uint8 internal constant REQUEST_NOT_ACTIVE = 13;
    uint8 internal constant CLAIM_ALREADY_CONSUMED = 14;
    uint8 internal constant REFUND_ALREADY_CONSUMED = 15;
    uint8 internal constant ZERO_AMOUNT = 16;
    uint8 internal constant AMOUNT_BELOW_MINIMUM = 17;
    uint8 internal constant INSUFFICIENT_ASSET_BALANCE = 18;
    uint8 internal constant INSUFFICIENT_SHARE_BALANCE = 19;
    uint8 internal constant INSUFFICIENT_LIQUIDITY = 20;
    uint8 internal constant TIMESTAMP_NOT_MONOTONIC = 21;
    uint8 internal constant INVALID_CONFIGURATION = 22;
    uint8 internal constant ARITHMETIC_FAILURE = 23;
    /// Reserved for revert data the harness does not recognise.
    uint8 internal constant UNKNOWN = 255;

    function name(uint8 code) internal pure returns (string memory) {
        if (code == SUCCESS) return "Success";
        if (code == INVALID_VAULT_STATE) return "InvalidVaultState";
        if (code == UNAUTHORIZED) return "Unauthorized";
        if (code == EPOCH_NOT_OPEN) return "EpochNotOpen";
        if (code == EPOCH_ALREADY_OPEN) return "EpochAlreadyOpen";
        if (code == EPOCH_ALREADY_CUT_OFF) return "EpochAlreadyCutOff";
        if (code == EPOCH_NOT_CUT_OFF) return "EpochNotCutOff";
        if (code == EPOCH_ALREADY_SETTLED) return "EpochAlreadySettled";
        if (code == CUTOFF_NOT_REACHED) return "CutoffNotReached";
        if (code == CANCELLATION_AFTER_CUTOFF) return "CancellationAfterCutoff";
        if (code == EPOCH_NOT_FINALIZED) return "EpochNotFinalized";
        if (code == EPOCH_NOT_ABORTED) return "EpochNotAborted";
        if (code == REQUEST_NOT_FOUND) return "RequestNotFound";
        if (code == REQUEST_NOT_ACTIVE) return "RequestNotActive";
        if (code == CLAIM_ALREADY_CONSUMED) return "ClaimAlreadyConsumed";
        if (code == REFUND_ALREADY_CONSUMED) return "RefundAlreadyConsumed";
        if (code == ZERO_AMOUNT) return "ZeroAmount";
        if (code == AMOUNT_BELOW_MINIMUM) return "AmountBelowMinimum";
        if (code == INSUFFICIENT_ASSET_BALANCE) return "InsufficientAssetBalance";
        if (code == INSUFFICIENT_SHARE_BALANCE) return "InsufficientShareBalance";
        if (code == INSUFFICIENT_LIQUIDITY) return "InsufficientLiquidity";
        if (code == TIMESTAMP_NOT_MONOTONIC) return "TimestampNotMonotonic";
        if (code == INVALID_CONFIGURATION) return "InvalidConfiguration";
        if (code == ARITHMETIC_FAILURE) return "ArithmeticFailure";
        return "Unknown";
    }
}
