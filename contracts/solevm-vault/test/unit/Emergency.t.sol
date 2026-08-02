// SPDX-License-Identifier: Apache-2.0
pragma solidity 0.8.30;

import {SolEVMVault} from "../../src/SolEVMVault.sol";
import {
    EpochOutcome,
    ISolEVMVault,
    SettledEpoch,
    VaultStatus
} from "../../src/interfaces/ISolEVMVault.sol";
import {VaultTestBase} from "../VaultTestBase.sol";
import {EighteenDecimalAsset, MockAsset} from "../mocks/MockAsset.sol";

contract EmergencyTest is VaultTestBase {
    function _pause() private {
        vm.prank(admin);
        vault.pause();
    }

    function _freeze() private {
        vm.prank(guardian);
        vault.freeze();
    }

    // Authority

    function test_only_admin_or_guardian_may_pause() public {
        vm.expectRevert(abi.encodeWithSelector(ISolEVMVault.Unauthorized.selector, outsider));
        vm.prank(outsider);
        vault.pause();

        vm.prank(guardian);
        vault.pause();
        assertEq(uint8(vault.vaultStatus()), uint8(VaultStatus.Paused));
    }

    function test_the_guardian_cannot_unpause() public {
        _pause();
        vm.expectRevert(abi.encodeWithSelector(ISolEVMVault.Unauthorized.selector, guardian));
        vm.prank(guardian);
        vault.unpause();

        vm.prank(admin);
        vault.unpause();
        assertEq(uint8(vault.vaultStatus()), uint8(VaultStatus.Active));
    }

    function test_both_roles_may_freeze() public {
        vm.prank(admin);
        vault.freeze();
        assertEq(uint8(vault.vaultStatus()), uint8(VaultStatus.Frozen));
    }

    function test_only_admin_or_guardian_may_abort() public {
        _freeze();
        vm.expectRevert(abi.encodeWithSelector(ISolEVMVault.Unauthorized.selector, outsider));
        vm.prank(outsider);
        vault.abortEpoch();
    }

    function test_freezing_is_terminal() public {
        _freeze();
        vm.expectRevert(
            abi.encodeWithSelector(ISolEVMVault.InvalidVaultStatus.selector, VaultStatus.Frozen)
        );
        vm.prank(admin);
        vault.unpause();

        vm.expectRevert(
            abi.encodeWithSelector(ISolEVMVault.InvalidVaultStatus.selector, VaultStatus.Frozen)
        );
        vm.prank(admin);
        vault.freeze();
    }

    function test_a_paused_vault_can_still_be_frozen() public {
        _pause();
        _freeze();
        assertEq(uint8(vault.vaultStatus()), uint8(VaultStatus.Frozen));
    }

    function test_zero_addresses_are_refused_at_construction() public {
        vm.expectRevert(ISolEVMVault.InvalidAddress.selector);
        new SolEVMVault(
            MockAsset(address(0)),
            admin,
            guardian,
            EPOCH_DURATION,
            MIN_DEPOSIT,
            MIN_REDEEM,
            1,
            "n",
            "s"
        );

        vm.expectRevert(ISolEVMVault.InvalidAddress.selector);
        new SolEVMVault(
            token, address(0), guardian, EPOCH_DURATION, MIN_DEPOSIT, MIN_REDEEM, 1, "n", "s"
        );

        vm.expectRevert(ISolEVMVault.InvalidAddress.selector);
        new SolEVMVault(
            token, admin, address(0), EPOCH_DURATION, MIN_DEPOSIT, MIN_REDEEM, 1, "n", "s"
        );
    }

    function test_equal_authority_roles_are_refused() public {
        vm.expectRevert(ISolEVMVault.InvalidAddress.selector);
        new SolEVMVault(token, admin, admin, EPOCH_DURATION, MIN_DEPOSIT, MIN_REDEEM, 1, "n", "s");
    }

    function test_an_asset_with_the_wrong_decimals_is_refused() public {
        EighteenDecimalAsset wide = new EighteenDecimalAsset();
        vm.expectRevert(ISolEVMVault.InvalidAddress.selector);
        new SolEVMVault(wide, admin, guardian, EPOCH_DURATION, MIN_DEPOSIT, MIN_REDEEM, 1, "n", "s");
    }

    function test_zero_configuration_values_are_refused() public {
        vm.expectRevert(ISolEVMVault.ZeroAmount.selector);
        new SolEVMVault(token, admin, guardian, 0, MIN_DEPOSIT, MIN_REDEEM, 1, "n", "s");

        vm.expectRevert(ISolEVMVault.ZeroAmount.selector);
        new SolEVMVault(token, admin, guardian, EPOCH_DURATION, 0, MIN_REDEEM, 1, "n", "s");

        vm.expectRevert(ISolEVMVault.ZeroAmount.selector);
        new SolEVMVault(token, admin, guardian, EPOCH_DURATION, MIN_DEPOSIT, 0, 1, "n", "s");
    }

    // Paused behavior

    function test_requests_are_refused_while_paused() public {
        _bootstrap(alice, 10e6);
        _pause();

        vm.expectRevert(
            abi.encodeWithSelector(ISolEVMVault.InvalidVaultStatus.selector, VaultStatus.Paused)
        );
        vm.prank(alice);
        vault.requestDeposit(1e6);

        vm.expectRevert(
            abi.encodeWithSelector(ISolEVMVault.InvalidVaultStatus.selector, VaultStatus.Paused)
        );
        vm.prank(alice);
        vault.requestRedeem(1e18);
    }

    function test_cancellation_still_works_while_paused() public {
        _deposit(alice, 5e6);
        uint256 before = token.balanceOf(alice);
        _pause();

        _cancelDeposit(alice);
        assertEq(token.balanceOf(alice), before + 5e6);
    }

    function test_cutoff_and_finalization_are_refused_while_paused() public {
        _deposit(alice, 5e6);
        vm.warp(vault.currentEpochCutoffAt());
        _pause();

        vm.expectRevert(
            abi.encodeWithSelector(ISolEVMVault.InvalidVaultStatus.selector, VaultStatus.Paused)
        );
        vault.cutoffEpoch();

        vm.expectRevert(
            abi.encodeWithSelector(ISolEVMVault.InvalidVaultStatus.selector, VaultStatus.Paused)
        );
        vault.finalizeEpoch();
    }

    function test_claims_still_work_while_paused() public {
        _bootstrap(alice, 10e6);
        _deposit(bob, 4e6);
        _redeem(alice, 2e18);
        _settle();
        _pause();

        assertEq(_claimDeposit(bob, 1), 4e18);
        assertEq(_claimRedeem(alice, 1), 2e6);
    }

    // Frozen behavior

    function test_requests_and_cancellation_are_refused_while_frozen() public {
        _deposit(alice, 5e6);
        _freeze();

        vm.expectRevert(
            abi.encodeWithSelector(ISolEVMVault.InvalidVaultStatus.selector, VaultStatus.Frozen)
        );
        vm.prank(alice);
        vault.requestDeposit(1e6);

        vm.expectRevert(
            abi.encodeWithSelector(ISolEVMVault.InvalidVaultStatus.selector, VaultStatus.Frozen)
        );
        vm.prank(alice);
        vault.cancelDepositRequest();
    }

    function test_finalization_is_refused_while_frozen() public {
        _deposit(alice, 5e6);
        vm.warp(vault.currentEpochCutoffAt());
        vault.cutoffEpoch();
        _freeze();

        vm.expectRevert(
            abi.encodeWithSelector(ISolEVMVault.InvalidVaultStatus.selector, VaultStatus.Frozen)
        );
        vault.finalizeEpoch();
    }

    function test_claims_still_work_while_frozen() public {
        _bootstrap(alice, 10e6);
        _deposit(bob, 4e6);
        _redeem(alice, 2e18);
        _settle();
        _freeze();

        assertEq(_claimDeposit(bob, 1), 4e18);
        assertEq(_claimRedeem(alice, 1), 2e6);
    }

    // Abort

    function test_aborting_requires_a_frozen_vault() public {
        _deposit(alice, 5e6);
        vm.expectRevert(
            abi.encodeWithSelector(ISolEVMVault.InvalidVaultStatus.selector, VaultStatus.Active)
        );
        vm.prank(admin);
        vault.abortEpoch();
    }

    function test_aborting_an_open_epoch_records_the_refunds() public {
        _bootstrap(alice, 10e6);
        _deposit(bob, 7e6);
        _redeem(alice, 4e18);
        _freeze();

        vm.prank(guardian);
        vault.abortEpoch();

        SettledEpoch memory record = _terms(1);
        assertEq(uint8(record.outcome), uint8(EpochOutcome.Aborted));
        assertEq(record.depositAssets, 7e6);
        assertEq(record.redeemShares, 4e18);
        assertFalse(vault.epochOpen());

        // Escrow is preserved until each holder pulls it back.
        assertEq(vault.pendingDepositEscrow(), 7e6);
        assertEq(vault.balanceOf(address(vault)), 4e18);
        assertEq(vault.idleBacking(), 10e6);
    }

    function test_aborting_a_cut_off_epoch_works() public {
        _bootstrap(alice, 10e6);
        _deposit(bob, 7e6);
        vm.warp(vault.currentEpochCutoffAt());
        vault.cutoffEpoch();
        _freeze();

        vm.prank(admin);
        vault.abortEpoch();
        assertEq(uint8(_terms(1).outcome), uint8(EpochOutcome.Aborted));
    }

    function test_aborting_without_an_open_epoch_reverts() public {
        _deposit(alice, 5e6);
        _settle();
        _freeze();

        vm.expectRevert(ISolEVMVault.EpochNotOpen.selector);
        vm.prank(admin);
        vault.abortEpoch();
    }

    function test_refunds_return_the_exact_original_amounts() public {
        _bootstrap(alice, 10e6);
        uint256 bobAssets = token.balanceOf(bob);
        _deposit(bob, 7e6);
        _redeem(alice, 4e18);
        _freeze();
        vm.prank(guardian);
        vault.abortEpoch();

        vm.prank(bob);
        assertEq(vault.refundDeposit(1), 7e6);
        assertEq(token.balanceOf(bob), bobAssets);
        assertEq(vault.pendingDepositEscrow(), 0);

        vm.prank(alice);
        assertEq(vault.refundRedeem(1), 4e18);
        assertEq(vault.balanceOf(alice), 10e18);
        assertEq(vault.balanceOf(address(vault)), 0);
        _assertSolvent();
    }

    function test_a_refund_cannot_be_repeated() public {
        _bootstrap(alice, 10e6);
        _deposit(bob, 7e6);
        _freeze();
        vm.prank(guardian);
        vault.abortEpoch();

        vm.prank(bob);
        vault.refundDeposit(1);

        vm.expectRevert(abi.encodeWithSelector(ISolEVMVault.RefundAlreadyConsumed.selector, 1, bob));
        vm.prank(bob);
        vault.refundDeposit(1);
    }

    function test_a_refund_on_a_finalized_epoch_reverts() public {
        _deposit(alice, 5e6);
        _settle();

        vm.expectRevert(abi.encodeWithSelector(ISolEVMVault.EpochNotAborted.selector, uint64(0)));
        vm.prank(alice);
        vault.refundDeposit(0);
    }

    function test_a_claim_on_an_aborted_epoch_reverts() public {
        _bootstrap(alice, 10e6);
        _deposit(bob, 7e6);
        _freeze();
        vm.prank(guardian);
        vault.abortEpoch();

        vm.expectRevert(abi.encodeWithSelector(ISolEVMVault.EpochNotFinalized.selector, uint64(1)));
        vm.prank(bob);
        vault.claimDeposit(1);
    }

    function test_a_finalized_claim_survives_a_later_freeze_and_abort() public {
        _bootstrap(alice, 10e6);
        _deposit(bob, 4e6);
        _redeem(alice, 2e18);
        _settle();
        _openNext();

        // A fresh epoch collects requests and is then abandoned.
        _deposit(carol, 3e6);
        _freeze();
        vm.prank(guardian);
        vault.abortEpoch();

        assertEq(_claimDeposit(bob, 1), 4e18);
        assertEq(_claimRedeem(alice, 1), 2e6);

        vm.prank(carol);
        assertEq(vault.refundDeposit(2), 3e6);
        _assertSolvent();
    }

    function test_an_epoch_is_never_both_finalized_and_aborted() public {
        _deposit(alice, 5e6);
        _settle();
        _freeze();

        // The slot is already empty, so there is nothing left to abort.
        vm.expectRevert(ISolEVMVault.EpochNotOpen.selector);
        vm.prank(admin);
        vault.abortEpoch();
        assertEq(uint8(_terms(0).outcome), uint8(EpochOutcome.Finalized));
    }
}
