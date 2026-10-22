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

    function test_resolve_bytes_match_rust() public {
        bytes memory rust = _ffi("encode-resolve");
        bytes memory sol = ProtocolCodec.encodeResolve(_header(), _resolveBody());
        assertEq(keccak256(sol), keccak256(rust));
        assertEq(sol.length, 296);
        assertEq(uint8(sol[TYPE_OFF]), ProtocolCodec.TYPE_RESOLVE);
    }

    function test_resolve_id_and_commitment_match_rust() public {
        bytes memory ids = _ffi("ids-resolve");
        assertEq(ids.length, 64);
        bytes32 rustId;
        bytes32 rustNext;
        assembly {
            rustId := mload(add(ids, 32))
            rustNext := mload(add(ids, 64))
        }
        bytes32 solId =
            ProtocolCodec.messageId(ProtocolCodec.encodeResolve(_header(), _resolveBody()));
        assertEq(solId, rustId);
        assertEq(ProtocolCodec.nextCommitment(_header().previousCommitment, solId), rustNext);
    }

    function test_resolve_target_must_precede_the_resolution() public {
        ProtocolCodec.ResolveBody memory body = _resolveBody();
        body.targetSequence = _header().sequence;
        vm.expectRevert(ProtocolCodec.TargetNotBeforeResolution.selector);
        this.encodeResolve(_header(), body);

        body.targetSequence = 0;
        vm.expectRevert(ProtocolCodec.ZeroIdentifier.selector);
        this.encodeResolve(_header(), body);
    }

    function test_resolve_accepts_only_defined_actions() public {
        ProtocolCodec.ResolveBody memory body = _resolveBody();
        body.action = 4;
        vm.expectRevert(abi.encodeWithSelector(ProtocolCodec.UnknownResolveAction.selector, 4));
        this.encodeResolve(_header(), body);

        body.action = 0;
        vm.expectRevert(abi.encodeWithSelector(ProtocolCodec.UnknownResolveAction.selector, 0));
        this.encodeResolve(_header(), body);
    }

    function test_close_and_recover_differ_from_admit_only_in_the_action_and_body_hash() public {
        bytes memory admit = ProtocolCodec.encodeResolve(_header(), _resolveBody());
        uint8[2] memory actions = [ProtocolCodec.RESOLVE_CLOSE, ProtocolCodec.RESOLVE_RECOVER];
        for (uint256 a; a < actions.length; ++a) {
            ProtocolCodec.ResolveBody memory body = _resolveBody();
            body.action = actions[a];
            bytes memory other = ProtocolCodec.encodeResolve(_header(), body);
            assertEq(other.length, admit.length);
            for (uint256 i; i < admit.length; ++i) {
                bool action = i == BODY_OFF + 40;
                bool bodyHash = i >= BODY_HASH_OFF && i < BODY_HASH_OFF + 32;
                if (action) assertEq(uint8(other[i]), actions[a]);
                else if (!bodyHash) assertEq(other[i], admit[i]);
            }
        }
    }

    function test_transport_settlement_bytes_match_rust() public {
        bytes memory rust = _ffi("encode-transport-settlement");
        bytes memory sol = ProtocolCodec.encodeTransportSettlement(_header(), _settlementBody());
        assertEq(keccak256(sol), keccak256(rust));
        assertEq(sol.length, 336);
        assertEq(uint8(sol[TYPE_OFF]), ProtocolCodec.TYPE_TRANSPORT_SETTLEMENT);
    }

    function test_transport_settlement_id_and_commitment_match_rust() public {
        bytes memory ids = _ffi("ids-transport-settlement");
        assertEq(ids.length, 64);
        bytes32 rustId;
        bytes32 rustNext;
        assembly {
            rustId := mload(add(ids, 32))
            rustNext := mload(add(ids, 64))
        }
        bytes32 solId = ProtocolCodec.messageId(
            ProtocolCodec.encodeTransportSettlement(_header(), _settlementBody())
        );
        assertEq(solId, rustId);
        assertEq(ProtocolCodec.nextCommitment(_header().previousCommitment, solId), rustNext);
    }

    function test_rust_transport_settlement_bytes_decode_to_the_fixture() public {
        (ProtocolCodec.Header memory header, ProtocolCodec.TransportSettlementBody memory body) =
            ProtocolCodec.decodeTransportSettlement(_ffi("encode-transport-settlement"));
        ProtocolCodec.TransportSettlementBody memory expected = _settlementBody();
        assertEq(header.sequence, _header().sequence);
        assertEq(body.transferId, expected.transferId);
        assertEq(body.kind, expected.kind);
        assertEq(body.claimAmount, expected.claimAmount);
        assertEq(body.creditedAmount, expected.creditedAmount);
        assertEq(body.impairedOutstanding, expected.impairedOutstanding);
        assertEq(body.configVersion, expected.configVersion);
    }

    function test_transport_settlement_rejects_every_inconsistent_body() public {
        ProtocolCodec.TransportSettlementBody memory body = _settlementBody();
        body.creditedAmount = body.claimAmount + 1;
        vm.expectRevert(ProtocolCodec.CreditAboveClaim.selector);
        this.encodeSettlement(_header(), body);

        body = _settlementBody();
        body.impairedOutstanding = 399_999;
        vm.expectRevert(ProtocolCodec.ImpairmentBelowShortfall.selector);
        this.encodeSettlement(_header(), body);

        body = _settlementBody();
        body.kind = ProtocolCodec.SETTLEMENT_RECOVERED;
        body.impairedOutstanding = 400_001;
        vm.expectRevert(ProtocolCodec.RecoveryImpairmentMismatch.selector);
        this.encodeSettlement(_header(), body);
        body.impairedOutstanding = 400_000;
        this.encodeSettlement(_header(), body);

        body = _settlementBody();
        body.claimAmount = 0;
        body.creditedAmount = 0;
        body.impairedOutstanding = 0;
        vm.expectRevert(ProtocolCodec.ZeroAmount.selector);
        this.encodeSettlement(_header(), body);

        body = _settlementBody();
        body.transferId = bytes32(0);
        vm.expectRevert(ProtocolCodec.ZeroIdentifier.selector);
        this.encodeSettlement(_header(), body);
    }

    function test_transport_settlement_rejects_unknown_kinds() public {
        uint8[3] memory kinds = [uint8(0), 3, 255];
        for (uint256 i; i < kinds.length; ++i) {
            ProtocolCodec.TransportSettlementBody memory body = _settlementBody();
            body.kind = kinds[i];
            vm.expectRevert(
                abi.encodeWithSelector(ProtocolCodec.UnknownSettlementKind.selector, kinds[i])
            );
            this.encodeSettlement(_header(), body);
        }
    }

    function test_a_resealed_settlement_with_a_tampered_kind_does_not_decode() public {
        bytes memory encoded = _ffi("encode-transport-settlement");
        encoded[BODY_OFF + 32] = 0x03;
        bytes memory body = new bytes(ProtocolCodec.TRANSPORT_SETTLEMENT_BODY_LEN);
        for (uint256 i; i < body.length; ++i) {
            body[i] = encoded[BODY_OFF + i];
        }
        bytes32 resealed = keccak256(body);
        for (uint256 i; i < 32; ++i) {
            encoded[BODY_HASH_OFF + i] = resealed[i];
        }
        vm.expectRevert(abi.encodeWithSelector(ProtocolCodec.UnknownSettlementKind.selector, 3));
        this.decodeSettlement(encoded);
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

    function encodeResolve(
        ProtocolCodec.Header memory header,
        ProtocolCodec.ResolveBody memory body
    ) external pure returns (bytes memory) {
        return ProtocolCodec.encodeResolve(header, body);
    }

    function encodeSettlement(
        ProtocolCodec.Header memory header,
        ProtocolCodec.TransportSettlementBody memory body
    ) external pure returns (bytes memory) {
        return ProtocolCodec.encodeTransportSettlement(header, body);
    }

    function decodeSettlement(bytes memory encoded) external pure {
        ProtocolCodec.decodeTransportSettlement(encoded);
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

    function _resolveBody() internal pure returns (ProtocolCodec.ResolveBody memory) {
        return ProtocolCodec.ResolveBody({
            transferId: bytes32(
                hex"5555555555555555555555555555555555555555555555555555555555555555"
            ),
            targetSequence: 41,
            action: ProtocolCodec.RESOLVE_ADMIT,
            configVersion: 4
        });
    }

    function _settlementBody()
        internal
        pure
        returns (ProtocolCodec.TransportSettlementBody memory)
    {
        return ProtocolCodec.TransportSettlementBody({
                transferId: bytes32(
                    hex"6666666666666666666666666666666666666666666666666666666666666666"
                ),
                kind: ProtocolCodec.SETTLEMENT_CLOSED,
                claimAmount: 1_000_000,
                creditedAmount: 600_000,
                impairedOutstanding: 400_000,
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
