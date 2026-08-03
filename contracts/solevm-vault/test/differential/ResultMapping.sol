// SPDX-License-Identifier: Apache-2.0
pragma solidity 0.8.30;

import {IERC20Errors} from "@openzeppelin/contracts/interfaces/draft-IERC6093.sol";

import {ISolEVMVault} from "../../src/interfaces/ISolEVMVault.sol";
import {Code} from "./DifferentialTypes.sol";

/// @notice Turns vault revert data into the result codes the generator uses.
///
/// The two implementations name their errors differently, so a few pairs
/// deliberately share one code:
/// - a cancellation cannot tell a missing request from a spent one, so both
///   arrive as `RequestAlreadyCancelled`
/// - a spent claim and a spent refund keep separate codes, because the vault
///   raises separate errors and the generator follows the operation
///
/// Errors outside the shared operation set stay unmapped on purpose. They are
/// covered by the vault's own tests, and the harness fails loudly if one shows
/// up during a differential run.
library ResultMapping {
    function codeFor(bytes4 selector) internal pure returns (uint8) {
        if (selector == ISolEVMVault.Unauthorized.selector) return Code.UNAUTHORIZED;
        if (selector == ISolEVMVault.InvalidVaultStatus.selector) return Code.INVALID_VAULT_STATE;
        if (selector == ISolEVMVault.EpochNotOpen.selector) return Code.EPOCH_NOT_OPEN;
        if (selector == ISolEVMVault.EpochAlreadyOpen.selector) return Code.EPOCH_ALREADY_OPEN;
        if (selector == ISolEVMVault.EpochAlreadyCutOff.selector) {
            return Code.EPOCH_ALREADY_CUT_OFF;
        }
        if (selector == ISolEVMVault.EpochNotCutOff.selector) return Code.EPOCH_NOT_CUT_OFF;
        if (selector == ISolEVMVault.EpochAlreadySettled.selector) {
            return Code.EPOCH_ALREADY_SETTLED;
        }
        if (selector == ISolEVMVault.CutoffNotReached.selector) return Code.CUTOFF_NOT_REACHED;
        if (selector == ISolEVMVault.CancellationAfterCutoff.selector) {
            return Code.CANCELLATION_AFTER_CUTOFF;
        }
        if (selector == ISolEVMVault.EpochNotFinalized.selector) return Code.EPOCH_NOT_FINALIZED;
        if (selector == ISolEVMVault.EpochNotAborted.selector) return Code.EPOCH_NOT_ABORTED;
        if (selector == ISolEVMVault.RequestNotFound.selector) return Code.REQUEST_NOT_FOUND;
        if (selector == ISolEVMVault.RequestAlreadyCancelled.selector) {
            return Code.REQUEST_NOT_ACTIVE;
        }
        if (selector == ISolEVMVault.ClaimAlreadyConsumed.selector) {
            return Code.CLAIM_ALREADY_CONSUMED;
        }
        if (selector == ISolEVMVault.RefundAlreadyConsumed.selector) {
            return Code.REFUND_ALREADY_CONSUMED;
        }
        if (selector == ISolEVMVault.ZeroAmount.selector) return Code.ZERO_AMOUNT;
        if (selector == ISolEVMVault.AmountBelowMinimum.selector) return Code.AMOUNT_BELOW_MINIMUM;
        // The vault checks share balances itself.
        if (selector == ISolEVMVault.InsufficientBalance.selector) {
            return Code.INSUFFICIENT_SHARE_BALANCE;
        }
        // The asset refuses a transfer the sender cannot cover.
        if (selector == IERC20Errors.ERC20InsufficientBalance.selector) {
            return Code.INSUFFICIENT_ASSET_BALANCE;
        }
        if (selector == ISolEVMVault.InsufficientLiquidity.selector) {
            return Code.INSUFFICIENT_LIQUIDITY;
        }
        if (selector == ISolEVMVault.TimestampNotMonotonic.selector) {
            return Code.TIMESTAMP_NOT_MONOTONIC;
        }
        if (selector == ISolEVMVault.InvariantViolation.selector) return Code.ARITHMETIC_FAILURE;
        return Code.UNKNOWN;
    }

    /// Reads the leading selector without touching assembly.
    function selectorOf(bytes memory data) internal pure returns (bytes4) {
        if (data.length < 4) return bytes4(0);
        return bytes4(bytes.concat(data[0], data[1], data[2], data[3]));
    }
}

/// @notice Fixed actor slots shared with the generator.
library Actors {
    uint8 internal constant USER_COUNT = 4;
    uint8 internal constant ADMIN_SLOT = 4;
    uint8 internal constant GUARDIAN_SLOT = 5;

    address internal constant ADMIN = address(0xA1);
    address internal constant GUARDIAN = address(0xA2);

    /// Account id `n` in the model becomes `0x1000 + n` here.
    function addressOf(uint8 slot) internal pure returns (address) {
        if (slot == ADMIN_SLOT) return ADMIN;
        if (slot == GUARDIAN_SLOT) return GUARDIAN;
        return address(uint160(0x1001 + uint256(slot)));
    }
}
