// SPDX-License-Identifier: Apache-2.0
pragma solidity 0.8.30;

import {SettledEpoch} from "../../src/interfaces/ISolEVMVault.sol";
import {VaultTestBase} from "../VaultTestBase.sol";

/// Fixed scenarios whose expected numbers come from separate runs of the Rust
/// reference model. Nothing here is derived from the contract under test.
contract DifferentialTest is VaultTestBase {
    function test_fixture_first_deposit() public {
        _deposit(alice, 1_000_000);
        _settle();

        SettledEpoch memory terms = _terms(0);
        assertEq(terms.totalAssets, 0);
        assertEq(terms.totalSupply, 0);
        assertEq(terms.depositAssets, 1_000_000);
        assertEq(terms.mintedShares, 1_000_000_000_000_000_000);
        assertEq(terms.redeemShares, 0);
        assertEq(terms.redeemAssets, 0);
        assertEq(terms.depositDust, 0);
        assertEq(terms.redeemDust, 0);

        _claimDeposit(alice, 0);
        assertEq(vault.idleBacking(), 1_000_000);
        assertEq(vault.pendingDepositEscrow(), 0);
        assertEq(vault.claimReserve(), 0);
        assertEq(vault.totalSupply(), 1_000_000_000_000_000_000);
        assertEq(token.balanceOf(alice), FUNDING - 1_000_000);
        assertEq(vault.balanceOf(alice), 1_000_000_000_000_000_000);
    }

    function test_fixture_mixed_epoch() public {
        _bootstrap(alice, 10_000_000);
        _deposit(bob, 4_000_000);
        _deposit(carol, 3_000_000);
        _redeem(alice, 2_500_000_000_000_000_000);
        _settle();

        SettledEpoch memory terms = _terms(1);
        assertEq(terms.totalAssets, 10_000_000);
        assertEq(terms.totalSupply, 10_000_000_000_000_000_000);
        assertEq(terms.depositAssets, 7_000_000);
        assertEq(terms.mintedShares, 7_000_000_000_000_000_000);
        assertEq(terms.redeemShares, 2_500_000_000_000_000_000);
        assertEq(terms.redeemAssets, 2_500_000);
        assertEq(terms.depositDust, 0);
        assertEq(terms.redeemDust, 0);

        assertEq(vault.idleBacking(), 14_500_000);
        assertEq(vault.claimReserve(), 2_500_000);
        assertEq(vault.totalSupply(), 14_500_000_000_000_000_000);

        _claimDeposit(bob, 1);
        _claimDeposit(carol, 1);
        _claimRedeem(alice, 1);

        assertEq(token.balanceOf(alice), FUNDING - 10_000_000 + 2_500_000);
        assertEq(vault.balanceOf(alice), 7_500_000_000_000_000_000);
        assertEq(token.balanceOf(bob), FUNDING - 4_000_000);
        assertEq(vault.balanceOf(bob), 4_000_000_000_000_000_000);
        assertEq(vault.balanceOf(carol), 3_000_000_000_000_000_000);
        assertEq(vault.claimReserve(), 0);
        assertEq(vault.totalSupply(), 14_500_000_000_000_000_000);
    }

    function test_fixture_cancellation_and_reopen() public {
        _deposit(alice, 5_000_000);
        _cancelDeposit(alice);
        _deposit(alice, 2_000_000);

        assertEq(vault.pendingDepositEscrow(), 2_000_000);
        assertEq(token.balanceOf(alice), FUNDING - 2_000_000);

        _settle();
        SettledEpoch memory terms = _terms(0);
        assertEq(terms.totalAssets, 0);
        assertEq(terms.totalSupply, 0);
        assertEq(terms.depositAssets, 2_000_000);
        assertEq(terms.mintedShares, 2_000_000_000_000_000_000);
        assertEq(terms.depositDust, 0);

        _claimDeposit(alice, 0);
        assertEq(token.balanceOf(alice), FUNDING - 2_000_000);
        assertEq(vault.balanceOf(alice), 2_000_000_000_000_000_000);
    }

    function test_fixture_abort_and_refund() public {
        _bootstrap(alice, 10_000_000);
        assertEq(vault.balanceOf(alice), 10_000_000_000_000_000_000);

        _deposit(bob, 7_000_000);
        _redeem(alice, 4_000_000_000_000_000_000);

        vm.prank(guardian);
        vault.freeze();
        vm.prank(guardian);
        vault.abortEpoch();

        SettledEpoch memory record = _terms(1);
        assertEq(record.depositAssets, 7_000_000);
        assertEq(record.redeemShares, 4_000_000_000_000_000_000);
        assertEq(vault.idleBacking(), 10_000_000);
        assertEq(vault.pendingDepositEscrow(), 7_000_000);
        assertEq(vault.claimReserve(), 0);
        assertEq(vault.totalSupply(), 10_000_000_000_000_000_000);

        vm.prank(bob);
        vault.refundDeposit(1);
        vm.prank(alice);
        vault.refundRedeem(1);

        assertEq(token.balanceOf(alice), FUNDING - 10_000_000);
        assertEq(vault.balanceOf(alice), 10_000_000_000_000_000_000);
        assertEq(token.balanceOf(bob), FUNDING);
        assertEq(vault.balanceOf(bob), 0);
        assertEq(vault.idleBacking(), 10_000_000);
        assertEq(vault.pendingDepositEscrow(), 0);
        assertEq(vault.totalSupply(), 10_000_000_000_000_000_000);
    }

    function test_fixture_rounding_dust() public {
        _bootstrap(alice, 10_000_000);
        _redeem(alice, 3_000_000_000_001);
        _settle();

        SettledEpoch memory first = _terms(1);
        assertEq(first.totalAssets, 10_000_000);
        assertEq(first.totalSupply, 10_000_000_000_000_000_000);
        assertEq(first.redeemShares, 3_000_000_000_001);
        assertEq(first.redeemAssets, 3);
        assertEq(first.redeemDust, 0);

        _claimRedeem(alice, 1);
        _openNext();

        _deposit(alice, 1_000_003);
        _deposit(bob, 1_000_007);
        _deposit(carol, 1_000_011);
        _settle();

        SettledEpoch memory second = _terms(2);
        assertEq(second.totalAssets, 9_999_997);
        assertEq(second.totalSupply, 9_999_996_999_999_999_999);
        assertEq(second.depositAssets, 3_000_021);
        assertEq(second.mintedShares, 3_000_020_999_999_999_999);
        assertEq(second.depositDust, 2);
        assertEq(second.redeemDust, 0);

        assertEq(_claimDeposit(alice, 2), 1_000_002_999_999_999_999);
        assertEq(_claimDeposit(bob, 2), 1_000_006_999_999_999_999);
        assertEq(_claimDeposit(carol, 2), 1_000_010_999_999_999_999);
    }

    function test_fixture_multi_epoch_price_progression() public {
        _bootstrap(alice, 10_000_000);

        uint256[3] memory redeemShares = [
            uint256(1_428_571_428_571_428_574), 1_224_489_795_918_367_349, 1_049_562_682_215_743_442
        ];
        uint256[3] memory expectedAssets = [uint256(10_000_000), 10_571_430, 11_346_942];
        uint256[3] memory expectedSupply = [
            uint256(10_000_000_000_000_000_000),
            10_571_429_571_428_571_426,
            11_346_940_694_429_101_081
        ];
        uint256[3] memory expectedMinted = [
            uint256(2_000_001_000_000_000_000), 2_000_000_918_918_897_004, 2_000_000_769_881_358_934
        ];
        uint256[3] memory expectedRedeemed = [uint256(1_428_571), 1_224_489, 1_049_562];

        for (uint256 i = 0; i < 3; ++i) {
            uint64 epochId = uint64(i + 1);
            _redeem(alice, redeemShares[i]);
            _deposit(bob, 2_000_001);
            _settle();

            SettledEpoch memory terms = _terms(epochId);
            assertEq(terms.totalAssets, expectedAssets[i], "total assets");
            assertEq(terms.totalSupply, expectedSupply[i], "total supply");
            assertEq(terms.depositAssets, 2_000_001, "deposit assets");
            assertEq(terms.mintedShares, expectedMinted[i], "minted shares");
            assertEq(terms.redeemShares, redeemShares[i], "redeem shares");
            assertEq(terms.redeemAssets, expectedRedeemed[i], "redeem assets");
            assertEq(terms.depositDust, 0, "deposit dust");
            assertEq(terms.redeemDust, 0, "redeem dust");

            _claimRedeem(alice, epochId);
            _claimDeposit(bob, epochId);
            _openNext();
        }

        assertEq(vault.idleBacking(), 12_297_381);
        assertEq(vault.totalSupply(), 12_297_378_782_094_716_573);
        assertEq(vault.claimReserve(), 0);
        assertEq(vault.pendingDepositEscrow(), 0);
    }

    function test_fixture_donation_leaves_the_price_alone() public {
        token.mint(address(vault), 750);
        _deposit(alice, 4_000_000);
        _settle();

        SettledEpoch memory terms = _terms(0);
        assertEq(terms.totalAssets, 0);
        assertEq(terms.totalSupply, 0);
        assertEq(terms.depositAssets, 4_000_000);
        assertEq(terms.mintedShares, 4_000_000_000_000_000_000);
        assertEq(vault.idleBacking(), 4_000_000);
        assertEq(vault.unattributedBalance(), 750);
        assertEq(vault.totalSupply(), 4_000_000_000_000_000_000);
    }
}
