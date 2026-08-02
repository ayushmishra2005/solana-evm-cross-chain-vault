// SPDX-License-Identifier: Apache-2.0
pragma solidity 0.8.30;

import {DepositRequest, ISolEVMVault, RedeemRequest} from "../../src/interfaces/ISolEVMVault.sol";
import {VaultTestBase} from "../VaultTestBase.sol";

contract RequestsTest is VaultTestBase {
    function test_a_zero_deposit_reverts() public {
        vm.expectRevert(ISolEVMVault.ZeroAmount.selector);
        vm.prank(alice);
        vault.requestDeposit(0);
    }

    function test_a_deposit_below_the_minimum_reverts() public {
        vm.expectRevert(
            abi.encodeWithSelector(
                ISolEVMVault.AmountBelowMinimum.selector, MIN_DEPOSIT - 1, MIN_DEPOSIT
            )
        );
        vm.prank(alice);
        vault.requestDeposit(MIN_DEPOSIT - 1);
    }

    function test_a_redemption_below_the_minimum_reverts() public {
        _bootstrap(alice, 10e6);
        vm.expectRevert(
            abi.encodeWithSelector(
                ISolEVMVault.AmountBelowMinimum.selector, MIN_REDEEM - 1, MIN_REDEEM
            )
        );
        vm.prank(alice);
        vault.requestRedeem(MIN_REDEEM - 1);
    }

    function test_redeeming_more_shares_than_held_reverts() public {
        _bootstrap(alice, 10e6);
        vm.expectRevert(
            abi.encodeWithSelector(ISolEVMVault.InsufficientBalance.selector, 10e18, 11e18)
        );
        vm.prank(alice);
        vault.requestRedeem(11e18);
    }

    function test_repeat_deposits_add_up_in_one_request() public {
        _deposit(alice, 2e6);
        _deposit(alice, 3e6);

        DepositRequest memory request = vault.depositRequestOf(0, alice);
        assertEq(request.assets, 5e6);
        assertEq(vault.pendingDepositEscrow(), 5e6);
        assertEq(vault.epochDepositAssets(), 5e6);
        assertEq(vault.depositControllers(0).length, 1, "controller listed twice");
    }

    function test_repeat_redemptions_add_up_in_one_request() public {
        _bootstrap(alice, 10e6);
        _redeem(alice, 2e18);
        _redeem(alice, 3e18);

        RedeemRequest memory request = vault.redeemRequestOf(1, alice);
        assertEq(request.shares, 5e18);
        assertEq(vault.epochRedeemShares(), 5e18);
        assertEq(vault.redeemControllers(1).length, 1, "controller listed twice");
        assertEq(vault.balanceOf(address(vault)), 5e18, "shares not escrowed");
        assertEq(vault.totalSupply(), 10e18, "escrow left total supply");
    }

    function test_cancelling_a_deposit_returns_the_exact_assets() public {
        uint256 before = token.balanceOf(alice);
        _deposit(alice, 5e6);
        _cancelDeposit(alice);

        assertEq(token.balanceOf(alice), before);
        assertEq(vault.pendingDepositEscrow(), 0);
        assertEq(vault.epochDepositAssets(), 0);

        DepositRequest memory request = vault.depositRequestOf(0, alice);
        assertEq(request.assets, 0);
        assertEq(request.cancelledAssets, 5e6);
        assertTrue(request.cancelled);
    }

    function test_cancelling_a_redemption_returns_the_exact_shares() public {
        _bootstrap(alice, 10e6);
        _redeem(alice, 4e18);
        _cancelRedeem(alice);

        assertEq(vault.balanceOf(alice), 10e18);
        assertEq(vault.epochRedeemShares(), 0);

        RedeemRequest memory request = vault.redeemRequestOf(1, alice);
        assertEq(request.shares, 0);
        assertEq(request.cancelledShares, 4e18);
        assertTrue(request.cancelled);
    }

    function test_cancelling_frees_the_request_slot() public {
        _deposit(alice, 2e6);
        _deposit(bob, 2e6);
        assertEq(vault.depositControllers(0).length, 2);

        _cancelDeposit(alice);
        address[] memory listed = vault.depositControllers(0);
        assertEq(listed.length, 1);
        assertEq(listed[0], bob, "swap and pop kept the wrong controller");
    }

    function test_a_cancelled_controller_may_request_again() public {
        _deposit(alice, 5e6);
        _cancelDeposit(alice);
        _deposit(alice, 2e6);

        DepositRequest memory request = vault.depositRequestOf(0, alice);
        assertEq(request.assets, 2e6);
        assertEq(request.cancelledAssets, 5e6);
        assertFalse(request.cancelled);
        assertEq(vault.pendingDepositEscrow(), 2e6);
        assertEq(vault.depositControllers(0).length, 1);

        _settle();
        assertEq(_terms(0).depositAssets, 2e6, "cancelled amount was finalized");
        assertEq(_claimDeposit(alice, 0), 2e18);
    }

    function test_a_cancelled_redemption_may_be_reopened() public {
        _bootstrap(alice, 10e6);
        _redeem(alice, 4e18);
        _cancelRedeem(alice);
        _redeem(alice, 2e18);

        assertEq(vault.epochRedeemShares(), 2e18);
        assertEq(vault.redeemControllers(1).length, 1);
        _settle();
        assertEq(_terms(1).redeemShares, 2e18);
    }

    function test_cancelling_without_a_request_reverts() public {
        vm.expectRevert(
            abi.encodeWithSelector(ISolEVMVault.RequestAlreadyCancelled.selector, alice)
        );
        vm.prank(alice);
        vault.cancelDepositRequest();
    }

    function test_cancelling_twice_reverts() public {
        _deposit(alice, 2e6);
        _cancelDeposit(alice);
        vm.expectRevert(
            abi.encodeWithSelector(ISolEVMVault.RequestAlreadyCancelled.selector, alice)
        );
        vm.prank(alice);
        vault.cancelDepositRequest();
    }

    function test_cancelling_after_cutoff_reverts() public {
        _deposit(alice, 2e6);
        vm.warp(vault.currentEpochCutoffAt());
        vault.cutoffEpoch();

        vm.expectRevert(ISolEVMVault.CancellationAfterCutoff.selector);
        vm.prank(alice);
        vault.cancelDepositRequest();
    }

    function test_cancelling_a_redemption_after_cutoff_reverts() public {
        _bootstrap(alice, 10e6);
        _redeem(alice, 2e18);
        vm.warp(vault.currentEpochCutoffAt());
        vault.cutoffEpoch();

        vm.expectRevert(ISolEVMVault.CancellationAfterCutoff.selector);
        vm.prank(alice);
        vault.cancelRedeemRequest();
    }

    function test_the_deposit_request_cap_is_enforced() public {
        uint256 cap = vault.MAX_DEPOSIT_REQUESTS_PER_EPOCH();
        for (uint256 i = 0; i < cap; ++i) {
            address who = address(uint160(0x1000 + i));
            _fund(who);
            _deposit(who, 1e6);
        }
        assertEq(vault.depositControllers(0).length, cap);

        vm.expectRevert(abi.encodeWithSelector(ISolEVMVault.RequestLimitReached.selector, cap));
        vm.prank(alice);
        vault.requestDeposit(1e6);
    }

    function test_a_listed_controller_may_still_add_at_the_cap() public {
        uint256 cap = vault.MAX_DEPOSIT_REQUESTS_PER_EPOCH();
        address last;
        for (uint256 i = 0; i < cap; ++i) {
            last = address(uint160(0x2000 + i));
            _fund(last);
            _deposit(last, 1e6);
        }

        vm.prank(last);
        vault.requestDeposit(1e6);
        assertEq(vault.depositRequestOf(0, last).assets, 2e6);
        assertEq(vault.depositControllers(0).length, cap);
    }

    function test_a_cancellation_at_the_cap_makes_room_again() public {
        uint256 cap = vault.MAX_DEPOSIT_REQUESTS_PER_EPOCH();
        address first = address(uint160(0x3000));
        for (uint256 i = 0; i < cap; ++i) {
            address who = address(uint160(0x3000 + i));
            _fund(who);
            _deposit(who, 1e6);
        }

        vm.prank(first);
        vault.cancelDepositRequest();
        _deposit(alice, 1e6);
        assertEq(vault.depositControllers(0).length, cap);
    }

    function test_the_redeem_request_cap_is_enforced() public {
        _bootstrap(alice, 100e6);
        uint256 cap = vault.MAX_REDEEM_REQUESTS_PER_EPOCH();
        for (uint256 i = 0; i < cap; ++i) {
            address who = address(uint160(0x4000 + i));
            vm.prank(alice);
            vault.transfer(who, 1e18);
            vm.prank(who);
            vault.requestRedeem(1e18);
        }

        vm.expectRevert(abi.encodeWithSelector(ISolEVMVault.RequestLimitReached.selector, cap));
        vm.prank(alice);
        vault.requestRedeem(1e18);
    }

    function test_requesting_after_a_claim_in_the_same_epoch_reverts() public {
        _deposit(alice, 2e6);
        _settle();
        _claimDeposit(alice, 0);

        // The genesis epoch already settled, so its slot cannot be reused.
        vm.expectRevert(ISolEVMVault.EpochNotOpen.selector);
        vm.prank(alice);
        vault.requestDeposit(2e6);
    }
}
