// SPDX-License-Identifier: Apache-2.0
pragma solidity 0.8.30;

import {IRemoteAccounting} from "../../src/interfaces/IRemoteAccounting.sol";
import {ProtocolCodec} from "../../src/libraries/ProtocolCodec.sol";
import {RemoteTestBase} from "../RemoteTestBase.sol";

/// Emits live EVM control bytes that the Solana fixture identities can accept.
contract ControlLaneInteropTest is RemoteTestBase {
    uint32 internal constant SOLANA_SOURCE_CHAIN = 8453;
    uint32 internal constant SOLANA_DEST_CHAIN = 900;
    uint32 internal constant CONTROL_LANE = 1;
    uint256 internal constant SEQUENCE_OFF = 151;
    uint256 internal constant PREV_COMMIT_OFF = 159;
    uint256 internal constant BODY_OFF = 247;

    bytes32 internal constant ALLOCATE_TID =
        hex"7777777777777777777777777777777777777777777777777777777777777777";
    bytes32 internal constant RECALL_TID =
        hex"8888888888888888888888888888888888888888888888888888888888888888";
    bytes32 internal constant RECOVERY_TID =
        hex"9999999999999999999999999999999999999999999999999999999999999999";

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

    function test_evm_allocate_then_recall_share_one_control_commitment_chain() public {
        _bootstrap(alice, 10e6);
        vm.warp(1_700_000_000);

        bytes memory allocate = _allocate(ALLOCATE_TID, 1e6);
        _report(1, 1e6, 1e6, 0, ALLOCATE_TID);
        bytes memory recall = _recall(RECALL_TID, 1e6);

        assertEq(allocate.length, 343);
        assertEq(recall.length, 327);
        assertEq(_word64(allocate, SEQUENCE_OFF), 1);
        assertEq(_word32(allocate, PREV_COMMIT_OFF), bytes32(0));
        assertEq(_word64(recall, SEQUENCE_OFF), 2);
        assertEq(
            _word32(recall, PREV_COMMIT_OFF),
            ProtocolCodec.nextCommitment(bytes32(0), ProtocolCodec.messageId(allocate))
        );

        string memory root = string.concat(vm.projectRoot(), "/testdata/control-lane/");
        assertEq(allocate, vm.parseBytes(vm.readFile(string.concat(root, "evm-allocate.hex"))));
        assertEq(recall, vm.parseBytes(vm.readFile(string.concat(root, "evm-recall.hex"))));
    }

    function test_evm_admit_follows_its_allocation_on_the_same_commitment_chain() public {
        _bootstrap(alice, 10e6);
        vm.warp(1_700_000_000);

        bytes memory allocate = _allocate(ALLOCATE_TID, 1e6);
        (uint64 observedAt, uint64 publishedAt, uint64 expiresAt,) = _times();
        vm.prank(admin);
        bytes memory admit = coordinator.admit(ALLOCATE_TID, observedAt, publishedAt, expiresAt);

        assertEq(admit.length, 296);
        assertEq(_word64(admit, SEQUENCE_OFF), 2);
        assertEq(
            _word32(admit, PREV_COMMIT_OFF),
            ProtocolCodec.nextCommitment(bytes32(0), ProtocolCodec.messageId(allocate))
        );

        string memory root = string.concat(vm.projectRoot(), "/testdata/control-lane/");
        assertEq(allocate, vm.parseBytes(vm.readFile(string.concat(root, "evm-allocate.hex"))));
        assertEq(admit, vm.parseBytes(vm.readFile(string.concat(root, "evm-admit.hex"))));
    }

    function test_evm_close_then_recover_follow_their_allocation_on_one_commitment_chain() public {
        (bytes memory allocate, bytes memory close, bytes memory recover) = _closeThenRecover();

        assertEq(close.length, 296);
        assertEq(recover.length, 296);
        assertEq(_word64(close, SEQUENCE_OFF), 2);
        bytes32 afterAllocate =
            ProtocolCodec.nextCommitment(bytes32(0), ProtocolCodec.messageId(allocate));
        assertEq(_word32(close, PREV_COMMIT_OFF), afterAllocate);
        assertEq(_word64(recover, SEQUENCE_OFF), 3);
        assertEq(
            _word32(recover, PREV_COMMIT_OFF),
            ProtocolCodec.nextCommitment(afterAllocate, ProtocolCodec.messageId(close))
        );
        assertEq(uint8(close[BODY_OFF + 40]), ProtocolCodec.RESOLVE_CLOSE);
        assertEq(_word32(recover, BODY_OFF), RECOVERY_TID);
        assertEq(_word64(recover, BODY_OFF + 32), 1);
        assertEq(uint8(recover[BODY_OFF + 40]), ProtocolCodec.RESOLVE_RECOVER);

        string memory root = string.concat(vm.projectRoot(), "/testdata/control-lane/");
        assertEq(allocate, vm.parseBytes(vm.readFile(string.concat(root, "evm-allocate.hex"))));
        assertEq(close, vm.parseBytes(vm.readFile(string.concat(root, "evm-close.hex"))));
        assertEq(recover, vm.parseBytes(vm.readFile(string.concat(root, "evm-recover.hex"))));
    }

    /// Allocates, closes after the deadline with 0.4 of 1.0 arrived, then
    /// opens a recovery against the 0.6 impairment.
    function _closeThenRecover()
        internal
        returns (bytes memory allocate, bytes memory close, bytes memory recover)
    {
        _bootstrap(alice, 10e6);
        vm.warp(1_700_000_000);
        allocate = _allocate(ALLOCATE_TID, 1e6);

        vm.warp(uint256(coordinator.activeDeadline()) + 1);
        (uint64 observedAt, uint64 publishedAt, uint64 expiresAt,) = _times();
        vm.prank(admin);
        close = coordinator.close(ALLOCATE_TID, observedAt, publishedAt, expiresAt);

        coordinator.processTransportSettlement(
            ProtocolCodec.encodeTransportSettlement(
                _inboundHeader(),
                ProtocolCodec.TransportSettlementBody({
                    transferId: ALLOCATE_TID,
                    kind: ProtocolCodec.SETTLEMENT_CLOSED,
                    claimAmount: 1e6,
                    creditedAmount: 400_000,
                    impairedOutstanding: 600_000,
                    configVersion: uint64(vault.configVersion())
                })
            )
        );
        assertEq(vault.impairedTransport(), 600_000);

        vm.prank(admin);
        recover = coordinator.recover(RECOVERY_TID, observedAt, publishedAt, expiresAt);
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
