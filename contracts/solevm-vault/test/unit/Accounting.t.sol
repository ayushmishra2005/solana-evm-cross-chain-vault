// SPDX-License-Identifier: Apache-2.0
pragma solidity 0.8.30;

import {SolEVMVault} from "../../src/SolEVMVault.sol";
import {ISolEVMVault, SettledEpoch} from "../../src/interfaces/ISolEVMVault.sol";
import {VaultTestBase} from "../VaultTestBase.sol";
import {FeeOnTransferAsset} from "../mocks/MockAsset.sol";

contract AccountingTest is VaultTestBase {
    /// Reaches the epoch that the Rust model settles with a two unit remainder.
    function _stateWithDust() private {
        _bootstrap(alice, 10e6);
        _redeem(alice, 3_000_000_000_001);
        _settle();
        _claimRedeem(alice, 1);
        _openNext();

        _deposit(alice, 1_000_003);
        _deposit(bob, 1_000_007);
        _deposit(carol, 1_000_011);
        _settle();
    }

    function test_a_single_claimant_leaves_no_dust() public {
        _deposit(alice, 3_141_593);
        _settle();
        assertEq(_terms(0).depositDust, 0);
        assertEq(_terms(0).redeemDust, 0);
    }

    function test_many_claimants_leave_measured_dust() public {
        _stateWithDust();

        SettledEpoch memory terms = _terms(2);
        assertEq(terms.totalAssets, 9_999_997);
        assertEq(terms.totalSupply, 9_999_996_999_999_999_999);
        assertEq(terms.depositAssets, 3_000_021);
        assertEq(terms.mintedShares, 3_000_020_999_999_999_999);
        assertEq(terms.depositDust, 2);

        assertEq(vault.claimableDepositShares(2, alice), 1_000_002_999_999_999_999);
        assertEq(vault.claimableDepositShares(2, bob), 1_000_006_999_999_999_999);
        assertEq(vault.claimableDepositShares(2, carol), 1_000_010_999_999_999_999);
    }

    function test_dust_stays_below_the_claim_count() public {
        _stateWithDust();
        SettledEpoch memory terms = _terms(2);
        uint256 count = vault.depositControllers(2).length;
        assertLe(terms.depositDust, count - 1, "dust above one unit per claim");
    }

    function test_claims_plus_dust_equal_the_minted_total() public {
        _stateWithDust();
        SettledEpoch memory terms = _terms(2);

        uint256 claimed = _claimDeposit(alice, 2) + _claimDeposit(bob, 2) + _claimDeposit(carol, 2);
        assertEq(claimed + terms.depositDust, terms.mintedShares);
        assertEq(vault.balanceOf(address(vault)), terms.depositDust, "dust left custody");
    }

    function test_claim_order_does_not_change_any_entitlement() public {
        _stateWithDust();
        uint256 aliceShare = vault.claimableDepositShares(2, alice);
        uint256 bobShare = vault.claimableDepositShares(2, bob);

        assertEq(_claimDeposit(carol, 2), vault.claimableDepositShares(2, carol));
        assertEq(_claimDeposit(bob, 2), bobShare);
        assertEq(_claimDeposit(alice, 2), aliceShare);
    }

    function test_redemption_dust_stays_in_the_claim_reserve() public {
        _bootstrap(alice, 10e6);
        vm.prank(alice);
        vault.transfer(bob, 3e18);
        vm.prank(alice);
        vault.transfer(carol, 3e18);
        _openNextIfClosed();

        _redeem(alice, 1_333_333_333_333_333_333);
        _redeem(bob, 1_333_333_333_333_333_333);
        _redeem(carol, 1_333_333_333_333_333_333);
        _settle();

        SettledEpoch memory terms = _terms(1);
        uint256 paid = _claimRedeem(alice, 1) + _claimRedeem(bob, 1) + _claimRedeem(carol, 1);
        assertEq(paid + terms.redeemDust, terms.redeemAssets);
        assertEq(vault.claimReserve(), terms.redeemDust);
        _assertSolvent();
    }

    function _openNextIfClosed() private {
        if (!vault.epochOpen()) vault.openNextEpoch();
    }

    // Donations and reconciliation

    function test_a_donation_does_not_change_the_settlement_price() public {
        _deposit(alice, 4e6);
        token.mint(address(vault), 750);
        _settle();

        SettledEpoch memory terms = _terms(0);
        assertEq(terms.totalAssets, 0);
        assertEq(terms.mintedShares, 4e18, "donation moved the price");
        assertEq(vault.idleBacking(), 4e6);
        assertEq(vault.unattributedBalance(), 750);
    }

    function test_a_donation_after_settlement_stays_outside_nav() public {
        _bootstrap(alice, 10e6);
        uint256 navBefore = vault.managedNav();

        token.mint(address(vault), 1_000_000);
        vault.reconcile();

        assertEq(vault.managedNav(), navBefore, "donation entered nav");
        assertEq(vault.unattributedBalance(), 1_000_000);

        // A later deposit is still priced one to one.
        _deposit(bob, 5e6);
        _settle();
        assertEq(_terms(1).mintedShares, 5e18);
    }

    function test_reconciliation_is_permissionless_and_moves_nothing() public {
        _bootstrap(alice, 10e6);
        token.mint(address(vault), 321);
        uint256 held = token.balanceOf(address(vault));

        vm.prank(outsider);
        uint256 surplus = vault.reconcile();

        assertEq(surplus, 321);
        assertEq(token.balanceOf(address(vault)), held, "reconciliation moved assets");
    }

    function test_reconciliation_without_a_surplus_reports_zero() public {
        _bootstrap(alice, 10e6);
        assertEq(vault.reconcile(), 0);
        assertEq(vault.unattributedBalance(), 0);
    }

    function test_an_accounting_deficit_rejects_new_work() public {
        _bootstrap(alice, 10e6);
        uint256 accounted = vault.accountedAssets();

        // Simulate assets leaving without the vault knowing.
        vm.prank(address(vault));
        token.transfer(outsider, 1e6);

        vm.expectRevert(
            abi.encodeWithSelector(
                ISolEVMVault.AccountingDeficit.selector, accounted, accounted - 1e6
            )
        );
        vm.prank(bob);
        vault.requestDeposit(1e6);

        vm.expectRevert(
            abi.encodeWithSelector(
                ISolEVMVault.AccountingDeficit.selector, accounted, accounted - 1e6
            )
        );
        vault.reconcile();
    }

    function test_a_deficit_blocks_finalization() public {
        _bootstrap(alice, 10e6);
        _deposit(bob, 5e6);
        vm.prank(address(vault));
        token.transfer(outsider, 1);

        vm.warp(vault.currentEpochCutoffAt());
        vault.cutoffEpoch();
        vm.expectRevert();
        vault.finalizeEpoch();
    }

    function test_a_fee_on_transfer_asset_is_rejected_on_deposit() public {
        FeeOnTransferAsset feeAsset = new FeeOnTransferAsset(7);
        SolEVMVault feeVault = new SolEVMVault(
            feeAsset, admin, guardian, EPOCH_DURATION, MIN_DEPOSIT, MIN_REDEEM, 1, "fee", "fee"
        );

        feeAsset.mint(alice, FUNDING);
        vm.prank(alice);
        feeAsset.approve(address(feeVault), type(uint256).max);

        vm.expectRevert(
            abi.encodeWithSelector(ISolEVMVault.InvalidTransferAmount.selector, 1e6, 1e6 - 7)
        );
        vm.prank(alice);
        feeVault.requestDeposit(1e6);
    }

    // Buckets

    function test_managed_nav_equals_idle_backing_only() public {
        _bootstrap(alice, 10e6);
        _deposit(bob, 4e6);
        _redeem(alice, 2e18);
        _settle();

        assertEq(vault.managedNav(), vault.idleBacking());
        assertTrue(vault.claimReserve() > 0);
        assertEq(
            vault.accountedAssets(),
            vault.idleBacking() + vault.pendingDepositEscrow() + vault.claimReserve()
                + vault.unattributedBalance()
        );
    }

    function test_pending_deposits_stay_outside_nav() public {
        _bootstrap(alice, 10e6);
        _deposit(bob, 25e6);

        assertEq(vault.managedNav(), 10e6, "escrow entered nav");
        assertEq(vault.pendingDepositEscrow(), 25e6);
    }

    function test_escrowed_redemption_shares_stay_in_total_supply() public {
        _bootstrap(alice, 10e6);
        uint256 supply = vault.totalSupply();
        _redeem(alice, 4e18);

        assertEq(vault.totalSupply(), supply, "escrow left total supply");
        assertEq(vault.balanceOf(address(vault)), 4e18);
        assertEq(vault.balanceOf(alice), 6e18);
    }

    function test_settled_shares_burn_exactly_once() public {
        _bootstrap(alice, 10e6);
        _redeem(alice, 4e18);
        uint256 supply = vault.totalSupply();
        _settle();

        assertEq(vault.totalSupply(), supply - 4e18);
        assertEq(vault.balanceOf(address(vault)), 0);
    }

    function test_a_claim_cannot_be_consumed_twice() public {
        _bootstrap(alice, 10e6);
        _deposit(bob, 4e6);
        _settle();
        _claimDeposit(bob, 1);

        vm.expectRevert(abi.encodeWithSelector(ISolEVMVault.ClaimAlreadyConsumed.selector, 1, bob));
        vm.prank(bob);
        vault.claimDeposit(1);
    }

    function test_a_redemption_claim_cannot_be_consumed_twice() public {
        _bootstrap(alice, 10e6);
        _redeem(alice, 4e18);
        _settle();
        _claimRedeem(alice, 1);

        vm.expectRevert(
            abi.encodeWithSelector(ISolEVMVault.ClaimAlreadyConsumed.selector, 1, alice)
        );
        vm.prank(alice);
        vault.claimRedeem(1);
    }

    function test_claiming_without_a_request_reverts() public {
        _bootstrap(alice, 10e6);
        _deposit(bob, 4e6);
        _settle();

        vm.expectRevert(abi.encodeWithSelector(ISolEVMVault.RequestNotFound.selector, carol));
        vm.prank(carol);
        vault.claimDeposit(1);
    }

    function test_claiming_a_cancelled_request_reverts() public {
        _bootstrap(alice, 10e6);
        _deposit(bob, 4e6);
        _cancelDeposit(bob);
        _settle();

        vm.expectRevert(abi.encodeWithSelector(ISolEVMVault.RequestAlreadyCancelled.selector, bob));
        vm.prank(bob);
        vault.claimDeposit(1);
    }

    function test_claiming_an_unsettled_epoch_reverts() public {
        _deposit(alice, 4e6);
        vm.expectRevert(abi.encodeWithSelector(ISolEVMVault.EpochNotFinalized.selector, uint64(0)));
        vm.prank(alice);
        vault.claimDeposit(0);
    }
}
