// SPDX-License-Identifier: Apache-2.0
pragma solidity 0.8.30;

/// @title EVM coordinator for one remote Solana leg.
interface IRemoteAccounting {
    enum CycleKind {
        None,
        Allocate,
        Recall
    }

    /// Why an authentic, in-order report advanced the lane without pricing.
    enum ReportSkipReason {
        None,
        Expired,
        WrongEpoch,
        StaleObservation,
        ValuationFixed
    }

    struct Identities {
        uint32 evmChainId;
        uint32 solanaChainId;
        bytes32 evmApplication;
        bytes32 solanaApplication;
        bytes32 deploymentId;
        bytes32 vaultId;
        uint32 outboundLaneId;
        uint32 inboundLaneId;
        bytes32 expectedDestinationReference;
    }

    event CanonicalMessagePublished(
        uint8 indexed messageType, uint64 indexed sequence, bytes message
    );
    event AllocationInitiated(bytes32 indexed transferId, uint256 amount, bytes message);
    event AllocationCompleted(bytes32 indexed transferId, uint256 amount, uint256 acceptedValue);
    event RecallInitiated(bytes32 indexed transferId, uint256 amount, bytes message);
    event RecallSentAccepted(
        bytes32 indexed transferId,
        uint256 principalSent,
        uint256 actualAmountSent,
        uint256 realizedLoss
    );
    event RemoteReportAccepted(
        uint64 indexed sequence,
        uint64 indexed epochId,
        uint256 acceptedValue,
        bytes32 latestCompletedTransferId
    );
    event RemoteReportSkipped(
        uint64 indexed sequence, uint64 indexed epochId, ReportSkipReason reason
    );
    event EpochRemoteValuationLocked(uint64 indexed epochId, uint256 acceptedValue);

    function vault() external view returns (address);
    function hasActiveRecall() external view returns (bool);
    function hasActiveAllocate() external view returns (bool);
    function activeTransferId() external view returns (bytes32);
    function activeCycleKind() external view returns (CycleKind);
}
