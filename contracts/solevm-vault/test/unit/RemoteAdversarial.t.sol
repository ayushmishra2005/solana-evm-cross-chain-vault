// SPDX-License-Identifier: Apache-2.0
pragma solidity 0.8.30;

import {RemoteAccounting} from "../../src/RemoteAccounting.sol";
import {IRemoteAccounting} from "../../src/interfaces/IRemoteAccounting.sol";
import {ProtocolCodec} from "../../src/libraries/ProtocolCodec.sol";
import {RemoteTestBase} from "../RemoteTestBase.sol";

/// Adversarial arrival-order and identity cases for the remote coordinator.
contract RemoteAdversarialTest is RemoteTestBase {
    bytes32 internal constant TID_A = bytes32(uint256(1));
    bytes32 internal constant TID_B = bytes32(uint256(2));

    function test_message_without_assets_leaves_allocation_open() public {
        _bootstrap(alice, 100e6);
        _allocate(TID_A, 40e6);
        assertEq(vault.outboundInTransit(), 40e6);
        assertEq(vault.acceptedRemoteValue(), 0);
        assertTrue(coordinator.hasActiveAllocate());
        (uint64 observedAt, uint64 publishedAt, uint64 expiresAt, uint64 deadline) = _times();
        vm.prank(admin);
        vm.expectRevert(
            abi.encodeWithSelector(RemoteAccounting.ActiveTransferExists.selector, TID_A)
        );
        coordinator.allocate(TID_B, 1e6, 1e6, deadline, observedAt, publishedAt, expiresAt);
    }

    function test_assets_without_completion_do_not_create_remote_principal() public {
        _bootstrap(alice, 100e6);
        _allocate(TID_A, 40e6);
        _observe(40e6);
        assertEq(vault.remotePrincipalCap(), 0);
        assertEq(vault.acceptedRemoteValue(), 0);
        assertEq(vault.unattributedBalance(), 40e6);
        assertEq(_managed(), 100e6);
    }

    function test_recall_sent_without_return_assets_stays_in_inbound() public {
        _bootstrap(alice, 100e6);
        _allocate(TID_A, 40e6);
        _report(1, 40e6, 40e6, 0, TID_A);
        _recall(TID_B, 40e6);
        _recallSent(TID_B, 40e6, 35e6, 5e6);
        assertEq(vault.inboundInTransit(), 35e6);
        assertEq(vault.idleBacking(), 60e6);
        assertEq(_managed(), 95e6);
    }

    function test_wrong_lane_is_rejected() public {
        _bootstrap(alice, 100e6);
        ProtocolCodec.Header memory header = _inboundHeader();
        header.laneId = 99;
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
        vm.expectRevert(
            abi.encodeWithSelector(
                RemoteAccounting.WrongLane.selector, uint32(INBOUND_LANE), uint32(99)
            )
        );
        coordinator.processRemoteReport(encoded);
    }

    function test_wrong_transfer_id_on_recall_sent_is_rejected() public {
        _bootstrap(alice, 100e6);
        _allocate(TID_A, 40e6);
        _report(1, 40e6, 40e6, 0, TID_A);
        _recall(TID_B, 40e6);
        bytes memory encoded = ProtocolCodec.encodeRecallSent(
            _inboundHeader(),
            ProtocolCodec.RecallSentBody({
                transferId: TID_A,
                principalSent: 40e6,
                actualAmountSent: 40e6,
                realizedLoss: 0,
                destinationReference: destRef,
                sentTimestamp: uint64(block.timestamp),
                configVersion: uint64(vault.configVersion())
            })
        );
        vm.expectRevert(
            abi.encodeWithSelector(RemoteAccounting.WrongTransferId.selector, TID_B, TID_A)
        );
        coordinator.processRecallSent(encoded);
        assertTrue(coordinator.hasActiveRecall());
        assertEq(vault.remotePrincipalCap(), 40e6);
    }

    function test_expired_report_is_consumed_without_effect() public {
        _bootstrap(alice, 100e6);
        _allocate(TID_A, 40e6);
        _report(1, 40e6, 40e6, 0, TID_A);
        ProtocolCodec.Header memory header = _inboundHeader();
        header.expiresAt = uint64(block.timestamp) + 1;
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
        vm.warp(uint256(header.expiresAt) + 1);
        _expectSkipped(1, IRemoteAccounting.ReportSkipReason.Expired);
        coordinator.processRemoteReport(encoded);
        assertEq(coordinator.inboundExpectedSequence(), header.sequence + 1);
        assertEq(vault.acceptedRemoteValue(), 40e6);
        assertEq(_managed(), 100e6);
    }
}
