// SPDX-License-Identifier: Apache-2.0
pragma solidity 0.8.30;

import {Test} from "forge-std/Test.sol";

import {ProtocolCodec} from "../../src/libraries/ProtocolCodec.sol";

/// Cross-language codec checks against the Rust `protocol-codec-ffi` binary.
contract ProtocolCodecTest is Test {
    uint256 internal constant TYPE_OFF = 8;
    uint256 internal constant SOURCE_CHAIN_OFF = 11;
    uint256 internal constant DEST_CHAIN_OFF = 15;
    uint256 internal constant SOURCE_APP_OFF = 19;
    uint256 internal constant DEST_APP_OFF = 51;
    uint256 internal constant DEPLOYMENT_OFF = 83;
    uint256 internal constant VAULT_OFF = 115;
    uint256 internal constant LANE_OFF = 147;
    uint256 internal constant SEQUENCE_OFF = 151;
    uint256 internal constant PREV_COMMIT_OFF = 159;
    uint256 internal constant EXPIRES_OFF = 207;
    uint256 internal constant BODY_HASH_OFF = 215;
    uint256 internal constant BODY_OFF = 247;

    function _header() internal pure returns (ProtocolCodec.Header memory header) {
        header = ProtocolCodec.Header({
            sourceChain: 1,
            destinationChain: 2,
            sourceApplication: ProtocolCodec.applicationFromAddress(
                address(uint160(bytes20(hex"A1A1A1A1A1A1A1A1A1A1A1A1A1A1A1A1A1A1A1A1")))
            ),
            destinationApplication: bytes32(
                hex"B2B2B2B2B2B2B2B2B2B2B2B2B2B2B2B2B2B2B2B2B2B2B2B2B2B2B2B2B2B2B2B2"
            ),
            deploymentId: bytes32(
                hex"D0D0D0D0D0D0D0D0D0D0D0D0D0D0D0D0D0D0D0D0D0D0D0D0D0D0D0D0D0D0D0D0"
            ),
            vaultId: bytes32(hex"0E0E0E0E0E0E0E0E0E0E0E0E0E0E0E0E0E0E0E0E0E0E0E0E0E0E0E0E0E0E0E0E"),
            laneId: 7,
            sequence: 42,
            previousCommitment: bytes32(
                hex"C1C1C1C1C1C1C1C1C1C1C1C1C1C1C1C1C1C1C1C1C1C1C1C1C1C1C1C1C1C1C1C1"
            ),
            observedAt: 1_700_000_000,
            publishedAt: 1_700_000_060,
            expiresAt: 1_700_003_600
        });
    }

    function _ffi(string memory command) internal returns (bytes memory) {
        string[] memory cmds = new string[](3);
        cmds[0] = "python3";
        cmds[1] = string.concat(vm.projectRoot(), "/test/ffi/protocol_codec_ffi.py");
        cmds[2] = command;
        return vm.ffi(cmds);
    }

    function test_allocate_bytes_match_rust() public {
        bytes memory rust = _ffi("encode-allocate");
        bytes memory sol = ProtocolCodec.encodeAllocate(_header(), _allocBody());
        assertEq(keccak256(sol), keccak256(rust));
        assertEq(sol.length, 343);
    }

    function test_recall_bytes_match_rust() public {
        bytes memory rust = _ffi("encode-recall");
        bytes memory sol = ProtocolCodec.encodeRecall(
            _header(),
            ProtocolCodec.RecallBody({
                transferId: bytes32(
                    hex"3333333333333333333333333333333333333333333333333333333333333333"
                ),
                requestedAmount: 750_000,
                minimumReturnAmount: 740_000,
                deadline: 1_700_001_860,
                configVersion: 9
            })
        );
        assertEq(keccak256(sol), keccak256(rust));
        assertEq(sol.length, 327);
    }

    function test_solidity_decodes_rust_remote_report() public {
        bytes memory rust = _ffi("encode-remote-report");
        (ProtocolCodec.Header memory header, ProtocolCodec.RemoteReportBody memory body) =
            ProtocolCodec.decodeRemoteReport(rust);
        assertEq(header.sequence, 42);
        assertEq(body.epochId, 12);
        assertEq(body.remotePrincipal, 2_000_000);
        assertEq(body.reportedValue, 2_050_000);
        assertEq(body.realizedLoss, 1500);
        assertEq(body.unattributedBalance, 25);
        assertEq(
            body.latestCompletedTransferId,
            bytes32(hex"7777777777777777777777777777777777777777777777777777777777777777")
        );
        assertEq(body.probeStatus, ProtocolCodec.PROBE_FRESH);
        assertEq(body.configVersion, 4);
        assertEq(keccak256(ProtocolCodec.encodeRemoteReport(header, body)), keccak256(rust));
    }

    function test_solidity_decodes_rust_recall_sent() public {
        bytes memory rust = _ffi("encode-recall-sent");
        (ProtocolCodec.Header memory header, ProtocolCodec.RecallSentBody memory body) =
            ProtocolCodec.decodeRecallSent(rust);
        assertEq(header.sequence, 42);
        assertEq(body.principalSent, 500_000);
        assertEq(body.actualAmountSent, 498_000);
        assertEq(body.realizedLoss, 2000);
        assertEq(keccak256(ProtocolCodec.encodeRecallSent(header, body)), keccak256(rust));
    }

    function test_message_id_and_commitment_match_rust() public {
        bytes memory ids = _ffi("ids-allocate");
        assertEq(ids.length, 64);
        bytes32 rustId;
        bytes32 rustNext;
        assembly {
            rustId := mload(add(ids, 32))
            rustNext := mload(add(ids, 64))
        }
        bytes memory encoded = ProtocolCodec.encodeAllocate(_header(), _allocBody());
        bytes32 solId = ProtocolCodec.messageId(encoded);
        assertEq(solId, rustId);
        assertEq(ProtocolCodec.nextCommitment(_header().previousCommitment, solId), rustNext);
    }

    function test_wrong_type_is_rejected() public {
        bytes memory encoded = _validReport();
        encoded[TYPE_OFF] = 0x01;
        vm.expectRevert();
        this.decodeReport(encoded);
    }

    function test_truncated_message_is_rejected() public {
        bytes memory encoded = _validReport();
        bytes memory cut = new bytes(encoded.length - 1);
        for (uint256 i; i < cut.length; ++i) {
            cut[i] = encoded[i];
        }
        vm.expectRevert();
        this.decodeReport(cut);
    }

    function test_extra_bytes_are_rejected() public {
        vm.expectRevert();
        this.decodeReport(bytes.concat(_validReport(), hex"00"));
    }

    function test_wrong_body_hash_is_rejected() public {
        bytes memory encoded = _validReport();
        encoded[BODY_HASH_OFF] = encoded[BODY_HASH_OFF] ^ 0x01;
        vm.expectRevert();
        this.decodeReport(encoded);
    }

    function test_corrupt_amount_is_rejected() public {
        bytes memory encoded = _validReport();
        encoded[BODY_OFF + 16] = encoded[BODY_OFF + 16] ^ 0x01;
        vm.expectRevert();
        this.decodeReport(encoded);
    }

    function test_corrupt_transfer_id_is_rejected() public {
        bytes memory encoded = _validRecallSent();
        encoded[BODY_OFF] = encoded[BODY_OFF] ^ 0x01;
        vm.expectRevert();
        this.decodeRecallSent(encoded);
    }

    function test_wrong_source_chain_survives_codec_but_is_detectable() public {
        bytes memory encoded = _validReport();
        encoded[SOURCE_CHAIN_OFF + 3] = 0x09;
        (ProtocolCodec.Header memory header,) = ProtocolCodec.decodeRemoteReport(encoded);
        assertEq(header.sourceChain, 9);
        assertTrue(header.sourceChain != 1);
    }

    function test_zero_source_application_is_rejected() public {
        ProtocolCodec.Header memory header = _header();
        header.sourceApplication = bytes32(0);
        vm.expectRevert(ProtocolCodec.ZeroIdentifier.selector);
        this.encodeAlloc(header, _allocBody());
    }

    function encodeAlloc(ProtocolCodec.Header memory header, ProtocolCodec.AllocateBody memory body)
        external
        pure
        returns (bytes memory)
    {
        return ProtocolCodec.encodeAllocate(header, body);
    }

    function decodeReport(bytes memory encoded) external pure {
        ProtocolCodec.decodeRemoteReport(encoded);
    }

    function decodeRecallSent(bytes memory encoded) external pure {
        ProtocolCodec.decodeRecallSent(encoded);
    }

    function _allocBody() internal pure returns (ProtocolCodec.AllocateBody memory) {
        return ProtocolCodec.AllocateBody({
            transferId: bytes32(
                hex"1111111111111111111111111111111111111111111111111111111111111111"
            ),
            amount: 1_000_000,
            expectedSourceBalance: 5_000_000,
            minimumDestinationAmount: 999_000,
            deadline: 1_700_000_960,
            configVersion: 4
        });
    }

    function _validReport() internal pure returns (bytes memory) {
        return ProtocolCodec.encodeRemoteReport(
            _header(),
            ProtocolCodec.RemoteReportBody({
                epochId: 12,
                remotePrincipal: 2_000_000,
                reportedValue: 2_050_000,
                realizedLoss: 1500,
                unattributedBalance: 25,
                latestCompletedTransferId: bytes32(
                    hex"7777777777777777777777777777777777777777777777777777777777777777"
                ),
                probeStatus: ProtocolCodec.PROBE_FRESH,
                probeTimestamp: 1_699_999_880,
                configVersion: 4,
                remoteStateCommitment: bytes32(
                    hex"8888888888888888888888888888888888888888888888888888888888888888"
                )
            })
        );
    }

    function _validRecallSent() internal pure returns (bytes memory) {
        return ProtocolCodec.encodeRecallSent(
            _header(),
            ProtocolCodec.RecallSentBody({
                transferId: bytes32(
                    hex"9999999999999999999999999999999999999999999999999999999999999999"
                ),
                principalSent: 500_000,
                actualAmountSent: 498_000,
                realizedLoss: 2000,
                destinationReference: bytes32(
                    hex"BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB"
                ),
                sentTimestamp: 1_699_999_970,
                configVersion: 4
            })
        );
    }
}
