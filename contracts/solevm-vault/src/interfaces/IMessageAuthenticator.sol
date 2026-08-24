// SPDX-License-Identifier: Apache-2.0
pragma solidity 0.8.30;

/// @title Transport authenticity check.
/// @notice Returns whether a byte string was delivered by the configured
/// transport. It must not interpret accounting fields or rewrite amounts.
interface IMessageAuthenticator {
    function authenticate(bytes calldata message) external view returns (bool);
}
