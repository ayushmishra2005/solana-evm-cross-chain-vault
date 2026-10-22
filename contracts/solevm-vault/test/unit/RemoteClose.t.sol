// SPDX-License-Identifier: Apache-2.0
pragma solidity 0.8.30;

import {RemoteAccounting} from "../../src/RemoteAccounting.sol";
import {SolEVMVault} from "../../src/SolEVMVault.sol";
import {IRemoteAccounting} from "../../src/interfaces/IRemoteAccounting.sol";
import {EpochPhase, ISolEVMVault} from "../../src/interfaces/ISolEVMVault.sol";
import {ProtocolCodec} from "../../src/libraries/ProtocolCodec.sol";
import {RemoteTestBase} from "../RemoteTestBase.sol";

/// A close settles an allocation by what arrived and impairs the rest outside
/// NAV. A recovery returns pooled impairment to principal at par.
contract RemoteCloseTest is RemoteTestBase {
    bytes32 internal constant TID_1 = bytes32(uint256(1));
    bytes32 internal constant TID_2 = bytes32(uint256(2));
    bytes32 internal constant RID_1 = bytes32(uint256(0x51));
    bytes32 internal constant RID_2 = bytes32(uint256(0x52));
    uint256 internal constant TYPE_OFF = 8;
    uint256 internal constant SEQUENCE_OFF = 151;
    uint256 internal constant PREV_COMMIT_OFF = 159;
    uint256 internal constant BODY_OFF = 247;

    // Closing

    function test_close_waits_until_the_deadline_has_passed() public {
        _bootstrap(alice, 100e6);
        _allocate(TID_1, 40e6);
        uint64 deadline = coordinator.activeDeadline();
        assertEq(deadline, uint64(block.timestamp) + 1 days);

        _expectClose(
            TID_1,
            abi.encodeWithSelector(
                RemoteAccounting.DeadlineNotPassed.selector, deadline, uint64(block.timestamp)
            )
        );
        vm.warp(deadline);
        _expectClose(
            TID_1,
            abi.encodeWithSelector(RemoteAccounting.DeadlineNotPassed.selector, deadline, deadline)
        );

        vm.warp(uint256(deadline) + 1);
        _close(TID_1);
        assertTrue(coordinator.closeRequested());
    }

    function test_only_the_admin_may_close() public {
        _bootstrap(alice, 100e6);
        _allocate(TID_1, 40e6);
        _pastDeadline();
        (uint64 observedAt, uint64 publishedAt, uint64 expiresAt,) = _times();
        vm.prank(alice);
        vm.expectRevert(abi.encodeWithSelector(RemoteAccounting.Unauthorized.selector, alice));
        coordinator.close(TID_1, observedAt, publishedAt, expiresAt);
    }

    function test_close_names_only_the_active_allocation() public {
        _bootstrap(alice, 100e6);
        _expectClose(TID_1, abi.encodeWithSelector(RemoteAccounting.NoActiveTransfer.selector));

        _allocate(TID_1, 40e6);
        _pastDeadline();
        _expectClose(
            TID_2, abi.encodeWithSelector(RemoteAccounting.WrongTransferId.selector, TID_1, TID_2)
        );

        _settleClose(TID_1, 40e6, 0);
        _recall(TID_2, 10e6);
        _pastDeadline();
        _expectClose(TID_2, abi.encodeWithSelector(RemoteAccounting.NoActiveTransfer.selector));
    }

    function test_close_publishes_one_resolution_and_moves_no_value() public {
        _bootstrap(alice, 100e6);
        _allocate(TID_1, 40e6);
        _pastDeadline();
        uint256 nav = _managed();
        uint256 outbound = vault.outboundInTransit();
        bytes32 previous = coordinator.outboundLatestCommitment();

        vm.expectEmit(true, true, false, false, address(coordinator));
        emit IRemoteAccounting.CanonicalMessagePublished(ProtocolCodec.TYPE_RESOLVE, 2, "");
        vm.expectEmit(true, false, false, false, address(coordinator));
        emit IRemoteAccounting.CloseRequested(TID_1, 1, "");
        bytes memory message = _close(TID_1);

        assertEq(message.length, ProtocolCodec.RESOLVE_MESSAGE_LEN);
        assertEq(uint8(message[TYPE_OFF]), ProtocolCodec.TYPE_RESOLVE);
        assertEq(_word64(message, SEQUENCE_OFF), 2);
        assertEq(_word32(message, PREV_COMMIT_OFF), previous);
        assertEq(_word32(message, BODY_OFF), TID_1);
        assertEq(_word64(message, BODY_OFF + 32), 1);
        assertEq(uint8(message[BODY_OFF + 40]), ProtocolCodec.RESOLVE_CLOSE);
        assertEq(
            coordinator.outboundLatestCommitment(),
            ProtocolCodec.nextCommitment(previous, ProtocolCodec.messageId(message))
        );

        assertTrue(coordinator.hasActiveAllocate());
        assertEq(_managed(), nav);
        assertEq(vault.outboundInTransit(), outbound);
        assertEq(vault.impairedTransport(), 0);

        _expectClose(TID_1, abi.encodeWithSelector(RemoteAccounting.CloseAlreadyRequested.selector));
        (uint64 observedAt, uint64 publishedAt, uint64 expiresAt,) = _times();
        vm.prank(admin);
        vm.expectRevert(RemoteAccounting.CloseAlreadyRequested.selector);
        coordinator.admit(TID_1, observedAt, publishedAt, expiresAt);
    }

    /// The settlement is the one path for a closing allocation: a report that
    /// names it as completed still only advances the lane.
    function test_reports_skip_but_advance_the_lane_once_while_a_close_is_pending() public {
        _bootstrap(alice, 100e6);
        _allocate(TID_1, 40e6);
        _pastDeadline();
        _close(TID_1);
        uint256 nav = _managed();

        for (uint256 i; i < 2; ++i) {
            uint64 sequence = coordinator.inboundExpectedSequence();
            bytes32 previous = coordinator.inboundLatestCommitment();
            bytes memory encoded = _encodeReport(1, 40e6, 40e6, 0, TID_1);
            _expectSkipped(1, IRemoteAccounting.ReportSkipReason.SettlementPending);
            coordinator.processRemoteReport(encoded);
            assertEq(coordinator.inboundExpectedSequence(), sequence + 1);
            assertEq(
                coordinator.inboundLatestCommitment(),
                ProtocolCodec.nextCommitment(previous, ProtocolCodec.messageId(encoded))
            );
        }

        assertTrue(coordinator.hasActiveAllocate());
        assertEq(vault.outboundInTransit(), 40e6);
        assertEq(vault.remotePrincipalCap(), 0);
        assertEq(_managed(), nav);
    }

    /// Worked example: 40 allocated, 25 arrived. NAV falls by exactly the 15
    /// shortfall, which waits outside NAV and is not recognized loss.
    function test_a_short_close_impairs_only_the_shortfall_outside_nav() public {
        _bootstrap(alice, 100e6);
        _allocate(TID_1, 40e6);
        _pastDeadline();
        _close(TID_1);
        uint256 nav = _managed();
        uint64 sequence = coordinator.inboundExpectedSequence();

        vm.expectEmit(address(coordinator));
        emit IRemoteAccounting.AllocationClosed(TID_1, 40e6, 25e6, 15e6);
        _settleClose(TID_1, 25e6, 15e6);

        assertEq(vault.outboundInTransit(), 0);
        assertEq(vault.remotePrincipalCap(), 25e6);
        assertEq(vault.acceptedRemoteValue(), 25e6);
        assertEq(vault.impairedTransport(), 15e6);
        assertEq(vault.recognizedLoss(), 0);
        assertEq(_managed(), nav - 15e6);
        assertEq(vault.managedBacking(), nav - 15e6);
        assertEq(coordinator.inboundExpectedSequence(), sequence + 1);
        assertEq(coordinator.lastImpairedSequence(), 1);
        assertFalse(coordinator.hasActiveAllocate());
        assertFalse(coordinator.closeRequested());
        assertEq(coordinator.activeDeadline(), 0);
        _assertSolvent();

        _report(1, 25e6, 25e6, 0, bytes32(0));
        assertEq(vault.acceptedRemoteValue(), 25e6);
        assertEq(_managed(), nav - 15e6);
    }

    function test_a_full_close_settles_exactly_like_a_completion() public {
        _bootstrap(alice, 100e6);
        _allocate(TID_1, 40e6);
        _pastDeadline();
        _close(TID_1);
        uint256 nav = _managed();

        _settleClose(TID_1, 40e6, 0);
        assertEq(vault.outboundInTransit(), 0);
        assertEq(vault.remotePrincipalCap(), 40e6);
        assertEq(vault.acceptedRemoteValue(), 40e6);
        assertEq(vault.impairedTransport(), 0);
        assertEq(coordinator.lastImpairedSequence(), 0);
        assertEq(_managed(), nav);
    }

    function test_a_closed_transfer_id_stays_spent() public {
        _bootstrap(alice, 100e6);
        _allocate(TID_1, 40e6);
        _pastDeadline();
        _close(TID_1);
        _settleClose(TID_1, 25e6, 15e6);

        (uint64 observedAt, uint64 publishedAt, uint64 expiresAt, uint64 deadline) = _times();
        vm.prank(admin);
        vm.expectRevert(abi.encodeWithSelector(RemoteAccounting.TransferIdReused.selector, TID_1));
        coordinator.allocate(TID_1, 10e6, 10e6, deadline, observedAt, publishedAt, expiresAt);
    }

    function test_settlement_mismatches_revert_without_advancing_the_lane() public {
        _bootstrap(alice, 100e6);
        _allocate(TID_1, 40e6);
        uint64 sequence = coordinator.inboundExpectedSequence();

        _expectSettlementRevert(
            _encodeSettlement(TID_1, ProtocolCodec.SETTLEMENT_CLOSED, 40e6, 25e6, 15e6),
            abi.encodeWithSelector(RemoteAccounting.NoPendingSettlement.selector)
        );

        _pastDeadline();
        _close(TID_1);
        _expectSettlementRevert(
            _encodeSettlement(TID_1, ProtocolCodec.SETTLEMENT_CLOSED, 39e6, 25e6, 14e6),
            abi.encodeWithSelector(RemoteAccounting.SettlementMismatch.selector)
        );
        _expectSettlementRevert(
            _encodeSettlement(TID_1, ProtocolCodec.SETTLEMENT_CLOSED, 40e6, 25e6, 15e6 + 1),
            abi.encodeWithSelector(RemoteAccounting.SettlementMismatch.selector)
        );
        _expectSettlementRevert(
            _encodeSettlement(TID_2, ProtocolCodec.SETTLEMENT_CLOSED, 40e6, 25e6, 15e6),
            abi.encodeWithSelector(RemoteAccounting.WrongTransferId.selector, TID_1, TID_2)
        );
        _expectSettlementRevert(
            _encodeSettlement(TID_1, ProtocolCodec.SETTLEMENT_RECOVERED, 40e6, 25e6, 15e6),
            abi.encodeWithSelector(RemoteAccounting.NoPendingSettlement.selector)
        );

        ProtocolCodec.Header memory header = _inboundHeader();
        header.sequence += 1;
        _expectSettlementRevert(
            ProtocolCodec.encodeTransportSettlement(
                header, _settlementBody(TID_1, ProtocolCodec.SETTLEMENT_CLOSED, 40e6, 25e6, 15e6)
            ),
            abi.encodeWithSelector(RemoteAccounting.WrongSequence.selector, sequence, sequence + 1)
        );

        ProtocolCodec.TransportSettlementBody memory body =
            _settlementBody(TID_1, ProtocolCodec.SETTLEMENT_CLOSED, 40e6, 25e6, 15e6);
        body.configVersion += 1;
        _expectSettlementRevert(
            ProtocolCodec.encodeTransportSettlement(_inboundHeader(), body),
            abi.encodeWithSelector(
                RemoteAccounting.WrongConfigVersion.selector,
                body.configVersion - 1,
                body.configVersion
            )
        );

        bytes memory valid =
            _encodeSettlement(TID_1, ProtocolCodec.SETTLEMENT_CLOSED, 40e6, 25e6, 15e6);
        authenticator.setApproveAll(false);
        _expectSettlementRevert(
            valid, abi.encodeWithSelector(RemoteAccounting.UnauthenticatedMessage.selector)
        );
        authenticator.setApproveAll(true);

        assertEq(coordinator.inboundExpectedSequence(), sequence);
        assertEq(vault.outboundInTransit(), 40e6);
        coordinator.processTransportSettlement(valid);
        assertEq(vault.impairedTransport(), 15e6);

        vm.expectRevert(abi.encodeWithSelector(RemoteAccounting.Replay.selector, sequence));
        coordinator.processTransportSettlement(valid);
    }

    /// A terminal notice binds even after its expiry, like RecallSent.
    function test_an_expired_settlement_still_applies() public {
        _bootstrap(alice, 100e6);
        _allocate(TID_1, 40e6);
        _pastDeadline();
        _close(TID_1);
        bytes memory encoded =
            _encodeSettlement(TID_1, ProtocolCodec.SETTLEMENT_CLOSED, 40e6, 25e6, 15e6);
        vm.warp(block.timestamp + 30 days);

        coordinator.processTransportSettlement(encoded);
        assertEq(vault.impairedTransport(), 15e6);
    }

    // Recovering

    function test_recover_needs_impairment_a_fresh_id_and_no_open_cycle() public {
        _bootstrap(alice, 100e6);
        _expectRecover(RID_1, abi.encodeWithSelector(RemoteAccounting.NothingToRecover.selector));

        _allocate(TID_1, 40e6);
        _expectRecover(
            RID_1, abi.encodeWithSelector(RemoteAccounting.ActiveTransferExists.selector, TID_1)
        );

        _pastDeadline();
        _close(TID_1);
        _settleClose(TID_1, 25e6, 15e6);
        _expectRecover(
            TID_1, abi.encodeWithSelector(RemoteAccounting.TransferIdReused.selector, TID_1)
        );
        _expectRecover(bytes32(0), abi.encodeWithSelector(RemoteAccounting.ZeroTransferId.selector));

        (uint64 observedAt, uint64 publishedAt, uint64 expiresAt,) = _times();
        vm.prank(alice);
        vm.expectRevert(abi.encodeWithSelector(RemoteAccounting.Unauthorized.selector, alice));
        coordinator.recover(RID_1, observedAt, publishedAt, expiresAt);
    }

    function test_recover_binds_to_the_last_impaired_allocation_and_blocks_other_cycles() public {
        _closedShort();
        uint256 nav = _managed();
        bytes32 previous = coordinator.outboundLatestCommitment();

        vm.expectEmit(true, false, false, false, address(coordinator));
        emit IRemoteAccounting.RecoveryRequested(RID_1, 1, "");
        bytes memory message = _recover(RID_1);

        assertEq(uint8(message[TYPE_OFF]), ProtocolCodec.TYPE_RESOLVE);
        assertEq(_word64(message, SEQUENCE_OFF), 3);
        assertEq(_word32(message, PREV_COMMIT_OFF), previous);
        assertEq(_word32(message, BODY_OFF), RID_1);
        assertEq(_word64(message, BODY_OFF + 32), 1);
        assertEq(uint8(message[BODY_OFF + 40]), ProtocolCodec.RESOLVE_RECOVER);
        assertEq(uint8(coordinator.activeCycleKind()), uint8(IRemoteAccounting.CycleKind.Recover));
        assertEq(coordinator.activeTransferId(), RID_1);
        assertEq(coordinator.activeSequence(), 3);
        assertTrue(coordinator.usedTransferIds(RID_1));
        assertEq(_managed(), nav);

        (uint64 observedAt, uint64 publishedAt, uint64 expiresAt, uint64 deadline) = _times();
        vm.startPrank(admin);
        vm.expectRevert(
            abi.encodeWithSelector(RemoteAccounting.ActiveTransferExists.selector, RID_1)
        );
        coordinator.allocate(TID_2, 1e6, 1e6, deadline, observedAt, publishedAt, expiresAt);
        vm.expectRevert(
            abi.encodeWithSelector(RemoteAccounting.ActiveTransferExists.selector, RID_1)
        );
        coordinator.recall(TID_2, 1e6, 1, deadline, observedAt, publishedAt, expiresAt);
        vm.expectRevert(RemoteAccounting.NoActiveTransfer.selector);
        coordinator.admit(RID_1, observedAt, publishedAt, expiresAt);
        vm.expectRevert(RemoteAccounting.NoActiveTransfer.selector);
        coordinator.close(RID_1, observedAt, publishedAt, expiresAt);
        vm.stopPrank();
    }

    function test_a_recovery_returns_impairment_to_principal_at_par() public {
        _closedShort();
        _recover(RID_1);
        uint256 nav = _managed();

        vm.expectEmit(address(coordinator));
        emit IRemoteAccounting.ImpairmentRecovered(RID_1, 15e6, 10e6, 5e6);
        coordinator.processTransportSettlement(
            _encodeSettlement(RID_1, ProtocolCodec.SETTLEMENT_RECOVERED, 15e6, 10e6, 5e6)
        );

        assertEq(vault.impairedTransport(), 5e6);
        assertEq(vault.remotePrincipalCap(), 35e6);
        assertEq(vault.acceptedRemoteValue(), 35e6);
        assertEq(vault.recognizedLoss(), 0);
        assertEq(_managed(), nav + 10e6);
        assertEq(uint8(coordinator.activeCycleKind()), uint8(IRemoteAccounting.CycleKind.None));
        _assertSolvent();

        _recover(RID_2);
        assertEq(coordinator.lastImpairedSequence(), 1);
        coordinator.processTransportSettlement(
            _encodeSettlement(RID_2, ProtocolCodec.SETTLEMENT_RECOVERED, 5e6, 0, 5e6)
        );
        assertEq(vault.impairedTransport(), 5e6);
        assertEq(_managed(), nav + 10e6);
    }

    function test_recovery_mismatches_revert_without_advancing_the_lane() public {
        _closedShort();
        _recover(RID_1);
        uint64 sequence = coordinator.inboundExpectedSequence();

        _expectSettlementRevert(
            _encodeSettlement(RID_1, ProtocolCodec.SETTLEMENT_RECOVERED, 15e6 + 1, 0, 15e6 + 1),
            abi.encodeWithSelector(RemoteAccounting.SettlementMismatch.selector)
        );
        _expectSettlementRevert(
            _encodeSettlement(RID_1, ProtocolCodec.SETTLEMENT_CLOSED, 15e6, 0, 15e6),
            abi.encodeWithSelector(RemoteAccounting.NoPendingSettlement.selector)
        );
        assertEq(coordinator.inboundExpectedSequence(), sequence);
        assertEq(vault.impairedTransport(), 15e6);
    }

    function test_reports_still_price_during_a_recovery() public {
        _closedShort();
        _recover(RID_1);
        _report(1, 25e6, 20e6, 5e6, bytes32(0));
        assertEq(vault.acceptedRemoteValue(), 20e6);
        assertEq(uint8(coordinator.activeCycleKind()), uint8(IRemoteAccounting.CycleKind.Recover));
    }

    // Pricing

    /// The locked snapshot prices the epoch; a later close moves NAV only for
    /// the next one.
    function test_a_close_after_the_valuation_lock_leaves_the_locked_price_alone() public {
        _bootstrap(alice, 100e6);
        _allocate(TID_1, 40e6);
        vm.warp(vault.currentEpochCutoffAt());
        vault.cutoffEpoch();
        uint64 epochId = vault.currentEpochId();
        _report(epochId, 0, 0, 0, bytes32(0));
        assertTrue(vault.epochRemoteValuationLocked(epochId));
        uint256 locked = vault.lockedPricingAssets(epochId);
        assertEq(locked, 100e6);

        vm.warp(uint256(coordinator.activeDeadline()) + 1);
        _close(TID_1);
        _settleClose(TID_1, 25e6, 15e6);
        assertEq(vault.lockedPricingAssets(epochId), locked);
        assertEq(vault.managedBacking(), 85e6);
    }

    /// Before the lock, a pending close holds pricing back until the leg says
    /// what arrived, so the epoch never prices the unsettled allocation.
    function test_a_close_before_the_lock_defers_pricing_until_its_settlement() public {
        _bootstrap(alice, 100e6);
        _allocate(TID_1, 40e6);
        vm.warp(uint256(coordinator.activeDeadline()) + 1);
        _close(TID_1);
        if (vault.currentEpochPhase() == EpochPhase.Open) {
            vm.warp(
                vault.currentEpochCutoffAt() > block.timestamp
                    ? vault.currentEpochCutoffAt()
                    : block.timestamp
            );
            vault.cutoffEpoch();
        }
        uint64 epochId = vault.currentEpochId();

        bytes memory early = _encodeReport(epochId, 0, 0, 0, bytes32(0));
        _expectSkipped(epochId, IRemoteAccounting.ReportSkipReason.SettlementPending);
        coordinator.processRemoteReport(early);
        assertFalse(vault.epochRemoteValuationLocked(epochId));

        _settleClose(TID_1, 25e6, 15e6);
        _report(epochId, 25e6, 25e6, 0, bytes32(0));
        assertTrue(vault.epochRemoteValuationLocked(epochId));
        assertEq(vault.lockedPricingAssets(epochId), 85e6);
    }

    // Vault entry points

    function test_only_the_coordinator_moves_transport_buckets() public {
        vm.expectRevert(abi.encodeWithSelector(ISolEVMVault.Unauthorized.selector, alice));
        vm.prank(alice);
        vault.closeAllocation(1, 0);
        vm.expectRevert(abi.encodeWithSelector(ISolEVMVault.Unauthorized.selector, alice));
        vm.prank(alice);
        vault.recoverImpairment(0);
    }

    function test_the_vault_refuses_impossible_transport_amounts() public {
        _bootstrap(alice, 100e6);
        _allocate(TID_1, 40e6);
        vm.startPrank(address(coordinator));
        vm.expectRevert(SolEVMVault.InvalidAssetAttribution.selector);
        vault.closeAllocation(40e6, 40e6 + 1);
        vm.expectRevert(SolEVMVault.InvalidAssetAttribution.selector);
        vault.closeAllocation(40e6 + 1, 0);
        vm.expectRevert(ISolEVMVault.ZeroAmount.selector);
        vault.closeAllocation(0, 0);
        vm.expectRevert(SolEVMVault.InvalidAssetAttribution.selector);
        vault.recoverImpairment(1);
        vm.stopPrank();
    }

    // Helpers

    function _closedShort() internal {
        _bootstrap(alice, 100e6);
        _allocate(TID_1, 40e6);
        _pastDeadline();
        _close(TID_1);
        _settleClose(TID_1, 25e6, 15e6);
    }

    function _pastDeadline() internal {
        vm.warp(uint256(coordinator.activeDeadline()) + 1);
    }

    function _close(bytes32 transferId) internal returns (bytes memory message) {
        (uint64 observedAt, uint64 publishedAt, uint64 expiresAt,) = _times();
        vm.prank(admin);
        message = coordinator.close(transferId, observedAt, publishedAt, expiresAt);
    }

    function _recover(bytes32 recoveryId) internal returns (bytes memory message) {
        (uint64 observedAt, uint64 publishedAt, uint64 expiresAt,) = _times();
        vm.prank(admin);
        message = coordinator.recover(recoveryId, observedAt, publishedAt, expiresAt);
    }

    function _expectClose(bytes32 transferId, bytes memory reason) internal {
        (uint64 observedAt, uint64 publishedAt, uint64 expiresAt,) = _times();
        vm.prank(admin);
        vm.expectRevert(reason);
        coordinator.close(transferId, observedAt, publishedAt, expiresAt);
    }

    function _expectRecover(bytes32 recoveryId, bytes memory reason) internal {
        (uint64 observedAt, uint64 publishedAt, uint64 expiresAt,) = _times();
        vm.prank(admin);
        vm.expectRevert(reason);
        coordinator.recover(recoveryId, observedAt, publishedAt, expiresAt);
    }

    function _settleClose(bytes32 transferId, uint128 credited, uint128 impaired) internal {
        uint128 claim = uint128(coordinator.activeAmount());
        if (!coordinator.closeRequested()) _close(transferId);
        coordinator.processTransportSettlement(
            _encodeSettlement(
                transferId, ProtocolCodec.SETTLEMENT_CLOSED, claim, credited, impaired
            )
        );
    }

    function _expectSettlementRevert(bytes memory encoded, bytes memory reason) internal {
        vm.expectRevert(reason);
        coordinator.processTransportSettlement(encoded);
    }

    function _settlementBody(
        bytes32 transferId,
        uint8 kind,
        uint128 claim,
        uint128 credited,
        uint128 impaired
    ) internal view returns (ProtocolCodec.TransportSettlementBody memory) {
        return ProtocolCodec.TransportSettlementBody({
                transferId: transferId,
                kind: kind,
                claimAmount: claim,
                creditedAmount: credited,
                impairedOutstanding: impaired,
                configVersion: uint64(vault.configVersion())
            });
    }

    function _encodeSettlement(
        bytes32 transferId,
        uint8 kind,
        uint128 claim,
        uint128 credited,
        uint128 impaired
    ) internal view returns (bytes memory) {
        return ProtocolCodec.encodeTransportSettlement(
            _inboundHeader(), _settlementBody(transferId, kind, claim, credited, impaired)
        );
    }

    function _word64(bytes memory encoded, uint256 offset) internal pure returns (uint64 value) {
        value = uint64(bytes8(_word32(encoded, offset)));
    }

    function _word32(bytes memory encoded, uint256 offset) internal pure returns (bytes32 value) {
        assembly {
            value := mload(add(add(encoded, 32), offset))
        }
    }
}
