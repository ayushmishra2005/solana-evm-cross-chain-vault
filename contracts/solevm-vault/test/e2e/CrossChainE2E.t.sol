// SPDX-License-Identifier: Apache-2.0
pragma solidity 0.8.30;

import {RemoteAccounting} from "../../src/RemoteAccounting.sol";
import {IRemoteAccounting} from "../../src/interfaces/IRemoteAccounting.sol";
import {EpochPhase} from "../../src/interfaces/ISolEVMVault.sol";
import {ProtocolCodec} from "../../src/libraries/ProtocolCodec.sol";
import {RemoteTestBase} from "../RemoteTestBase.sol";

contract CrossChainE2ETest is RemoteTestBase {
    uint32 internal constant SOLANA_SOURCE_CHAIN = 8453;
    uint32 internal constant SOLANA_DEST_CHAIN = 900;
    uint32 internal constant CONTROL_LANE = 1;

    bytes32 internal constant ALLOCATE_TID =
        hex"7777777777777777777777777777777777777777777777777777777777777777";
    bytes32 internal constant RECALL_TID =
        hex"8888888888888888888888888888888888888888888888888888888888888888";
    bytes32 internal constant SECOND_ALLOCATE =
        hex"7979797979797979797979797979797979797979797979797979797979797979";
    bytes32 internal constant RECOVERY_TID =
        hex"9999999999999999999999999999999999999999999999999999999999999999";

    uint256 internal constant BACKING = 10e6;
    uint256 internal constant AMOUNT = 1e6;

    string[] internal script;
    bytes internal lastOutbound;

    struct Leg {
        uint256 principal;
        uint256 deployed;
        uint256 custody;
        uint256 unattributed;
        uint256 recalled;
        uint256 realizedLoss;
        uint64 laneSequence;
        uint64 outboundNext;
        uint256 inboundAsset;
        uint256 impaired;
        uint256 lastEventId;
        bool transferOpen;
        bool frozen;
        bool lastAccepted;
        string lastError;
        bytes latestReport;
        bytes latestRecallSent;
        bytes latestSettlement;
        string[] reports;
        string[] recallSents;
        string[] settlements;
    }

    function _identities()
        internal
        pure
        override
        returns (IRemoteAccounting.Identities memory identities)
    {
        identities = IRemoteAccounting.Identities({
            evmChainId: SOLANA_SOURCE_CHAIN,
            solanaChainId: SOLANA_DEST_CHAIN,
            evmApplication: hex"3333333333333333333333333333333333333333333333333333333333333333",
            solanaApplication: hex"4444444444444444444444444444444444444444444444444444444444444444",
            deploymentId: hex"1111111111111111111111111111111111111111111111111111111111111111",
            vaultId: hex"2222222222222222222222222222222222222222222222222222222222222222",
            outboundLaneId: CONTROL_LANE,
            inboundLaneId: 2,
            expectedDestinationReference: keccak256("destination-ref")
        });
    }

    function test_allocate_message_before_assets() public {
        uint256 origin = _start();
        uint256 shares = vault.totalSupply();
        bytes memory allocate = _outAllocate(ALLOCATE_TID, AMOUNT);

        assertEq(vault.idleBacking(), BACKING - AMOUNT);
        assertEq(vault.outboundInTransit(), AMOUNT);
        assertEq(_managed(), origin);

        _scheduleControl(allocate, 1);
        _idle();
        _applyControl();
        Leg memory beforeAssets = _leg();
        assertTrue(beforeAssets.lastAccepted, beforeAssets.lastError);
        assertEq(beforeAssets.principal, 0);
        assertEq(beforeAssets.deployed, 0);
        assertEq(beforeAssets.custody, 0);
        assertTrue(beforeAssets.transferOpen);
        assertEq(vault.outboundInTransit(), AMOUNT);
        assertEq(vault.acceptedRemoteValue(), 0);
        assertEq(_managed(), origin);
        assertEq(vault.totalSupply(), shares);

        _scheduleAsset(ALLOCATE_TID, AMOUNT, 2);
        _idle();
        _applyAsset();
        _attribute(ALLOCATE_TID);
        Leg memory afterAssets = _leg();
        assertTrue(afterAssets.lastAccepted, afterAssets.lastError);
        assertEq(afterAssets.principal, AMOUNT);
        assertEq(afterAssets.custody, AMOUNT);
        assertEq(afterAssets.unattributed, 0);
        assertFalse(afterAssets.transferOpen);

        _completeAllocateOnHub(1);
        assertEq(vault.outboundInTransit(), 0);
        assertEq(vault.remotePrincipalCap(), AMOUNT);
        assertEq(vault.acceptedRemoteValue(), AMOUNT);
        assertEq(_managed(), origin);
        assertEq(vault.totalSupply(), shares);
    }

    function test_assets_before_allocate_message() public {
        uint256 origin = _start();
        bytes memory allocate = _outAllocate(ALLOCATE_TID, AMOUNT);

        _scheduleAsset(ALLOCATE_TID, AMOUNT + 250_000, 1);
        _idle();
        _applyAsset();
        Leg memory unattributed = _leg();
        assertEq(unattributed.custody, AMOUNT + 250_000);
        assertEq(unattributed.principal, 0);
        assertEq(unattributed.unattributed, 0);
        assertFalse(unattributed.transferOpen);

        _scheduleControl(allocate, 2);
        _idle();
        _applyControl();
        _attribute(ALLOCATE_TID);
        Leg memory attributed = _leg();
        assertTrue(attributed.lastAccepted, attributed.lastError);
        assertEq(attributed.principal, AMOUNT);
        assertEq(attributed.unattributed, 250_000);
        assertEq(vault.outboundInTransit(), AMOUNT);
        assertEq(vault.acceptedRemoteValue(), 0);

        _completeAllocateOnHub(1);
        assertEq(vault.acceptedRemoteValue(), AMOUNT);
        assertEq(_managed(), origin);
    }

    function test_healthy_full_allocation_and_recall() public {
        uint256 origin = _start();
        uint256 shares = vault.totalSupply();
        _fundRemote(AMOUNT);
        assertEq(vault.outboundInTransit(), 0);
        assertEq(vault.acceptedRemoteValue(), AMOUNT);
        assertEq(vault.inboundInTransit(), 0);
        assertEq(_managed(), origin);
        assertEq(vault.totalSupply(), shares);

        bytes memory recall = _outRecall(RECALL_TID, AMOUNT);
        _scheduleControl(recall, 8);
        _idle();
        _applyControl();
        _withdraw(RECALL_TID, AMOUNT);
        _send(RECALL_TID, AMOUNT, 9);
        _publishRecallSent(RECALL_TID, 10);
        _idle();
        Leg memory sent = _leg();
        assertTrue(sent.lastAccepted, sent.lastError);
        assertEq(sent.latestRecallSent.length, 375);
        assertEq(sent.inboundAsset, AMOUNT);

        coordinator.processRecallSent(sent.latestRecallSent);
        assertEq(vault.acceptedRemoteValue(), 0);
        assertEq(vault.remotePrincipalCap(), 0);
        assertEq(vault.inboundInTransit(), AMOUNT);
        assertEq(vault.idleBacking(), BACKING - AMOUNT);
        assertEq(_managed(), origin);
        assertFalse(coordinator.hasActiveRecall());

        _observe(sent.inboundAsset);
        assertEq(vault.inboundInTransit(), 0);
        assertEq(vault.idleBacking(), BACKING);
        assertEq(_managed(), origin);
        assertEq(vault.totalSupply(), shares);

        _publishReport(1, 11);
        _idle();
        coordinator.processRemoteReport(_leg().latestReport);
        assertEq(vault.acceptedRemoteValue(), 0);
        assertEq(_managed(), origin);
    }

    function test_partial_loss_recall() public {
        uint256 origin = _start();
        _fundRemote(AMOUNT);
        _outRecall(RECALL_TID, AMOUNT);
        _scheduleLastOutbound(8);
        _idle();
        _applyControl();
        _configureAdapter(type(uint64).max, 2500);
        _withdraw(RECALL_TID, AMOUNT);
        _send(RECALL_TID, AMOUNT, 9);
        _publishRecallSent(RECALL_TID, 10);
        _idle();
        Leg memory sent = _leg();
        assertTrue(sent.lastAccepted, sent.lastError);
        assertEq(sent.realizedLoss, 250_000);

        coordinator.processRecallSent(sent.latestRecallSent);
        assertEq(vault.recognizedLoss(), 250_000);
        assertEq(vault.inboundInTransit(), 750_000);
        assertEq(_managed() + vault.recognizedLoss(), origin);
        _observe(sent.inboundAsset);
        assertEq(vault.inboundInTransit(), 0);
        assertEq(vault.idleBacking(), BACKING - 250_000);
        assertEq(_managed() + vault.recognizedLoss(), origin);

        _publishReport(1, 11);
        _idle();
        coordinator.processRemoteReport(_leg().latestReport);
        assertEq(vault.acceptedRemoteValue(), 0);
        assertEq(vault.recognizedLoss(), 250_000);
    }

    function test_report_published_between_send_and_recall_sent() public {
        uint256 origin = _start();
        _fundRemote(AMOUNT);
        _lossyHalfRecall();
        _publishReport(1, 10);
        _idle();
        bytes memory report = _leg().latestReport;
        _publishRecallSent(RECALL_TID, 11);
        _idle();
        Leg memory sent = _leg();
        assertTrue(sent.lastAccepted, sent.lastError);
        assertEq(sent.realizedLoss, 125_000);

        coordinator.processRemoteReport(report);
        assertEq(_managed(), origin - 125_000, "report before notice");
        coordinator.processRecallSent(sent.latestRecallSent);
        assertEq(_managed(), origin - 125_000, "notice");
        assertEq(vault.acceptedRemoteValue(), AMOUNT / 2);
        assertEq(vault.recognizedLoss(), 125_000);
        _observe(sent.inboundAsset);
        assertEq(_managed(), origin - 125_000, "arrival");
    }

    function test_lossy_recall_across_cutoff_prices_the_epoch_net_of_loss() public {
        uint256 origin = _start();
        _fundRemote(AMOUNT);
        _lossyHalfRecall();
        uint64 epochId = vault.currentEpochId();
        uint64 cutoffAt = vault.currentEpochCutoffAt();
        if (block.timestamp < cutoffAt) vm.warp(cutoffAt);
        vault.cutoffEpoch();
        _sync();
        _publishReport(epochId, 20);
        _idle();
        bytes memory report = _leg().latestReport;
        _publishRecallSent(RECALL_TID, 21);
        _idle();
        Leg memory sent = _leg();
        assertTrue(sent.lastAccepted, sent.lastError);

        coordinator.processRemoteReport(report);
        assertEq(vault.lockedPricingAssets(epochId), origin - 125_000, "locked value");
        vault.finalizeEpoch();
        assertEq(vault.settledEpoch(epochId).totalAssets, origin - 125_000, "pricing basis");
        coordinator.processRecallSent(sent.latestRecallSent);
        _observe(sent.inboundAsset);
        assertEq(_managed(), origin - 125_000, "settled");
    }

    function test_total_loss_recall() public {
        uint256 origin = _start();
        _fundRemote(AMOUNT);
        _outRecall(RECALL_TID, AMOUNT);
        _scheduleLastOutbound(8);
        _idle();
        _applyControl();
        _configureAdapter(type(uint64).max, 10_000);
        _withdraw(RECALL_TID, AMOUNT);
        _publishRecallSent(RECALL_TID, 9);
        _idle();
        Leg memory sent = _leg();
        assertTrue(sent.lastAccepted, sent.lastError);
        assertEq(sent.realizedLoss, AMOUNT);
        assertEq(sent.inboundAsset, 0);

        coordinator.processRecallSent(sent.latestRecallSent);
        assertEq(vault.recognizedLoss(), AMOUNT);
        assertEq(vault.inboundInTransit(), 0);
        assertEq(_managed() + vault.recognizedLoss(), origin);
        assertFalse(coordinator.hasActiveRecall());

        _publishReport(1, 10);
        _idle();
        coordinator.processRemoteReport(_leg().latestReport);
        assertEq(vault.acceptedRemoteValue(), 0);
        assertEq(_managed() + vault.recognizedLoss(), origin);
    }

    function test_duplicate_control_message_is_rejected() public {
        uint256 origin = _start();
        bytes memory allocate = _outAllocate(ALLOCATE_TID, AMOUNT);
        _scheduleControl(allocate, 1);
        uint256 eventId = _leg().lastEventId;
        _push(
            _obj(
                "fault_duplicate", string.concat('"event":', _u(eventId), ',"copies":1,"spacing":0')
            )
        );
        _idle();
        _applyControl();
        Leg memory view_ = _leg();
        assertFalse(view_.lastAccepted);
        assertEq(view_.principal, 0);
        assertTrue(view_.transferOpen);
        assertEq(view_.laneSequence, 1);
        assertEq(vault.outboundInTransit(), AMOUNT);
        assertEq(_managed(), origin);
    }

    function test_delayed_and_reordered_delivery() public {
        uint256 origin = _start();
        bytes memory allocate = _outAllocate(ALLOCATE_TID, AMOUNT);
        _scheduleAsset(ALLOCATE_TID, AMOUNT, 1);
        _scheduleControl(allocate, 4);
        _idle();
        _applyAsset();
        _applyControl();
        _attribute(ALLOCATE_TID);
        assertEq(_leg().principal, AMOUNT);
        _completeAllocateOnHub(5);
        bytes memory recall = _outRecall(RECALL_TID, AMOUNT);
        _scheduleControl(recall, 6);
        _idle();
        _applyControl();
        _withdraw(RECALL_TID, AMOUNT);
        _send(RECALL_TID, AMOUNT, 20);
        _publishRecallSent(RECALL_TID, 7);
        _advance(7);
        _deliverReady();
        Leg memory beforeAssets = _leg();
        coordinator.processRecallSent(beforeAssets.latestRecallSent);
        assertEq(vault.inboundInTransit(), AMOUNT);
        assertEq(vault.idleBacking(), BACKING - AMOUNT);
        assertEq(beforeAssets.inboundAsset, 0);
        _advance(20);
        _deliverReady();
        uint256 arrived = _leg().inboundAsset;
        assertEq(arrived, AMOUNT);
        _observe(arrived);
        assertEq(vault.inboundInTransit(), 0);
        assertEq(_managed(), origin);
    }

    function test_freeze_during_recall_preserves_unwind() public {
        _start();
        _fundRemote(AMOUNT);
        bytes memory recall = _outRecall(RECALL_TID, AMOUNT);
        _scheduleControl(recall, 8);
        _idle();
        _applyControl();
        assertTrue(_leg().transferOpen);
        _push('{"op":"freeze"}');
        Leg memory frozen = _leg();
        assertTrue(frozen.lastAccepted, frozen.lastError);
        assertTrue(frozen.frozen);

        (uint64 observedAt, uint64 publishedAt, uint64 expiresAt, uint64 deadline) = _times();
        vm.prank(admin);
        vm.expectRevert();
        coordinator.allocate(SECOND_ALLOCATE, 1, 1, deadline, observedAt, publishedAt, expiresAt);
        _push(
            _obj(
                "allocate_next",
                string.concat('"transfer":"', vm.toString(SECOND_ALLOCATE), '","amount":1')
            )
        );
        _idle();
        _applyControl();
        assertFalse(_leg().lastAccepted);

        _withdraw(RECALL_TID, AMOUNT);
        _send(RECALL_TID, AMOUNT, 10);
        _publishRecallSent(RECALL_TID, 11);
        _publishReport(1, 12);
        _idle();
        Leg memory done = _leg();
        assertTrue(done.lastAccepted, done.lastError);
        coordinator.processRecallSent(done.latestRecallSent);
        assertFalse(coordinator.hasActiveRecall());
    }

    function test_dropped_asset_leaves_allocation_unresolved() public {
        uint256 origin = _start();
        bytes memory allocate = _outAllocate(ALLOCATE_TID, AMOUNT);
        _scheduleControl(allocate, 1);
        _scheduleAsset(ALLOCATE_TID, AMOUNT, 1);
        uint256 assetEvent = _leg().lastEventId;
        _push(_obj("fault_drop", string.concat('"event":', _u(assetEvent))));
        _idle();
        _applyControl();
        _applyAsset();
        _attribute(ALLOCATE_TID);
        Leg memory stuck = _leg();
        assertFalse(stuck.lastAccepted);
        assertEq(stuck.principal, 0);
        assertEq(stuck.custody, 0);
        assertTrue(stuck.transferOpen);
        assertEq(vault.outboundInTransit(), AMOUNT);
        assertEq(vault.acceptedRemoteValue(), 0);
        assertEq(_managed(), origin);

        vm.expectRevert();
        _allocate(SECOND_ALLOCATE, 1);
    }

    function test_duplicate_asset_delivery_cannot_over_attribute() public {
        _start();
        bytes memory allocate = _outAllocate(ALLOCATE_TID, AMOUNT);
        _scheduleControl(allocate, 1);
        _scheduleAsset(ALLOCATE_TID, AMOUNT, 1);
        uint256 assetEvent = _leg().lastEventId;
        _push(
            _obj(
                "fault_duplicate",
                string.concat('"event":', _u(assetEvent), ',"copies":1,"spacing":0')
            )
        );
        _idle();
        _applyControl();
        _applyAsset();
        _attribute(ALLOCATE_TID);
        Leg memory view_ = _leg();
        assertTrue(view_.lastAccepted, view_.lastError);
        assertEq(view_.principal, AMOUNT);
        assertEq(view_.unattributed, AMOUNT);
        assertEq(view_.custody, AMOUNT * 2);
        _completeAllocateOnHub(2);
        assertEq(vault.acceptedRemoteValue(), AMOUNT);
        assertEq(vault.remotePrincipalCap(), AMOUNT);
    }

    function test_outbound_report_ordering_and_failed_publish() public {
        _start();
        _fundRemote(AMOUNT);
        assertEq(_leg().outboundNext, 2);
        _push(_obj("publish_report", '"epoch":0,"at":8'));
        Leg memory failed = _leg();
        assertFalse(failed.lastAccepted);
        assertEq(failed.outboundNext, 2);
        assertEq(failed.reports.length, 1);

        _outRecall(RECALL_TID, AMOUNT);
        _scheduleLastOutbound(8);
        _idle();
        _applyControl();
        _withdraw(RECALL_TID, AMOUNT);
        _send(RECALL_TID, AMOUNT, 9);
        _publishRecallSent(RECALL_TID, 10);
        _publishReport(1, 11);
        _idle();
        Leg memory ordered = _leg();
        assertEq(ordered.recallSents.length, 1);
        assertEq(ordered.reports.length, 2);
        (ProtocolCodec.Header memory noticeHeader,) =
            ProtocolCodec.decodeRecallSent(ordered.latestRecallSent);
        (ProtocolCodec.Header memory reportHeader,) =
            ProtocolCodec.decodeRemoteReport(ordered.latestReport);
        assertEq(noticeHeader.sequence, 2);
        assertEq(reportHeader.sequence, 3);
        coordinator.processRecallSent(ordered.latestRecallSent);
        coordinator.processRemoteReport(ordered.latestReport);
    }

    function test_remote_capital_is_load_bearing_for_nav() public {
        uint256 origin = _start();
        bytes memory allocate = _outAllocate(ALLOCATE_TID, AMOUNT);
        assertEq(vault.idleBacking() + vault.outboundInTransit(), origin);
        assertEq(vault.acceptedRemoteValue(), 0);
        _scheduleControl(allocate, 1);
        _scheduleAsset(ALLOCATE_TID, AMOUNT, 1);
        _idle();
        _applyControl();
        _applyAsset();
        _attribute(ALLOCATE_TID);
        _deploy(AMOUNT);
        _publishReport(1, 2);
        _idle();
        coordinator.processRemoteReport(_leg().latestReport);
        assertEq(vault.idleBacking(), BACKING - AMOUNT);
        assertEq(vault.outboundInTransit(), 0);
        assertEq(vault.acceptedRemoteValue(), AMOUNT);
        assertEq(vault.inboundInTransit(), 0);
        assertEq(_managed(), origin);

        _outRecall(RECALL_TID, AMOUNT);
        assertEq(_managed(), origin);
        _scheduleLastOutbound(3);
        _idle();
        _applyControl();
        _withdraw(RECALL_TID, AMOUNT);
        _send(RECALL_TID, AMOUNT, 4);
        _publishRecallSent(RECALL_TID, 5);
        _idle();
        coordinator.processRecallSent(_leg().latestRecallSent);
        assertEq(vault.acceptedRemoteValue(), 0);
        assertEq(vault.inboundInTransit(), AMOUNT);
        assertEq(vault.idleBacking(), BACKING - AMOUNT);
        assertEq(_managed(), origin);
        _observe(_leg().inboundAsset);
        assertEq(vault.inboundInTransit(), 0);
        assertEq(vault.idleBacking(), BACKING);
        assertEq(_managed(), origin);
    }

    function test_fixed_epoch_valuation_with_remote_capital() public {
        _start();
        _fundRemote(AMOUNT);
        _deposit(alice, AMOUNT);
        uint64 epochId = vault.currentEpochId();
        uint256 sharesBefore = vault.totalSupply();
        vm.warp(vault.currentEpochCutoffAt());
        vault.cutoffEpoch();
        assertEq(uint8(vault.currentEpochPhase()), uint8(EpochPhase.CutOff));
        _sync();
        _publishReport(epochId, 20);
        _idle();
        coordinator.processRemoteReport(_leg().latestReport);
        assertTrue(vault.epochRemoteValuationLocked(epochId));
        uint256 locked = vault.lockedPricingAssets(epochId);
        _publishReport(epochId, 21);
        _idle();
        bytes memory later = _leg().latestReport;
        _expectSkipped(epochId, IRemoteAccounting.ReportSkipReason.ValuationFixed);
        coordinator.processRemoteReport(later);
        vault.finalizeEpoch();
        uint256 priced = vault.claimableDepositShares(epochId, alice);
        assertGt(priced, 0);
        assertEq(vault.lockedPricingAssets(epochId), locked);
        uint256 claimed = _claimDeposit(alice, epochId);
        assertEq(claimed, priced);
        assertEq(vault.totalSupply(), sharesBefore + claimed);
    }

    function test_config_update_shares_the_control_lane() public {
        _start();
        _fundRemote(AMOUNT);
        assertEq(_leg().laneSequence, 1);
        _push('{"op":"config_update_next"}');
        _idle();
        _applyControl();
        Leg memory updated = _leg();
        assertTrue(updated.lastAccepted, updated.lastError);
        assertEq(updated.laneSequence, 2);

        _push('{"op":"config_update_reuse_sequence"}');
        _idle();
        _applyControl();
        assertFalse(_leg().lastAccepted);
        assertEq(_leg().laneSequence, 2);

        _push('{"op":"config_update_wrong_lane"}');
        _idle();
        _applyControl();
        assertFalse(_leg().lastAccepted);
        assertEq(_leg().laneSequence, 2);
    }

    function test_wrong_destination_report_is_rejected() public {
        _start();
        _fundRemote(AMOUNT);
        bytes memory report = _leg().latestReport;
        (ProtocolCodec.Header memory header, ProtocolCodec.RemoteReportBody memory body) =
            ProtocolCodec.decodeRemoteReport(report);
        header.destinationApplication = keccak256("other-app");
        bytes memory encoded = ProtocolCodec.encodeRemoteReport(header, body);
        vm.expectRevert(RemoteAccounting.WrongDestination.selector);
        coordinator.processRemoteReport(encoded);
        assertEq(coordinator.inboundExpectedSequence(), 2);
    }

    function test_exact_solana_report_and_recall_sent_are_accepted() public {
        _start();
        _fundRemote(AMOUNT);
        _outRecall(RECALL_TID, AMOUNT);
        _scheduleLastOutbound(3);
        _idle();
        _applyControl();
        _withdraw(RECALL_TID, AMOUNT);
        _send(RECALL_TID, AMOUNT, 4);
        _publishRecallSent(RECALL_TID, 5);
        _idle();
        bytes memory notice = _leg().latestRecallSent;
        coordinator.processRecallSent(notice);
        (ProtocolCodec.Header memory header,) = ProtocolCodec.decodeRecallSent(notice);
        assertEq(header.sequence, 2);
    }

    /// 0.4 of a 1.0 allocation arrives in time. The close after the deadline
    /// impairs 0.6 on both chains; the late 0.6 is then recovered against the
    /// pool, and no report ever counts the allocation twice.
    function test_short_allocation_closes_then_late_custody_is_recovered() public {
        uint256 origin = _start();
        _allocateWithPartialArrival(origin);
        _closeAfterTheDeadline(origin);
        _recoverTheLateArrival(origin);
    }

    function _allocateWithPartialArrival(uint256 origin) internal {
        bytes memory allocate = _outAllocate(ALLOCATE_TID, AMOUNT);
        _scheduleControl(allocate, 1);
        _scheduleAsset(ALLOCATE_TID, 400_000, 1);
        _scheduleAsset(ALLOCATE_TID, 600_000, 50);
        _advance(1);
        _deliverReady();
        _applyControl();
        _applyAsset();
        _attribute(ALLOCATE_TID);
        _publishReport(1, 2);
        _advance(2);
        _deliverReady();
        Leg memory arriving = _leg();
        assertTrue(arriving.lastAccepted, arriving.lastError);
        assertEq(arriving.principal, 400_000);
        assertTrue(arriving.transferOpen);

        (, ProtocolCodec.RemoteReportBody memory early) =
            ProtocolCodec.decodeRemoteReport(arriving.latestReport);
        assertEq(early.remotePrincipal, 0);
        coordinator.processRemoteReport(arriving.latestReport);
        assertEq(vault.outboundInTransit(), AMOUNT);
        assertEq(vault.acceptedRemoteValue(), 0);
        assertEq(_managed(), origin);
    }

    function _closeAfterTheDeadline(uint256 origin) internal {
        vm.warp(uint256(coordinator.activeDeadline()) + 1);
        _sync();
        (uint64 observedAt, uint64 publishedAt, uint64 expiresAt,) = _times();
        vm.prank(admin);
        bytes memory close = coordinator.close(ALLOCATE_TID, observedAt, publishedAt, expiresAt);

        // A report the leg publishes before it sees the close is skipped.
        _publishReport(1, 3);
        _advance(3);
        _deliverReady();
        bytes memory crossing = _leg().latestReport;
        _expectSkipped(1, IRemoteAccounting.ReportSkipReason.SettlementPending);
        coordinator.processRemoteReport(crossing);
        assertEq(vault.outboundInTransit(), AMOUNT);

        _scheduleControl(close, 4);
        _advance(4);
        _deliverReady();
        _applyControl();
        _deliverReady();
        Leg memory closed = _leg();
        assertTrue(closed.lastAccepted, closed.lastError);
        assertFalse(closed.transferOpen);
        assertEq(closed.principal, 400_000);
        assertEq(closed.impaired, 600_000);
        assertEq(closed.settlements.length, 1);

        coordinator.processTransportSettlement(closed.latestSettlement);
        assertEq(vault.outboundInTransit(), 0);
        assertEq(vault.remotePrincipalCap(), 400_000);
        assertEq(vault.acceptedRemoteValue(), 400_000);
        assertEq(vault.impairedTransport(), closed.impaired);
        assertEq(vault.recognizedLoss(), 0);
        assertEq(_managed(), origin - 600_000);
    }

    function _recoverTheLateArrival(uint256 origin) internal {
        _advance(50);
        _deliverReady();
        _applyAsset();
        Leg memory late = _leg();
        assertEq(late.custody, AMOUNT);
        assertEq(late.principal, 400_000);
        assertEq(late.impaired, 600_000);

        (uint64 observedAt, uint64 publishedAt, uint64 expiresAt,) = _times();
        vm.prank(admin);
        bytes memory recovery =
            coordinator.recover(RECOVERY_TID, observedAt, publishedAt, expiresAt);
        _scheduleControl(recovery, 51);
        _advance(51);
        _deliverReady();
        _applyControl();
        _deliverReady();
        Leg memory recovered = _leg();
        assertTrue(recovered.lastAccepted, recovered.lastError);
        assertEq(recovered.impaired, 0);
        assertEq(recovered.principal, AMOUNT);
        assertEq(recovered.unattributed, 0);
        assertEq(recovered.settlements.length, 2);

        coordinator.processTransportSettlement(recovered.latestSettlement);
        assertEq(vault.impairedTransport(), recovered.impaired);
        assertEq(vault.remotePrincipalCap(), AMOUNT);
        assertEq(vault.acceptedRemoteValue(), AMOUNT);
        assertEq(_managed(), origin);

        _publishReport(1, 52);
        _advance(52);
        _deliverReady();
        Leg memory settled = _leg();
        (, ProtocolCodec.RemoteReportBody memory last) =
            ProtocolCodec.decodeRemoteReport(settled.latestReport);
        assertEq(last.remotePrincipal, AMOUNT);
        coordinator.processRemoteReport(settled.latestReport);
        assertEq(vault.acceptedRemoteValue(), AMOUNT);
        assertEq(_managed(), origin);
        assertEq(coordinator.inboundExpectedSequence(), 6);
        assertEq(settled.outboundNext, 6);
    }

    function _start() internal returns (uint256 origin) {
        _bootstrap(alice, BACKING);
        vm.warp(1_700_000_000);
        origin = _managed();
        _sync();
    }

    function _fundRemote(uint256 amount) internal {
        bytes memory allocate = _outAllocate(ALLOCATE_TID, amount);
        _scheduleControl(allocate, 1);
        _scheduleAsset(ALLOCATE_TID, amount, 1);
        _idle();
        _applyControl();
        _applyAsset();
        _attribute(ALLOCATE_TID);
        _deploy(amount);
        _publishReport(1, 2);
        _idle();
        bytes memory report = _leg().latestReport;
        assertGt(report.length, 0);
        coordinator.processRemoteReport(report);
    }

    /// Recalls half the position at a 25% loss and sends what came back.
    function _lossyHalfRecall() internal {
        _outRecall(RECALL_TID, AMOUNT / 2);
        _scheduleLastOutbound(8);
        _idle();
        _applyControl();
        _configureAdapter(type(uint64).max, 2500);
        _withdraw(RECALL_TID, AMOUNT / 2);
        _send(RECALL_TID, 375_000, 9);
    }

    function _completeAllocateOnHub(uint64 at) internal {
        _deploy(AMOUNT);
        _publishReport(1, at);
        _idle();
        bytes memory report = _leg().latestReport;
        assertEq(report.length, 400);
        coordinator.processRemoteReport(report);
    }

    function _outAllocate(bytes32 transferId, uint256 amount)
        internal
        returns (bytes memory message)
    {
        message = _allocate(transferId, uint128(amount));
        lastOutbound = message;
    }

    function _outRecall(bytes32 transferId, uint256 amount)
        internal
        returns (bytes memory message)
    {
        message = _recall(transferId, uint128(amount));
        lastOutbound = message;
    }

    function _scheduleLastOutbound(uint64 at) internal {
        _scheduleControl(lastOutbound, at);
    }

    function _scheduleControl(bytes memory payload, uint64 at) internal {
        _push(
            _obj(
                "schedule_control",
                string.concat('"bytes":"', vm.toString(payload), '","at":', _u(at))
            )
        );
    }

    function _scheduleAsset(bytes32 transferId, uint256 amount, uint64 at) internal {
        _push(
            _obj(
                "schedule_asset",
                string.concat(
                    '"transfer":"',
                    vm.toString(transferId),
                    '","amount":',
                    _u(amount),
                    ',"at":',
                    _u(at)
                )
            )
        );
    }

    function _idle() internal {
        _push('{"op":"run_until_idle"}');
    }

    function _deliverReady() internal {
        _push('{"op":"deliver_ready"}');
    }

    function _advance(uint64 tick) internal {
        _push(_obj("advance_to", string.concat('"tick":', _u(tick))));
    }

    function _applyControl() internal {
        _push('{"op":"apply_control"}');
    }

    function _applyAsset() internal {
        _push('{"op":"apply_asset"}');
    }

    function _attribute(bytes32 transferId) internal {
        _push(_obj("attribute", string.concat('"transfer":"', vm.toString(transferId), '"')));
    }

    function _deploy(uint256 amount) internal {
        _push(_obj("deploy", string.concat('"amount":', _u(amount))));
    }

    function _configureAdapter(uint256 maxLiquid, uint256 lossBps) internal {
        _push(
            _obj(
                "configure_adapter",
                string.concat('"max_liquid":', _u(maxLiquid), ',"loss_bps":', _u(lossBps))
            )
        );
    }

    function _withdraw(bytes32 transferId, uint256 amount) internal {
        _push(
            _obj(
                "withdraw",
                string.concat('"transfer":"', vm.toString(transferId), '","amount":', _u(amount))
            )
        );
    }

    function _send(bytes32 transferId, uint256 amount, uint64 at) internal {
        _push(
            _obj(
                "send_recall",
                string.concat(
                    '"transfer":"',
                    vm.toString(transferId),
                    '","amount":',
                    _u(amount),
                    ',"at":',
                    _u(at)
                )
            )
        );
    }

    function _publishReport(uint64 epoch, uint64 at) internal {
        _push(_obj("publish_report", string.concat('"epoch":', _u(epoch), ',"at":', _u(at))));
    }

    function _publishRecallSent(bytes32 transferId, uint64 at) internal {
        _push(
            _obj(
                "publish_recall_sent",
                string.concat('"transfer":"', vm.toString(transferId), '","at":', _u(at))
            )
        );
    }

    function _sync() internal {
        _push(_obj("warp", string.concat('"ts":', _u(block.timestamp))));
    }

    function _obj(string memory op, string memory fields) internal pure returns (string memory) {
        return string.concat('{"op":"', op, '",', fields, "}");
    }

    function _u(uint256 value) internal pure returns (string memory) {
        return vm.toString(value);
    }

    function _push(string memory op) internal {
        script.push(op);
    }

    function _leg() internal returns (Leg memory view_) {
        string memory json = _exec();
        view_.principal = vm.parseJsonUint(json, ".principal");
        view_.deployed = vm.parseJsonUint(json, ".deployed");
        view_.custody = vm.parseJsonUint(json, ".custody");
        view_.unattributed = vm.parseJsonUint(json, ".unattributed");
        view_.recalled = vm.parseJsonUint(json, ".recalled");
        view_.realizedLoss = vm.parseJsonUint(json, ".realized_loss");
        view_.laneSequence = uint64(vm.parseJsonUint(json, ".lane_sequence"));
        view_.outboundNext = uint64(vm.parseJsonUint(json, ".outbound_next"));
        view_.inboundAsset = vm.parseJsonUint(json, ".inbound_asset");
        view_.impaired = vm.parseJsonUint(json, ".impaired_outstanding");
        view_.lastEventId = vm.parseJsonUint(json, ".last_event_id");
        view_.transferOpen = vm.parseJsonBool(json, ".transfer_open");
        view_.frozen = vm.parseJsonBool(json, ".frozen");
        view_.lastAccepted = vm.parseJsonBool(json, ".last_accepted");
        view_.lastError = vm.parseJsonString(json, ".last_error");
        view_.latestReport = _bytesField(json, ".latest_report");
        view_.latestRecallSent = _bytesField(json, ".latest_recall_sent");
        view_.latestSettlement = _bytesField(json, ".latest_settlement");
        view_.reports = vm.parseJsonStringArray(json, ".reports");
        view_.recallSents = vm.parseJsonStringArray(json, ".recall_sents");
        view_.settlements = vm.parseJsonStringArray(json, ".settlements");
    }

    function _bytesField(string memory json, string memory key)
        internal
        pure
        returns (bytes memory payload)
    {
        string memory hexValue = vm.parseJsonString(json, key);
        if (bytes(hexValue).length > 2) {
            payload = vm.parseBytes(hexValue);
        }
    }

    function _exec() internal returns (string memory json) {
        string memory body = '{"ops":[';
        for (uint256 i; i < script.length; ++i) {
            if (i != 0) body = string.concat(body, ",");
            body = string.concat(body, script[i]);
        }
        body = string.concat(body, "]}");
        string[] memory argv = new string[](3);
        argv[0] = "python3";
        argv[1] = string.concat(vm.projectRoot(), "/test/ffi/e2e_ffi.py");
        argv[2] = body;
        json = string(vm.ffi(argv));
    }
}
