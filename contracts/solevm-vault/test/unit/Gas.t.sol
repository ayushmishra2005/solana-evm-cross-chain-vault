// SPDX-License-Identifier: Apache-2.0
pragma solidity 0.8.30;

import {console} from "forge-std/console.sol";

import {VaultTestBase} from "../VaultTestBase.sol";

/// Measures the operations that decide whether the request cap is workable.
contract GasTest is VaultTestBase {
    /// A conservative share of a thirty million gas block.
    uint256 private constant FINALIZE_BUDGET = 15_000_000;

    function _report(string memory label, uint256 used) private pure {
        console.log(label, used);
    }

    /// Fills the open epoch with `count` distinct depositors.
    function _fillDeposits(uint256 count) private {
        for (uint256 i = 0; i < count; ++i) {
            address who = address(uint160(0x10000 + i));
            _fund(who);
            _deposit(who, 1e6 + i);
        }
    }

    /// Gives `count` distinct holders shares and queues an exit for each.
    function _fillRedemptions(uint256 count) private {
        for (uint256 i = 0; i < count; ++i) {
            address who = address(uint160(0x20000 + i));
            vm.prank(alice);
            vault.transfer(who, 2e18);
            vm.prank(who);
            vault.requestRedeem(1e18 + i);
        }
    }

    function test_gas_request_deposit() public {
        uint256 start = gasleft();
        vm.prank(alice);
        vault.requestDeposit(5e6);
        _report("requestDeposit first", start - gasleft());

        start = gasleft();
        vm.prank(alice);
        vault.requestDeposit(5e6);
        _report("requestDeposit repeat", start - gasleft());
    }

    function test_gas_request_redeem() public {
        _bootstrap(alice, 100e6);
        uint256 start = gasleft();
        vm.prank(alice);
        vault.requestRedeem(5e18);
        _report("requestRedeem", start - gasleft());
    }

    function test_gas_cancel() public {
        _deposit(alice, 5e6);
        uint256 start = gasleft();
        vm.prank(alice);
        vault.cancelDepositRequest();
        _report("cancelDepositRequest", start - gasleft());
    }

    function test_gas_cutoff() public {
        _deposit(alice, 5e6);
        vm.warp(vault.currentEpochCutoffAt());
        uint256 start = gasleft();
        vault.cutoffEpoch();
        _report("cutoffEpoch", start - gasleft());
    }

    function test_gas_open_next_epoch() public {
        _deposit(alice, 5e6);
        _settle();
        uint256 start = gasleft();
        vault.openNextEpoch();
        _report("openNextEpoch", start - gasleft());
    }

    function test_gas_claims() public {
        _bootstrap(alice, 100e6);
        _deposit(bob, 5e6);
        _redeem(alice, 5e18);
        _settle();

        uint256 start = gasleft();
        vm.prank(bob);
        vault.claimDeposit(1);
        _report("claimDeposit", start - gasleft());

        start = gasleft();
        vm.prank(alice);
        vault.claimRedeem(1);
        _report("claimRedeem", start - gasleft());
    }

    function test_gas_refunds() public {
        _bootstrap(alice, 100e6);
        _deposit(bob, 5e6);
        _redeem(alice, 5e18);
        vm.prank(guardian);
        vault.freeze();
        vm.prank(guardian);
        vault.abortEpoch();

        uint256 start = gasleft();
        vm.prank(bob);
        vault.refundDeposit(1);
        _report("refundDeposit", start - gasleft());

        start = gasleft();
        vm.prank(alice);
        vault.refundRedeem(1);
        _report("refundRedeem", start - gasleft());
    }

    function test_gas_abort() public {
        _deposit(alice, 5e6);
        vm.prank(guardian);
        vault.freeze();

        uint256 start = gasleft();
        vm.prank(guardian);
        vault.abortEpoch();
        _report("abortEpoch", start - gasleft());
    }

    function test_gas_finalize_one_deposit_and_one_redemption() public {
        _bootstrap(alice, 100e6);
        _deposit(bob, 5e6);
        _redeem(alice, 5e18);
        vm.warp(vault.currentEpochCutoffAt());
        vault.cutoffEpoch();

        uint256 start = gasleft();
        vault.finalizeEpoch();
        _report("finalizeEpoch 1 deposit 1 redemption", start - gasleft());
    }

    function test_gas_finalize_at_the_deposit_cap() public {
        uint256 cap = vault.MAX_DEPOSIT_REQUESTS_PER_EPOCH();
        _fillDeposits(cap);
        vm.warp(vault.currentEpochCutoffAt());
        vault.cutoffEpoch();

        uint256 start = gasleft();
        vault.finalizeEpoch();
        uint256 used = start - gasleft();
        _report("finalizeEpoch at deposit cap", used);
        assertLt(used, FINALIZE_BUDGET, "finalization at the deposit cap is too costly");
    }

    function test_gas_finalize_at_the_redeem_cap() public {
        _bootstrap(alice, 500e6);
        uint256 cap = vault.MAX_REDEEM_REQUESTS_PER_EPOCH();
        _fillRedemptions(cap);
        vm.warp(vault.currentEpochCutoffAt());
        vault.cutoffEpoch();

        uint256 start = gasleft();
        vault.finalizeEpoch();
        uint256 used = start - gasleft();
        _report("finalizeEpoch at redeem cap", used);
        assertLt(used, FINALIZE_BUDGET, "finalization at the redeem cap is too costly");
    }

    function test_gas_finalize_with_both_arrays_at_the_cap() public {
        _bootstrap(alice, 500e6);
        uint256 depositCap = vault.MAX_DEPOSIT_REQUESTS_PER_EPOCH();
        uint256 redeemCap = vault.MAX_REDEEM_REQUESTS_PER_EPOCH();
        _fillDeposits(depositCap);
        _fillRedemptions(redeemCap);

        assertEq(vault.depositControllers(1).length, depositCap);
        assertEq(vault.redeemControllers(1).length, redeemCap);

        vm.warp(vault.currentEpochCutoffAt());
        vault.cutoffEpoch();

        uint256 start = gasleft();
        vault.finalizeEpoch();
        uint256 used = start - gasleft();
        _report("finalizeEpoch at both caps", used);
        assertLt(used, FINALIZE_BUDGET, "finalization at both caps is too costly");
    }
}
