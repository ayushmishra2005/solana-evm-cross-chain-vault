// SPDX-License-Identifier: Apache-2.0
pragma solidity 0.8.30;

/// Whether the vault accepts new work, only exits, or only claims.
enum VaultStatus {
    Active,
    Paused,
    Frozen
}

/// Whether the current epoch still takes requests.
enum EpochPhase {
    Open,
    CutOff
}

/// How an epoch left the active slot. An epoch has exactly one outcome.
enum EpochOutcome {
    None,
    Finalized,
    Aborted
}

/// Immutable settlement terms written once when an epoch leaves the slot.
///
/// An aborted epoch reuses `depositAssets` and `redeemShares` as the amounts
/// it owes back, and leaves the priced fields at zero.
struct SettledEpoch {
    EpochOutcome outcome;
    uint32 configVersion;
    uint64 cutoffAt;
    uint256 totalAssets;
    uint256 totalSupply;
    uint256 depositAssets;
    uint256 mintedShares;
    uint256 redeemShares;
    uint256 redeemAssets;
    uint256 depositDust;
    uint256 redeemDust;
}

/// One controller's deposit position inside a single epoch.
struct DepositRequest {
    uint256 assets;
    uint256 cancelledAssets;
    bool cancelled;
    bool settled;
}

/// One controller's redemption position inside a single epoch.
struct RedeemRequest {
    uint256 shares;
    uint256 cancelledShares;
    bool cancelled;
    bool settled;
}

/// @title Canonical asynchronous vault on the EVM side.
/// @notice Requests join the open epoch, settle at one price, and are claimed
/// later by the controller. Claims stay open in every vault status.
interface ISolEVMVault {
    event EpochOpened(uint64 indexed epochId, uint64 openedAt, uint64 cutoffAt);
    event EpochCutOff(uint64 indexed epochId, uint64 cutoffAt);
    event EpochFinalized(
        uint64 indexed epochId,
        uint256 totalAssets,
        uint256 totalSupply,
        uint256 depositAssets,
        uint256 mintedShares,
        uint256 redeemShares,
        uint256 redeemAssets,
        uint256 depositDust,
        uint256 redeemDust
    );
    event EpochAborted(uint64 indexed epochId, uint256 refundAssets, uint256 refundShares);

    event DepositRequested(uint64 indexed epochId, address indexed controller, uint256 assets);
    event DepositCancelled(uint64 indexed epochId, address indexed controller, uint256 assets);
    event DepositClaimed(uint64 indexed epochId, address indexed controller, uint256 shares);
    event DepositRefunded(uint64 indexed epochId, address indexed controller, uint256 assets);

    event RedeemRequested(uint64 indexed epochId, address indexed controller, uint256 shares);
    event RedeemCancelled(uint64 indexed epochId, address indexed controller, uint256 shares);
    event RedeemClaimed(uint64 indexed epochId, address indexed controller, uint256 assets);
    event RedeemRefunded(uint64 indexed epochId, address indexed controller, uint256 shares);

    event VaultPaused(address indexed actor);
    event VaultUnpaused(address indexed actor);
    event VaultFrozen(address indexed actor);
    event UnattributedAssetsReconciled(uint256 surplus, uint256 total);

    error Unauthorized(address actor);
    error InvalidAddress();
    error InvalidVaultStatus(VaultStatus status);
    error EpochNotOpen();
    error EpochAlreadyOpen();
    error EpochAlreadyCutOff();
    error EpochNotCutOff();
    error CutoffNotReached(uint64 nowAt, uint64 cutoffAt);
    error CancellationAfterCutoff();
    error EpochNotFinalized(uint64 epochId);
    error EpochNotAborted(uint64 epochId);
    error EpochAlreadySettled(uint64 epochId);
    error RequestNotFound(address controller);
    error RequestAlreadyCancelled(address controller);
    error RequestLimitReached(uint256 limit);
    error ZeroAmount();
    error AmountBelowMinimum(uint256 amount, uint256 minimum);
    error InsufficientBalance(uint256 available, uint256 requested);
    error InsufficientLiquidity(uint256 available, uint256 requested);
    error ClaimAlreadyConsumed(uint64 epochId, address controller);
    error RefundAlreadyConsumed(uint64 epochId, address controller);
    error InvalidTransferAmount(uint256 expected, uint256 actual);
    error AccountingDeficit(uint256 accounted, uint256 actual);
    error TimestampNotMonotonic(uint64 nowAt, uint64 earliest);
    error InvariantViolation();

    function requestDeposit(uint256 assets) external;

    function cancelDepositRequest() external;

    function requestRedeem(uint256 shares) external;

    function cancelRedeemRequest() external;

    function cutoffEpoch() external;

    function finalizeEpoch() external;

    function openNextEpoch() external;

    function abortEpoch() external;

    function claimDeposit(uint64 epochId) external returns (uint256 shares);

    function claimRedeem(uint64 epochId) external returns (uint256 assets);

    function refundDeposit(uint64 epochId) external returns (uint256 assets);

    function refundRedeem(uint64 epochId) external returns (uint256 shares);

    function pause() external;

    function unpause() external;

    function freeze() external;

    function reconcile() external returns (uint256 surplus);

    function managedNav() external view returns (uint256);
}
