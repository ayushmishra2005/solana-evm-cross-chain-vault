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

    bytes32 internal constant ALLOCATE_TID =
        hex"7777777777777777777777777777777777777777777777777777777777777777";
    bytes32 internal constant RECALL_TID =
        hex"8888888888888888888888888888888888888888888888888888888888888888";

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

    function _word64(bytes memory encoded, uint256 offset) internal pure returns (uint64 value) {
        value = uint64(bytes8(_word32(encoded, offset)));
    }

    function _word32(bytes memory encoded, uint256 offset) internal pure returns (bytes32 value) {
        assembly {
            value := mload(add(add(encoded, 32), offset))
        }
    }
}
