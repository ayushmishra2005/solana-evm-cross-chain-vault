// SPDX-License-Identifier: Apache-2.0
pragma solidity 0.8.30;

import {VaultMath} from "../../src/libraries/VaultMath.sol";
import {RemoteTestBase} from "../RemoteTestBase.sol";

/// Reports that Solana publishes while a recall unwinds, checked against an
/// independent ledger of the Solana leg instead of the vault's own buckets.
contract RecallValuationTest is RemoteTestBase {
    bytes32 internal constant ALLOCATION = bytes32(uint256(1));
    bytes32 internal constant RECALL = bytes32(uint256(2));
    bytes32 internal constant SECOND_RECALL = bytes32(uint256(3));

    uint128 internal constant DEPLOYED = 80e6;

    // Solana leg as publish_report sees it: principal is attributed + deployed
    // + recalled custody, and loss accrues only when a withdrawal returns less
    // than it unwinds.
    uint128 internal legPrincipal;
    uint128 internal legLoss;
    uint128 internal legCustody;
    uint128 internal recallUnwound;
    uint128 internal recallSent;

    /// Assets users have put in and not taken out.
    uint256 internal contributed;

    function setUp() public override {
        super.setUp();
        _bootstrap(alice, 100e6);
        contributed = 100e6;
        _allocate(ALLOCATION, DEPLOYED);
        legPrincipal = DEPLOYED;
        _legReport(ALLOCATION);
        assertEq(_managed(), _trueNav());
    }

    function test_report_after_send_keeps_nav_until_notice() public {
        _recall(RECALL, 40e6);
        _legWithdraw(40e6, 40e6);
        _legSend(40e6);
        _legReport(bytes32(0));
        assertEq(_managed(), _trueNav(), "report before notice");
        _observe(40e6);
        assertEq(_managed(), _trueNav(), "assets before notice");
        _legRecallSent(RECALL);
        assertEq(_managed(), _trueNav(), "notice");
        assertEq(vault.remotePrincipalCap(), 40e6);
        assertEq(vault.acceptedRemoteValue(), 40e6);
        assertEq(vault.idleBacking(), 60e6);
        _legReport(bytes32(0));
        assertEq(_managed(), _trueNav(), "next report");
        _assertSolvent();
    }

    function test_partial_send_report_keeps_nav() public {
        _recall(RECALL, 40e6);
        _legWithdraw(40e6, 40e6);
        _legSend(25e6);
        _legReport(bytes32(0));
        assertEq(_managed(), _trueNav(), "partial send");
        _legSend(15e6);
        _legReport(bytes32(0));
        assertEq(_managed(), _trueNav(), "full send");
        _legRecallSent(RECALL);
        assertEq(_managed(), _trueNav(), "notice");
        assertEq(vault.acceptedRemoteValue(), 40e6);
        _observe(40e6);
        assertEq(_managed(), _trueNav(), "assets after notice");
    }

    function test_lossy_withdrawal_is_counted_once() public {
        _recall(RECALL, 40e6);
        _legWithdraw(40e6, 30e6);
        _legReport(bytes32(0));
        assertEq(_managed(), _trueNav(), "loss before send");
        _legSend(30e6);
        _legReport(bytes32(0));
        assertEq(_managed(), _trueNav(), "loss after send");
        _legRecallSent(RECALL);
        assertEq(_managed(), _trueNav(), "notice");
        assertEq(_managed(), 90e6);
        assertEq(vault.recognizedLoss(), 10e6);
        assertEq(vault.acceptedRemoteValue(), 40e6);
        _observe(30e6);
        _legReport(bytes32(0));
        assertEq(_managed(), _trueNav(), "settled");
        _assertSolvent();
    }

    function test_total_loss_during_recall_is_counted_once() public {
        _recall(RECALL, 40e6);
        _legWithdraw(40e6, 0);
        _legReport(bytes32(0));
        assertEq(_managed(), _trueNav(), "report");
        _legRecallSent(RECALL);
        assertEq(_managed(), _trueNav(), "notice");
        assertEq(_managed(), 60e6);
        assertEq(vault.recognizedLoss(), 40e6);
    }

    function test_losing_the_whole_position_leaves_only_idle() public {
        _recall(RECALL, DEPLOYED);
        _legWithdraw(DEPLOYED, 0);
        _legReport(bytes32(0));
        _legRecallSent(RECALL);
        assertEq(_managed(), _trueNav());
        assertEq(_managed(), 20e6);
        assertEq(vault.remotePrincipalCap(), 0);
        assertEq(vault.acceptedRemoteValue(), 0);
    }

    function test_settled_loss_is_not_deducted_again_by_the_next_recall() public {
        _recall(RECALL, 40e6);
        _legWithdraw(40e6, 30e6);
        _legSend(30e6);
        _legRecallSent(RECALL);
        assertEq(_managed(), _trueNav(), "first notice");
        _recall(SECOND_RECALL, 20e6);
        _legWithdraw(20e6, 20e6);
        _legSend(20e6);
        _legReport(bytes32(0));
        assertEq(_managed(), _trueNav(), "report in second recall");
        _legRecallSent(SECOND_RECALL);
        assertEq(_managed(), _trueNav(), "second notice");
        assertEq(vault.acceptedRemoteValue(), 20e6);
        assertEq(vault.recognizedLoss(), 10e6);
    }

    function test_report_during_recall_cannot_create_value() public {
        _recall(RECALL, 40e6);
        _report(vault.currentEpochId(), 1000e6, 1000e6, 0, bytes32(0));
        assertEq(_managed(), _trueNav());
        assertEq(vault.acceptedRemoteValue(), DEPLOYED);
    }

    function test_cutoff_during_recall_prices_the_epoch_at_true_value() public {
        uint64 epochId = vault.currentEpochId();
        uint256 supply = vault.totalSupply();
        _deposit(bob, 100e6);
        uint256 redeemShares = vault.balanceOf(alice) / 10;
        _redeem(alice, redeemShares);

        _recall(RECALL, 40e6);
        _legWithdraw(40e6, 40e6);
        _legSend(40e6);
        vm.warp(vault.currentEpochCutoffAt());
        vault.cutoffEpoch();
        _legReport(bytes32(0));

        uint256 nav = _trueNav();
        assertEq(vault.lockedPricingAssets(epochId), nav, "locked value");
        vault.finalizeEpoch();
        assertEq(vault.settledEpoch(epochId).totalAssets, nav, "pricing basis");
        assertEq(
            vault.claimableDepositShares(epochId, bob),
            VaultMath.assetsToShares(100e6, nav, supply),
            "deposit shares"
        );
        uint256 redeemed = VaultMath.sharesToAssets(redeemShares, nav, supply);
        assertEq(vault.claimableRedeemAssets(epochId, alice), redeemed, "redemption assets");

        contributed = contributed + 100e6 - redeemed;
        _legRecallSent(RECALL);
        _observe(40e6);
        assertEq(_managed(), _trueNav(), "after settlement");
        _assertSolvent();
    }

    function test_lossy_recall_across_cutoff_locks_value_net_of_loss() public {
        uint64 epochId = vault.currentEpochId();
        _recall(RECALL, 40e6);
        _legWithdraw(40e6, 30e6);
        _legSend(30e6);
        vm.warp(vault.currentEpochCutoffAt());
        vault.cutoffEpoch();
        _legReport(bytes32(0));
        assertEq(vault.lockedPricingAssets(epochId), _trueNav());
        assertEq(vault.lockedPricingAssets(epochId), 90e6);
        vault.finalizeEpoch();
        assertEq(vault.settledEpoch(epochId).totalAssets, 90e6);
        _legRecallSent(RECALL);
        assertEq(_managed(), _trueNav());
    }

    function testFuzz_reports_during_recall_track_true_value(
        uint128 requested,
        uint128 returned,
        uint128 firstSend,
        uint8 reports,
        bool assetsBeforeNotice
    ) public {
        requested = uint128(bound(requested, 1, DEPLOYED));
        returned = uint128(bound(returned, 0, requested));
        firstSend = uint128(bound(firstSend, 0, returned));

        _recall(RECALL, requested);
        if (reports & 1 != 0) _checkedReport("before withdrawal");
        _legWithdraw(requested, returned);
        if (reports & 2 != 0) _checkedReport("after withdrawal");
        _legSend(firstSend);
        if (reports & 4 != 0) _checkedReport("after partial send");
        _legSend(returned - firstSend);
        if (reports & 8 != 0) _checkedReport("after full send");
        if (assetsBeforeNotice && returned != 0) _observe(returned);

        _legRecallSent(RECALL);
        assertEq(_managed(), _trueNav(), "notice");
        if (!assetsBeforeNotice && returned != 0) _observe(returned);
        _checkedReport("settled");
        assertEq(vault.recognizedLoss(), requested - returned);
        assertEq(vault.remotePrincipalCap(), DEPLOYED - requested);
        _assertSolvent();
    }

    /// Value the vault owns: what users put in less every loss Solana realized.
    function _trueNav() internal view returns (uint256) {
        return contributed - legLoss;
    }

    function _legWithdraw(uint128 unwound, uint128 returned) internal {
        uint128 loss = unwound - returned;
        legPrincipal -= loss;
        legLoss += loss;
        legCustody += returned;
        recallUnwound += unwound;
    }

    function _legSend(uint128 amount) internal {
        legCustody -= amount;
        legPrincipal -= amount;
        recallSent += amount;
    }

    function _legReport(bytes32 completed) internal {
        _report(vault.currentEpochId(), legPrincipal, legPrincipal, legLoss, completed);
    }

    function _legRecallSent(bytes32 transferId) internal {
        _recallSent(transferId, recallUnwound, recallSent, recallUnwound - recallSent);
        recallUnwound = 0;
        recallSent = 0;
    }

    function _checkedReport(string memory step) internal {
        _legReport(bytes32(0));
        assertEq(_managed(), _trueNav(), step);
    }
}
