// SPDX-License-Identifier: Apache-2.0
pragma solidity 0.8.30;

import {RemoteTestBase} from "../RemoteTestBase.sol";

contract RemoteFuzzTest is RemoteTestBase {
    bytes32 internal constant TID_1 = bytes32(uint256(1));
    bytes32 internal constant TID_2 = bytes32(uint256(2));

    function testFuzz_accepted_remote_value_never_exceeds_cap(uint128 reported) public {
        _bootstrap(alice, 100e6);
        uint128 amount = 40e6;
        _allocate(TID_1, amount);
        _report(1, amount, reported, 0, TID_1);
        assertLe(vault.acceptedRemoteValue(), vault.remotePrincipalCap());
        assertEq(vault.remotePrincipalCap(), amount);
        assertEq(vault.outboundInTransit(), 0);
        uint256 expected = uint256(reported) < amount ? reported : amount;
        assertEq(vault.acceptedRemoteValue(), expected);
        assertEq(_managed(), 60e6 + expected);
    }

    function testFuzz_location_change_does_not_increase_nav(uint128 amount) public {
        amount = uint128(bound(amount, 1e6, 80e6));
        _bootstrap(alice, 100e6);
        uint256 nav = _managed();
        _allocate(TID_1, amount);
        assertEq(_managed(), nav);
        _report(1, amount, amount, 0, TID_1);
        assertEq(_managed(), nav);
    }

    function testFuzz_recall_sent_reduces_nav_by_loss(uint128 actual) public {
        _bootstrap(alice, 100e6);
        uint128 principal = 40e6;
        actual = uint128(bound(actual, 0, principal));
        uint128 loss = principal - actual;
        _allocate(TID_1, principal);
        _report(1, principal, principal, 0, TID_1);
        _recall(TID_2, principal);
        uint256 nav = _managed();
        _recallSent(TID_2, principal, actual, loss);
        assertEq(_managed(), nav - loss);
        assertEq(vault.inboundInTransit(), actual);
        assertEq(vault.recognizedLoss(), loss);
    }

    function testFuzz_unattributed_assets_do_not_move_price(uint128 gift) public {
        gift = uint128(bound(gift, 1, 1_000_000e6));
        _bootstrap(alice, 100e6);
        uint256 nav = _managed();
        uint256 supply = vault.totalSupply();
        _observe(gift);
        assertEq(_managed(), nav);
        assertEq(vault.totalSupply(), supply);
        assertEq(vault.unattributedBalance(), gift);
    }

    function testFuzz_duplicate_report_is_financially_idempotent(uint128 reported) public {
        _bootstrap(alice, 100e6);
        _allocate(TID_1, 40e6);
        bytes memory encoded = _report(1, 40e6, reported, 0, TID_1);
        uint256 nav = _managed();
        uint256 accepted = vault.acceptedRemoteValue();
        uint64 seq = coordinator.inboundExpectedSequence();
        vm.expectRevert();
        coordinator.processRemoteReport(encoded);
        assertEq(_managed(), nav);
        assertEq(vault.acceptedRemoteValue(), accepted);
        assertEq(coordinator.inboundExpectedSequence(), seq);
    }
}
