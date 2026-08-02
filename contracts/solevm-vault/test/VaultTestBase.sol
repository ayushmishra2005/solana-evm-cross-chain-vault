// SPDX-License-Identifier: Apache-2.0
pragma solidity 0.8.30;

import {Test} from "forge-std/Test.sol";

import {SolEVMVault} from "../src/SolEVMVault.sol";
import {SettledEpoch} from "../src/interfaces/ISolEVMVault.sol";
import {MockAsset} from "./mocks/MockAsset.sol";

/// Shared fixture. Values mirror the Rust reference model configuration.
abstract contract VaultTestBase is Test {
    uint64 internal constant EPOCH_DURATION = 3600;
    uint256 internal constant MIN_DEPOSIT = 1e6;
    uint256 internal constant MIN_REDEEM = 1e12;
    uint256 internal constant FUNDING = 1_000_000e6;
    uint32 internal constant CONFIG_VERSION = 1;

    address internal admin = makeAddr("admin");
    address internal guardian = makeAddr("guardian");
    address internal outsider = makeAddr("outsider");
    address internal alice = makeAddr("alice");
    address internal bob = makeAddr("bob");
    address internal carol = makeAddr("carol");

    MockAsset internal token;
    SolEVMVault internal vault;

    function setUp() public virtual {
        token = new MockAsset();
        vault = new SolEVMVault(
            token,
            admin,
            guardian,
            EPOCH_DURATION,
            MIN_DEPOSIT,
            MIN_REDEEM,
            CONFIG_VERSION,
            "SolEVM Vault Share",
            "svUSD"
        );
        _fund(alice);
        _fund(bob);
        _fund(carol);
    }

    function _fund(address who) internal {
        token.mint(who, FUNDING);
        vm.prank(who);
        token.approve(address(vault), type(uint256).max);
    }

    function _deposit(address who, uint256 assets) internal {
        vm.prank(who);
        vault.requestDeposit(assets);
    }

    function _redeem(address who, uint256 shares) internal {
        vm.prank(who);
        vault.requestRedeem(shares);
    }

    function _cancelDeposit(address who) internal {
        vm.prank(who);
        vault.cancelDepositRequest();
    }

    function _cancelRedeem(address who) internal {
        vm.prank(who);
        vault.cancelRedeemRequest();
    }

    function _claimDeposit(address who, uint64 epochId) internal returns (uint256) {
        vm.prank(who);
        return vault.claimDeposit(epochId);
    }

    function _claimRedeem(address who, uint64 epochId) internal returns (uint256) {
        vm.prank(who);
        return vault.claimRedeem(epochId);
    }

    /// Moves to the cutoff of the open epoch and settles it.
    function _settle() internal {
        vm.warp(vault.currentEpochCutoffAt());
        vault.cutoffEpoch();
        vault.finalizeEpoch();
    }

    function _openNext() internal {
        vault.openNextEpoch();
    }

    function _terms(uint64 epochId) internal view returns (SettledEpoch memory) {
        return vault.settledEpoch(epochId);
    }

    /// Deposits, settles the genesis epoch, claims and opens the next epoch.
    function _bootstrap(address who, uint256 assets) internal {
        _deposit(who, assets);
        _settle();
        _claimDeposit(who, 0);
        _openNext();
    }

    /// Every liquid bucket must be covered by the real token balance.
    function _assertSolvent() internal view {
        assertGe(
            token.balanceOf(address(vault)), vault.accountedAssets(), "token balance below buckets"
        );
    }
}
