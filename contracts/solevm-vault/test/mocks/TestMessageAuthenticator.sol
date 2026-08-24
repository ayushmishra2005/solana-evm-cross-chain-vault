// SPDX-License-Identifier: Apache-2.0
pragma solidity 0.8.30;

import {IMessageAuthenticator} from "../../src/interfaces/IMessageAuthenticator.sol";

/// @notice Test-only transport stub. It is not a bridge.
contract TestMessageAuthenticator is IMessageAuthenticator {
    mapping(bytes32 => bool) public approved;
    bool public approveAll;

    function setApproveAll(bool enabled) external {
        approveAll = enabled;
    }

    function approve(bytes calldata message) external {
        approved[keccak256(message)] = true;
    }

    function approveHash(bytes32 hash) external {
        approved[hash] = true;
    }

    function authenticate(bytes calldata message) external view returns (bool) {
        return approveAll || approved[keccak256(message)];
    }
}
