// SPDX-License-Identifier: Apache-2.0
pragma solidity 0.8.30;

import {ERC20} from "@openzeppelin/contracts/token/ERC20/ERC20.sol";

/// @notice Plain six decimal test asset with open minting.
contract MockAsset is ERC20 {
    constructor() ERC20("Mock USD", "mUSD") {}

    function decimals() public pure override returns (uint8) {
        return 6;
    }

    function mint(address to, uint256 amount) external {
        _mint(to, amount);
    }
}

/// @notice Asset that keeps part of every transfer, which the vault rejects.
contract FeeOnTransferAsset is ERC20 {
    uint256 public fee;

    constructor(uint256 transferFee) ERC20("Fee USD", "fUSD") {
        fee = transferFee;
    }

    function decimals() public pure override returns (uint8) {
        return 6;
    }

    function mint(address to, uint256 amount) external {
        _mint(to, amount);
    }

    function _update(address from, address to, uint256 value) internal override {
        if (from == address(0) || to == address(0) || value <= fee) {
            super._update(from, to, value);
            return;
        }
        super._update(from, to, value - fee);
        super._update(from, address(0xdead), fee);
    }
}

/// @notice Asset with the wrong decimals, used to check constructor guards.
contract EighteenDecimalAsset is ERC20 {
    constructor() ERC20("Wide USD", "wUSD") {}

    function mint(address to, uint256 amount) external {
        _mint(to, amount);
    }
}
