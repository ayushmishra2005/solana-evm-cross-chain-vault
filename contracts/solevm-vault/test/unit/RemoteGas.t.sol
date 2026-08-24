// SPDX-License-Identifier: Apache-2.0
pragma solidity 0.8.30;

import {console} from "forge-std/console.sol";

import {RemoteTestBase} from "../RemoteTestBase.sol";

contract RemoteGasTest is RemoteTestBase {
    bytes32 internal constant TID_1 = bytes32(uint256(1));
    bytes32 internal constant TID_2 = bytes32(uint256(2));

    function test_gas_remote_hot_paths() public {
        _bootstrap(alice, 100e6);

        uint256 start = gasleft();
        _allocate(TID_1, 40e6);
        console.log("allocate", start - gasleft());

        start = gasleft();
        _report(1, 40e6, 40e6, 0, TID_1);
        console.log("processRemoteReport complete", start - gasleft());

        start = gasleft();
        _recall(TID_2, 40e6);
        console.log("recall", start - gasleft());

        start = gasleft();
        _recallSent(TID_2, 40e6, 35e6, 5e6);
        console.log("processRecallSent", start - gasleft());

        start = gasleft();
        _observe(35e6);
        console.log("observeInbound", start - gasleft());
    }
}
