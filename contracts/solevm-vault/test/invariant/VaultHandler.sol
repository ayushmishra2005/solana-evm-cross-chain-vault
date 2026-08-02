// SPDX-License-Identifier: Apache-2.0
pragma solidity 0.8.30;

import {CommonBase} from "forge-std/Base.sol";
import {StdCheats} from "forge-std/StdCheats.sol";
import {StdUtils} from "forge-std/StdUtils.sol";

import {SolEVMVault} from "../../src/SolEVMVault.sol";
import {
    DepositRequest,
    EpochPhase,
    RedeemRequest,
    SettledEpoch,
    VaultStatus
} from "../../src/interfaces/ISolEVMVault.sol";
import {MockAsset} from "../mocks/MockAsset.sol";

/// Drives the vault with valid calls only, so any revert is a real failure.
contract VaultHandler is CommonBase, StdCheats, StdUtils {
    uint256 internal constant ACTOR_COUNT = 5;
    uint256 internal constant MIN_DEPOSIT = 1e6;
    uint256 internal constant MIN_REDEEM = 1e12;
    uint256 internal constant MAX_DEPOSIT = 50_000e6;
    uint256 internal constant ACTOR_FUNDING = 1_000_000e6;

    SolEVMVault public immutable vault;
    MockAsset public immutable token;
    address public immutable admin;
    address public immutable guardian;

    /// Freezing is terminal, so it ends a run. One suite leaves it off to reach
    /// deep lifecycle states and another turns it on to reach abort and refund.
    bool public immutable freezeEnabled;

    address[] public actors;

    uint64[] public finalizedEpochs;
    uint64[] public abortedEpochs;

    /// Terms recorded the moment an epoch left the slot, used to spot edits.
    mapping(uint64 epochId => bytes32) public termsFingerprint;

    uint256 public ghostMintedShares;
    uint256 public ghostBurnedShares;
    uint256 public ghostDonated;
    uint256 public ghostDepositClaims;
    uint256 public ghostRedeemClaims;
    uint256 public ghostDepositRefunds;
    uint256 public ghostRedeemRefunds;

    /// Attempts counts every call the fuzzer made. Effective counts the ones
    /// that passed the guards and reached the vault.
    mapping(bytes32 action => uint256) public attemptCount;
    mapping(bytes32 action => uint256) public effectiveCount;
    string[] public actionNames;

    constructor(
        SolEVMVault vaultAddress,
        MockAsset assetAddress,
        address adminAddress,
        address guardianAddress,
        bool allowFreeze
    ) {
        vault = vaultAddress;
        token = assetAddress;
        admin = adminAddress;
        guardian = guardianAddress;
        freezeEnabled = allowFreeze;

        for (uint256 i = 0; i < ACTOR_COUNT; ++i) {
            address actor = address(uint160(uint256(keccak256(abi.encode("actor", i)))));
            actors.push(actor);
            token.mint(actor, ACTOR_FUNDING);
            vm.prank(actor);
            token.approve(address(vault), type(uint256).max);
        }

        actionNames = [
            "requestDeposit",
            "cancelDeposit",
            "requestRedeem",
            "cancelRedeem",
            "cutoff",
            "finalize",
            "openNext",
            "claimDeposit",
            "claimRedeem",
            "refundDeposit",
            "refundRedeem",
            "pause",
            "unpause",
            "freeze",
            "abort",
            "donate",
            "reconcile",
            "advanceTime"
        ];

        _bootstrap();
    }

    /// Settles one epoch up front so every actor starts with shares and the
    /// redemption side is reachable from the first fuzzed call.
    function _bootstrap() private {
        uint64 epochId = vault.currentEpochId();
        for (uint256 i = 0; i < actors.length; ++i) {
            vm.prank(actors[i]);
            vault.requestDeposit(10_000e6);
        }

        vm.warp(vault.currentEpochCutoffAt());
        vault.cutoffEpoch();
        vault.finalizeEpoch();

        SettledEpoch memory terms = vault.settledEpoch(epochId);
        ghostMintedShares += terms.mintedShares;
        ghostBurnedShares += terms.redeemShares;
        finalizedEpochs.push(epochId);
        termsFingerprint[epochId] = _fingerprint(epochId);

        for (uint256 i = 0; i < actors.length; ++i) {
            vm.prank(actors[i]);
            vault.claimDeposit(epochId);
            ghostDepositClaims += 1;
        }

        vault.openNextEpoch();
    }

    modifier record(string memory name) {
        attemptCount[keccak256(bytes(name))] += 1;
        _;
    }

    function _did(string memory name) private {
        effectiveCount[keccak256(bytes(name))] += 1;
    }

    function actorCount() external view returns (uint256) {
        return actors.length;
    }

    function actionCount() external view returns (uint256) {
        return actionNames.length;
    }

    function finalizedEpochCount() external view returns (uint256) {
        return finalizedEpochs.length;
    }

    function abortedEpochCount() external view returns (uint256) {
        return abortedEpochs.length;
    }

    function _actor(uint256 seed) private view returns (address) {
        return actors[seed % actors.length];
    }

    function _open() private view returns (bool) {
        return vault.epochOpen() && vault.currentEpochPhase() == EpochPhase.Open;
    }

    function _active() private view returns (bool) {
        return vault.vaultStatus() == VaultStatus.Active;
    }

    /// Opens the next epoch when the slot is empty, the way a keeper would.
    function _ensureOpenEpoch() private {
        if (!_active() || vault.epochOpen()) return;
        if (block.timestamp < vault.lastCutoffAt()) return;
        vault.openNextEpoch();
    }

    function _fingerprint(uint64 epochId) private view returns (bytes32) {
        return keccak256(abi.encode(vault.settledEpoch(epochId)));
    }

    /// Cross multiplied comparison so no division is needed.
    function _priceIsAtLeast(
        uint256 assetsNow,
        uint256 supplyNow,
        uint256 assetsBefore,
        uint256 supplyBefore
    ) private pure returns (bool) {
        return (assetsNow + 1) * (supplyBefore + 1e12) >= (assetsBefore + 1) * (supplyNow + 1e12);
    }

    // Requests

    function requestDeposit(uint256 actorSeed, uint256 amount) external record("requestDeposit") {
        _ensureOpenEpoch();
        if (!_active() || !_open()) return;
        address actor = _actor(actorSeed);
        if (vault.depositRequestOf(vault.currentEpochId(), actor).settled) return;
        if (
            vault.depositControllers(vault.currentEpochId()).length
                == vault.MAX_DEPOSIT_REQUESTS_PER_EPOCH()
        ) {
            if (vault.depositRequestOf(vault.currentEpochId(), actor).assets == 0) return;
        }
        amount = bound(amount, MIN_DEPOSIT, MAX_DEPOSIT);
        if (token.balanceOf(actor) < amount) return;

        vm.prank(actor);
        vault.requestDeposit(amount);
        _did("requestDeposit");
    }

    function cancelDeposit(uint256 seed) external record("cancelDeposit") {
        if (vault.vaultStatus() == VaultStatus.Frozen || !_open()) return;
        uint64 epochId = vault.currentEpochId();
        address[] memory listed = vault.depositControllers(epochId);
        if (listed.length == 0) return;
        address actor = listed[seed % listed.length];
        DepositRequest memory request = vault.depositRequestOf(epochId, actor);
        if (request.settled || request.assets == 0) return;

        vm.prank(actor);
        vault.cancelDepositRequest();
        _did("cancelDeposit");
    }

    function requestRedeem(uint256 actorSeed, uint256 amount) external record("requestRedeem") {
        _ensureOpenEpoch();
        if (!_active() || !_open()) return;
        address actor = _actor(actorSeed);
        uint64 epochId = vault.currentEpochId();
        if (vault.redeemRequestOf(epochId, actor).settled) return;
        if (vault.redeemControllers(epochId).length == vault.MAX_REDEEM_REQUESTS_PER_EPOCH()) {
            if (vault.redeemRequestOf(epochId, actor).shares == 0) return;
        }
        uint256 held = vault.balanceOf(actor);
        if (held < MIN_REDEEM) return;
        amount = bound(amount, MIN_REDEEM, held);

        vm.prank(actor);
        vault.requestRedeem(amount);
        _did("requestRedeem");
    }

    function cancelRedeem(uint256 seed) external record("cancelRedeem") {
        if (vault.vaultStatus() == VaultStatus.Frozen || !_open()) return;
        uint64 epochId = vault.currentEpochId();
        address[] memory listed = vault.redeemControllers(epochId);
        if (listed.length == 0) return;
        address actor = listed[seed % listed.length];
        RedeemRequest memory request = vault.redeemRequestOf(epochId, actor);
        if (request.settled || request.shares == 0) return;

        vm.prank(actor);
        vault.cancelRedeemRequest();
        _did("cancelRedeem");
    }

    // Epoch lifecycle

    /// Waits for the cutoff timestamp when it is still ahead, the way a keeper
    /// would. The timestamp guard itself is covered by the unit tests.
    function cutoff() external record("cutoff") {
        if (!_active() || !_open()) return;
        uint64 cutoffAt = vault.currentEpochCutoffAt();
        if (block.timestamp < cutoffAt) vm.warp(cutoffAt);
        vault.cutoffEpoch();
        _did("cutoff");
    }

    function finalize() external record("finalize") {
        if (!_active() || !vault.epochOpen()) return;
        if (vault.currentEpochPhase() != EpochPhase.CutOff) return;

        uint64 epochId = vault.currentEpochId();
        uint256 assetsBefore = vault.idleBacking();
        uint256 supplyBefore = vault.totalSupply();

        vault.finalizeEpoch();

        SettledEpoch memory terms = vault.settledEpoch(epochId);
        require(terms.totalAssets == assetsBefore, "basis moved");
        require(terms.redeemAssets <= assetsBefore, "exit funded by fresh deposits");
        require(
            _priceIsAtLeast(vault.idleBacking(), vault.totalSupply(), assetsBefore, supplyBefore),
            "settlement lowered the price"
        );

        ghostMintedShares += terms.mintedShares;
        ghostBurnedShares += terms.redeemShares;
        finalizedEpochs.push(epochId);
        termsFingerprint[epochId] = _fingerprint(epochId);
        _did("finalize");
    }

    function openNext() external record("openNext") {
        if (!_active() || vault.epochOpen()) return;
        if (block.timestamp < vault.lastCutoffAt()) return;
        vault.openNextEpoch();
        _did("openNext");
    }

    function abort() external record("abort") {
        if (vault.vaultStatus() != VaultStatus.Frozen || !vault.epochOpen()) return;
        uint64 epochId = vault.currentEpochId();

        vm.prank(guardian);
        vault.abortEpoch();

        abortedEpochs.push(epochId);
        termsFingerprint[epochId] = _fingerprint(epochId);
        _did("abort");
    }

    // Claims and refunds

    function claimDeposit(uint256 actorSeed, uint256 epochSeed) external record("claimDeposit") {
        if (finalizedEpochs.length == 0) return;
        uint64 epochId = finalizedEpochs[epochSeed % finalizedEpochs.length];
        address[] memory listed = vault.depositControllers(epochId);
        if (listed.length == 0) return;
        address actor = listed[actorSeed % listed.length];
        DepositRequest memory request = vault.depositRequestOf(epochId, actor);
        if (request.cancelled || request.settled || request.assets == 0) return;

        vm.prank(actor);
        vault.claimDeposit(epochId);
        ghostDepositClaims += 1;
        _did("claimDeposit");
    }

    function claimRedeem(uint256 actorSeed, uint256 epochSeed) external record("claimRedeem") {
        if (finalizedEpochs.length == 0) return;
        uint64 epochId = finalizedEpochs[epochSeed % finalizedEpochs.length];
        address[] memory listed = vault.redeemControllers(epochId);
        if (listed.length == 0) return;
        address actor = listed[actorSeed % listed.length];
        RedeemRequest memory request = vault.redeemRequestOf(epochId, actor);
        if (request.cancelled || request.settled || request.shares == 0) return;

        vm.prank(actor);
        vault.claimRedeem(epochId);
        ghostRedeemClaims += 1;
        _did("claimRedeem");
    }

    function refundDeposit(uint256 actorSeed, uint256 epochSeed) external record("refundDeposit") {
        if (abortedEpochs.length == 0) return;
        uint64 epochId = abortedEpochs[epochSeed % abortedEpochs.length];
        address[] memory listed = vault.depositControllers(epochId);
        if (listed.length == 0) return;
        address actor = listed[actorSeed % listed.length];
        DepositRequest memory request = vault.depositRequestOf(epochId, actor);
        if (request.cancelled || request.settled || request.assets == 0) return;

        vm.prank(actor);
        vault.refundDeposit(epochId);
        ghostDepositRefunds += 1;
        _did("refundDeposit");
    }

    function refundRedeem(uint256 actorSeed, uint256 epochSeed) external record("refundRedeem") {
        if (abortedEpochs.length == 0) return;
        uint64 epochId = abortedEpochs[epochSeed % abortedEpochs.length];
        address[] memory listed = vault.redeemControllers(epochId);
        if (listed.length == 0) return;
        address actor = listed[actorSeed % listed.length];
        RedeemRequest memory request = vault.redeemRequestOf(epochId, actor);
        if (request.cancelled || request.settled || request.shares == 0) return;

        vm.prank(actor);
        vault.refundRedeem(epochId);
        ghostRedeemRefunds += 1;
        _did("refundRedeem");
    }

    // Emergency controls

    /// Pausing blocks most of the lifecycle, so it stays occasional.
    function pause(uint256 seed) external record("pause") {
        if (!_active() || seed % 4 != 0) return;
        vm.prank(admin);
        vault.pause();
        _did("pause");
    }

    function unpause() external record("unpause") {
        if (vault.vaultStatus() != VaultStatus.Paused) return;
        vm.prank(admin);
        vault.unpause();
        _did("unpause");
    }

    /// Waits for a settled epoch and a loaded open epoch, so a frozen run has
    /// claims to protect and requests to refund.
    function freeze() external record("freeze") {
        if (!freezeEnabled || vault.vaultStatus() == VaultStatus.Frozen) return;
        if (finalizedEpochs.length == 0 || !vault.epochOpen()) return;
        if (vault.epochDepositAssets() == 0 && vault.epochRedeemShares() == 0) return;
        vm.prank(guardian);
        vault.freeze();
        _did("freeze");
    }

    // Outside interference

    function donate(uint256 amount) external record("donate") {
        amount = bound(amount, 1, 1000e6);
        uint256 navBefore = vault.managedNav();

        token.mint(address(vault), amount);
        ghostDonated += amount;

        require(vault.managedNav() == navBefore, "a donation entered nav");
        _did("donate");
    }

    function reconcile() external record("reconcile") {
        uint256 navBefore = vault.managedNav();
        uint256 supplyBefore = vault.totalSupply();

        vault.reconcile();

        require(vault.managedNav() == navBefore, "reconciliation changed nav");
        require(vault.totalSupply() == supplyBefore, "reconciliation changed supply");
        _did("reconcile");
    }

    function advanceTime(uint256 seconds_) external record("advanceTime") {
        vm.warp(block.timestamp + bound(seconds_, 1, 3600));
        _did("advanceTime");
    }

    // Aggregates used by the invariants

    function totalActorShares() external view returns (uint256 total) {
        for (uint256 i = 0; i < actors.length; ++i) {
            total += vault.balanceOf(actors[i]);
        }
    }

    function totalActorAssets() external view returns (uint256 total) {
        for (uint256 i = 0; i < actors.length; ++i) {
            total += token.balanceOf(actors[i]);
        }
    }

    /// Deposit shares that finalized epochs still owe, plus their dust.
    function outstandingDepositShares() external view returns (uint256 total) {
        for (uint256 i = 0; i < finalizedEpochs.length; ++i) {
            uint64 epochId = finalizedEpochs[i];
            address[] memory list = vault.depositControllers(epochId);
            for (uint256 j = 0; j < list.length; ++j) {
                if (!vault.depositRequestOf(epochId, list[j]).settled) {
                    total += vault.claimableDepositShares(epochId, list[j]);
                }
            }
            total += vault.settledEpoch(epochId).depositDust;
        }
    }

    /// Assets that finalized epochs still owe, plus their dust.
    function outstandingRedeemAssets() external view returns (uint256 total) {
        for (uint256 i = 0; i < finalizedEpochs.length; ++i) {
            uint64 epochId = finalizedEpochs[i];
            address[] memory list = vault.redeemControllers(epochId);
            for (uint256 j = 0; j < list.length; ++j) {
                if (!vault.redeemRequestOf(epochId, list[j]).settled) {
                    total += vault.claimableRedeemAssets(epochId, list[j]);
                }
            }
            total += vault.settledEpoch(epochId).redeemDust;
        }
    }

    /// Assets aborted epochs still hold for their depositors.
    function outstandingRefundAssets() external view returns (uint256 total) {
        for (uint256 i = 0; i < abortedEpochs.length; ++i) {
            uint64 epochId = abortedEpochs[i];
            address[] memory list = vault.depositControllers(epochId);
            for (uint256 j = 0; j < list.length; ++j) {
                DepositRequest memory request = vault.depositRequestOf(epochId, list[j]);
                if (!request.settled) total += request.assets;
            }
        }
    }

    /// Shares aborted epochs still hold for their redeemers.
    function outstandingRefundShares() external view returns (uint256 total) {
        for (uint256 i = 0; i < abortedEpochs.length; ++i) {
            uint64 epochId = abortedEpochs[i];
            address[] memory list = vault.redeemControllers(epochId);
            for (uint256 j = 0; j < list.length; ++j) {
                RedeemRequest memory request = vault.redeemRequestOf(epochId, list[j]);
                if (!request.settled) total += request.shares;
            }
        }
    }

    /// Assets held for requests in the epoch that is still in the slot.
    function currentEpochEscrow() external view returns (uint256) {
        return vault.epochOpen() ? vault.epochDepositAssets() : 0;
    }

    function currentEpochShareEscrow() external view returns (uint256) {
        return vault.epochOpen() ? vault.epochRedeemShares() : 0;
    }

    function allSettledEpochs() external view returns (uint64[] memory ids) {
        ids = new uint64[](finalizedEpochs.length + abortedEpochs.length);
        for (uint256 i = 0; i < finalizedEpochs.length; ++i) {
            ids[i] = finalizedEpochs[i];
        }
        for (uint256 i = 0; i < abortedEpochs.length; ++i) {
            ids[finalizedEpochs.length + i] = abortedEpochs[i];
        }
    }
}
