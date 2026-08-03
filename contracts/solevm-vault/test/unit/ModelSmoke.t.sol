// SPDX-License-Identifier: Apache-2.0
pragma solidity 0.8.30;

import {SettledEpoch} from "../../src/interfaces/ISolEVMVault.sol";
import {VaultTestBase} from "../VaultTestBase.sol";

/// Two short readable examples of the settlement arithmetic.
///
/// Full parity with the Rust reference model is proven by the differential
/// harness, which generates its scenarios instead of copying numbers. These
/// two stay because a reader should be able to see the arithmetic without
/// running the generator.
contract ModelSmokeTest is VaultTestBase {
    function test_the_first_deposit_mints_one_share_per_asset_unit() public {
        _deposit(alice, 1_000_000);
        _settle();

        SettledEpoch memory terms = _terms(0);
        assertEq(terms.totalAssets, 0);
        assertEq(terms.totalSupply, 0);
        assertEq(terms.depositAssets, 1_000_000);
        assertEq(terms.mintedShares, 1_000_000_000_000_000_000);
        assertEq(terms.depositDust, 0);

        _claimDeposit(alice, 0);
        assertEq(vault.idleBacking(), 1_000_000);
        assertEq(vault.totalSupply(), 1_000_000_000_000_000_000);
        assertEq(vault.balanceOf(alice), 1_000_000_000_000_000_000);
    }

    /// Every claim rounds down, so the epoch keeps the remainder.
    function test_uneven_deposits_leave_dust_the_vault_keeps() public {
        _bootstrap(alice, 10_000_000);
        _redeem(alice, 3_000_000_000_001);
        _settle();
        _claimRedeem(alice, 1);
        _openNext();

        _deposit(alice, 1_000_003);
        _deposit(bob, 1_000_007);
        _deposit(carol, 1_000_011);
        _settle();

        SettledEpoch memory terms = _terms(2);
        assertEq(terms.totalAssets, 9_999_997);
        assertEq(terms.totalSupply, 9_999_996_999_999_999_999);
        assertEq(terms.depositAssets, 3_000_021);
        assertEq(terms.mintedShares, 3_000_020_999_999_999_999);
        assertEq(terms.depositDust, 2);

        assertEq(_claimDeposit(alice, 2), 1_000_002_999_999_999_999);
        assertEq(_claimDeposit(bob, 2), 1_000_006_999_999_999_999);
        assertEq(_claimDeposit(carol, 2), 1_000_010_999_999_999_999);
    }
}
