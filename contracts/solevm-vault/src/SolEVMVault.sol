// SPDX-License-Identifier: Apache-2.0
pragma solidity 0.8.30;

import {ERC20} from "@openzeppelin/contracts/token/ERC20/ERC20.sol";
import {IERC20} from "@openzeppelin/contracts/token/ERC20/IERC20.sol";
import {ERC20Permit} from "@openzeppelin/contracts/token/ERC20/extensions/ERC20Permit.sol";
import {IERC20Metadata} from "@openzeppelin/contracts/token/ERC20/extensions/IERC20Metadata.sol";
import {SafeERC20} from "@openzeppelin/contracts/token/ERC20/utils/SafeERC20.sol";
import {ReentrancyGuard} from "@openzeppelin/contracts/utils/ReentrancyGuard.sol";
import {ERC165} from "@openzeppelin/contracts/utils/introspection/ERC165.sol";

import {
    DepositRequest,
    EpochOutcome,
    EpochPhase,
    ISolEVMVault,
    RedeemRequest,
    SettledEpoch,
    VaultStatus
} from "./interfaces/ISolEVMVault.sol";
import {VaultMath} from "./libraries/VaultMath.sol";

/// @title SolEVM Vault core.
/// @notice An immutable asynchronous vault. Deposits and redemptions join the
/// open epoch, settle together at one price, and are claimed afterwards.
/// @dev This is not a drop in ERC-4626 vault. Entry and exit are asynchronous,
/// so the synchronous preview and conversion calls of that standard are absent.
contract SolEVMVault is ISolEVMVault, ERC20, ERC20Permit, ERC165, ReentrancyGuard {
    using SafeERC20 for IERC20;

    /// Upper bound on priced controllers per epoch, so finalization stays bounded.
    uint256 public constant MAX_DEPOSIT_REQUESTS_PER_EPOCH = 32;
    uint256 public constant MAX_REDEEM_REQUESTS_PER_EPOCH = 32;

    uint8 private constant ASSET_DECIMALS = 6;
    uint8 private constant SHARE_DECIMALS = 18;

    IERC20 public immutable asset;
    address public immutable admin;
    address public immutable guardian;
    uint64 public immutable epochDuration;
    uint256 public immutable minDepositAssets;
    uint256 public immutable minRedeemShares;
    uint32 public immutable configVersion;

    /// Assets received for requests that have not settled yet.
    uint256 public pendingDepositEscrow;
    /// The only bucket that backs the share price.
    uint256 public idleBacking;
    /// Assets set aside for finalized redemption claims.
    uint256 public claimReserve;
    /// Assets that arrived without a request behind them.
    uint256 public unattributedBalance;

    VaultStatus public vaultStatus;

    bool public epochOpen;
    EpochPhase public currentEpochPhase;
    uint64 public currentEpochId;
    uint64 public currentEpochOpenedAt;
    uint64 public currentEpochCutoffAt;
    uint64 public nextEpochId;
    uint64 public lastCutoffAt;
    uint256 public epochDepositAssets;
    uint256 public epochRedeemShares;

    mapping(uint64 epochId => SettledEpoch) private _settled;
    mapping(uint64 epochId => mapping(address controller => DepositRequest)) private _depositOf;
    mapping(uint64 epochId => mapping(address controller => RedeemRequest)) private _redeemOf;
    mapping(uint64 epochId => mapping(address controller => uint256)) private _depositShareClaim;
    mapping(uint64 epochId => mapping(address controller => uint256)) private _redeemAssetClaim;

    mapping(uint64 epochId => address[]) private _depositControllers;
    mapping(uint64 epochId => address[]) private _redeemControllers;
    mapping(uint64 epochId => mapping(address controller => uint256)) private _depositSlot;
    mapping(uint64 epochId => mapping(address controller => uint256)) private _redeemSlot;

    /// Scratch values carried through finalization so nothing is read twice.
    struct Settlement {
        uint64 epochId;
        uint256 totalAssets;
        uint256 totalSupply;
        uint256 depositAssets;
        uint256 redeemShares;
        uint256 mintedShares;
        uint256 redeemAssets;
        uint256 owedShares;
        uint256 owedAssets;
    }

    constructor(
        IERC20 underlying,
        address vaultAdmin,
        address vaultGuardian,
        uint64 duration,
        uint256 minDeposit,
        uint256 minRedeem,
        uint32 version,
        string memory shareName,
        string memory shareSymbol
    ) ERC20(shareName, shareSymbol) ERC20Permit(shareName) {
        if (address(underlying) == address(0) || vaultAdmin == address(0)) revert InvalidAddress();
        if (vaultGuardian == address(0) || vaultAdmin == vaultGuardian) revert InvalidAddress();
        if (duration == 0 || minDeposit == 0 || minRedeem == 0) revert ZeroAmount();
        if (IERC20Metadata(address(underlying)).decimals() != ASSET_DECIMALS) {
            revert InvalidAddress();
        }

        asset = underlying;
        admin = vaultAdmin;
        guardian = vaultGuardian;
        epochDuration = duration;
        minDepositAssets = minDeposit;
        minRedeemShares = minRedeem;
        configVersion = version;

        vaultStatus = VaultStatus.Active;
        _openEpoch(0);
    }

    function decimals() public pure override returns (uint8) {
        return SHARE_DECIMALS;
    }

    // Requests

    /// @notice Joins the open epoch with an exact asset amount.
    function requestDeposit(uint256 assets) external nonReentrant {
        _requireStatus(VaultStatus.Active);
        _requireOpenEpoch();
        if (assets == 0) revert ZeroAmount();
        if (assets < minDepositAssets) revert AmountBelowMinimum(assets, minDepositAssets);
        _requireNoDeficit();

        uint64 epochId = currentEpochId;
        DepositRequest storage request = _depositOf[epochId][msg.sender];
        if (request.settled) revert ClaimAlreadyConsumed(epochId, msg.sender);

        if (_depositSlot[epochId][msg.sender] == 0) {
            address[] storage list = _depositControllers[epochId];
            if (list.length == MAX_DEPOSIT_REQUESTS_PER_EPOCH) {
                revert RequestLimitReached(MAX_DEPOSIT_REQUESTS_PER_EPOCH);
            }
            list.push(msg.sender);
            _depositSlot[epochId][msg.sender] = list.length;
        }

        request.assets += assets;
        request.cancelled = false;
        pendingDepositEscrow += assets;
        epochDepositAssets += assets;

        _pullExact(msg.sender, assets);
        emit DepositRequested(epochId, msg.sender, assets);
    }

    /// @notice Withdraws a deposit request before the epoch is cut off.
    function cancelDepositRequest() external nonReentrant {
        _requireCancellationAllowed();
        uint64 epochId = _requireCancellableEpoch();

        DepositRequest storage request = _depositOf[epochId][msg.sender];
        if (request.settled) revert ClaimAlreadyConsumed(epochId, msg.sender);
        uint256 assets = request.assets;
        if (assets == 0) revert RequestAlreadyCancelled(msg.sender);

        request.assets = 0;
        request.cancelledAssets += assets;
        request.cancelled = true;
        pendingDepositEscrow -= assets;
        epochDepositAssets -= assets;
        _removeDepositController(epochId, msg.sender);

        _pushExact(msg.sender, assets);
        emit DepositCancelled(epochId, msg.sender, assets);
    }

    /// @notice Moves shares into vault escrow for the open epoch.
    /// @dev Escrowed shares stay in total supply until the epoch settles.
    function requestRedeem(uint256 shares) external nonReentrant {
        _requireStatus(VaultStatus.Active);
        _requireOpenEpoch();
        if (shares == 0) revert ZeroAmount();
        if (shares < minRedeemShares) revert AmountBelowMinimum(shares, minRedeemShares);
        uint256 held = balanceOf(msg.sender);
        if (held < shares) revert InsufficientBalance(held, shares);
        _requireNoDeficit();

        uint64 epochId = currentEpochId;
        RedeemRequest storage request = _redeemOf[epochId][msg.sender];
        if (request.settled) revert ClaimAlreadyConsumed(epochId, msg.sender);

        if (_redeemSlot[epochId][msg.sender] == 0) {
            address[] storage list = _redeemControllers[epochId];
            if (list.length == MAX_REDEEM_REQUESTS_PER_EPOCH) {
                revert RequestLimitReached(MAX_REDEEM_REQUESTS_PER_EPOCH);
            }
            list.push(msg.sender);
            _redeemSlot[epochId][msg.sender] = list.length;
        }

        request.shares += shares;
        request.cancelled = false;
        epochRedeemShares += shares;

        _transfer(msg.sender, address(this), shares);
        emit RedeemRequested(epochId, msg.sender, shares);
    }

    /// @notice Returns escrowed shares before the epoch is cut off.
    function cancelRedeemRequest() external nonReentrant {
        _requireCancellationAllowed();
        uint64 epochId = _requireCancellableEpoch();

        RedeemRequest storage request = _redeemOf[epochId][msg.sender];
        if (request.settled) revert ClaimAlreadyConsumed(epochId, msg.sender);
        uint256 shares = request.shares;
        if (shares == 0) revert RequestAlreadyCancelled(msg.sender);

        request.shares = 0;
        request.cancelledShares += shares;
        request.cancelled = true;
        epochRedeemShares -= shares;
        _removeRedeemController(epochId, msg.sender);

        _transfer(address(this), msg.sender, shares);
        emit RedeemCancelled(epochId, msg.sender, shares);
    }

    // Epoch lifecycle

    /// @notice Closes the open epoch to new requests. Anyone may call it once
    /// the cutoff timestamp is reached.
    function cutoffEpoch() external {
        _requireStatus(VaultStatus.Active);
        _requireOpenEpoch();
        uint64 nowAt = uint64(block.timestamp);
        if (nowAt < currentEpochCutoffAt) revert CutoffNotReached(nowAt, currentEpochCutoffAt);
        currentEpochPhase = EpochPhase.CutOff;
        emit EpochCutOff(currentEpochId, currentEpochCutoffAt);
    }

    /// @notice Prices the cut off epoch and writes its immutable terms.
    /// @dev Redemptions draw only on liquidity that existed before this epoch,
    /// so deposits arriving in the same epoch never fund an exit.
    function finalizeEpoch() external nonReentrant {
        _requireStatus(VaultStatus.Active);
        if (!epochOpen) revert EpochNotOpen();
        if (currentEpochPhase != EpochPhase.CutOff) revert EpochNotCutOff();

        Settlement memory step;
        step.epochId = currentEpochId;
        if (_settled[step.epochId].outcome != EpochOutcome.None) {
            revert EpochAlreadySettled(step.epochId);
        }

        _recordSurplus();

        step.totalAssets = idleBacking;
        step.totalSupply = totalSupply();
        step.depositAssets = epochDepositAssets;
        step.redeemShares = epochRedeemShares;
        step.mintedShares =
            VaultMath.assetsToShares(step.depositAssets, step.totalAssets, step.totalSupply);
        step.redeemAssets =
            VaultMath.sharesToAssets(step.redeemShares, step.totalAssets, step.totalSupply);

        if (step.redeemAssets > step.totalAssets) {
            revert InsufficientLiquidity(step.totalAssets, step.redeemAssets);
        }

        step.owedShares = _priceDepositClaims(step);
        step.owedAssets = _priceRedeemClaims(step);
        if (step.owedShares > step.mintedShares || step.owedAssets > step.redeemAssets) {
            revert InvariantViolation();
        }

        _settled[step.epochId] = SettledEpoch({
            outcome: EpochOutcome.Finalized,
            configVersion: configVersion,
            cutoffAt: currentEpochCutoffAt,
            totalAssets: step.totalAssets,
            totalSupply: step.totalSupply,
            depositAssets: step.depositAssets,
            mintedShares: step.mintedShares,
            redeemShares: step.redeemShares,
            redeemAssets: step.redeemAssets,
            depositDust: step.mintedShares - step.owedShares,
            redeemDust: step.redeemAssets - step.owedAssets
        });

        idleBacking = step.totalAssets - step.redeemAssets + step.depositAssets;
        pendingDepositEscrow -= step.depositAssets;
        claimReserve += step.redeemAssets;

        if (step.redeemShares != 0) _burn(address(this), step.redeemShares);
        if (step.mintedShares != 0) _mint(address(this), step.mintedShares);

        lastCutoffAt = currentEpochCutoffAt;
        nextEpochId = step.epochId + 1;
        _closeEpochSlot();

        emit EpochFinalized(
            step.epochId,
            step.totalAssets,
            step.totalSupply,
            step.depositAssets,
            step.mintedShares,
            step.redeemShares,
            step.redeemAssets,
            step.mintedShares - step.owedShares,
            step.redeemAssets - step.owedAssets
        );
    }

    /// @notice Starts the next epoch after the previous one left the slot.
    function openNextEpoch() external {
        _requireStatus(VaultStatus.Active);
        if (epochOpen) revert EpochAlreadyOpen();
        uint64 nowAt = uint64(block.timestamp);
        if (nowAt < lastCutoffAt) revert TimestampNotMonotonic(nowAt, lastCutoffAt);
        _openEpoch(nextEpochId);
    }

    /// @notice Abandons the unsettled epoch so frozen funds keep a way out.
    function abortEpoch() external nonReentrant {
        _requireAdminOrGuardian();
        _requireStatus(VaultStatus.Frozen);
        if (!epochOpen) revert EpochNotOpen();

        uint64 epochId = currentEpochId;
        if (_settled[epochId].outcome != EpochOutcome.None) revert EpochAlreadySettled(epochId);

        uint256 refundAssets = epochDepositAssets;
        uint256 refundShares = epochRedeemShares;

        SettledEpoch storage record = _settled[epochId];
        record.outcome = EpochOutcome.Aborted;
        record.configVersion = configVersion;
        record.cutoffAt = currentEpochCutoffAt;
        record.depositAssets = refundAssets;
        record.redeemShares = refundShares;

        lastCutoffAt = currentEpochCutoffAt;
        nextEpochId = epochId + 1;
        _closeEpochSlot();

        emit EpochAborted(epochId, refundAssets, refundShares);
    }

    // Claims and refunds

    /// @notice Collects the shares a finalized epoch owes. Open in every status.
    function claimDeposit(uint64 epochId) external nonReentrant returns (uint256 shares) {
        if (_settled[epochId].outcome != EpochOutcome.Finalized) {
            revert EpochNotFinalized(epochId);
        }
        DepositRequest storage request = _depositOf[epochId][msg.sender];
        if (request.cancelled) revert RequestAlreadyCancelled(msg.sender);
        if (request.settled) revert ClaimAlreadyConsumed(epochId, msg.sender);
        if (request.assets == 0) revert RequestNotFound(msg.sender);

        shares = _depositShareClaim[epochId][msg.sender];
        request.settled = true;

        if (shares != 0) _transfer(address(this), msg.sender, shares);
        emit DepositClaimed(epochId, msg.sender, shares);
    }

    /// @notice Collects the assets a finalized epoch owes. Open in every status.
    function claimRedeem(uint64 epochId) external nonReentrant returns (uint256 assets) {
        if (_settled[epochId].outcome != EpochOutcome.Finalized) {
            revert EpochNotFinalized(epochId);
        }
        RedeemRequest storage request = _redeemOf[epochId][msg.sender];
        if (request.cancelled) revert RequestAlreadyCancelled(msg.sender);
        if (request.settled) revert ClaimAlreadyConsumed(epochId, msg.sender);
        if (request.shares == 0) revert RequestNotFound(msg.sender);

        assets = _redeemAssetClaim[epochId][msg.sender];
        request.settled = true;
        claimReserve -= assets;

        if (assets != 0) _pushExact(msg.sender, assets);
        emit RedeemClaimed(epochId, msg.sender, assets);
    }

    /// @notice Takes back the assets an aborted epoch never priced.
    function refundDeposit(uint64 epochId) external nonReentrant returns (uint256 assets) {
        if (_settled[epochId].outcome != EpochOutcome.Aborted) revert EpochNotAborted(epochId);
        DepositRequest storage request = _depositOf[epochId][msg.sender];
        if (request.cancelled) revert RequestAlreadyCancelled(msg.sender);
        if (request.settled) revert RefundAlreadyConsumed(epochId, msg.sender);

        assets = request.assets;
        if (assets == 0) revert RequestNotFound(msg.sender);

        request.settled = true;
        pendingDepositEscrow -= assets;

        _pushExact(msg.sender, assets);
        emit DepositRefunded(epochId, msg.sender, assets);
    }

    /// @notice Takes back the shares an aborted epoch never burned.
    function refundRedeem(uint64 epochId) external nonReentrant returns (uint256 shares) {
        if (_settled[epochId].outcome != EpochOutcome.Aborted) revert EpochNotAborted(epochId);
        RedeemRequest storage request = _redeemOf[epochId][msg.sender];
        if (request.cancelled) revert RequestAlreadyCancelled(msg.sender);
        if (request.settled) revert RefundAlreadyConsumed(epochId, msg.sender);

        shares = request.shares;
        if (shares == 0) revert RequestNotFound(msg.sender);

        request.settled = true;

        _transfer(address(this), msg.sender, shares);
        emit RedeemRefunded(epochId, msg.sender, shares);
    }

    // Emergency controls

    function pause() external {
        _requireAdminOrGuardian();
        _requireStatus(VaultStatus.Active);
        vaultStatus = VaultStatus.Paused;
        emit VaultPaused(msg.sender);
    }

    function unpause() external {
        if (msg.sender != admin) revert Unauthorized(msg.sender);
        _requireStatus(VaultStatus.Paused);
        vaultStatus = VaultStatus.Active;
        emit VaultUnpaused(msg.sender);
    }

    /// @notice Freezing is terminal. Only claims, abort and refunds remain.
    function freeze() external {
        _requireAdminOrGuardian();
        if (vaultStatus == VaultStatus.Frozen) revert InvalidVaultStatus(vaultStatus);
        vaultStatus = VaultStatus.Frozen;
        emit VaultFrozen(msg.sender);
    }

    // Reconciliation

    /// @notice Records assets that arrived without a request. Moves nothing and
    /// never changes the share price.
    function reconcile() external nonReentrant returns (uint256 surplus) {
        return _recordSurplus();
    }

    // Views

    function managedNav() external view returns (uint256) {
        return idleBacking;
    }

    function settledEpoch(uint64 epochId) external view returns (SettledEpoch memory) {
        return _settled[epochId];
    }

    function depositRequestOf(uint64 epochId, address controller)
        external
        view
        returns (DepositRequest memory)
    {
        return _depositOf[epochId][controller];
    }

    function redeemRequestOf(uint64 epochId, address controller)
        external
        view
        returns (RedeemRequest memory)
    {
        return _redeemOf[epochId][controller];
    }

    function claimableDepositShares(uint64 epochId, address controller)
        external
        view
        returns (uint256)
    {
        return _depositShareClaim[epochId][controller];
    }

    function claimableRedeemAssets(uint64 epochId, address controller)
        external
        view
        returns (uint256)
    {
        return _redeemAssetClaim[epochId][controller];
    }

    function depositControllers(uint64 epochId) external view returns (address[] memory) {
        return _depositControllers[epochId];
    }

    function redeemControllers(uint64 epochId) external view returns (address[] memory) {
        return _redeemControllers[epochId];
    }

    /// @notice Assets the vault believes it holds across every bucket.
    function accountedAssets() public view returns (uint256) {
        return pendingDepositEscrow + idleBacking + claimReserve + unattributedBalance;
    }

    function supportsInterface(bytes4 interfaceId) public view override returns (bool) {
        return interfaceId == type(ISolEVMVault).interfaceId
            || interfaceId == type(IERC20).interfaceId
            || interfaceId == type(IERC20Metadata).interfaceId
            || super.supportsInterface(interfaceId);
    }

    // Internals

    function _openEpoch(uint64 epochId) private {
        uint64 nowAt = uint64(block.timestamp);
        uint64 cutoffAt = nowAt + epochDuration;

        epochOpen = true;
        currentEpochPhase = EpochPhase.Open;
        currentEpochId = epochId;
        currentEpochOpenedAt = nowAt;
        currentEpochCutoffAt = cutoffAt;
        nextEpochId = epochId + 1;
        epochDepositAssets = 0;
        epochRedeemShares = 0;

        emit EpochOpened(epochId, nowAt, cutoffAt);
    }

    function _closeEpochSlot() private {
        epochOpen = false;
        epochDepositAssets = 0;
        epochRedeemShares = 0;
    }

    /// Writes one immutable share claim per active deposit controller.
    function _priceDepositClaims(Settlement memory step) private returns (uint256 owed) {
        address[] storage list = _depositControllers[step.epochId];
        uint256 count = list.length;
        for (uint256 i = 0; i < count; ++i) {
            address controller = list[i];
            uint256 assets = _depositOf[step.epochId][controller].assets;
            uint256 shares = VaultMath.assetsToShares(assets, step.totalAssets, step.totalSupply);
            _depositShareClaim[step.epochId][controller] = shares;
            owed += shares;
        }
    }

    /// Writes one immutable asset claim per active redemption controller.
    function _priceRedeemClaims(Settlement memory step) private returns (uint256 owed) {
        address[] storage list = _redeemControllers[step.epochId];
        uint256 count = list.length;
        for (uint256 i = 0; i < count; ++i) {
            address controller = list[i];
            uint256 shares = _redeemOf[step.epochId][controller].shares;
            uint256 assets = VaultMath.sharesToAssets(shares, step.totalAssets, step.totalSupply);
            _redeemAssetClaim[step.epochId][controller] = assets;
            owed += assets;
        }
    }

    function _removeDepositController(uint64 epochId, address controller) private {
        address[] storage list = _depositControllers[epochId];
        uint256 slot = _depositSlot[epochId][controller];
        if (slot == 0) revert RequestNotFound(controller);
        uint256 last = list.length;
        if (slot != last) {
            address moved = list[last - 1];
            list[slot - 1] = moved;
            _depositSlot[epochId][moved] = slot;
        }
        list.pop();
        _depositSlot[epochId][controller] = 0;
    }

    function _removeRedeemController(uint64 epochId, address controller) private {
        address[] storage list = _redeemControllers[epochId];
        uint256 slot = _redeemSlot[epochId][controller];
        if (slot == 0) revert RequestNotFound(controller);
        uint256 last = list.length;
        if (slot != last) {
            address moved = list[last - 1];
            list[slot - 1] = moved;
            _redeemSlot[epochId][moved] = slot;
        }
        list.pop();
        _redeemSlot[epochId][controller] = 0;
    }

    function _recordSurplus() private returns (uint256 surplus) {
        uint256 accounted = accountedAssets();
        uint256 actual = asset.balanceOf(address(this));
        if (actual < accounted) revert AccountingDeficit(accounted, actual);
        surplus = actual - accounted;
        if (surplus != 0) {
            uint256 total = unattributedBalance + surplus;
            unattributedBalance = total;
            emit UnattributedAssetsReconciled(surplus, total);
        }
    }

    function _requireNoDeficit() private view {
        uint256 accounted = accountedAssets();
        uint256 actual = asset.balanceOf(address(this));
        if (actual < accounted) revert AccountingDeficit(accounted, actual);
    }

    function _pullExact(address from, uint256 amount) private {
        uint256 before = asset.balanceOf(address(this));
        asset.safeTransferFrom(from, address(this), amount);
        uint256 received = asset.balanceOf(address(this)) - before;
        if (received != amount) revert InvalidTransferAmount(amount, received);
    }

    function _pushExact(address to, uint256 amount) private {
        uint256 before = asset.balanceOf(address(this));
        asset.safeTransfer(to, amount);
        uint256 sent = before - asset.balanceOf(address(this));
        if (sent != amount) revert InvalidTransferAmount(amount, sent);
    }

    function _requireStatus(VaultStatus expected) private view {
        if (vaultStatus != expected) revert InvalidVaultStatus(vaultStatus);
    }

    function _requireOpenEpoch() private view {
        if (!epochOpen) revert EpochNotOpen();
        if (currentEpochPhase != EpochPhase.Open) revert EpochAlreadyCutOff();
    }

    /// Cancellation stays open while paused so users are never trapped.
    function _requireCancellationAllowed() private view {
        if (vaultStatus == VaultStatus.Frozen) revert InvalidVaultStatus(vaultStatus);
    }

    function _requireCancellableEpoch() private view returns (uint64) {
        if (!epochOpen) revert EpochNotOpen();
        if (currentEpochPhase != EpochPhase.Open) revert CancellationAfterCutoff();
        return currentEpochId;
    }

    function _requireAdminOrGuardian() private view {
        if (msg.sender != admin && msg.sender != guardian) revert Unauthorized(msg.sender);
    }
}
