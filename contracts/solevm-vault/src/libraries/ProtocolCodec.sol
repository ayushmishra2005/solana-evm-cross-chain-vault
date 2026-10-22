// SPDX-License-Identifier: Apache-2.0
pragma solidity 0.8.30;

/// @title Canonical SolEVM control-message codec.
/// @notice Fixed-width big-endian layout identical to the Rust `protocol-types`
/// crate. Values are not ABI-encoded.
library ProtocolCodec {
    bytes4 internal constant MAGIC = 0x53564531; // "SVE1"

    uint16 internal constant PROTOCOL_VERSION = 1;
    uint16 internal constant SCHEMA_VERSION = 1;

    uint8 internal constant TYPE_ALLOCATE = 1;
    uint8 internal constant TYPE_RECALL = 2;
    uint8 internal constant TYPE_REMOTE_REPORT = 3;
    uint8 internal constant TYPE_RECALL_SENT = 4;
    uint8 internal constant TYPE_RESOLVE = 6;
    uint8 internal constant TYPE_TRANSPORT_SETTLEMENT = 7;

    uint256 internal constant HEADER_LEN = 247;
    uint256 internal constant ALLOCATE_BODY_LEN = 96;
    uint256 internal constant RECALL_BODY_LEN = 80;
    uint256 internal constant REMOTE_REPORT_BODY_LEN = 153;
    uint256 internal constant RECALL_SENT_BODY_LEN = 128;
    uint256 internal constant RESOLVE_BODY_LEN = 49;
    uint256 internal constant TRANSPORT_SETTLEMENT_BODY_LEN = 89;

    uint256 internal constant ALLOCATE_MESSAGE_LEN = 343;
    uint256 internal constant RECALL_MESSAGE_LEN = 327;
    uint256 internal constant REMOTE_REPORT_MESSAGE_LEN = 400;
    uint256 internal constant RECALL_SENT_MESSAGE_LEN = 375;
    uint256 internal constant RESOLVE_MESSAGE_LEN = 296;
    uint256 internal constant TRANSPORT_SETTLEMENT_MESSAGE_LEN = 336;

    /// Runs a quarantined transfer on its original terms.
    uint8 internal constant RESOLVE_ADMIT = 1;
    uint8 internal constant RESOLVE_CLOSE = 2;
    uint8 internal constant RESOLVE_RECOVER = 3;

    uint8 internal constant SETTLEMENT_CLOSED = 1;
    uint8 internal constant SETTLEMENT_RECOVERED = 2;

    bytes internal constant MESSAGE_ID_DOMAIN = "SOLEVM_MESSAGE_ID_V1";
    bytes internal constant COMMITMENT_DOMAIN = "SOLEVM_COMMITMENT_V1";

    uint8 internal constant PROBE_NOT_REQUIRED = 0;
    uint8 internal constant PROBE_FRESH = 1;
    uint8 internal constant PROBE_STALE = 2;
    uint8 internal constant PROBE_FAILED = 3;

    struct Header {
        uint32 sourceChain;
        uint32 destinationChain;
        bytes32 sourceApplication;
        bytes32 destinationApplication;
        bytes32 deploymentId;
        bytes32 vaultId;
        uint32 laneId;
        uint64 sequence;
        bytes32 previousCommitment;
        uint64 observedAt;
        uint64 publishedAt;
        uint64 expiresAt;
    }

    struct AllocateBody {
        bytes32 transferId;
        uint128 amount;
        uint128 expectedSourceBalance;
        uint128 minimumDestinationAmount;
        uint64 deadline;
        uint64 configVersion;
    }

    struct RecallBody {
        bytes32 transferId;
        uint128 requestedAmount;
        uint128 minimumReturnAmount;
        uint64 deadline;
        uint64 configVersion;
    }

    struct RemoteReportBody {
        uint64 epochId;
        uint128 remotePrincipal;
        uint128 reportedValue;
        uint128 realizedLoss;
        uint128 unattributedBalance;
        bytes32 latestCompletedTransferId;
        uint8 probeStatus;
        uint64 probeTimestamp;
        uint64 configVersion;
        bytes32 remoteStateCommitment;
    }

    struct ResolveBody {
        bytes32 transferId;
        uint64 targetSequence;
        uint8 action;
        uint64 configVersion;
    }

    /// @dev The claim is a closed allocation's authorized amount, or the
    /// pooled impairment before a recovery. Impairment is pooled because
    /// custody is fungible.
    struct TransportSettlementBody {
        bytes32 transferId;
        uint8 kind;
        uint128 claimAmount;
        uint128 creditedAmount;
        uint128 impairedOutstanding;
        uint64 configVersion;
    }

    struct RecallSentBody {
        bytes32 transferId;
        uint128 principalSent;
        uint128 actualAmountSent;
        uint128 realizedLoss;
        bytes32 destinationReference;
        uint64 sentTimestamp;
        uint64 configVersion;
    }

    error InvalidMagic();
    error UnsupportedProtocolVersion(uint16 version);
    error UnsupportedSchemaVersion(uint16 version);
    error InvalidMessageType(uint8 messageType);
    error ReservedFlagsSet(uint16 flags);
    error TruncatedMessage(uint256 needed, uint256 found);
    error TrailingBytes(uint256 expected, uint256 found);
    error InvalidBodyHash(bytes32 expected, bytes32 actual);
    error ZeroIdentifier();
    error SourceEqualsDestinationChain();
    error SourceEqualsDestinationApplication();
    error PublicationBeforeObservation();
    error ExpirationBeforePublication();
    error ZeroAmount();
    error MinimumAboveAmount();
    error DeadlineBeforePublication();
    error InvalidProbeStatus(uint8 status);
    error ProbeTimestampAfterObservation();
    error MissingProbeTimestamp();
    error SentTimestampAfterObservation();
    error RealizedLossAbovePrincipal();
    error UnknownResolveAction(uint8 action);
    error TargetNotBeforeResolution();
    error UnknownSettlementKind(uint8 kind);
    error CreditAboveClaim();
    error ImpairmentBelowShortfall();
    error RecoveryImpairmentMismatch();

    function encodeAllocate(Header memory header, AllocateBody memory body)
        internal
        pure
        returns (bytes memory encoded)
    {
        _validateHeader(header);
        _validateAllocate(body, header.publishedAt);
        encoded = new bytes(ALLOCATE_MESSAGE_LEN);
        _writeAllocateBody(encoded, HEADER_LEN, body);
        _seal(encoded, header, TYPE_ALLOCATE, ALLOCATE_BODY_LEN);
    }

    function encodeRecall(Header memory header, RecallBody memory body)
        internal
        pure
        returns (bytes memory encoded)
    {
        _validateHeader(header);
        _validateRecall(body, header.publishedAt);
        encoded = new bytes(RECALL_MESSAGE_LEN);
        _writeRecallBody(encoded, HEADER_LEN, body);
        _seal(encoded, header, TYPE_RECALL, RECALL_BODY_LEN);
    }

    function encodeResolve(Header memory header, ResolveBody memory body)
        internal
        pure
        returns (bytes memory encoded)
    {
        _validateHeader(header);
        _validateResolve(body, header.sequence);
        encoded = new bytes(RESOLVE_MESSAGE_LEN);
        _writeResolveBody(encoded, HEADER_LEN, body);
        _seal(encoded, header, TYPE_RESOLVE, RESOLVE_BODY_LEN);
    }

    function encodeRemoteReport(Header memory header, RemoteReportBody memory body)
        internal
        pure
        returns (bytes memory encoded)
    {
        _validateHeader(header);
        _validateRemoteReport(body, header.observedAt);
        encoded = new bytes(REMOTE_REPORT_MESSAGE_LEN);
        _writeRemoteReportBody(encoded, HEADER_LEN, body);
        _seal(encoded, header, TYPE_REMOTE_REPORT, REMOTE_REPORT_BODY_LEN);
    }

    function encodeRecallSent(Header memory header, RecallSentBody memory body)
        internal
        pure
        returns (bytes memory encoded)
    {
        _validateHeader(header);
        _validateRecallSent(body, header.observedAt);
        encoded = new bytes(RECALL_SENT_MESSAGE_LEN);
        _writeRecallSentBody(encoded, HEADER_LEN, body);
        _seal(encoded, header, TYPE_RECALL_SENT, RECALL_SENT_BODY_LEN);
    }

    function encodeTransportSettlement(Header memory header, TransportSettlementBody memory body)
        internal
        pure
        returns (bytes memory encoded)
    {
        _validateHeader(header);
        _validateTransportSettlement(body);
        encoded = new bytes(TRANSPORT_SETTLEMENT_MESSAGE_LEN);
        _writeTransportSettlementBody(encoded, HEADER_LEN, body);
        _seal(encoded, header, TYPE_TRANSPORT_SETTLEMENT, TRANSPORT_SETTLEMENT_BODY_LEN);
    }

    function decodeRemoteReport(bytes memory encoded)
        internal
        pure
        returns (Header memory header, RemoteReportBody memory body)
    {
        _inspectFrame(encoded, TYPE_REMOTE_REPORT, REMOTE_REPORT_MESSAGE_LEN);
        header = _readHeader(encoded);
        _validateHeader(header);
        body = _readRemoteReportBody(encoded, HEADER_LEN);
        _validateRemoteReport(body, header.observedAt);
    }

    function decodeRecallSent(bytes memory encoded)
        internal
        pure
        returns (Header memory header, RecallSentBody memory body)
    {
        _inspectFrame(encoded, TYPE_RECALL_SENT, RECALL_SENT_MESSAGE_LEN);
        header = _readHeader(encoded);
        _validateHeader(header);
        body = _readRecallSentBody(encoded, HEADER_LEN);
        _validateRecallSent(body, header.observedAt);
    }

    function decodeTransportSettlement(bytes memory encoded)
        internal
        pure
        returns (Header memory header, TransportSettlementBody memory body)
    {
        _inspectFrame(encoded, TYPE_TRANSPORT_SETTLEMENT, TRANSPORT_SETTLEMENT_MESSAGE_LEN);
        header = _readHeader(encoded);
        _validateHeader(header);
        body = _readTransportSettlementBody(encoded, HEADER_LEN);
        _validateTransportSettlement(body);
    }

    function messageId(bytes memory encoded) internal pure returns (bytes32) {
        return keccak256(bytes.concat(MESSAGE_ID_DOMAIN, encoded));
    }

    function nextCommitment(bytes32 previous, bytes32 id) internal pure returns (bytes32) {
        return keccak256(bytes.concat(COMMITMENT_DOMAIN, previous, id));
    }

    /// @notice Pads a 20-byte address into the low 20 bytes of a 32-byte id.
    function applicationFromAddress(address account) internal pure returns (bytes32) {
        return bytes32(uint256(uint160(account)));
    }

    function _seal(bytes memory encoded, Header memory header, uint8 messageType, uint256 bodyLen)
        private
        pure
    {
        bytes32 bodyHash = keccak256(_slice(encoded, HEADER_LEN, bodyLen));
        _writeHeader(encoded, header, messageType, bodyHash);
    }

    function _inspectFrame(bytes memory encoded, uint8 expectedType, uint256 expectedLen)
        private
        pure
    {
        if (encoded.length < HEADER_LEN) {
            revert TruncatedMessage(HEADER_LEN, encoded.length);
        }
        if (_readBytes4(encoded, 0) != MAGIC) revert InvalidMagic();
        uint16 protocol = _readU16(encoded, 4);
        if (protocol != PROTOCOL_VERSION) revert UnsupportedProtocolVersion(protocol);
        uint16 schema = _readU16(encoded, 6);
        if (schema != SCHEMA_VERSION) revert UnsupportedSchemaVersion(schema);
        uint8 messageType = uint8(encoded[8]);
        if (messageType != expectedType) revert InvalidMessageType(messageType);
        if (encoded.length < expectedLen) revert TruncatedMessage(expectedLen, encoded.length);
        if (encoded.length > expectedLen) revert TrailingBytes(expectedLen, encoded.length);
        bytes32 expectedHash = keccak256(_slice(encoded, HEADER_LEN, expectedLen - HEADER_LEN));
        bytes32 actualHash = _readBytes32(encoded, 215);
        if (expectedHash != actualHash) revert InvalidBodyHash(expectedHash, actualHash);
    }

    function _validateHeader(Header memory header) private pure {
        if (header.sourceChain == 0 || header.destinationChain == 0) revert ZeroIdentifier();
        if (header.sourceChain == header.destinationChain) revert SourceEqualsDestinationChain();
        if (header.sourceApplication == bytes32(0) || header.destinationApplication == bytes32(0)) {
            revert ZeroIdentifier();
        }
        if (header.sourceApplication == header.destinationApplication) {
            revert SourceEqualsDestinationApplication();
        }
        if (header.deploymentId == bytes32(0) || header.vaultId == bytes32(0)) {
            revert ZeroIdentifier();
        }
        if (header.laneId == 0 || header.sequence == 0) revert ZeroIdentifier();
        if (header.publishedAt < header.observedAt) revert PublicationBeforeObservation();
        if (header.expiresAt < header.publishedAt) revert ExpirationBeforePublication();
    }

    function _validateAllocate(AllocateBody memory body, uint64 publishedAt) private pure {
        if (body.transferId == bytes32(0) || body.configVersion == 0) revert ZeroIdentifier();
        if (body.amount == 0 || body.minimumDestinationAmount == 0) revert ZeroAmount();
        if (body.minimumDestinationAmount > body.amount) revert MinimumAboveAmount();
        if (body.deadline < publishedAt) revert DeadlineBeforePublication();
    }

    function _validateRecall(RecallBody memory body, uint64 publishedAt) private pure {
        if (body.transferId == bytes32(0) || body.configVersion == 0) revert ZeroIdentifier();
        if (body.requestedAmount == 0 || body.minimumReturnAmount == 0) revert ZeroAmount();
        if (body.minimumReturnAmount > body.requestedAmount) revert MinimumAboveAmount();
        if (body.deadline < publishedAt) revert DeadlineBeforePublication();
    }

    function _validateResolve(ResolveBody memory body, uint64 sequence) private pure {
        if (body.transferId == bytes32(0) || body.configVersion == 0) revert ZeroIdentifier();
        if (body.targetSequence == 0) revert ZeroIdentifier();
        if (body.targetSequence >= sequence) revert TargetNotBeforeResolution();
        if (body.action == 0 || body.action > RESOLVE_RECOVER) {
            revert UnknownResolveAction(body.action);
        }
    }

    /// @dev A close adds its shortfall to the pool. A recovery leaves exactly
    /// the uncredited part of the pool behind.
    function _validateTransportSettlement(TransportSettlementBody memory body) private pure {
        if (body.transferId == bytes32(0) || body.configVersion == 0) revert ZeroIdentifier();
        if (body.kind != SETTLEMENT_CLOSED && body.kind != SETTLEMENT_RECOVERED) {
            revert UnknownSettlementKind(body.kind);
        }
        if (body.claimAmount == 0) revert ZeroAmount();
        if (body.creditedAmount > body.claimAmount) revert CreditAboveClaim();
        uint128 shortfall = body.claimAmount - body.creditedAmount;
        if (body.kind == SETTLEMENT_CLOSED) {
            if (body.impairedOutstanding < shortfall) revert ImpairmentBelowShortfall();
        } else if (body.impairedOutstanding != shortfall) {
            revert RecoveryImpairmentMismatch();
        }
    }

    function _validateRemoteReport(RemoteReportBody memory body, uint64 observedAt) private pure {
        if (body.epochId == 0 || body.configVersion == 0) revert ZeroIdentifier();
        if (body.remoteStateCommitment == bytes32(0)) revert ZeroIdentifier();
        if (body.probeStatus > PROBE_FAILED) revert InvalidProbeStatus(body.probeStatus);
        if (body.probeTimestamp > observedAt) revert ProbeTimestampAfterObservation();
        if (body.probeStatus == PROBE_FRESH && body.probeTimestamp == 0) {
            revert MissingProbeTimestamp();
        }
    }

    function _validateRecallSent(RecallSentBody memory body, uint64 observedAt) private pure {
        if (body.transferId == bytes32(0) || body.configVersion == 0) revert ZeroIdentifier();
        if (body.destinationReference == bytes32(0)) revert ZeroIdentifier();
        if (body.principalSent == 0) revert ZeroAmount();
        if (body.realizedLoss > body.principalSent) revert RealizedLossAbovePrincipal();
        if (body.sentTimestamp > observedAt) revert SentTimestampAfterObservation();
    }

    function _writeHeader(
        bytes memory encoded,
        Header memory header,
        uint8 messageType,
        bytes32 bodyHash
    ) private pure {
        _writeBytes4(encoded, 0, MAGIC);
        _writeU16(encoded, 4, PROTOCOL_VERSION);
        _writeU16(encoded, 6, SCHEMA_VERSION);
        encoded[8] = bytes1(messageType);
        _writeU16(encoded, 9, 0);
        _writeU32(encoded, 11, header.sourceChain);
        _writeU32(encoded, 15, header.destinationChain);
        _writeBytes32(encoded, 19, header.sourceApplication);
        _writeBytes32(encoded, 51, header.destinationApplication);
        _writeBytes32(encoded, 83, header.deploymentId);
        _writeBytes32(encoded, 115, header.vaultId);
        _writeU32(encoded, 147, header.laneId);
        _writeU64(encoded, 151, header.sequence);
        _writeBytes32(encoded, 159, header.previousCommitment);
        _writeU64(encoded, 191, header.observedAt);
        _writeU64(encoded, 199, header.publishedAt);
        _writeU64(encoded, 207, header.expiresAt);
        _writeBytes32(encoded, 215, bodyHash);
    }

    function _readHeader(bytes memory encoded) private pure returns (Header memory header) {
        uint16 flags = _readU16(encoded, 9);
        if (flags != 0) revert ReservedFlagsSet(flags);
        header.sourceChain = _readU32(encoded, 11);
        header.destinationChain = _readU32(encoded, 15);
        header.sourceApplication = _readBytes32(encoded, 19);
        header.destinationApplication = _readBytes32(encoded, 51);
        header.deploymentId = _readBytes32(encoded, 83);
        header.vaultId = _readBytes32(encoded, 115);
        header.laneId = _readU32(encoded, 147);
        header.sequence = _readU64(encoded, 151);
        header.previousCommitment = _readBytes32(encoded, 159);
        header.observedAt = _readU64(encoded, 191);
        header.publishedAt = _readU64(encoded, 199);
        header.expiresAt = _readU64(encoded, 207);
    }

    function _writeAllocateBody(bytes memory encoded, uint256 off, AllocateBody memory body)
        private
        pure
    {
        _writeBytes32(encoded, off, body.transferId);
        _writeU128(encoded, off + 32, body.amount);
        _writeU128(encoded, off + 48, body.expectedSourceBalance);
        _writeU128(encoded, off + 64, body.minimumDestinationAmount);
        _writeU64(encoded, off + 80, body.deadline);
        _writeU64(encoded, off + 88, body.configVersion);
    }

    function _writeRecallBody(bytes memory encoded, uint256 off, RecallBody memory body)
        private
        pure
    {
        _writeBytes32(encoded, off, body.transferId);
        _writeU128(encoded, off + 32, body.requestedAmount);
        _writeU128(encoded, off + 48, body.minimumReturnAmount);
        _writeU64(encoded, off + 64, body.deadline);
        _writeU64(encoded, off + 72, body.configVersion);
    }

    function _writeResolveBody(bytes memory encoded, uint256 off, ResolveBody memory body)
        private
        pure
    {
        _writeBytes32(encoded, off, body.transferId);
        _writeU64(encoded, off + 32, body.targetSequence);
        encoded[off + 40] = bytes1(body.action);
        _writeU64(encoded, off + 41, body.configVersion);
    }

    function _writeTransportSettlementBody(
        bytes memory encoded,
        uint256 off,
        TransportSettlementBody memory body
    ) private pure {
        _writeBytes32(encoded, off, body.transferId);
        encoded[off + 32] = bytes1(body.kind);
        _writeU128(encoded, off + 33, body.claimAmount);
        _writeU128(encoded, off + 49, body.creditedAmount);
        _writeU128(encoded, off + 65, body.impairedOutstanding);
        _writeU64(encoded, off + 81, body.configVersion);
    }

    function _readTransportSettlementBody(bytes memory encoded, uint256 off)
        private
        pure
        returns (TransportSettlementBody memory body)
    {
        body.transferId = _readBytes32(encoded, off);
        body.kind = uint8(encoded[off + 32]);
        body.claimAmount = _readU128(encoded, off + 33);
        body.creditedAmount = _readU128(encoded, off + 49);
        body.impairedOutstanding = _readU128(encoded, off + 65);
        body.configVersion = _readU64(encoded, off + 81);
    }

    function _writeRemoteReportBody(bytes memory encoded, uint256 off, RemoteReportBody memory body)
        private
        pure
    {
        _writeU64(encoded, off, body.epochId);
        _writeU128(encoded, off + 8, body.remotePrincipal);
        _writeU128(encoded, off + 24, body.reportedValue);
        _writeU128(encoded, off + 40, body.realizedLoss);
        _writeU128(encoded, off + 56, body.unattributedBalance);
        _writeBytes32(encoded, off + 72, body.latestCompletedTransferId);
        encoded[off + 104] = bytes1(body.probeStatus);
        _writeU64(encoded, off + 105, body.probeTimestamp);
        _writeU64(encoded, off + 113, body.configVersion);
        _writeBytes32(encoded, off + 121, body.remoteStateCommitment);
    }

    function _readRemoteReportBody(bytes memory encoded, uint256 off)
        private
        pure
        returns (RemoteReportBody memory body)
    {
        body.epochId = _readU64(encoded, off);
        body.remotePrincipal = _readU128(encoded, off + 8);
        body.reportedValue = _readU128(encoded, off + 24);
        body.realizedLoss = _readU128(encoded, off + 40);
        body.unattributedBalance = _readU128(encoded, off + 56);
        body.latestCompletedTransferId = _readBytes32(encoded, off + 72);
        body.probeStatus = uint8(encoded[off + 104]);
        body.probeTimestamp = _readU64(encoded, off + 105);
        body.configVersion = _readU64(encoded, off + 113);
        body.remoteStateCommitment = _readBytes32(encoded, off + 121);
    }

    function _writeRecallSentBody(bytes memory encoded, uint256 off, RecallSentBody memory body)
        private
        pure
    {
        _writeBytes32(encoded, off, body.transferId);
        _writeU128(encoded, off + 32, body.principalSent);
        _writeU128(encoded, off + 48, body.actualAmountSent);
        _writeU128(encoded, off + 64, body.realizedLoss);
        _writeBytes32(encoded, off + 80, body.destinationReference);
        _writeU64(encoded, off + 112, body.sentTimestamp);
        _writeU64(encoded, off + 120, body.configVersion);
    }

    function _readRecallSentBody(bytes memory encoded, uint256 off)
        private
        pure
        returns (RecallSentBody memory body)
    {
        body.transferId = _readBytes32(encoded, off);
        body.principalSent = _readU128(encoded, off + 32);
        body.actualAmountSent = _readU128(encoded, off + 48);
        body.realizedLoss = _readU128(encoded, off + 64);
        body.destinationReference = _readBytes32(encoded, off + 80);
        body.sentTimestamp = _readU64(encoded, off + 112);
        body.configVersion = _readU64(encoded, off + 120);
    }

    function _writeBytes4(bytes memory buf, uint256 off, bytes4 value) private pure {
        buf[off] = value[0];
        buf[off + 1] = value[1];
        buf[off + 2] = value[2];
        buf[off + 3] = value[3];
    }

    function _writeBytes32(bytes memory buf, uint256 off, bytes32 value) private pure {
        for (uint256 i; i < 32; ++i) {
            buf[off + i] = value[i];
        }
    }

    function _writeU16(bytes memory buf, uint256 off, uint16 value) private pure {
        buf[off] = bytes1(uint8(value >> 8));
        buf[off + 1] = bytes1(uint8(value));
    }

    function _writeU32(bytes memory buf, uint256 off, uint32 value) private pure {
        buf[off] = bytes1(uint8(value >> 24));
        buf[off + 1] = bytes1(uint8(value >> 16));
        buf[off + 2] = bytes1(uint8(value >> 8));
        buf[off + 3] = bytes1(uint8(value));
    }

    function _writeU64(bytes memory buf, uint256 off, uint64 value) private pure {
        buf[off] = bytes1(uint8(value >> 56));
        buf[off + 1] = bytes1(uint8(value >> 48));
        buf[off + 2] = bytes1(uint8(value >> 40));
        buf[off + 3] = bytes1(uint8(value >> 32));
        buf[off + 4] = bytes1(uint8(value >> 24));
        buf[off + 5] = bytes1(uint8(value >> 16));
        buf[off + 6] = bytes1(uint8(value >> 8));
        buf[off + 7] = bytes1(uint8(value));
    }

    function _writeU128(bytes memory buf, uint256 off, uint128 value) private pure {
        for (uint256 i; i < 16; ++i) {
            buf[off + i] = bytes1(uint8(value >> (8 * (15 - i))));
        }
    }

    function _readBytes4(bytes memory data, uint256 off) private pure returns (bytes4) {
        return bytes4(
            (uint32(uint8(data[off])) << 24) | (uint32(uint8(data[off + 1])) << 16)
                | (uint32(uint8(data[off + 2])) << 8) | uint32(uint8(data[off + 3]))
        );
    }

    function _readU16(bytes memory data, uint256 off) private pure returns (uint16) {
        return (uint16(uint8(data[off])) << 8) | uint8(data[off + 1]);
    }

    function _readU32(bytes memory data, uint256 off) private pure returns (uint32) {
        return (uint32(uint8(data[off])) << 24) | (uint32(uint8(data[off + 1])) << 16)
            | (uint32(uint8(data[off + 2])) << 8) | uint8(data[off + 3]);
    }

    function _readU64(bytes memory data, uint256 off) private pure returns (uint64) {
        uint64 value;
        for (uint256 i; i < 8; ++i) {
            value = (value << 8) | uint8(data[off + i]);
        }
        return value;
    }

    function _readU128(bytes memory data, uint256 off) private pure returns (uint128) {
        uint128 value;
        for (uint256 i; i < 16; ++i) {
            value = (value << 8) | uint8(data[off + i]);
        }
        return value;
    }

    function _readBytes32(bytes memory data, uint256 off) private pure returns (bytes32 value) {
        for (uint256 i; i < 32; ++i) {
            value |= bytes32(uint256(uint8(data[off + i])) << (8 * (31 - i)));
        }
    }

    function _slice(bytes memory data, uint256 off, uint256 len)
        private
        pure
        returns (bytes memory out)
    {
        out = new bytes(len);
        for (uint256 i; i < len; ++i) {
            out[i] = data[off + i];
        }
    }
}
