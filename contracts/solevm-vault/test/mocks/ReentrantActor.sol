// SPDX-License-Identifier: Apache-2.0
pragma solidity 0.8.30;

import {ERC20} from "@openzeppelin/contracts/token/ERC20/ERC20.sol";

/// @notice Six decimal asset that calls a target while a transfer is running.
contract CallbackAsset is ERC20 {
    address public target;
    bytes public payload;
    bool private _inside;

    constructor() ERC20("Callback USD", "cUSD") {}

    function decimals() public pure override returns (uint8) {
        return 6;
    }

    function mint(address to, uint256 amount) external {
        _mint(to, amount);
    }

    function arm(address callTarget, bytes calldata callData) external {
        target = callTarget;
        payload = callData;
    }

    function _update(address from, address to, uint256 value) internal override {
        super._update(from, to, value);
        if (target == address(0) || _inside) return;
        _inside = true;
        (bool ok, bytes memory reason) = target.call(payload);
        _inside = false;
        if (!ok) {
            assembly ("memory-safe") {
                revert(add(reason, 0x20), mload(reason))
            }
        }
    }
}
