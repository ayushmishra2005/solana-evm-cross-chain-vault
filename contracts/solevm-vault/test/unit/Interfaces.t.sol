// SPDX-License-Identifier: Apache-2.0
pragma solidity 0.8.30;

import {IERC1363} from "@openzeppelin/contracts/interfaces/IERC1363.sol";
import {IERC20} from "@openzeppelin/contracts/token/ERC20/IERC20.sol";
import {IERC20Metadata} from "@openzeppelin/contracts/token/ERC20/extensions/IERC20Metadata.sol";
import {ReentrancyGuard} from "@openzeppelin/contracts/utils/ReentrancyGuard.sol";
import {IERC165} from "@openzeppelin/contracts/utils/introspection/IERC165.sol";

import {SolEVMVault} from "../../src/SolEVMVault.sol";
import {ISolEVMVault} from "../../src/interfaces/ISolEVMVault.sol";
import {VaultTestBase} from "../VaultTestBase.sol";
import {CallbackAsset} from "../mocks/ReentrantActor.sol";

contract InterfacesTest is VaultTestBase {
    /// Rebuilds the vault interface id from the selectors it actually exposes.
    function _vaultInterfaceId() private pure returns (bytes4) {
        return ISolEVMVault.requestDeposit.selector ^ ISolEVMVault.cancelDepositRequest.selector
            ^ ISolEVMVault.requestRedeem.selector ^ ISolEVMVault.cancelRedeemRequest.selector
            ^ ISolEVMVault.cutoffEpoch.selector ^ ISolEVMVault.finalizeEpoch.selector
            ^ ISolEVMVault.openNextEpoch.selector ^ ISolEVMVault.abortEpoch.selector
            ^ ISolEVMVault.claimDeposit.selector ^ ISolEVMVault.claimRedeem.selector
            ^ ISolEVMVault.refundDeposit.selector ^ ISolEVMVault.refundRedeem.selector
            ^ ISolEVMVault.pause.selector ^ ISolEVMVault.unpause.selector
            ^ ISolEVMVault.freeze.selector ^ ISolEVMVault.reconcile.selector
            ^ ISolEVMVault.managedNav.selector;
    }

    function _erc20InterfaceId() private pure returns (bytes4) {
        return IERC20.totalSupply.selector ^ IERC20.balanceOf.selector ^ IERC20.transfer.selector
            ^ IERC20.allowance.selector ^ IERC20.approve.selector ^ IERC20.transferFrom.selector;
    }

    function test_the_vault_interface_id_matches_its_selectors() public pure {
        assertEq(type(ISolEVMVault).interfaceId, _vaultInterfaceId());
    }

    function test_the_vault_reports_the_interfaces_it_implements() public view {
        assertTrue(vault.supportsInterface(_vaultInterfaceId()), "vault interface missing");
        assertTrue(vault.supportsInterface(_erc20InterfaceId()), "erc20 interface missing");
        assertTrue(
            vault.supportsInterface(type(IERC20Metadata).interfaceId), "metadata interface missing"
        );
        assertTrue(vault.supportsInterface(type(IERC165).interfaceId), "erc165 interface missing");
    }

    function test_the_vault_does_not_claim_interfaces_it_lacks() public view {
        assertFalse(vault.supportsInterface(0xffffffff));
        assertFalse(vault.supportsInterface(type(IERC1363).interfaceId));

        // Synchronous ERC-4626 entry and exit are absent by design.
        assertFalse(vault.supportsInterface(bytes4(keccak256("deposit(uint256,address)"))));
    }

    // Permit

    function test_permit_lets_a_spender_move_shares() public {
        (address owner, uint256 key) = makeAddrAndKey("permit-owner");
        token.mint(owner, FUNDING);
        vm.prank(owner);
        token.approve(address(vault), type(uint256).max);

        _deposit(owner, 5e6);
        _settle();
        vm.prank(owner);
        vault.claimDeposit(0);

        uint256 deadline = block.timestamp + 1 hours;
        bytes32 structHash = keccak256(
            abi.encode(
                keccak256(
                    "Permit(address owner,address spender,uint256 value,uint256 nonce,uint256 deadline)"
                ),
                owner,
                bob,
                4e18,
                vault.nonces(owner),
                deadline
            )
        );
        bytes32 digest =
            keccak256(abi.encodePacked("\x19\x01", vault.DOMAIN_SEPARATOR(), structHash));
        (uint8 v, bytes32 r, bytes32 s) = vm.sign(key, digest);

        vault.permit(owner, bob, 4e18, deadline, v, r, s);
        assertEq(vault.allowance(owner, bob), 4e18);

        vm.prank(bob);
        vault.transferFrom(owner, bob, 4e18);
        assertEq(vault.balanceOf(bob), 4e18);
    }

    function test_a_permit_signature_cannot_be_replayed() public {
        (address owner, uint256 key) = makeAddrAndKey("replay-owner");
        uint256 deadline = block.timestamp + 1 hours;
        bytes32 structHash = keccak256(
            abi.encode(
                keccak256(
                    "Permit(address owner,address spender,uint256 value,uint256 nonce,uint256 deadline)"
                ),
                owner,
                bob,
                1e18,
                vault.nonces(owner),
                deadline
            )
        );
        bytes32 digest =
            keccak256(abi.encodePacked("\x19\x01", vault.DOMAIN_SEPARATOR(), structHash));
        (uint8 v, bytes32 r, bytes32 s) = vm.sign(key, digest);

        vault.permit(owner, bob, 1e18, deadline, v, r, s);
        assertEq(vault.nonces(owner), 1);

        vm.expectRevert();
        vault.permit(owner, bob, 1e18, deadline, v, r, s);
    }

    // Reentrancy

    function _callbackVault()
        private
        returns (SolEVMVault reenterable, CallbackAsset callbackAsset)
    {
        callbackAsset = new CallbackAsset();
        reenterable = new SolEVMVault(
            callbackAsset,
            admin,
            guardian,
            EPOCH_DURATION,
            MIN_DEPOSIT,
            MIN_REDEEM,
            1,
            "callback",
            "cb"
        );
        callbackAsset.mint(alice, FUNDING);
        vm.prank(alice);
        callbackAsset.approve(address(reenterable), type(uint256).max);
    }

    function test_a_reentrant_call_during_a_deposit_pull_reverts() public {
        (SolEVMVault reenterable, CallbackAsset callbackAsset) = _callbackVault();
        callbackAsset.arm(address(reenterable), abi.encodeCall(ISolEVMVault.reconcile, ()));

        vm.expectRevert(ReentrancyGuard.ReentrancyGuardReentrantCall.selector);
        vm.prank(alice);
        reenterable.requestDeposit(5e6);
    }

    function test_a_reentrant_call_during_a_redemption_payout_reverts() public {
        (SolEVMVault reenterable, CallbackAsset callbackAsset) = _callbackVault();

        vm.prank(alice);
        reenterable.requestDeposit(10e6);
        vm.warp(reenterable.currentEpochCutoffAt());
        reenterable.cutoffEpoch();
        reenterable.finalizeEpoch();
        vm.prank(alice);
        reenterable.claimDeposit(0);
        reenterable.openNextEpoch();

        vm.prank(alice);
        reenterable.requestRedeem(4e18);
        vm.warp(reenterable.currentEpochCutoffAt());
        reenterable.cutoffEpoch();
        reenterable.finalizeEpoch();

        callbackAsset.arm(address(reenterable), abi.encodeCall(ISolEVMVault.reconcile, ()));
        vm.expectRevert(ReentrancyGuard.ReentrancyGuardReentrantCall.selector);
        vm.prank(alice);
        reenterable.claimRedeem(1);
    }

    function test_a_reentrant_call_during_a_cancellation_refund_reverts() public {
        (SolEVMVault reenterable, CallbackAsset callbackAsset) = _callbackVault();
        vm.prank(alice);
        reenterable.requestDeposit(5e6);

        callbackAsset.arm(address(reenterable), abi.encodeCall(ISolEVMVault.finalizeEpoch, ()));
        vm.expectRevert(ReentrancyGuard.ReentrancyGuardReentrantCall.selector);
        vm.prank(alice);
        reenterable.cancelDepositRequest();
    }

    function test_a_claim_still_works_when_the_asset_does_not_reenter() public {
        (SolEVMVault reenterable,) = _callbackVault();
        vm.prank(alice);
        reenterable.requestDeposit(10e6);
        vm.warp(reenterable.currentEpochCutoffAt());
        reenterable.cutoffEpoch();
        reenterable.finalizeEpoch();

        vm.prank(alice);
        assertEq(reenterable.claimDeposit(0), 10e18);
    }
}
