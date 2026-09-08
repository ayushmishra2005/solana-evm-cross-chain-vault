// SPDX-License-Identifier: Apache-2.0
pragma solidity 0.8.30;

import {RemoteAccounting} from "../src/RemoteAccounting.sol";
import {IRemoteAccounting} from "../src/interfaces/IRemoteAccounting.sol";
import {ProtocolCodec} from "../src/libraries/ProtocolCodec.sol";
import {VaultTestBase} from "./VaultTestBase.sol";
import {TestMessageAuthenticator} from "./mocks/TestMessageAuthenticator.sol";

/// Shared fixture for the remote coordinator and its test authenticator.
abstract contract RemoteTestBase is VaultTestBase {
    uint32 internal constant EVM_CHAIN = 1;
    uint32 internal constant SOLANA_CHAIN = 2;
    uint32 internal constant OUTBOUND_LANE = 1;
    uint32 internal constant INBOUND_LANE = 2;

    TestMessageAuthenticator internal authenticator;
    RemoteAccounting internal coordinator;
    address internal sink;
    bytes32 internal deploymentId;
    bytes32 internal vaultId;
    bytes32 internal destRef;
    bytes32 internal evmApp;
    bytes32 internal solanaApp;

    function setUp() public virtual override {
        super.setUp();
        vm.warp(1_700_000_000);

        sink = makeAddr("outbound-sink");
        authenticator = new TestMessageAuthenticator();
        authenticator.setApproveAll(true);

        IRemoteAccounting.Identities memory identities = _identities();
        evmApp = identities.evmApplication;
        solanaApp = identities.solanaApplication;
        deploymentId = identities.deploymentId;
        vaultId = identities.vaultId;
        destRef = identities.expectedDestinationReference;

        coordinator = new RemoteAccounting(vault, authenticator, sink, identities);
        vm.prank(admin);
        vault.bindRemoteAccounting(address(coordinator));
    }

    function _identities()
        internal
        virtual
        returns (IRemoteAccounting.Identities memory identities)
    {
        identities =
            IRemoteAccounting.Identities({
                evmChainId: EVM_CHAIN,
                solanaChainId: SOLANA_CHAIN,
                evmApplication: ProtocolCodec.applicationFromAddress(makeAddr("evm-app")),
                solanaApplication: keccak256("solana-app"),
                deploymentId: keccak256("deployment"),
                vaultId: keccak256("vault"),
                outboundLaneId: OUTBOUND_LANE,
                inboundLaneId: INBOUND_LANE,
                expectedDestinationReference: keccak256("destination-ref")
            });
    }

    function _times()
        internal
        view
        returns (uint64 observedAt, uint64 publishedAt, uint64 expiresAt, uint64 deadline)
    {
        observedAt = uint64(block.timestamp);
        publishedAt = observedAt;
        expiresAt = observedAt + 7 days;
        deadline = publishedAt + 1 days;
    }

    function _allocate(bytes32 transferId, uint128 amount) internal returns (bytes memory message) {
        (uint64 observedAt, uint64 publishedAt, uint64 expiresAt, uint64 deadline) = _times();
        vm.prank(admin);
        message = coordinator.allocate(
            transferId, amount, amount, deadline, observedAt, publishedAt, expiresAt
        );
    }

    function _recall(bytes32 transferId, uint128 amount) internal returns (bytes memory message) {
        (uint64 observedAt, uint64 publishedAt, uint64 expiresAt, uint64 deadline) = _times();
        vm.prank(admin);
        message =
            coordinator.recall(transferId, amount, 1, deadline, observedAt, publishedAt, expiresAt);
    }

    function _inboundHeader() internal view returns (ProtocolCodec.Header memory header) {
        uint64 nowAt = uint64(block.timestamp);
        header = ProtocolCodec.Header({
            sourceChain: coordinator.solanaChainId(),
            destinationChain: coordinator.evmChainId(),
            sourceApplication: solanaApp,
            destinationApplication: evmApp,
            deploymentId: deploymentId,
            vaultId: vaultId,
            laneId: coordinator.inboundLaneId(),
            sequence: coordinator.inboundExpectedSequence(),
            previousCommitment: coordinator.inboundLatestCommitment(),
            observedAt: nowAt,
            publishedAt: nowAt,
            expiresAt: nowAt + 7 days
        });
    }

    function _encodeReport(
        uint64 epochId,
        uint128 remotePrincipal,
        uint128 reportedValue,
        uint128 realizedLoss,
        bytes32 completedTransferId
    ) internal view returns (bytes memory encoded) {
        encoded = ProtocolCodec.encodeRemoteReport(
            _inboundHeader(),
            ProtocolCodec.RemoteReportBody({
                epochId: epochId == 0 ? uint64(1) : epochId,
                remotePrincipal: remotePrincipal,
                reportedValue: reportedValue,
                realizedLoss: realizedLoss,
                unattributedBalance: 0,
                latestCompletedTransferId: completedTransferId,
                probeStatus: ProtocolCodec.PROBE_NOT_REQUIRED,
                probeTimestamp: 0,
                configVersion: uint64(vault.configVersion()),
                remoteStateCommitment: keccak256("remote-state")
            })
        );
    }

    function _report(
        uint64 epochId,
        uint128 remotePrincipal,
        uint128 reportedValue,
        uint128 realizedLoss,
        bytes32 completedTransferId
    ) internal returns (bytes memory encoded) {
        encoded = _encodeReport(
            epochId, remotePrincipal, reportedValue, realizedLoss, completedTransferId
        );
        coordinator.processRemoteReport(encoded);
    }

    function _recallSent(bytes32 transferId, uint128 principal, uint128 actual, uint128 loss)
        internal
        returns (bytes memory encoded)
    {
        uint64 nowAt = uint64(block.timestamp);
        encoded = ProtocolCodec.encodeRecallSent(
            _inboundHeader(),
            ProtocolCodec.RecallSentBody({
                transferId: transferId,
                principalSent: principal,
                actualAmountSent: actual,
                realizedLoss: loss,
                destinationReference: destRef,
                sentTimestamp: nowAt,
                configVersion: uint64(vault.configVersion())
            })
        );
        coordinator.processRecallSent(encoded);
    }

    function _observe(uint256 amount) internal {
        token.mint(address(vault), amount);
        vault.observeInbound();
    }

    function _managed() internal view returns (uint256) {
        return vault.idleBacking() + vault.outboundInTransit() + vault.acceptedRemoteValue()
            + vault.inboundInTransit();
    }
}
