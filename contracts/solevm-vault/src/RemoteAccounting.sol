// SPDX-License-Identifier: Apache-2.0
pragma solidity 0.8.30;

import {SolEVMVault} from "./SolEVMVault.sol";
import {IMessageAuthenticator} from "./interfaces/IMessageAuthenticator.sol";
import {IRemoteAccounting} from "./interfaces/IRemoteAccounting.sol";
import {EpochPhase, ISolEVMVault, VaultStatus} from "./interfaces/ISolEVMVault.sol";
import {ProtocolCodec} from "./libraries/ProtocolCodec.sol";

/// @title Canonical remote-leg coordinator for one Solana strategy.
/// @notice Owns message lanes, transfer identity and economic policy. Token
/// buckets stay on the vault. This contract is not a bridge.
contract RemoteAccounting is IRemoteAccounting {
    SolEVMVault private immutable _vault;
    IMessageAuthenticator public immutable authenticator;
    address public immutable outboundSink;

    uint32 public immutable evmChainId;
    uint32 public immutable solanaChainId;
    bytes32 public immutable evmApplication;
    bytes32 public immutable solanaApplication;
    bytes32 public immutable deploymentId;
    bytes32 public immutable boundVaultId;
    uint32 public immutable outboundLaneId;
    uint32 public immutable inboundLaneId;
    bytes32 public immutable expectedDestinationReference;

    uint64 public outboundNextSequence = 1;
    bytes32 public outboundLatestCommitment;
    uint64 public inboundExpectedSequence = 1;
    bytes32 public inboundLatestCommitment;
    uint128 public lastInboundCumulativeLoss;

    CycleKind public override activeCycleKind;
    bytes32 public override activeTransferId;
    uint128 public activeAmount;

    /// Accepted remote value and acknowledged cumulative loss when the active
    /// recall began.
    uint128 public recallBaselineValue;
    uint128 public recallBaselineLoss;

    mapping(bytes32 transferId => bool) public usedTransferIds;

    error InvalidAddress();
    error InvalidIdentities();
    error Unauthorized(address actor);
    error UnauthenticatedMessage();
    error ActiveTransferExists(bytes32 transferId);
    error NoActiveTransfer();
    error WrongTransferId(bytes32 expected, bytes32 actual);
    error TransferIdReused(bytes32 transferId);
    error ZeroTransferId();
    error AmountTooLarge();
    error WrongSequence(uint64 expected, uint64 actual);
    error Replay(uint64 sequence);
    error WrongCommitment(bytes32 expected, bytes32 actual);
    error WrongSource();
    error WrongDestination();
    error WrongDeployment();
    error WrongVault();
    error WrongLane(uint32 expected, uint32 actual);
    error WrongConfigVersion(uint64 expected, uint64 actual);
    error AllocateNotAllowed();
    error InsufficientRemotePrincipal(uint256 available, uint256 requested);
    error RecallIdentityMismatch();
    error InvalidRecallSentAmounts(uint128 principal, uint128 actual, uint128 loss);
    error StaleValuation();

    constructor(
        SolEVMVault vault_,
        IMessageAuthenticator authenticator_,
        address outboundSink_,
        Identities memory identities
    ) {
        if (address(vault_) == address(0) || address(authenticator_) == address(0)) {
            revert InvalidAddress();
        }
        if (outboundSink_ == address(0) || outboundSink_ == address(vault_)) {
            revert InvalidAddress();
        }
        if (
            identities.evmChainId == 0 || identities.solanaChainId == 0
                || identities.evmChainId == identities.solanaChainId
                || identities.evmApplication == bytes32(0)
                || identities.solanaApplication == bytes32(0)
                || identities.evmApplication == identities.solanaApplication
                || identities.deploymentId == bytes32(0) || identities.vaultId == bytes32(0)
                || identities.outboundLaneId == 0 || identities.inboundLaneId == 0
                || identities.outboundLaneId == identities.inboundLaneId
                || identities.expectedDestinationReference == bytes32(0)
        ) {
            revert InvalidIdentities();
        }

        _vault = vault_;
        authenticator = authenticator_;
        outboundSink = outboundSink_;
        evmChainId = identities.evmChainId;
        solanaChainId = identities.solanaChainId;
        evmApplication = identities.evmApplication;
        solanaApplication = identities.solanaApplication;
        deploymentId = identities.deploymentId;
        boundVaultId = identities.vaultId;
        outboundLaneId = identities.outboundLaneId;
        inboundLaneId = identities.inboundLaneId;
        expectedDestinationReference = identities.expectedDestinationReference;
    }

    function vault() external view returns (address) {
        return address(_vault);
    }

    function hasActiveRecall() external view returns (bool) {
        return activeCycleKind == CycleKind.Recall;
    }

    function hasActiveAllocate() external view returns (bool) {
        return activeCycleKind == CycleKind.Allocate;
    }

    function allocate(
        bytes32 transferId,
        uint128 amount,
        uint128 minimumDestinationAmount,
        uint64 deadline,
        uint64 observedAt,
        uint64 publishedAt,
        uint64 expiresAt
    ) external returns (bytes memory message) {
        _requireAdmin();
        _requireCanAllocate();
        _requireFreshTransfer(transferId);

        uint256 idle = _vault.idleBacking();
        if (idle < amount) {
            revert ISolEVMVault.InsufficientLiquidity(idle, amount);
        }

        ProtocolCodec.Header memory header = _outboundHeader(observedAt, publishedAt, expiresAt);
        message = ProtocolCodec.encodeAllocate(
            header,
            ProtocolCodec.AllocateBody({
                transferId: transferId,
                amount: amount,
                expectedSourceBalance: _toU128(idle),
                minimumDestinationAmount: minimumDestinationAmount,
                deadline: deadline,
                configVersion: _configVersion()
            })
        );

        _vault.beginOutbound(amount, outboundSink);
        _consumeOutbound(transferId, CycleKind.Allocate, amount, message);
        emit AllocationInitiated(transferId, amount, message);
    }

    function recall(
        bytes32 transferId,
        uint128 requestedAmount,
        uint128 minimumReturnAmount,
        uint64 deadline,
        uint64 observedAt,
        uint64 publishedAt,
        uint64 expiresAt
    ) external returns (bytes memory message) {
        _requireAdmin();
        _requireNoActiveCycle();
        _requireFreshTransfer(transferId);

        uint256 cap = _vault.remotePrincipalCap();
        if (requestedAmount == 0) revert ProtocolCodec.ZeroAmount();
        if (requestedAmount > cap) {
            revert InsufficientRemotePrincipal(cap, requestedAmount);
        }

        ProtocolCodec.Header memory header = _outboundHeader(observedAt, publishedAt, expiresAt);
        message = ProtocolCodec.encodeRecall(
            header,
            ProtocolCodec.RecallBody({
                transferId: transferId,
                requestedAmount: requestedAmount,
                minimumReturnAmount: minimumReturnAmount,
                deadline: deadline,
                configVersion: _configVersion()
            })
        );

        recallBaselineValue = _toU128(_vault.acceptedRemoteValue());
        recallBaselineLoss = lastInboundCumulativeLoss;
        _vault.beginRecall(transferId, requestedAmount);
        _consumeOutbound(transferId, CycleKind.Recall, requestedAmount, message);
        emit RecallInitiated(transferId, requestedAmount, message);
    }

    function processRemoteReport(bytes calldata encoded) external {
        if (!authenticator.authenticate(encoded)) revert UnauthenticatedMessage();
        (ProtocolCodec.Header memory header, ProtocolCodec.RemoteReportBody memory body) =
            ProtocolCodec.decodeRemoteReport(encoded);
        _requireInboundEnvelope(header);
        if (body.configVersion != _configVersion()) {
            revert WrongConfigVersion(_configVersion(), body.configVersion);
        }
        if (body.realizedLoss < lastInboundCumulativeLoss) revert StaleValuation();
        lastInboundCumulativeLoss = body.realizedLoss;
        _advanceInbound(encoded);

        ReportSkipReason skipped = _applyReport(header, body);
        if (skipped == ReportSkipReason.None) {
            emit RemoteReportAccepted(
                header.sequence,
                body.epochId,
                _vault.acceptedRemoteValue(),
                body.latestCompletedTransferId
            );
        } else {
            emit RemoteReportSkipped(header.sequence, body.epochId, skipped);
        }
    }

    function processRecallSent(bytes calldata encoded) external {
        if (!authenticator.authenticate(encoded)) revert UnauthenticatedMessage();
        (ProtocolCodec.Header memory header, ProtocolCodec.RecallSentBody memory body) =
            ProtocolCodec.decodeRecallSent(encoded);
        _requireInboundEnvelope(header);
        if (body.configVersion != _configVersion()) {
            revert WrongConfigVersion(_configVersion(), body.configVersion);
        }
        if (activeCycleKind != CycleKind.Recall) revert NoActiveTransfer();
        if (body.transferId != activeTransferId) {
            revert WrongTransferId(activeTransferId, body.transferId);
        }
        if (body.principalSent > activeAmount) revert RecallIdentityMismatch();
        if (body.destinationReference != expectedDestinationReference) {
            revert RecallIdentityMismatch();
        }
        if (
            uint256(body.actualAmountSent) + uint256(body.realizedLoss)
                != uint256(body.principalSent)
        ) {
            revert InvalidRecallSentAmounts(
                body.principalSent, body.actualAmountSent, body.realizedLoss
            );
        }

        uint128 baselineValue = recallBaselineValue;
        uint128 settledLoss = recallBaselineLoss + body.realizedLoss;
        if (settledLoss > lastInboundCumulativeLoss) lastInboundCumulativeLoss = settledLoss;
        recallBaselineValue = 0;
        recallBaselineLoss = 0;
        _clearCycle();
        _advanceInbound(encoded);

        // Reports during the recall deducted only its loss; settle from the
        // pre-recall value so the loss is not deducted twice.
        if (_vault.acceptedRemoteValue() != baselineValue) {
            _vault.applyRemoteValuation(baselineValue);
        }
        _vault.applyRecallSent(body.principalSent, body.actualAmountSent, body.realizedLoss);
        emit RecallSentAccepted(
            body.transferId, body.principalSent, body.actualAmountSent, body.realizedLoss
        );
    }

    function _requireAdmin() private view {
        if (msg.sender != _vault.admin()) revert Unauthorized(msg.sender);
    }

    function _requireCanAllocate() private view {
        if (_vault.vaultStatus() != VaultStatus.Active) revert AllocateNotAllowed();
        if (!_vault.epochOpen() || _vault.currentEpochPhase() != EpochPhase.Open) {
            revert AllocateNotAllowed();
        }
        _requireNoActiveCycle();
    }

    function _requireNoActiveCycle() private view {
        if (activeCycleKind != CycleKind.None) revert ActiveTransferExists(activeTransferId);
    }

    function _requireFreshTransfer(bytes32 transferId) private view {
        if (transferId == bytes32(0)) revert ZeroTransferId();
        if (usedTransferIds[transferId]) revert TransferIdReused(transferId);
    }

    function _consumeOutbound(
        bytes32 transferId,
        CycleKind kind,
        uint128 amount,
        bytes memory message
    ) private {
        usedTransferIds[transferId] = true;
        activeCycleKind = kind;
        activeTransferId = transferId;
        activeAmount = amount;

        uint64 sequence = outboundNextSequence;
        bytes32 id = ProtocolCodec.messageId(message);
        outboundLatestCommitment = ProtocolCodec.nextCommitment(outboundLatestCommitment, id);
        outboundNextSequence = sequence + 1;
        emit CanonicalMessagePublished(
            kind == CycleKind.Allocate ? ProtocolCodec.TYPE_ALLOCATE : ProtocolCodec.TYPE_RECALL,
            sequence,
            message
        );
    }

    /// Prices the report, or names why it cannot price anything. A report that
    /// cannot be applied still advances the lane, so nothing queued behind it
    /// is stranded; the next applicable report supersedes it.
    function _applyReport(
        ProtocolCodec.Header memory header,
        ProtocolCodec.RemoteReportBody memory body
    ) private returns (ReportSkipReason) {
        if (header.expiresAt < uint64(block.timestamp)) {
            return ReportSkipReason.Expired;
        }

        bool completes = activeCycleKind == CycleKind.Allocate
            && body.latestCompletedTransferId == activeTransferId && activeTransferId != bytes32(0);
        bool cutoff = _vault.epochOpen() && _vault.currentEpochPhase() == EpochPhase.CutOff;
        uint64 epochId = _vault.currentEpochId();

        if (cutoff && _vault.epochRemoteValuationLocked(epochId)) {
            if (!completes || !_completionKeepsNav(body.reportedValue)) {
                return ReportSkipReason.ValuationFixed;
            }
            _completeAllocation(body.reportedValue);
            return ReportSkipReason.None;
        }
        if (cutoff) {
            if (body.epochId != epochId) return ReportSkipReason.WrongEpoch;
            if (header.observedAt < _vault.currentEpochCutoffAt()) {
                return ReportSkipReason.StaleObservation;
            }
        }

        if (completes) _completeAllocation(body.reportedValue);
        _vault.applyRemoteValuation(
            activeCycleKind == CycleKind.Recall
                ? _valueDuringRecall(body.realizedLoss)
                : body.reportedValue
        );
        if (cutoff) {
            _vault.lockRemoteValuation(epochId);
            emit EpochRemoteValuationLocked(epochId, _vault.acceptedRemoteValue());
        }
        return ReportSkipReason.None;
    }

    function _completeAllocation(uint256 reportedValue) private {
        bytes32 transferId = activeTransferId;
        uint128 amount = activeAmount;
        _clearCycle();
        _vault.completeAllocation(amount, reportedValue);
        emit AllocationCompleted(transferId, amount, _vault.acceptedRemoteValue());
    }

    /// Completion moves the open amount from outbound transit into remote
    /// value, so NAV is unchanged only if the report carries it in full.
    function _completionKeepsNav(uint256 reportedValue) private view returns (bool) {
        uint256 cap = _vault.remotePrincipalCap() + activeAmount;
        uint256 value = reportedValue < cap ? reportedValue : cap;
        return value == _vault.acceptedRemoteValue() + activeAmount;
    }

    /// Solana's reported value already excludes recalled principal it has sent
    /// but the vault has not settled, so during a recall only the loss realized
    /// since the recall began moves the pre-recall value.
    function _valueDuringRecall(uint128 cumulativeLoss) private view returns (uint256) {
        uint256 loss = cumulativeLoss > recallBaselineLoss ? cumulativeLoss - recallBaselineLoss : 0;
        return loss < recallBaselineValue ? recallBaselineValue - loss : 0;
    }

    function _clearCycle() private {
        activeCycleKind = CycleKind.None;
        activeTransferId = bytes32(0);
        activeAmount = 0;
    }

    function _outboundHeader(uint64 observedAt, uint64 publishedAt, uint64 expiresAt)
        private
        view
        returns (ProtocolCodec.Header memory header)
    {
        header = ProtocolCodec.Header({
            sourceChain: evmChainId,
            destinationChain: solanaChainId,
            sourceApplication: evmApplication,
            destinationApplication: solanaApplication,
            deploymentId: deploymentId,
            vaultId: boundVaultId,
            laneId: outboundLaneId,
            sequence: outboundNextSequence,
            previousCommitment: outboundLatestCommitment,
            observedAt: observedAt,
            publishedAt: publishedAt,
            expiresAt: expiresAt
        });
    }

    function _requireInboundEnvelope(ProtocolCodec.Header memory header) private view {
        if (header.sourceChain != solanaChainId || header.sourceApplication != solanaApplication) {
            revert WrongSource();
        }
        if (
            header.destinationChain != evmChainId || header.destinationApplication != evmApplication
        ) {
            revert WrongDestination();
        }
        if (header.deploymentId != deploymentId) revert WrongDeployment();
        if (header.vaultId != boundVaultId) revert WrongVault();
        if (header.laneId != inboundLaneId) revert WrongLane(inboundLaneId, header.laneId);
        if (header.sequence != inboundExpectedSequence) {
            if (header.sequence < inboundExpectedSequence) revert Replay(header.sequence);
            revert WrongSequence(inboundExpectedSequence, header.sequence);
        }
        if (header.previousCommitment != inboundLatestCommitment) {
            revert WrongCommitment(inboundLatestCommitment, header.previousCommitment);
        }
    }

    function _advanceInbound(bytes memory encoded) private {
        bytes32 id = ProtocolCodec.messageId(encoded);
        inboundLatestCommitment = ProtocolCodec.nextCommitment(inboundLatestCommitment, id);
        inboundExpectedSequence += 1;
    }

    function _configVersion() private view returns (uint64) {
        return uint64(_vault.configVersion());
    }

    function _toU128(uint256 value) private pure returns (uint128) {
        if (value > type(uint128).max) revert AmountTooLarge();
        return uint128(value);
    }
}
