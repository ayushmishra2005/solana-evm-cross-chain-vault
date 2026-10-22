// SPDX-License-Identifier: Apache-2.0
pragma solidity 0.8.30;

import {RemoteAccounting} from "../../src/RemoteAccounting.sol";
import {SolEVMVault} from "../../src/SolEVMVault.sol";
import {IRemoteAccounting} from "../../src/interfaces/IRemoteAccounting.sol";
import {EpochPhase, ISolEVMVault, VaultStatus} from "../../src/interfaces/ISolEVMVault.sol";
import {ProtocolCodec} from "../../src/libraries/ProtocolCodec.sol";
import {RemoteTestBase} from "../RemoteTestBase.sol";

contract RemoteAccountingTest is RemoteTestBase {
    bytes32 internal constant TID_1 = bytes32(uint256(1));
    bytes32 internal constant TID_2 = bytes32(uint256(2));
    bytes32 internal constant TID_3 = bytes32(uint256(3));

    function test_bind_is_one_shot() public {
        vm.prank(admin);
        vm.expectRevert(SolEVMVault.RemoteAccountingAlreadyBound.selector);
        vault.bindRemoteAccounting(address(coordinator));
    }

    function test_outsider_cannot_allocate() public {
        _bootstrap(alice, 100e6);
        (uint64 observedAt, uint64 publishedAt, uint64 expiresAt, uint64 deadline) = _times();
        vm.prank(alice);
        vm.expectRevert(abi.encodeWithSelector(RemoteAccounting.Unauthorized.selector, alice));
        coordinator.allocate(TID_1, 10e6, 10e6, deadline, observedAt, publishedAt, expiresAt);
    }

    function test_allocate_moves_idle_to_outbound_without_changing_nav() public {
        _bootstrap(alice, 100e6);
        uint256 nav = _managed();
        _allocate(TID_1, 40e6);
        assertEq(vault.idleBacking(), 60e6);
        assertEq(vault.outboundInTransit(), 40e6);
        assertEq(_managed(), nav);
        assertEq(token.balanceOf(sink), 40e6);
        assertTrue(coordinator.hasActiveAllocate());
        assertEq(coordinator.outboundNextSequence(), 2);
    }

    function test_second_cycle_is_rejected() public {
        _bootstrap(alice, 100e6);
        _allocate(TID_1, 10e6);
        (uint64 observedAt, uint64 publishedAt, uint64 expiresAt, uint64 deadline) = _times();
        vm.prank(admin);
        vm.expectRevert(
            abi.encodeWithSelector(RemoteAccounting.ActiveTransferExists.selector, TID_1)
        );
        coordinator.allocate(TID_2, 10e6, 10e6, deadline, observedAt, publishedAt, expiresAt);
    }

    function test_transfer_id_cannot_be_reused() public {
        _bootstrap(alice, 100e6);
        _allocate(TID_1, 10e6);
        _report(1, 10e6, 10e6, 0, TID_1);
        (uint64 observedAt, uint64 publishedAt, uint64 expiresAt, uint64 deadline) = _times();
        vm.prank(admin);
        vm.expectRevert(abi.encodeWithSelector(RemoteAccounting.TransferIdReused.selector, TID_1));
        coordinator.allocate(TID_1, 10e6, 10e6, deadline, observedAt, publishedAt, expiresAt);
    }

    function test_completion_caps_reported_value_to_principal() public {
        _bootstrap(alice, 100e6);
        _allocate(TID_1, 40e6);
        _report(1, 40e6, 99e6, 0, TID_1);
        assertEq(vault.outboundInTransit(), 0);
        assertEq(vault.remotePrincipalCap(), 40e6);
        assertEq(vault.acceptedRemoteValue(), 40e6);
        assertEq(_managed(), 100e6);
        assertFalse(coordinator.hasActiveAllocate());
    }

    function test_haircut_report_lowers_nav() public {
        _bootstrap(alice, 100e6);
        _allocate(TID_1, 40e6);
        _report(1, 40e6, 30e6, 0, TID_1);
        assertEq(vault.acceptedRemoteValue(), 30e6);
        assertEq(_managed(), 90e6);
    }

    function test_recall_request_does_not_change_nav() public {
        _bootstrap(alice, 100e6);
        _allocate(TID_1, 40e6);
        _report(1, 40e6, 40e6, 0, TID_1);
        uint256 nav = _managed();
        _recall(TID_2, 40e6);
        assertEq(_managed(), nav);
        assertTrue(coordinator.hasActiveRecall());
    }

    function test_recall_sent_before_assets_recognizes_exact_loss() public {
        _bootstrap(alice, 100e6);
        _allocate(TID_1, 40e6);
        _report(1, 40e6, 40e6, 0, TID_1);
        _recall(TID_2, 40e6);
        _recallSent(TID_2, 40e6, 35e6, 5e6);
        assertEq(_managed(), 95e6);
        assertEq(vault.remotePrincipalCap(), 0);
        assertEq(vault.inboundInTransit(), 35e6);
        assertEq(vault.recognizedLoss(), 5e6);
        assertEq(vault.idleBacking(), 60e6);
    }

    function test_assets_before_recall_sent_do_not_double_count() public {
        _bootstrap(alice, 100e6);
        _allocate(TID_1, 40e6);
        _report(1, 40e6, 40e6, 0, TID_1);
        _recall(TID_2, 40e6);
        _observe(35e6);
        assertEq(_managed(), 100e6);
        assertEq(vault.unattributedInbound(), 35e6);
        _recallSent(TID_2, 40e6, 35e6, 5e6);
        assertEq(_managed(), 95e6);
        assertEq(vault.inboundInTransit(), 0);
        assertEq(vault.unattributedInbound(), 0);
        assertEq(vault.idleBacking(), 95e6);
    }

    function test_arrival_after_recall_sent_preserves_nav() public {
        _bootstrap(alice, 100e6);
        _allocate(TID_1, 40e6);
        _report(1, 40e6, 40e6, 0, TID_1);
        _recall(TID_2, 40e6);
        _recallSent(TID_2, 40e6, 35e6, 5e6);
        uint256 nav = _managed();
        _observe(35e6);
        assertEq(_managed(), nav);
        assertEq(vault.inboundInTransit(), 0);
        assertEq(vault.idleBacking(), 95e6);
    }

    function test_duplicate_assets_stay_unattributed() public {
        _bootstrap(alice, 100e6);
        _allocate(TID_1, 40e6);
        _report(1, 40e6, 40e6, 0, TID_1);
        _recall(TID_2, 40e6);
        _recallSent(TID_2, 40e6, 35e6, 5e6);
        _observe(35e6);
        _observe(35e6);
        assertEq(_managed(), 95e6);
        assertEq(vault.unattributedBalance(), 35e6);
        assertEq(vault.inboundInTransit(), 0);
    }

    function test_total_loss_recall_needs_no_arrival() public {
        _bootstrap(alice, 100e6);
        _allocate(TID_1, 40e6);
        _report(1, 40e6, 40e6, 0, TID_1);
        _recall(TID_2, 40e6);
        _recallSent(TID_2, 40e6, 0, 40e6);
        assertEq(_managed(), 60e6);
        assertEq(vault.inboundInTransit(), 0);
        assertEq(vault.recognizedLoss(), 40e6);
        assertFalse(coordinator.hasActiveRecall());
        _report(1, 0, 0, 40e6, bytes32(0));
        assertEq(vault.acceptedRemoteValue(), 0);
        assertEq(_managed(), 60e6);
    }

    function test_freeze_blocks_allocate_and_allows_recall() public {
        _bootstrap(alice, 100e6);
        _allocate(TID_1, 40e6);
        _report(1, 40e6, 40e6, 0, TID_1);
        vm.prank(admin);
        vault.freeze();
        (uint64 observedAt, uint64 publishedAt, uint64 expiresAt, uint64 deadline) = _times();
        vm.prank(admin);
        vm.expectRevert(RemoteAccounting.AllocateNotAllowed.selector);
        coordinator.allocate(TID_2, 1e6, 1e6, deadline, observedAt, publishedAt, expiresAt);
        _recall(TID_3, 40e6);
        assertTrue(coordinator.hasActiveRecall());
    }

    function test_replayed_report_does_not_mutate_state() public {
        _bootstrap(alice, 100e6);
        _allocate(TID_1, 40e6);
        bytes memory encoded = _report(1, 40e6, 40e6, 0, TID_1);
        uint256 nav = _managed();
        vm.expectRevert(abi.encodeWithSelector(RemoteAccounting.Replay.selector, uint64(1)));
        coordinator.processRemoteReport(encoded);
        assertEq(_managed(), nav);
        assertEq(coordinator.inboundExpectedSequence(), 2);
    }

    function test_out_of_order_sequence_is_rejected() public {
        _bootstrap(alice, 100e6);
        _allocate(TID_1, 40e6);
        ProtocolCodec.Header memory header = _inboundHeader();
        header.sequence = 2;
        bytes memory encoded = ProtocolCodec.encodeRemoteReport(
            header,
            ProtocolCodec.RemoteReportBody({
                epochId: 1,
                remotePrincipal: 40e6,
                reportedValue: 40e6,
                realizedLoss: 0,
                unattributedBalance: 0,
                latestCompletedTransferId: TID_1,
                probeStatus: ProtocolCodec.PROBE_NOT_REQUIRED,
                probeTimestamp: 0,
                configVersion: uint64(vault.configVersion()),
                remoteStateCommitment: keccak256("remote-state")
            })
        );
        vm.expectRevert(
            abi.encodeWithSelector(RemoteAccounting.WrongSequence.selector, uint64(1), uint64(2))
        );
        coordinator.processRemoteReport(encoded);
        assertEq(vault.outboundInTransit(), 40e6);
        assertTrue(coordinator.hasActiveAllocate());
    }

    function test_wrong_destination_is_rejected() public {
        _bootstrap(alice, 100e6);
        ProtocolCodec.Header memory header = _inboundHeader();
        header.destinationApplication = keccak256("other-app");
        bytes memory encoded = ProtocolCodec.encodeRemoteReport(
            header,
            ProtocolCodec.RemoteReportBody({
                epochId: 1,
                remotePrincipal: 0,
                reportedValue: 0,
                realizedLoss: 0,
                unattributedBalance: 0,
                latestCompletedTransferId: bytes32(0),
                probeStatus: ProtocolCodec.PROBE_NOT_REQUIRED,
                probeTimestamp: 0,
                configVersion: uint64(vault.configVersion()),
                remoteStateCommitment: keccak256("remote-state")
            })
        );
        vm.expectRevert(RemoteAccounting.WrongDestination.selector);
        coordinator.processRemoteReport(encoded);
    }

    function test_allocate_after_cutoff_is_rejected() public {
        _bootstrap(alice, 100e6);
        vm.warp(vault.currentEpochCutoffAt());
        vault.cutoffEpoch();
        (uint64 observedAt, uint64 publishedAt, uint64 expiresAt, uint64 deadline) = _times();
        vm.prank(admin);
        vm.expectRevert(RemoteAccounting.AllocateNotAllowed.selector);
        coordinator.allocate(TID_1, 10e6, 10e6, deadline, observedAt, publishedAt, expiresAt);
    }

    function test_post_cutoff_valuation_locks() public {
        _bootstrap(alice, 100e6);
        _allocate(TID_1, 40e6);
        _report(1, 40e6, 40e6, 0, TID_1);
        vm.warp(vault.currentEpochCutoffAt());
        vault.cutoffEpoch();
        assertEq(uint8(vault.currentEpochPhase()), uint8(EpochPhase.CutOff));
        _report(vault.currentEpochId(), 40e6, 40e6, 0, bytes32(0));
        assertTrue(vault.epochRemoteValuationLocked(vault.currentEpochId()));
        bytes memory encoded = _encodeReport(vault.currentEpochId(), 40e6, 30e6, 0, bytes32(0));
        _expectSkipped(vault.currentEpochId(), IRemoteAccounting.ReportSkipReason.ValuationFixed);
        coordinator.processRemoteReport(encoded);
        assertEq(vault.acceptedRemoteValue(), 40e6);
    }

    function test_redemption_liquidity_uses_idle_only() public {
        _bootstrap(alice, 100e6);
        _allocate(TID_1, 80e6);
        _report(1, 80e6, 80e6, 0, TID_1);
        assertEq(_managed(), 100e6);
        assertEq(vault.idleBacking(), 20e6);
        _redeem(alice, vault.balanceOf(alice));
        vm.warp(vault.currentEpochCutoffAt());
        vault.cutoffEpoch();
        _report(vault.currentEpochId(), 80e6, 80e6, 0, bytes32(0));
        vm.expectRevert(
            abi.encodeWithSelector(ISolEVMVault.InsufficientLiquidity.selector, 20e6, 100e6)
        );
        vault.finalizeEpoch();
    }

    function test_unauthenticated_message_is_rejected() public {
        authenticator.setApproveAll(false);
        ProtocolCodec.Header memory header = _inboundHeader();
        bytes memory encoded = ProtocolCodec.encodeRemoteReport(
            header,
            ProtocolCodec.RemoteReportBody({
                epochId: 1,
                remotePrincipal: 0,
                reportedValue: 0,
                realizedLoss: 0,
                unattributedBalance: 0,
                latestCompletedTransferId: bytes32(0),
                probeStatus: ProtocolCodec.PROBE_NOT_REQUIRED,
                probeTimestamp: 0,
                configVersion: uint64(vault.configVersion()),
                remoteStateCommitment: keccak256("remote-state")
            })
        );
        vm.expectRevert(RemoteAccounting.UnauthenticatedMessage.selector);
        coordinator.processRemoteReport(encoded);
    }

    function test_first_post_cutoff_report_locks_pricing_assets() public {
        _bootstrap(alice, 100e6);
        _allocate(TID_1, 40e6);
        _report(1, 40e6, 40e6, 0, TID_1);
        uint256 nav = _managed();
        vm.warp(vault.currentEpochCutoffAt());
        vault.cutoffEpoch();
        _report(vault.currentEpochId(), 40e6, 40e6, 0, bytes32(0));
        uint64 epochId = vault.currentEpochId();
        assertTrue(vault.epochRemoteValuationLocked(epochId));
        assertEq(vault.lockedPricingAssets(epochId), nav);
    }

    function test_later_remote_report_cannot_replace_locked_pricing() public {
        _bootstrap(alice, 100e6);
        _allocate(TID_1, 40e6);
        _report(1, 40e6, 40e6, 0, TID_1);
        vm.warp(vault.currentEpochCutoffAt());
        vault.cutoffEpoch();
        _report(vault.currentEpochId(), 40e6, 40e6, 0, bytes32(0));
        uint256 locked = vault.lockedPricingAssets(vault.currentEpochId());
        bytes memory encoded = _encodeReport(vault.currentEpochId(), 40e6, 10e6, 0, bytes32(0));
        _expectSkipped(vault.currentEpochId(), IRemoteAccounting.ReportSkipReason.ValuationFixed);
        coordinator.processRemoteReport(encoded);
        assertEq(vault.lockedPricingAssets(vault.currentEpochId()), locked);
        assertEq(vault.acceptedRemoteValue(), 40e6);
    }

    function test_in_flight_allocate_may_complete_after_lock_without_changing_pricing() public {
        _bootstrap(alice, 100e6);
        _allocate(TID_1, 40e6);
        uint256 nav = _managed();
        vm.warp(vault.currentEpochCutoffAt());
        vault.cutoffEpoch();
        _report(vault.currentEpochId(), 0, 0, 0, bytes32(0));
        uint64 epochId = vault.currentEpochId();
        uint256 locked = vault.lockedPricingAssets(epochId);
        assertTrue(vault.epochRemoteValuationLocked(epochId));
        assertEq(locked, nav);
        _report(epochId, 40e6, 40e6, 0, TID_1);
        assertFalse(coordinator.hasActiveAllocate());
        assertEq(vault.outboundInTransit(), 0);
        assertEq(vault.acceptedRemoteValue(), 40e6);
        assertEq(vault.lockedPricingAssets(epochId), locked);
        assertEq(_managed(), nav);
    }

    function test_recall_sent_after_lock_cannot_change_locked_pricing() public {
        _bootstrap(alice, 100e6);
        _allocate(TID_1, 40e6);
        _report(1, 40e6, 40e6, 0, TID_1);
        vm.warp(vault.currentEpochCutoffAt());
        vault.cutoffEpoch();
        _report(vault.currentEpochId(), 40e6, 40e6, 0, bytes32(0));
        uint64 epochId = vault.currentEpochId();
        uint256 locked = vault.lockedPricingAssets(epochId);
        _recall(TID_2, 40e6);
        _recallSent(TID_2, 40e6, 35e6, 5e6);
        assertEq(vault.lockedPricingAssets(epochId), locked);
        assertEq(_managed(), locked - 5e6);
    }

    function test_inbound_arrival_after_lock_cannot_change_locked_pricing() public {
        _bootstrap(alice, 100e6);
        _allocate(TID_1, 40e6);
        _report(1, 40e6, 40e6, 0, TID_1);
        vm.warp(vault.currentEpochCutoffAt());
        vault.cutoffEpoch();
        _report(vault.currentEpochId(), 40e6, 40e6, 0, bytes32(0));
        uint64 epochId = vault.currentEpochId();
        uint256 locked = vault.lockedPricingAssets(epochId);
        _recall(TID_2, 40e6);
        _recallSent(TID_2, 40e6, 35e6, 5e6);
        _observe(35e6);
        _observe(35e6);
        assertEq(vault.lockedPricingAssets(epochId), locked);
        assertEq(vault.unattributedBalance(), 35e6);
        assertEq(_managed(), locked - 5e6);
    }

    function test_finalization_uses_exactly_the_locked_pricing_value() public {
        _bootstrap(alice, 100e6);
        _allocate(TID_1, 40e6);
        _report(1, 40e6, 40e6, 0, TID_1);
        vm.warp(vault.currentEpochCutoffAt());
        vault.cutoffEpoch();
        _report(vault.currentEpochId(), 40e6, 40e6, 0, bytes32(0));
        uint64 epochId = vault.currentEpochId();
        uint256 locked = vault.lockedPricingAssets(epochId);
        _recall(TID_2, 40e6);
        _recallSent(TID_2, 40e6, 35e6, 5e6);
        _observe(35e6);
        vault.finalizeEpoch();
        assertEq(vault.settledEpoch(epochId).totalAssets, locked);
        assertEq(vault.settledEpoch(epochId).totalAssets, 100e6);
    }
}
