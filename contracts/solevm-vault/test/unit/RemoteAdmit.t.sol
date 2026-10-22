// SPDX-License-Identifier: Apache-2.0
pragma solidity 0.8.30;

import {RemoteAccounting} from "../../src/RemoteAccounting.sol";
import {IRemoteAccounting} from "../../src/interfaces/IRemoteAccounting.sol";
import {ProtocolCodec} from "../../src/libraries/ProtocolCodec.sol";
import {RemoteTestBase} from "../RemoteTestBase.sol";

/// Admission asks the leg to run the active transfer and moves no value here.
contract RemoteAdmitTest is RemoteTestBase {
    bytes32 internal constant TID_1 = bytes32(uint256(1));
    bytes32 internal constant TID_2 = bytes32(uint256(2));
    uint256 internal constant TYPE_OFF = 8;
    uint256 internal constant SEQUENCE_OFF = 151;
    uint256 internal constant PREV_COMMIT_OFF = 159;
    uint256 internal constant BODY_OFF = 247;

    function test_admit_names_the_active_allocation_and_moves_no_value() public {
        _bootstrap(alice, 100e6);
        _allocate(TID_1, 40e6);
        uint256 nav = _managed();
        uint256 idle = vault.idleBacking();
        uint256 outbound = vault.outboundInTransit();
        bytes32 previous = coordinator.outboundLatestCommitment();

        vm.expectEmit(true, true, false, false, address(coordinator));
        emit IRemoteAccounting.CanonicalMessagePublished(ProtocolCodec.TYPE_RESOLVE, 2, "");
        vm.expectEmit(true, false, false, false, address(coordinator));
        emit IRemoteAccounting.AdmissionRequested(TID_1, 1, "");
        bytes memory message = _admit(TID_1);

        assertEq(message.length, ProtocolCodec.RESOLVE_MESSAGE_LEN);
        assertEq(uint8(message[TYPE_OFF]), ProtocolCodec.TYPE_RESOLVE);
        assertEq(_word64(message, SEQUENCE_OFF), 2);
        assertEq(_word32(message, PREV_COMMIT_OFF), previous);
        assertEq(_word32(message, BODY_OFF), TID_1);
        assertEq(_word64(message, BODY_OFF + 32), 1);
        assertEq(uint8(message[BODY_OFF + 40]), ProtocolCodec.RESOLVE_ADMIT);

        assertEq(coordinator.outboundNextSequence(), 3);
        assertEq(
            coordinator.outboundLatestCommitment(),
            ProtocolCodec.nextCommitment(previous, ProtocolCodec.messageId(message))
        );
        assertEq(coordinator.activeTransferId(), TID_1);
        assertEq(coordinator.activeSequence(), 1);
        assertEq(coordinator.activeAmount(), 40e6);
        assertTrue(coordinator.hasActiveAllocate());
        assertEq(_managed(), nav);
        assertEq(vault.idleBacking(), idle);
        assertEq(vault.outboundInTransit(), outbound);
    }

    function test_an_admitted_allocation_completes_through_the_ordinary_report() public {
        _bootstrap(alice, 100e6);
        _allocate(TID_1, 40e6);
        uint256 nav = _managed();
        _admit(TID_1);

        _report(1, 40e6, 40e6, 0, TID_1);
        assertFalse(coordinator.hasActiveAllocate());
        assertEq(coordinator.activeSequence(), 0);
        assertEq(vault.outboundInTransit(), 0);
        assertEq(vault.acceptedRemoteValue(), 40e6);
        assertEq(_managed(), nav);
    }

    function test_admit_names_the_sequence_that_carried_the_recall() public {
        _bootstrap(alice, 100e6);
        _allocate(TID_1, 40e6);
        _report(1, 40e6, 40e6, 0, TID_1);
        _recall(TID_2, 40e6);
        assertEq(coordinator.activeSequence(), 2);

        bytes memory message = _admit(TID_2);
        assertEq(_word64(message, SEQUENCE_OFF), 3);
        assertEq(_word32(message, BODY_OFF), TID_2);
        assertEq(_word64(message, BODY_OFF + 32), 2);
        assertTrue(coordinator.hasActiveRecall());
        assertEq(coordinator.activeSequence(), 2);
    }

    function test_only_the_admin_may_admit() public {
        _bootstrap(alice, 100e6);
        _allocate(TID_1, 40e6);
        (uint64 observedAt, uint64 publishedAt, uint64 expiresAt,) = _times();
        vm.prank(alice);
        vm.expectRevert(abi.encodeWithSelector(RemoteAccounting.Unauthorized.selector, alice));
        coordinator.admit(TID_1, observedAt, publishedAt, expiresAt);
    }

    function test_admit_needs_an_active_transfer() public {
        _bootstrap(alice, 100e6);
        _allocate(TID_1, 40e6);
        _report(1, 40e6, 40e6, 0, TID_1);
        (uint64 observedAt, uint64 publishedAt, uint64 expiresAt,) = _times();
        vm.prank(admin);
        vm.expectRevert(RemoteAccounting.NoActiveTransfer.selector);
        coordinator.admit(TID_1, observedAt, publishedAt, expiresAt);
    }

    function test_admit_refuses_any_transfer_but_the_active_one() public {
        _bootstrap(alice, 100e6);
        _allocate(TID_1, 40e6);
        (uint64 observedAt, uint64 publishedAt, uint64 expiresAt,) = _times();
        vm.prank(admin);
        vm.expectRevert(
            abi.encodeWithSelector(RemoteAccounting.WrongTransferId.selector, TID_1, TID_2)
        );
        coordinator.admit(TID_2, observedAt, publishedAt, expiresAt);
        assertEq(coordinator.outboundNextSequence(), 2);
    }

    function test_a_frozen_vault_admits_a_recall_but_not_an_allocation() public {
        _bootstrap(alice, 100e6);
        _allocate(TID_1, 40e6);
        vm.prank(admin);
        vault.freeze();
        (uint64 observedAt, uint64 publishedAt, uint64 expiresAt,) = _times();
        vm.prank(admin);
        vm.expectRevert(RemoteAccounting.AllocateNotAllowed.selector);
        coordinator.admit(TID_1, observedAt, publishedAt, expiresAt);
        assertEq(coordinator.outboundNextSequence(), 2);

        _report(1, 40e6, 40e6, 0, TID_1);
        _recall(TID_2, 40e6);
        bytes memory message = _admit(TID_2);
        assertEq(_word64(message, BODY_OFF + 32), 2);
        assertEq(coordinator.outboundNextSequence(), 4);
    }

    function _admit(bytes32 transferId) internal returns (bytes memory message) {
        (uint64 observedAt, uint64 publishedAt, uint64 expiresAt,) = _times();
        vm.prank(admin);
        message = coordinator.admit(transferId, observedAt, publishedAt, expiresAt);
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
