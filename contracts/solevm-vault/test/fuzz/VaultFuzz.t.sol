// SPDX-License-Identifier: Apache-2.0
pragma solidity 0.8.30;

import {ISolEVMVault, SettledEpoch} from "../../src/interfaces/ISolEVMVault.sol";
import {VaultMath} from "../../src/libraries/VaultMath.sol";
import {VaultTestBase} from "../VaultTestBase.sol";

contract VaultFuzzTest is VaultTestBase {
    uint256 private constant MAX_DEPOSIT = 100_000e6;

    // Conversion math

    function testFuzz_a_round_trip_never_returns_more_than_it_started_with(
        uint256 assets,
        uint256 totalAssets,
        uint256 totalSupply
    ) public pure {
        assets = bound(assets, 0, 1e30);
        totalAssets = bound(totalAssets, 0, 1e30);
        totalSupply = bound(totalSupply, 0, 1e42);

        uint256 shares = VaultMath.assetsToShares(assets, totalAssets, totalSupply);
        uint256 back = VaultMath.sharesToAssets(shares, totalAssets, totalSupply);
        assertLe(back, assets, "round trip created assets");
    }

    function testFuzz_conversions_never_decrease_with_the_input(
        uint256 lower,
        uint256 higher,
        uint256 totalAssets,
        uint256 totalSupply
    ) public pure {
        lower = bound(lower, 0, 1e24);
        higher = bound(higher, lower, 1e24);
        totalAssets = bound(totalAssets, 0, 1e24);
        totalSupply = bound(totalSupply, 0, 1e36);

        assertLe(
            VaultMath.assetsToShares(lower, totalAssets, totalSupply),
            VaultMath.assetsToShares(higher, totalAssets, totalSupply)
        );
        assertLe(
            VaultMath.sharesToAssets(lower, totalAssets, totalSupply),
            VaultMath.sharesToAssets(higher, totalAssets, totalSupply)
        );
    }

    function testFuzz_an_empty_vault_prices_at_the_virtual_offset(uint256 assets) public pure {
        assets = bound(assets, 0, 1e24);
        assertEq(VaultMath.assetsToShares(assets, 0, 0), assets * 1e12);
    }

    function testFuzz_splitting_an_amount_never_gains_a_unit(
        uint256 first,
        uint256 second,
        uint256 totalAssets,
        uint256 totalSupply
    ) public pure {
        first = bound(first, 0, 1e24);
        second = bound(second, 0, 1e24);
        totalAssets = bound(totalAssets, 0, 1e24);
        totalSupply = bound(totalSupply, 0, 1e36);

        uint256 apart = VaultMath.assetsToShares(first, totalAssets, totalSupply)
            + VaultMath.assetsToShares(second, totalAssets, totalSupply);
        uint256 together = VaultMath.assetsToShares(first + second, totalAssets, totalSupply);
        assertLe(apart, together, "splitting gained shares");
        assertLe(together - apart, 1, "floor lost more than one unit");
    }

    // Single user settlement

    function testFuzz_a_lone_deposit_mints_the_full_epoch_amount(uint256 assets) public {
        assets = bound(assets, MIN_DEPOSIT, MAX_DEPOSIT);
        _deposit(alice, assets);
        _settle();

        SettledEpoch memory terms = _terms(0);
        assertEq(terms.depositAssets, assets);
        assertEq(terms.mintedShares, assets * 1e12);
        assertEq(terms.depositDust, 0, "a lone claimant left dust");
        assertEq(_claimDeposit(alice, 0), assets * 1e12);
        _assertSolvent();
    }

    function testFuzz_a_deposit_and_a_full_exit_return_the_original_assets(uint256 assets) public {
        assets = bound(assets, MIN_DEPOSIT, MAX_DEPOSIT);
        uint256 before = token.balanceOf(alice);

        _bootstrap(alice, assets);
        _redeem(alice, vault.balanceOf(alice));
        _settle();
        _claimRedeem(alice, 1);

        assertEq(token.balanceOf(alice), before, "a lone holder lost value");
        assertEq(vault.totalSupply(), 0);
        assertEq(vault.idleBacking(), 0);
    }

    function testFuzz_repeat_requests_aggregate_exactly(uint256 first, uint256 second) public {
        first = bound(first, MIN_DEPOSIT, MAX_DEPOSIT);
        second = bound(second, MIN_DEPOSIT, MAX_DEPOSIT);

        _deposit(alice, first);
        _deposit(alice, second);
        assertEq(vault.depositRequestOf(0, alice).assets, first + second);
        assertEq(vault.pendingDepositEscrow(), first + second);
        assertEq(vault.depositControllers(0).length, 1);
    }

    function testFuzz_a_cancel_and_reopen_cycle_leaves_only_the_last_amount(
        uint256 first,
        uint256 second,
        uint8 rounds
    ) public {
        first = bound(first, MIN_DEPOSIT, MAX_DEPOSIT);
        second = bound(second, MIN_DEPOSIT, MAX_DEPOSIT);
        uint256 cycles = bound(rounds, 1, 8);
        uint256 before = token.balanceOf(alice);

        for (uint256 i = 0; i < cycles; ++i) {
            _deposit(alice, first);
            _cancelDeposit(alice);
            assertEq(token.balanceOf(alice), before, "cancellation returned the wrong amount");
        }
        _deposit(alice, second);

        assertEq(vault.pendingDepositEscrow(), second);
        assertEq(vault.depositRequestOf(0, alice).cancelledAssets, first * cycles);

        _settle();
        assertEq(_terms(0).depositAssets, second, "a cancelled amount was finalized");
        assertEq(_claimDeposit(alice, 0), second * 1e12);
    }

    // Multiple users

    function testFuzz_three_depositors_split_the_mint_within_one_unit(
        uint256 a,
        uint256 b,
        uint256 c
    ) public {
        a = bound(a, MIN_DEPOSIT, MAX_DEPOSIT);
        b = bound(b, MIN_DEPOSIT, MAX_DEPOSIT);
        c = bound(c, MIN_DEPOSIT, MAX_DEPOSIT);

        _bootstrap(alice, 10e6);
        _deposit(alice, a);
        _deposit(bob, b);
        _deposit(carol, c);
        _settle();

        SettledEpoch memory terms = _terms(1);
        uint256 claimed = _claimDeposit(alice, 1) + _claimDeposit(bob, 1) + _claimDeposit(carol, 1);
        assertEq(claimed + terms.depositDust, terms.mintedShares, "claims and dust do not add up");
        assertLe(terms.depositDust, 2, "dust above one unit per claim");
        _assertSolvent();
    }

    function testFuzz_claim_order_does_not_change_any_payout(uint256 a, uint256 b, uint256 seed)
        public
    {
        a = bound(a, MIN_DEPOSIT, MAX_DEPOSIT);
        b = bound(b, MIN_DEPOSIT, MAX_DEPOSIT);

        _bootstrap(alice, 10e6);
        _deposit(alice, a);
        _deposit(bob, b);
        _settle();

        uint256 aliceOwed = vault.claimableDepositShares(1, alice);
        uint256 bobOwed = vault.claimableDepositShares(1, bob);

        if (seed % 2 == 0) {
            assertEq(_claimDeposit(alice, 1), aliceOwed);
            assertEq(_claimDeposit(bob, 1), bobOwed);
        } else {
            assertEq(_claimDeposit(bob, 1), bobOwed);
            assertEq(_claimDeposit(alice, 1), aliceOwed);
        }
    }

    function testFuzz_request_order_does_not_change_the_epoch_total(
        uint256 a,
        uint256 b,
        bool aliceFirst
    ) public {
        a = bound(a, MIN_DEPOSIT, MAX_DEPOSIT);
        b = bound(b, MIN_DEPOSIT, MAX_DEPOSIT);

        if (aliceFirst) {
            _deposit(alice, a);
            _deposit(bob, b);
        } else {
            _deposit(bob, b);
            _deposit(alice, a);
        }
        _settle();

        assertEq(_terms(0).depositAssets, a + b);
        assertEq(_terms(0).mintedShares, (a + b) * 1e12);
    }

    function testFuzz_a_mixed_epoch_conserves_assets_and_shares(uint256 deposited, uint256 exited)
        public
    {
        _bootstrap(alice, 100e6);
        deposited = bound(deposited, MIN_DEPOSIT, MAX_DEPOSIT);
        exited = bound(exited, MIN_REDEEM, vault.balanceOf(alice));

        _deposit(bob, deposited);
        _redeem(alice, exited);
        _settle();

        SettledEpoch memory terms = _terms(1);
        assertEq(vault.idleBacking(), 100e6 - terms.redeemAssets + deposited);
        assertEq(vault.claimReserve(), terms.redeemAssets);
        assertEq(vault.pendingDepositEscrow(), 0);
        _assertSolvent();

        _claimDeposit(bob, 1);
        _claimRedeem(alice, 1);
        assertEq(vault.claimReserve(), terms.redeemDust);
        _assertSolvent();
    }

    // Boundaries

    function testFuzz_amounts_below_the_minimum_always_reject(uint256 assets) public {
        assets = bound(assets, 1, MIN_DEPOSIT - 1);
        vm.expectRevert(
            abi.encodeWithSelector(ISolEVMVault.AmountBelowMinimum.selector, assets, MIN_DEPOSIT)
        );
        vm.prank(alice);
        vault.requestDeposit(assets);
    }

    function testFuzz_the_minimum_amount_itself_is_accepted(uint256 extra) public {
        extra = bound(extra, 0, MAX_DEPOSIT);
        _deposit(alice, MIN_DEPOSIT + extra);
        assertEq(vault.pendingDepositEscrow(), MIN_DEPOSIT + extra);
    }

    function testFuzz_cutoff_only_lands_on_or_after_its_timestamp(uint64 offset) public {
        uint64 cutoffAt = vault.currentEpochCutoffAt();
        offset = uint64(bound(offset, 0, EPOCH_DURATION * 4));

        if (offset < EPOCH_DURATION) {
            vm.warp(uint256(vault.currentEpochOpenedAt()) + offset);
            vm.expectRevert();
            vault.cutoffEpoch();
        } else {
            vm.warp(uint256(vault.currentEpochOpenedAt()) + offset);
            vault.cutoffEpoch();
            assertEq(vault.currentEpochCutoffAt(), cutoffAt, "cutoff time moved");
        }
    }

    function testFuzz_a_donation_never_moves_the_settlement_price(uint256 gift, uint256 assets)
        public
    {
        gift = bound(gift, 1, 1_000_000e6);
        assets = bound(assets, MIN_DEPOSIT, MAX_DEPOSIT);

        _bootstrap(alice, 10e6);
        uint256 navBefore = vault.managedNav();
        token.mint(address(vault), gift);

        _deposit(bob, assets);
        _settle();

        SettledEpoch memory terms = _terms(1);
        assertEq(terms.totalAssets, navBefore, "the gift entered the basis");
        assertEq(terms.mintedShares, assets * 1e12, "the gift moved the price");
        assertEq(vault.unattributedBalance(), gift);
        _assertSolvent();
    }

    function testFuzz_the_token_balance_always_covers_every_bucket(
        uint256 deposited,
        uint256 gift,
        bool reconcileFirst
    ) public {
        deposited = bound(deposited, MIN_DEPOSIT, MAX_DEPOSIT);
        gift = bound(gift, 0, 1000e6);

        _bootstrap(alice, 10e6);
        if (gift != 0) token.mint(address(vault), gift);
        if (reconcileFirst) vault.reconcile();

        _deposit(bob, deposited);
        _assertSolvent();
        assertEq(vault.unattributedBalance(), reconcileFirst ? gift : 0);

        // Finalization reconciles too, so the gift is recorded either way.
        _settle();
        _assertSolvent();
        assertEq(vault.unattributedBalance(), gift);

        _claimDeposit(bob, 1);
        _assertSolvent();
        assertEq(vault.managedNav(), 10e6 + deposited, "the gift entered nav");
    }
}
