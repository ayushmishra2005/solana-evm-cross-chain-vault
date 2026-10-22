// SPDX-License-Identifier: Apache-2.0
pragma solidity 0.8.30;

import {RemoteAccounting} from "../../src/RemoteAccounting.sol";
import {IRemoteAccounting} from "../../src/interfaces/IRemoteAccounting.sol";
import {ISolEVMVault} from "../../src/interfaces/ISolEVMVault.sol";
import {ProtocolCodec} from "../../src/libraries/ProtocolCodec.sol";
import {VaultMath} from "../../src/libraries/VaultMath.sol";
import {RemoteTestBase} from "../RemoteTestBase.sol";

/// Authentic, in-order messages that cannot price anything must still move the
/// inbound lane, so nothing queued behind them is stranded.
contract InboundLaneTest is RemoteTestBase {
    bytes32 internal constant ALLOCATION = bytes32(uint256(1));
    bytes32 internal constant RECALL = bytes32(uint256(2));

    function setUp() public override {
        super.setUp();
        _bootstrap(alice, 100e6);
        _allocate(ALLOCATION, 80e6);
        _report(vault.currentEpochId(), 80e6, 80e6, 0, ALLOCATION);
    }

    function test_wrong_epoch_report_at_cutoff_does_not_block_the_lane() public {
        uint64 epochId = _cutoff();
        _assertConsumed(
            _encodeReport(epochId + 1, 80e6, 10e6, 0, bytes32(0)),
            epochId + 1,
            IRemoteAccounting.ReportSkipReason.WrongEpoch
        );
        assertFalse(vault.epochRemoteValuationLocked(epochId));
        _report(epochId, 80e6, 80e6, 0, bytes32(0));
        assertEq(vault.lockedPricingAssets(epochId), 100e6);
        vault.finalizeEpoch();
    }

    function test_stale_report_at_cutoff_does_not_block_the_lane() public {
        bytes memory stale = _encodeReport(vault.currentEpochId(), 80e6, 10e6, 0, bytes32(0));
        uint64 epochId = _cutoff();
        _assertConsumed(stale, epochId, IRemoteAccounting.ReportSkipReason.StaleObservation);
        assertFalse(vault.epochRemoteValuationLocked(epochId));
        _report(epochId, 80e6, 80e6, 0, bytes32(0));
        assertEq(vault.lockedPricingAssets(epochId), 100e6);
        vault.finalizeEpoch();
    }

    function test_report_after_the_lock_is_consumed() public {
        uint64 epochId = _cutoff();
        _report(epochId, 80e6, 80e6, 0, bytes32(0));
        uint256 locked = vault.lockedPricingAssets(epochId);
        _assertConsumed(
            _encodeReport(epochId, 80e6, 10e6, 0, bytes32(0)),
            epochId,
            IRemoteAccounting.ReportSkipReason.ValuationFixed
        );
        assertEq(vault.lockedPricingAssets(epochId), locked);
        vault.finalizeEpoch();
        assertEq(vault.settledEpoch(epochId).totalAssets, locked);
    }

    function test_recall_sent_behind_a_report_after_the_lock_still_settles() public {
        uint256 shares = vault.balanceOf(alice) / 2;
        uint256 supply = vault.totalSupply();
        _redeem(alice, shares);
        _recall(RECALL, 40e6);
        uint64 epochId = _cutoff();
        _report(epochId, 40e6, 40e6, 0, bytes32(0));
        assertEq(vault.lockedPricingAssets(epochId), 100e6);
        uint256 owed = VaultMath.sharesToAssets(shares, 100e6, supply);
        vm.expectRevert(
            abi.encodeWithSelector(ISolEVMVault.InsufficientLiquidity.selector, 20e6, owed)
        );
        vault.finalizeEpoch();

        _assertConsumed(
            _encodeReport(epochId, 40e6, 40e6, 0, bytes32(0)),
            epochId,
            IRemoteAccounting.ReportSkipReason.ValuationFixed
        );
        _recallSent(RECALL, 40e6, 40e6, 0);
        _observe(40e6);
        vault.finalizeEpoch();
        assertEq(vault.settledEpoch(epochId).totalAssets, 100e6);
        assertEq(vault.claimableRedeemAssets(epochId, alice), owed);
        _assertSolvent();
    }

    function test_expired_recall_sent_still_settles() public {
        _recall(RECALL, 40e6);
        ProtocolCodec.Header memory header = _inboundHeader();
        header.expiresAt = uint64(block.timestamp) + 1;
        bytes memory encoded = ProtocolCodec.encodeRecallSent(
            header,
            ProtocolCodec.RecallSentBody({
                transferId: RECALL,
                principalSent: 40e6,
                actualAmountSent: 40e6,
                realizedLoss: 0,
                destinationReference: destRef,
                sentTimestamp: uint64(block.timestamp),
                configVersion: uint64(vault.configVersion())
            })
        );
        vm.warp(uint256(header.expiresAt) + 1);
        coordinator.processRecallSent(encoded);
        assertFalse(coordinator.hasActiveRecall());
        assertEq(vault.remotePrincipalCap(), 40e6);
        assertEq(vault.inboundInTransit(), 40e6);
        assertEq(_managed(), 100e6);
    }

    function test_a_consumed_report_cannot_be_replayed() public {
        uint64 epochId = _cutoff();
        bytes memory wrongEpoch = _encodeReport(epochId + 1, 80e6, 10e6, 0, bytes32(0));
        uint64 sequence = coordinator.inboundExpectedSequence();
        _assertConsumed(wrongEpoch, epochId + 1, IRemoteAccounting.ReportSkipReason.WrongEpoch);
        vm.expectRevert(abi.encodeWithSelector(RemoteAccounting.Replay.selector, sequence));
        coordinator.processRemoteReport(wrongEpoch);
    }

    function test_a_report_lowering_cumulative_loss_still_reverts() public {
        _recall(RECALL, 40e6);
        _report(vault.currentEpochId(), 70e6, 70e6, 10e6, bytes32(0));
        bytes memory lower = _encodeReport(vault.currentEpochId(), 70e6, 70e6, 5e6, bytes32(0));
        vm.expectRevert(RemoteAccounting.StaleValuation.selector);
        coordinator.processRemoteReport(lower);
    }

    function _cutoff() internal returns (uint64 epochId) {
        epochId = vault.currentEpochId();
        vm.warp(vault.currentEpochCutoffAt());
        vault.cutoffEpoch();
    }

    /// The report advances the lane without changing any valuation.
    function _assertConsumed(
        bytes memory encoded,
        uint64 reportEpoch,
        IRemoteAccounting.ReportSkipReason reason
    ) internal {
        uint64 sequence = coordinator.inboundExpectedSequence();
        uint256 accepted = vault.acceptedRemoteValue();
        uint256 nav = _managed();
        _expectSkipped(reportEpoch, reason);
        coordinator.processRemoteReport(encoded);
        assertEq(coordinator.inboundExpectedSequence(), sequence + 1, "lane advanced");
        assertEq(vault.acceptedRemoteValue(), accepted, "valuation unchanged");
        assertEq(_managed(), nav, "nav unchanged");
    }
}
