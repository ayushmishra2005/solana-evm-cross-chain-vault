// SPDX-License-Identifier: Apache-2.0
pragma solidity 0.8.30;

import {RemoteAccounting} from "../../src/RemoteAccounting.sol";
import {SolEVMVault} from "../../src/SolEVMVault.sol";
import {IRemoteAccounting} from "../../src/interfaces/IRemoteAccounting.sol";
import {EpochPhase} from "../../src/interfaces/ISolEVMVault.sol";
import {ProtocolCodec} from "../../src/libraries/ProtocolCodec.sol";
import {VaultTestBase} from "../VaultTestBase.sol";
import {TestMessageAuthenticator} from "../mocks/TestMessageAuthenticator.sol";

/// Binding is one-shot and only legal before any economic activity.
contract RemoteBindTest is VaultTestBase {
    uint32 internal constant EVM_CHAIN = 1;
    uint32 internal constant SOLANA_CHAIN = 2;
    uint32 internal constant OUTBOUND_LANE = 1;
    uint32 internal constant INBOUND_LANE = 2;

    function _deployCoordinator() internal returns (RemoteAccounting coordinator) {
        TestMessageAuthenticator authenticator = new TestMessageAuthenticator();
        authenticator.setApproveAll(true);
        IRemoteAccounting.Identities memory identities = IRemoteAccounting.Identities({
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
        coordinator =
            new RemoteAccounting(vault, authenticator, makeAddr("outbound-sink"), identities);
    }

    function test_first_valid_binding_succeeds() public {
        RemoteAccounting coordinator = _deployCoordinator();
        assertEq(vault.remoteAccounting(), address(0));
        vm.prank(admin);
        vault.bindRemoteAccounting(address(coordinator));
        assertEq(vault.remoteAccounting(), address(coordinator));
    }

    function test_second_binding_fails() public {
        RemoteAccounting coordinator = _deployCoordinator();
        vm.prank(admin);
        vault.bindRemoteAccounting(address(coordinator));
        RemoteAccounting other = _deployCoordinator();
        vm.prank(admin);
        vm.expectRevert(SolEVMVault.RemoteAccountingAlreadyBound.selector);
        vault.bindRemoteAccounting(address(other));
        assertEq(vault.remoteAccounting(), address(coordinator));
    }

    function test_binding_after_deposit_fails() public {
        _deposit(alice, 10e6);
        RemoteAccounting coordinator = _deployCoordinator();
        vm.prank(admin);
        vm.expectRevert(SolEVMVault.RemoteAccountingBindAfterActivity.selector);
        vault.bindRemoteAccounting(address(coordinator));
        assertEq(vault.remoteAccounting(), address(0));
    }

    function test_binding_after_share_supply_exists_fails() public {
        _bootstrap(alice, 100e6);
        assertGt(vault.totalSupply(), 0);
        RemoteAccounting coordinator = _deployCoordinator();
        vm.prank(admin);
        vm.expectRevert(SolEVMVault.RemoteAccountingBindAfterActivity.selector);
        vault.bindRemoteAccounting(address(coordinator));
        assertEq(vault.remoteAccounting(), address(0));
    }

    function test_binding_after_epoch_activity_fails() public {
        vm.warp(vault.currentEpochCutoffAt());
        vault.cutoffEpoch();
        assertEq(uint8(vault.currentEpochPhase()), uint8(EpochPhase.CutOff));
        RemoteAccounting coordinator = _deployCoordinator();
        vm.prank(admin);
        vm.expectRevert(SolEVMVault.RemoteAccountingBindAfterActivity.selector);
        vault.bindRemoteAccounting(address(coordinator));
        assertEq(vault.remoteAccounting(), address(0));
    }
}
