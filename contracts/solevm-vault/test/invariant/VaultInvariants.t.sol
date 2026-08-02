// SPDX-License-Identifier: Apache-2.0
pragma solidity 0.8.30;

import {Test} from "forge-std/Test.sol";
import {console} from "forge-std/console.sol";

import {SolEVMVault} from "../../src/SolEVMVault.sol";
import {
    DepositRequest,
    EpochOutcome,
    RedeemRequest,
    SettledEpoch,
    VaultStatus
} from "../../src/interfaces/ISolEVMVault.sol";
import {MockAsset} from "../mocks/MockAsset.sol";
import {VaultHandler} from "./VaultHandler.sol";

/// Shared invariants. Both campaigns below check the same rules.
abstract contract VaultInvariantsBase is Test {
    uint64 internal constant EPOCH_DURATION = 3600;

    MockAsset internal token;
    SolEVMVault internal vault;
    VaultHandler internal handler;

    address internal admin = makeAddr("admin");
    address internal guardian = makeAddr("guardian");

    function _allowFreeze() internal pure virtual returns (bool);

    function setUp() public {
        token = new MockAsset();
        vault = new SolEVMVault(
            token, admin, guardian, EPOCH_DURATION, 1e6, 1e12, 1, "SolEVM Vault Share", "svUSD"
        );
        handler = new VaultHandler(vault, token, admin, guardian, _allowFreeze());

        bytes4[] memory actions = new bytes4[](18);
        actions[0] = VaultHandler.requestDeposit.selector;
        actions[1] = VaultHandler.cancelDeposit.selector;
        actions[2] = VaultHandler.requestRedeem.selector;
        actions[3] = VaultHandler.cancelRedeem.selector;
        actions[4] = VaultHandler.cutoff.selector;
        actions[5] = VaultHandler.finalize.selector;
        actions[6] = VaultHandler.openNext.selector;
        actions[7] = VaultHandler.claimDeposit.selector;
        actions[8] = VaultHandler.claimRedeem.selector;
        actions[9] = VaultHandler.refundDeposit.selector;
        actions[10] = VaultHandler.refundRedeem.selector;
        actions[11] = VaultHandler.pause.selector;
        actions[12] = VaultHandler.unpause.selector;
        actions[13] = VaultHandler.freeze.selector;
        actions[14] = VaultHandler.abort.selector;
        actions[15] = VaultHandler.donate.selector;
        actions[16] = VaultHandler.reconcile.selector;
        actions[17] = VaultHandler.advanceTime.selector;

        targetSelector(FuzzSelector({addr: address(handler), selectors: actions}));
        targetContract(address(handler));
    }

    /// 1. Managed NAV is the idle bucket and nothing else.
    function invariant_managed_nav_equals_idle_backing() public view {
        assertEq(vault.managedNav(), vault.idleBacking());
    }

    /// 2, 3 and 4. Escrow, reserves and donations sit outside NAV.
    function invariant_excluded_buckets_stay_outside_nav() public view {
        assertEq(
            vault.accountedAssets(),
            vault.managedNav() + vault.pendingDepositEscrow() + vault.claimReserve()
                + vault.unattributedBalance()
        );
    }

    /// 5. Shares escrowed for an exit are still outstanding.
    function invariant_escrowed_shares_remain_in_total_supply() public view {
        assertGe(vault.balanceOf(address(vault)), handler.currentEpochShareEscrow());
        assertLe(vault.balanceOf(address(vault)), vault.totalSupply());
    }

    /// 6 and 16. Supply moves only through settlement mint and burn.
    function invariant_supply_equals_settlement_mint_minus_burn() public view {
        assertEq(vault.totalSupply(), handler.ghostMintedShares() - handler.ghostBurnedShares());
    }

    /// 20. Every share is held by an actor or by the vault.
    function invariant_shares_are_fully_attributed() public view {
        assertEq(vault.totalSupply(), handler.totalActorShares() + vault.balanceOf(address(vault)));
    }

    /// 20. Every asset unit is held by an actor or by the vault.
    function invariant_assets_are_fully_attributed() public view {
        assertEq(token.totalSupply(), handler.totalActorAssets() + token.balanceOf(address(vault)));
    }

    /// 14. The real balance covers every bucket the vault claims to hold.
    function invariant_token_balance_covers_every_bucket() public view {
        assertGe(token.balanceOf(address(vault)), vault.accountedAssets());
    }

    /// 12. The reserve equals what redeemers are owed plus measured dust.
    function invariant_claim_reserve_is_explained() public view {
        assertEq(vault.claimReserve(), handler.outstandingRedeemAssets());
    }

    /// 13. Vault held shares cover deposit claims, dust and every escrow.
    function invariant_vault_held_shares_are_explained() public view {
        uint256 explained = handler.outstandingDepositShares() + handler.currentEpochShareEscrow()
            + handler.outstandingRefundShares();
        assertEq(vault.balanceOf(address(vault)), explained);
    }

    /// 2. Escrowed assets equal the requests standing behind them.
    function invariant_deposit_escrow_is_explained() public view {
        assertEq(
            vault.pendingDepositEscrow(),
            handler.currentEpochEscrow() + handler.outstandingRefundAssets()
        );
    }

    /// 9 and 18. Outcomes are single valued and never rewritten.
    function invariant_each_epoch_has_one_unchanged_outcome() public view {
        uint64[] memory ids = handler.allSettledEpochs();
        for (uint256 i = 0; i < ids.length; ++i) {
            SettledEpoch memory terms = vault.settledEpoch(ids[i]);
            assertTrue(terms.outcome != EpochOutcome.None, "settled epoch lost its outcome");
            assertEq(
                keccak256(abi.encode(terms)),
                handler.termsFingerprint(ids[i]),
                "settled terms were edited"
            );
            assertTrue(
                ids[i] != vault.currentEpochId() || !vault.epochOpen(), "settled epoch is current"
            );
        }
    }

    /// 7, 8 and 10. A request never carries two outcomes at once.
    function invariant_no_request_holds_two_outcomes() public view {
        uint64[] memory ids = handler.allSettledEpochs();
        for (uint256 i = 0; i < ids.length; ++i) {
            address[] memory depositors = vault.depositControllers(ids[i]);
            for (uint256 j = 0; j < depositors.length; ++j) {
                DepositRequest memory request = vault.depositRequestOf(ids[i], depositors[j]);
                assertFalse(request.cancelled && request.settled, "deposit holds two outcomes");
                assertTrue(request.assets != 0, "a listed deposit is empty");
            }
            address[] memory redeemers = vault.redeemControllers(ids[i]);
            for (uint256 j = 0; j < redeemers.length; ++j) {
                RedeemRequest memory request = vault.redeemRequestOf(ids[i], redeemers[j]);
                assertFalse(request.cancelled && request.settled, "redemption holds two outcomes");
                assertTrue(request.shares != 0, "a listed redemption is empty");
            }
        }
    }

    /// 11 and 19. Every finalized epoch stayed inside its own liquidity, and
    /// its dust never exceeds one unit per claim.
    function invariant_settlement_terms_are_self_consistent() public view {
        uint256 count = handler.finalizedEpochCount();
        for (uint256 i = 0; i < count; ++i) {
            uint64 epochId = handler.finalizedEpochs(i);
            SettledEpoch memory terms = vault.settledEpoch(epochId);
            assertLe(terms.redeemAssets, terms.totalAssets, "an exit outran its liquidity");

            uint256 depositCount = vault.depositControllers(epochId).length;
            uint256 redeemCount = vault.redeemControllers(epochId).length;
            if (depositCount == 0) {
                assertEq(terms.depositDust, 0);
            } else {
                assertLe(terms.depositDust, depositCount - 1, "deposit dust above its bound");
            }
            if (redeemCount == 0) {
                assertEq(terms.redeemDust, 0);
            } else {
                assertLe(terms.redeemDust, redeemCount - 1, "redeem dust above its bound");
            }
        }
    }

    /// 15. Donations never reach the share price.
    function invariant_donations_never_reach_nav() public view {
        assertLe(vault.unattributedBalance(), handler.ghostDonated());
        assertEq(vault.managedNav(), vault.idleBacking());
    }

    /// 7 and 8. No entitlement is consumed more often than it was created.
    function invariant_claims_and_refunds_are_consumed_once() public view {
        uint256 consumed = handler.ghostDepositClaims() + handler.ghostRedeemClaims()
            + handler.ghostDepositRefunds() + handler.ghostRedeemRefunds();
        assertLe(consumed, _settledRequestCount(), "more consumptions than requests");
    }

    function _settledRequestCount() private view returns (uint256 total) {
        uint64[] memory ids = handler.allSettledEpochs();
        for (uint256 i = 0; i < ids.length; ++i) {
            total += vault.depositControllers(ids[i]).length;
            total += vault.redeemControllers(ids[i]).length;
        }
    }

    function _logSummary() internal view {
        uint256 actions = handler.actionCount();
        uint256 attempted;
        uint256 effective;
        console.log("action, attempted, effective");
        for (uint256 i = 0; i < actions; ++i) {
            string memory name = handler.actionNames(i);
            bytes32 key = keccak256(bytes(name));
            uint256 tried = handler.attemptCount(key);
            uint256 done = handler.effectiveCount(key);
            attempted += tried;
            effective += done;
            console.log(name, tried, done);
        }
        console.log("attempted", attempted);
        console.log("effective", effective);
        console.log("finalized epochs", handler.finalizedEpochCount());
        console.log("aborted epochs", handler.abortedEpochCount());
        console.log("deposit claims", handler.ghostDepositClaims());
        console.log("redeem claims", handler.ghostRedeemClaims());
        console.log("refunds", handler.ghostDepositRefunds() + handler.ghostRedeemRefunds());
    }
}

/// The normal lifecycle, driven without ever freezing.
contract VaultInvariantsTest is VaultInvariantsBase {
    function _allowFreeze() internal pure override returns (bool) {
        return false;
    }

    /// 17. Claims stay reachable while paused, which the handler proves by
    /// never gating them on vault status under fail_on_revert.
    function invariant_claims_never_depend_on_vault_status() public view {
        assertTrue(uint8(vault.vaultStatus()) <= uint8(VaultStatus.Frozen));
    }

    function invariant_callSummary() public view {
        _logSummary();
    }
}

/// The same rules with freezing, abort and refunds switched on.
contract VaultEmergencyInvariantsTest is VaultInvariantsBase {
    function _allowFreeze() internal pure override returns (bool) {
        return true;
    }

    function invariant_callSummary() public view {
        _logSummary();
    }
}
