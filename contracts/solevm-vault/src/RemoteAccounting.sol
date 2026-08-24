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
    error MessageExpired(uint64 nowAt, uint64 expiresAt);
    error AllocateNotAllowed();
    error InsufficientRemotePrincipal(uint256 available, uint256 requested);
    error RecallIdentityMismatch();
    error InvalidRecallSentAmounts(uint128 principal, uint128 actual, uint128 loss);
    error InvalidReportEpoch(uint64 expected, uint64 actual);
    error StaleValuation();
    error ValuationAlreadyFixed();

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

        uint256 navBefore = _vault.managedNav();
        bool completedAllocate;
        if (
            activeCycleKind == CycleKind.Allocate
                && body.latestCompletedTransferId == activeTransferId
                && activeTransferId != bytes32(0)
        ) {
            _vault.completeAllocation(activeAmount, body.reportedValue);
            emit AllocationCompleted(activeTransferId, activeAmount, _vault.acceptedRemoteValue());
            _clearCycle();
            completedAllocate = true;
        }

        bool cutoff = _vault.epochOpen() && _vault.currentEpochPhase() == EpochPhase.CutOff;
        uint64 epochId = _vault.currentEpochId();
        bool locked = cutoff && _vault.epochRemoteValuationLocked(epochId);

        if (locked) {
            if (!completedAllocate || _vault.managedNav() != navBefore) {
                revert ValuationAlreadyFixed();
            }
        } else if (cutoff) {
            if (body.epochId != epochId) revert InvalidReportEpoch(epochId, body.epochId);
            if (header.observedAt < _vault.currentEpochCutoffAt()) revert StaleValuation();
            _vault.applyRemoteValuation(body.reportedValue);
            _vault.lockRemoteValuation(epochId);
            emit EpochRemoteValuationLocked(epochId, _vault.acceptedRemoteValue());
        } else {
            _vault.applyRemoteValuation(body.reportedValue);
        }

        if (body.realizedLoss < lastInboundCumulativeLoss) revert StaleValuation();
        lastInboundCumulativeLoss = body.realizedLoss;

        _advanceInbound(encoded);
        emit RemoteReportAccepted(
            header.sequence,
            body.epochId,
            _vault.acceptedRemoteValue(),
            body.latestCompletedTransferId
        );
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

        _vault.applyRecallSent(body.principalSent, body.actualAmountSent, body.realizedLoss);
        emit RecallSentAccepted(
            body.transferId, body.principalSent, body.actualAmountSent, body.realizedLoss
        );
        _clearCycle();
        _advanceInbound(encoded);
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
        if (header.expiresAt < uint64(block.timestamp)) {
            revert MessageExpired(uint64(block.timestamp), header.expiresAt);
        }
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
