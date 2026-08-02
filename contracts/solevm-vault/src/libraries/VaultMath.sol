// SPDX-License-Identifier: Apache-2.0
pragma solidity 0.8.30;

import {Math} from "@openzeppelin/contracts/utils/math/Math.sol";

/// @title Share and asset conversion with a fixed virtual offset.
/// @notice Both directions round down, so rounding always favours the vault.
library VaultMath {
    /// One virtual asset unit keeps the denominator away from zero.
    uint256 internal constant VIRTUAL_ASSETS = 1;

    /// Offset for the twelve decimal gap between a six decimal asset and an
    /// eighteen decimal share.
    uint256 internal constant VIRTUAL_SHARES = 1e12;

    /// @notice Shares owed for an asset amount at the given basis.
    function assetsToShares(uint256 assets, uint256 totalAssets, uint256 totalSupply)
        internal
        pure
        returns (uint256)
    {
        return Math.mulDiv(assets, totalSupply + VIRTUAL_SHARES, totalAssets + VIRTUAL_ASSETS);
    }

    /// @notice Assets owed for a share amount at the given basis.
    function sharesToAssets(uint256 shares, uint256 totalAssets, uint256 totalSupply)
        internal
        pure
        returns (uint256)
    {
        return Math.mulDiv(shares, totalAssets + VIRTUAL_ASSETS, totalSupply + VIRTUAL_SHARES);
    }
}
