// SPDX-License-Identifier: Apache-2.0
pragma solidity 0.8.30;

import {ISolEVMVault, SettledEpoch} from "../../src/interfaces/ISolEVMVault.sol";
import {VaultTestBase} from "../VaultTestBase.sol";

contract LifecycleTest is VaultTestBase {
    function test_first_deposit_prices_at_the_virtual_offset() public {
        _deposit(alice, 1e6);
        _settle();

        SettledEpoch memory terms = _terms(0);
        assertEq(terms.totalAssets, 0);
        assertEq(terms.totalSupply, 0);
        assertEq(terms.depositAssets, 1e6);
        assertEq(terms.mintedShares, 1e18);
        assertEq(terms.depositDust, 0);

        assertEq(vault.idleBacking(), 1e6);
        assertEq(vault.pendingDepositEscrow(), 0);
        assertEq(vault.totalSupply(), 1e18);
        _assertSolvent();
    }

    function test_a_claim_moves_shares_out_of_vault_custody() public {
        _deposit(alice, 1e6);
        _settle();

        assertEq(vault.balanceOf(address(vault)), 1e18);
        uint256 shares = _claimDeposit(alice, 0);
        assertEq(shares, 1e18);
        assertEq(vault.balanceOf(alice), 1e18);
        assertEq(vault.balanceOf(address(vault)), 0);
    }

    function test_several_deposits_share_one_epoch_price() public {
        _bootstrap(alice, 10e6);

        _deposit(bob, 4e6);
        _deposit(carol, 3e6);
        _settle();

        SettledEpoch memory terms = _terms(1);
        assertEq(terms.totalAssets, 10e6);
        assertEq(terms.totalSupply, 10e18);
        assertEq(terms.depositAssets, 7e6);
        assertEq(terms.mintedShares, 7e18);

        assertEq(_claimDeposit(bob, 1), 4e18);
        assertEq(_claimDeposit(carol, 1), 3e18);
        _assertSolvent();
    }

    function test_several_redemptions_share_one_epoch_price() public {
        _bootstrap(alice, 10e6);
        _deposit(bob, 10e6);
        _settle();
        _claimDeposit(bob, 1);
        _openNext();

        _redeem(alice, 2e18);
        _redeem(bob, 3e18);
        _settle();

        SettledEpoch memory terms = _terms(2);
        assertEq(terms.redeemShares, 5e18);
        assertEq(terms.redeemAssets, 5e6);
        assertEq(_claimRedeem(alice, 2), 2e6);
        assertEq(_claimRedeem(bob, 2), 3e6);
        assertEq(vault.claimReserve(), 0);
        _assertSolvent();
    }

    function test_a_mixed_epoch_settles_both_sides_at_one_price() public {
        _bootstrap(alice, 10e6);

        _deposit(bob, 4e6);
        _deposit(carol, 3e6);
        _redeem(alice, 2.5e18);
        _settle();

        SettledEpoch memory terms = _terms(1);
        assertEq(terms.totalAssets, 10e6);
        assertEq(terms.totalSupply, 10e18);
        assertEq(terms.depositAssets, 7e6);
        assertEq(terms.mintedShares, 7e18);
        assertEq(terms.redeemShares, 2.5e18);
        assertEq(terms.redeemAssets, 2.5e6);

        assertEq(vault.idleBacking(), 14.5e6);
        assertEq(vault.claimReserve(), 2.5e6);
        assertEq(vault.totalSupply(), 14.5e18);

        _claimDeposit(bob, 1);
        _claimDeposit(carol, 1);
        _claimRedeem(alice, 1);

        assertEq(vault.balanceOf(alice), 7.5e18);
        assertEq(vault.balanceOf(bob), 4e18);
        assertEq(vault.balanceOf(carol), 3e18);
        assertEq(vault.claimReserve(), 0);
        _assertSolvent();
    }

    function test_same_epoch_deposits_stay_out_of_the_settlement_basis() public {
        _bootstrap(alice, 10e6);

        // A large deposit lands in the same epoch as an exit. The exit must be
        // priced against the ten units that existed before the epoch.
        _deposit(bob, 500e6);
        _redeem(alice, 5e18);
        _settle();

        SettledEpoch memory terms = _terms(1);
        assertEq(terms.totalAssets, 10e6, "basis included the fresh deposit");
        assertEq(terms.redeemAssets, 5e6);
        assertEq(_claimRedeem(alice, 1), 5e6);
    }

    function test_redeeming_every_share_drains_only_pre_epoch_liquidity() public {
        _bootstrap(alice, 10e6);
        _deposit(bob, 10e6);
        _settle();
        _claimDeposit(bob, 1);
        _openNext();

        // Both holders leave at once while a deposit lands in the same epoch.
        _redeem(alice, 10e18);
        _redeem(bob, 10e18);
        _deposit(carol, 100e6);
        _settle();

        // The exits take the pre epoch pool, and only Carol's deposit remains.
        SettledEpoch memory terms = _terms(2);
        assertEq(terms.totalAssets, 20e6);
        assertEq(terms.redeemAssets, 20e6);
        assertEq(vault.idleBacking(), 100e6);
        assertEq(_claimRedeem(alice, 2), 10e6);
        assertEq(_claimRedeem(bob, 2), 10e6);
        _assertSolvent();
    }

    function test_price_rises_across_epochs_when_rounding_favours_the_vault() public {
        _bootstrap(alice, 10e6);
        _redeem(alice, 3_000_000_000_001);
        _settle();

        SettledEpoch memory terms = _terms(1);
        assertEq(terms.redeemShares, 3_000_000_000_001);
        assertEq(terms.redeemAssets, 3);

        _claimRedeem(alice, 1);
        _openNext();

        // The vault kept a fraction of an asset unit, so shares are worth more.
        assertEq(vault.idleBacking(), 9_999_997);
        assertEq(vault.totalSupply(), 9_999_996_999_999_999_999);
    }

    function test_opening_the_next_epoch_is_a_separate_step() public {
        _deposit(alice, 1e6);
        _settle();
        assertFalse(vault.epochOpen());

        vm.expectRevert(ISolEVMVault.EpochNotOpen.selector);
        vm.prank(alice);
        vault.requestDeposit(1e6);

        _openNext();
        assertTrue(vault.epochOpen());
        assertEq(vault.currentEpochId(), 1);
    }

    function test_cutoff_before_its_timestamp_reverts() public {
        _deposit(alice, 1e6);
        uint64 cutoffAt = vault.currentEpochCutoffAt();
        vm.warp(cutoffAt - 1);
        vm.expectRevert(
            abi.encodeWithSelector(ISolEVMVault.CutoffNotReached.selector, cutoffAt - 1, cutoffAt)
        );
        vault.cutoffEpoch();
    }

    function test_cutoff_is_permissionless_once_the_timestamp_passes() public {
        _deposit(alice, 1e6);
        vm.warp(vault.currentEpochCutoffAt());
        vm.prank(outsider);
        vault.cutoffEpoch();
        vm.prank(outsider);
        vault.finalizeEpoch();
        assertEq(uint8(_terms(0).outcome), uint8(1));
    }

    function test_requests_stop_once_the_epoch_is_cut_off() public {
        _deposit(alice, 1e6);
        vm.warp(vault.currentEpochCutoffAt());
        vault.cutoffEpoch();

        vm.expectRevert(ISolEVMVault.EpochAlreadyCutOff.selector);
        vm.prank(bob);
        vault.requestDeposit(1e6);
    }

    function test_finalizing_before_cutoff_reverts() public {
        _deposit(alice, 1e6);
        vm.expectRevert(ISolEVMVault.EpochNotCutOff.selector);
        vault.finalizeEpoch();
    }

    function test_an_epoch_cannot_be_finalized_twice() public {
        _deposit(alice, 1e6);
        _settle();
        vm.expectRevert(ISolEVMVault.EpochNotOpen.selector);
        vault.finalizeEpoch();
    }

    function test_an_empty_epoch_settles_without_changing_the_price() public {
        _bootstrap(alice, 10e6);
        _settle();

        SettledEpoch memory terms = _terms(1);
        assertEq(terms.depositAssets, 0);
        assertEq(terms.redeemShares, 0);
        assertEq(vault.idleBacking(), 10e6);
        assertEq(vault.totalSupply(), 10e18);
    }

    function test_share_decimals_are_eighteen_and_asset_decimals_are_six() public view {
        assertEq(vault.decimals(), 18);
        assertEq(token.decimals(), 6);
    }
}
