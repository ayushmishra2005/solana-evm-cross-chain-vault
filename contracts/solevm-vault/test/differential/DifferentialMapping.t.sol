// SPDX-License-Identifier: Apache-2.0
pragma solidity 0.8.30;

import {Test} from "forge-std/Test.sol";

import {IERC20Errors} from "@openzeppelin/contracts/interfaces/draft-IERC6093.sol";

import {ISolEVMVault} from "../../src/interfaces/ISolEVMVault.sol";
import {
    Action,
    Code,
    EpochActor,
    EpochTerms,
    Op,
    Scenario,
    Snapshot
} from "./DifferentialTypes.sol";
import {Actors, ResultMapping} from "./ResultMapping.sol";

/// @notice Guards the pieces the differential harness relies on. None of this
/// needs the generator, so it runs in the normal test suite.
contract DifferentialMappingTest is Test {
    /// Every error a shared operation can raise must have a code.
    function test_every_shared_vault_error_maps_to_a_known_code() public pure {
        bytes4[22] memory selectors = [
            ISolEVMVault.Unauthorized.selector,
            ISolEVMVault.InvalidVaultStatus.selector,
            ISolEVMVault.EpochNotOpen.selector,
            ISolEVMVault.EpochAlreadyOpen.selector,
            ISolEVMVault.EpochAlreadyCutOff.selector,
            ISolEVMVault.EpochNotCutOff.selector,
            ISolEVMVault.EpochAlreadySettled.selector,
            ISolEVMVault.CutoffNotReached.selector,
            ISolEVMVault.CancellationAfterCutoff.selector,
            ISolEVMVault.EpochNotFinalized.selector,
            ISolEVMVault.EpochNotAborted.selector,
            ISolEVMVault.RequestNotFound.selector,
            ISolEVMVault.RequestAlreadyCancelled.selector,
            ISolEVMVault.ClaimAlreadyConsumed.selector,
            ISolEVMVault.RefundAlreadyConsumed.selector,
            ISolEVMVault.ZeroAmount.selector,
            ISolEVMVault.AmountBelowMinimum.selector,
            ISolEVMVault.InsufficientBalance.selector,
            ISolEVMVault.InsufficientLiquidity.selector,
            ISolEVMVault.TimestampNotMonotonic.selector,
            ISolEVMVault.InvariantViolation.selector,
            IERC20Errors.ERC20InsufficientBalance.selector
        ];

        for (uint256 i = 0; i < selectors.length; ++i) {
            uint8 code = ResultMapping.codeFor(selectors[i]);
            assertTrue(code != Code.UNKNOWN, "selector has no code");
            assertTrue(code != Code.SUCCESS, "a revert cannot be a success");
            assertTrue(
                keccak256(bytes(Code.name(code))) != keccak256(bytes("Unknown")), "code is unnamed"
            );
        }
    }

    /// Errors outside the shared operation set must stay unmapped, so the
    /// harness stops instead of guessing.
    function test_errors_outside_the_shared_set_stay_unmapped() public pure {
        assertEq(ResultMapping.codeFor(ISolEVMVault.InvalidAddress.selector), Code.UNKNOWN);
        assertEq(ResultMapping.codeFor(ISolEVMVault.RequestLimitReached.selector), Code.UNKNOWN);
        assertEq(ResultMapping.codeFor(ISolEVMVault.InvalidTransferAmount.selector), Code.UNKNOWN);
        assertEq(ResultMapping.codeFor(ISolEVMVault.AccountingDeficit.selector), Code.UNKNOWN);
        assertEq(ResultMapping.codeFor(bytes4(0)), Code.UNKNOWN);
        assertEq(ResultMapping.codeFor(bytes4(0xdeadbeef)), Code.UNKNOWN);
    }

    /// A spent claim and a spent refund must stay apart.
    function test_a_spent_claim_and_a_spent_refund_use_different_codes() public pure {
        assertTrue(
            ResultMapping.codeFor(ISolEVMVault.ClaimAlreadyConsumed.selector)
                != ResultMapping.codeFor(ISolEVMVault.RefundAlreadyConsumed.selector)
        );
    }

    /// Missing share balance and missing asset balance must stay apart.
    function test_asset_and_share_shortfalls_use_different_codes() public pure {
        assertEq(
            ResultMapping.codeFor(ISolEVMVault.InsufficientBalance.selector),
            Code.INSUFFICIENT_SHARE_BALANCE
        );
        assertEq(
            ResultMapping.codeFor(IERC20Errors.ERC20InsufficientBalance.selector),
            Code.INSUFFICIENT_ASSET_BALANCE
        );
    }

    function test_the_selector_reader_handles_short_data() public pure {
        assertEq(ResultMapping.selectorOf(hex""), bytes4(0));
        assertEq(ResultMapping.selectorOf(hex"010203"), bytes4(0));
        assertEq(ResultMapping.selectorOf(hex"0102030405"), bytes4(hex"01020304"));
    }

    /// Account id `n` in the model must always give the same address.
    function test_actor_addresses_are_stable() public pure {
        assertEq(Actors.addressOf(0), address(0x1001));
        assertEq(Actors.addressOf(1), address(0x1002));
        assertEq(Actors.addressOf(2), address(0x1003));
        assertEq(Actors.addressOf(3), address(0x1004));
        assertEq(Actors.addressOf(Actors.ADMIN_SLOT), address(0xA1));
        assertEq(Actors.addressOf(Actors.GUARDIAN_SLOT), address(0xA2));
    }

    function test_every_actor_slot_has_its_own_address() public pure {
        address[6] memory seen;
        for (uint8 slot = 0; slot < 6; ++slot) {
            address who = Actors.addressOf(slot);
            assertTrue(who != address(0), "actor is the zero address");
            for (uint8 other = 0; other < slot; ++other) {
                assertTrue(who != seen[other], "two slots share an address");
            }
            seen[slot] = who;
        }
    }

    function test_result_codes_have_unique_names() public pure {
        uint8[24] memory codes = [
            Code.SUCCESS,
            Code.INVALID_VAULT_STATE,
            Code.UNAUTHORIZED,
            Code.EPOCH_NOT_OPEN,
            Code.EPOCH_ALREADY_OPEN,
            Code.EPOCH_ALREADY_CUT_OFF,
            Code.EPOCH_NOT_CUT_OFF,
            Code.EPOCH_ALREADY_SETTLED,
            Code.CUTOFF_NOT_REACHED,
            Code.CANCELLATION_AFTER_CUTOFF,
            Code.EPOCH_NOT_FINALIZED,
            Code.EPOCH_NOT_ABORTED,
            Code.REQUEST_NOT_FOUND,
            Code.REQUEST_NOT_ACTIVE,
            Code.CLAIM_ALREADY_CONSUMED,
            Code.REFUND_ALREADY_CONSUMED,
            Code.ZERO_AMOUNT,
            Code.AMOUNT_BELOW_MINIMUM,
            Code.INSUFFICIENT_ASSET_BALANCE,
            Code.INSUFFICIENT_SHARE_BALANCE,
            Code.INSUFFICIENT_LIQUIDITY,
            Code.TIMESTAMP_NOT_MONOTONIC,
            Code.INVALID_CONFIGURATION,
            Code.ARITHMETIC_FAILURE
        ];

        for (uint256 i = 0; i < codes.length; ++i) {
            assertEq(uint256(codes[i]), i, "codes must keep their wire numbers");
            for (uint256 j = i + 1; j < codes.length; ++j) {
                assertTrue(
                    keccak256(bytes(Code.name(codes[i]))) != keccak256(bytes(Code.name(codes[j]))),
                    "two codes share a name"
                );
            }
        }
    }

    function test_operation_codes_have_unique_names() public pure {
        for (uint8 kind = 0; kind < 15; ++kind) {
            string memory name = Op.name(kind);
            assertTrue(keccak256(bytes(name)) != keccak256(bytes("Unknown")), "kind is unnamed");
            for (uint8 other = 0; other < kind; ++other) {
                assertTrue(
                    keccak256(bytes(name)) != keccak256(bytes(Op.name(other))),
                    "two kinds share a name"
                );
            }
        }
        assertEq(Op.name(15), "Unknown");
    }

    /// The scenario layout must survive an encode and decode round trip, so a
    /// field added on one side and not the other shows up here.
    function test_the_scenario_layout_round_trips() public pure {
        Scenario memory scenario;
        scenario.index = 7;
        scenario.seed = 12_345;
        scenario.family = 3;
        scenario.startTimestamp = 1_700_000_000;
        scenario.epochDuration = 3600;
        scenario.minDeposit = 1_000_000;
        scenario.minRedeem = 1_000_000_000_000;
        scenario.configVersion = 1;
        scenario.initialAssets = [uint256(1), 2, 3, 4];

        scenario.actions = new Action[](1);
        scenario.actions[0] = Action({
            kind: Op.CLAIM_REDEEM,
            actor: 2,
            amount: 99,
            epochId: 5,
            timestamp: 1_700_000_500,
            expectedResult: Code.CLAIM_ALREADY_CONSUMED,
            expectedReturn: 77
        });

        scenario.snapshots = new Snapshot[](1);
        scenario.snapshots[0].idleBacking = 42;
        scenario.snapshots[0].consumedMask = 1 << 130;
        scenario.snapshots[0].actorShares = [uint256(9), 8, 7, 6];

        scenario.epochs = new EpochTerms[](1);
        scenario.epochs[0].epochId = 5;
        scenario.epochs[0].outcome = 1;
        scenario.epochs[0].settledAtStep = 3;
        scenario.epochs[0].cutoffAt = 1_700_003_600;
        scenario.epochs[0].depositDust = 2;
        scenario.epochs[0].actors[1] =
            EpochActor({depositAssets: 11, redeemShares: 12, claimShares: 13, claimAssets: 14});

        Scenario memory back = abi.decode(abi.encode(scenario), (Scenario));

        assertEq(back.index, scenario.index);
        assertEq(back.seed, scenario.seed);
        assertEq(back.initialAssets[3], 4);
        assertEq(back.actions[0].expectedResult, Code.CLAIM_ALREADY_CONSUMED);
        assertEq(back.actions[0].expectedReturn, 77);
        assertEq(back.snapshots[0].idleBacking, 42);
        assertEq(back.snapshots[0].consumedMask, 1 << 130);
        assertEq(back.snapshots[0].actorShares[0], 9);
        assertEq(back.epochs[0].cutoffAt, 1_700_003_600);
        assertEq(back.epochs[0].depositDust, 2);
        assertEq(back.epochs[0].actors[1].claimAssets, 14);
    }
}
