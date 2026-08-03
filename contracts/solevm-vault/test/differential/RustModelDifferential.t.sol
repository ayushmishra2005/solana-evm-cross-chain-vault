// SPDX-License-Identifier: Apache-2.0
pragma solidity 0.8.30;

import {Test} from "forge-std/Test.sol";
import {console} from "forge-std/console.sol";

import {SolEVMVault} from "../../src/SolEVMVault.sol";
import {ISolEVMVault, SettledEpoch} from "../../src/interfaces/ISolEVMVault.sol";
import {MockAsset} from "../mocks/MockAsset.sol";
import {Action, Code, EpochTerms, Op, Scenario, Snapshot} from "./DifferentialTypes.sol";
import {Actors, ResultMapping} from "./ResultMapping.sol";

/// @notice Replays traces produced by the Rust accounting model against the
/// vault and compares the result and the observable state after every step.
///
/// The generator runs once through FFI and returns every scenario in one
/// bundle. Nothing here derives an expected value from the contract itself.
///
/// A mismatch is recorded instead of reverting, so the report survives and the
/// run stops at the first disagreement.
contract RustModelDifferentialTest is Test {
    uint256 private constant USER_COUNT = 4;
    uint64 private constant MAX_TRACKED_EPOCHS = 16;

    uint256 private constant DEFAULT_SEED = 1;
    uint256 private constant DEFAULT_CASES = 64;
    uint256 private constant DEFAULT_STEPS = 4;

    /// Context for the failure report. Nothing reverts, so it stays readable.
    uint256 private ctxScenario;
    uint256 private ctxStep;
    Action private ctxAction;
    bool private mismatch;

    uint256 private runSeed;
    uint256 private runCases;
    uint256 private runSteps;

    uint256 private totalOperations;
    uint256 private totalSuccesses;
    uint256 private totalRejections;

    function test_the_vault_matches_the_rust_model_on_every_step() public {
        if (!_enabled()) {
            console.log("differential run skipped, set RUN_DIFFERENTIAL=1 to enable it");
            return;
        }

        runSeed = vm.envOr("DIFF_SEED", DEFAULT_SEED);
        runCases = vm.envOr("DIFF_CASES", DEFAULT_CASES);
        runSteps = vm.envOr("DIFF_STEPS", DEFAULT_STEPS);

        bytes memory raw = _generate(runSeed, runCases, runSteps);
        (uint64 bundleSeed, uint32 count, bytes[] memory parts) =
            abi.decode(raw, (uint64, uint32, bytes[]));

        assertEq(uint256(bundleSeed), runSeed, "bundle seed");
        assertEq(uint256(count), parts.length, "bundle count");
        assertGt(parts.length, 0, "bundle is empty");

        for (uint256 i = 0; i < parts.length; ++i) {
            this.replay(parts[i]);
            if (mismatch) break;
        }

        console.log("seed", runSeed);
        console.log("scenarios", parts.length);
        console.log("operations", totalOperations);
        console.log("successes", totalSuccesses);
        console.log("rejections", totalRejections);

        assertFalse(mismatch, "the vault disagreed with the model");
        assertGt(totalOperations, 0, "no operation ran");
        assertGt(totalSuccesses, 0, "no operation succeeded");
        assertGt(totalRejections, 0, "no operation was rejected");
    }

    /// The bundle must arrive in the shape both sides agreed on.
    function test_the_generator_bundle_decodes_with_the_expected_shape() public {
        if (!_enabled()) return;

        bytes memory raw = _generate(1, 14, 4);
        (uint64 seed, uint32 count, bytes[] memory parts) =
            abi.decode(raw, (uint64, uint32, bytes[]));

        assertEq(uint256(seed), 1, "seed");
        assertEq(uint256(count), 14, "count");
        assertEq(parts.length, 14, "parts");

        bool[7] memory families;
        for (uint256 i = 0; i < parts.length; ++i) {
            Scenario memory scenario = abi.decode(parts[i], (Scenario));

            assertEq(uint256(scenario.index), i, "scenario index");
            assertLt(scenario.family, 7, "family");
            families[scenario.family] = true;
            assertGt(scenario.epochDuration, 0, "epoch duration");
            assertGt(scenario.minDeposit, 0, "minimum deposit");
            assertGt(scenario.minRedeem, 0, "minimum redemption");
            assertGt(scenario.actions.length, 0, "actions");
            assertEq(scenario.actions.length, scenario.snapshots.length, "snapshot per action");

            uint64 previous = scenario.startTimestamp;
            for (uint256 step = 0; step < scenario.actions.length; ++step) {
                Action memory action = scenario.actions[step];
                assertLt(action.kind, 15, "action kind");
                assertLt(action.actor, 6, "actor slot");
                assertLe(action.expectedResult, Code.ARITHMETIC_FAILURE, "result code");
                assertGe(action.timestamp, previous, "time moves forward");
                previous = action.timestamp;
            }

            for (uint256 e = 0; e < scenario.epochs.length; ++e) {
                uint8 outcome = scenario.epochs[e].outcome;
                assertTrue(outcome == 1 || outcome == 2, "epoch outcome");
                assertLt(scenario.epochs[e].settledAtStep, scenario.actions.length, "settled step");
            }
        }

        for (uint256 family = 0; family < families.length; ++family) {
            assertTrue(families[family], "a family never ran");
        }
    }

    /// A seed must reproduce a run exactly, and a different seed must not.
    function test_a_seed_reproduces_the_same_bundle() public {
        if (!_enabled()) return;

        bytes memory first = _generate(7, 4, 4);
        bytes memory again = _generate(7, 4, 4);
        bytes memory other = _generate(8, 4, 4);

        assertEq(keccak256(first), keccak256(again), "same seed changed the bundle");
        assertTrue(keccak256(first) != keccak256(other), "different seeds matched");
    }

    /// Runs one scenario in its own call so memory does not pile up.
    function replay(bytes calldata encoded) external {
        require(msg.sender == address(this), "internal only");
        _run(abi.decode(encoded, (Scenario)));
    }

    // Scenario execution

    function _run(Scenario memory scenario) private {
        ctxScenario = scenario.index;

        vm.warp(scenario.startTimestamp);
        MockAsset token = new MockAsset();
        SolEVMVault vault = new SolEVMVault(
            token,
            Actors.ADMIN,
            Actors.GUARDIAN,
            scenario.epochDuration,
            scenario.minDeposit,
            scenario.minRedeem,
            scenario.configVersion,
            "SolEVM Vault Share",
            "svUSD"
        );

        address[USER_COUNT] memory users;
        for (uint256 i = 0; i < USER_COUNT; ++i) {
            users[i] = Actors.addressOf(uint8(i));
            token.mint(users[i], scenario.initialAssets[i]);
            vm.prank(users[i]);
            token.approve(address(vault), type(uint256).max);
        }

        ctxStep = type(uint256).max;
        delete ctxAction;
        _compare(scenario.initialSnapshot, _capture(vault, token, users), "initial state");
        if (mismatch) return;

        for (uint256 step = 0; step < scenario.actions.length; ++step) {
            Action memory action = scenario.actions[step];
            ctxStep = step;
            ctxAction = action;

            vm.warp(action.timestamp);
            Snapshot memory before = _capture(vault, token, users);

            (uint8 code, uint256 returned) = _invoke(vault, action, users);
            Snapshot memory got = _capture(vault, token, users);
            totalOperations += 1;

            _eq(action.expectedResult, code, "result", "result code");
            if (mismatch) return;

            if (code == Code.SUCCESS) {
                totalSuccesses += 1;
                _eq(action.expectedReturn, returned, "return", "returned value");
            } else {
                totalRejections += 1;
                // A refused call must leave nothing behind.
                _compare(before, got, "rejection changed state");
            }
            if (mismatch) return;

            _compare(scenario.snapshots[step], got, "state after operation");
            if (mismatch) return;

            _compareEpochs(vault, scenario.epochs, users, step);
            if (mismatch) return;
        }
    }

    /// Sends one operation and maps the outcome onto a shared code.
    function _invoke(SolEVMVault vault, Action memory action, address[USER_COUNT] memory users)
        private
        returns (uint8 code, uint256 returned)
    {
        bytes memory payload = _payload(action);
        vm.prank(_actorAddress(action.actor, users));
        (bool ok, bytes memory data) = address(vault).call(payload);

        if (!ok) return (_codeFor(data), 0);
        if (data.length == 32) returned = abi.decode(data, (uint256));
        return (Code.SUCCESS, returned);
    }

    function _payload(Action memory action) private pure returns (bytes memory) {
        uint8 kind = action.kind;
        if (kind == Op.REQUEST_DEPOSIT) {
            return abi.encodeCall(ISolEVMVault.requestDeposit, (action.amount));
        }
        if (kind == Op.CANCEL_DEPOSIT) {
            return abi.encodeCall(ISolEVMVault.cancelDepositRequest, ());
        }
        if (kind == Op.REQUEST_REDEEM) {
            return abi.encodeCall(ISolEVMVault.requestRedeem, (action.amount));
        }
        if (kind == Op.CANCEL_REDEEM) {
            return abi.encodeCall(ISolEVMVault.cancelRedeemRequest, ());
        }
        if (kind == Op.CUTOFF_EPOCH) return abi.encodeCall(ISolEVMVault.cutoffEpoch, ());
        if (kind == Op.FINALIZE_EPOCH) return abi.encodeCall(ISolEVMVault.finalizeEpoch, ());
        if (kind == Op.OPEN_NEXT_EPOCH) return abi.encodeCall(ISolEVMVault.openNextEpoch, ());
        if (kind == Op.CLAIM_DEPOSIT) {
            return abi.encodeCall(ISolEVMVault.claimDeposit, (action.epochId));
        }
        if (kind == Op.CLAIM_REDEEM) {
            return abi.encodeCall(ISolEVMVault.claimRedeem, (action.epochId));
        }
        if (kind == Op.REFUND_DEPOSIT) {
            return abi.encodeCall(ISolEVMVault.refundDeposit, (action.epochId));
        }
        if (kind == Op.REFUND_REDEEM) {
            return abi.encodeCall(ISolEVMVault.refundRedeem, (action.epochId));
        }
        if (kind == Op.PAUSE) return abi.encodeCall(ISolEVMVault.pause, ());
        if (kind == Op.UNPAUSE) return abi.encodeCall(ISolEVMVault.unpause, ());
        if (kind == Op.FREEZE) return abi.encodeCall(ISolEVMVault.freeze, ());
        return abi.encodeCall(ISolEVMVault.abortEpoch, ());
    }

    // Result mapping

    /// Translates revert data into the shared code.
    ///
    /// Unrecognised data fails the run, because letting it pass would hide a
    /// real difference behind a default value.
    function _codeFor(bytes memory data) private returns (uint8) {
        bytes4 selector = ResultMapping.selectorOf(data);
        uint8 code = ResultMapping.codeFor(selector);
        if (code != Code.UNKNOWN) return code;

        _fail("revert selector", "unrecognised revert data");
        console.logBytes4(selector);
        console.logBytes(data);
        return Code.UNKNOWN;
    }

    // State capture

    function _capture(SolEVMVault vault, MockAsset token, address[USER_COUNT] memory users)
        private
        view
        returns (Snapshot memory shot)
    {
        shot.pendingDepositEscrow = vault.pendingDepositEscrow();
        shot.idleBacking = vault.idleBacking();
        shot.claimReserve = vault.claimReserve();
        shot.unattributedBalance = vault.unattributedBalance();
        shot.totalSupply = vault.totalSupply();
        shot.vaultShareBalance = vault.balanceOf(address(vault));
        shot.vaultAssetBalance = token.balanceOf(address(vault));
        shot.status = uint8(vault.vaultStatus());
        shot.epochOpen = vault.epochOpen();
        shot.nextEpochId = vault.nextEpochId();
        shot.epochDepositAssets = vault.epochDepositAssets();
        shot.epochRedeemShares = vault.epochRedeemShares();

        // The model has no epoch identity once the slot is empty.
        uint64 epochId = 0;
        if (shot.epochOpen) {
            epochId = vault.currentEpochId();
            shot.epochId = epochId;
            shot.epochPhase = uint8(vault.currentEpochPhase());
        }

        for (uint256 i = 0; i < USER_COUNT; ++i) {
            shot.actorAssets[i] = token.balanceOf(users[i]);
            shot.actorShares[i] = vault.balanceOf(users[i]);
            if (shot.epochOpen) {
                shot.actorDepositAssets[i] = vault.depositRequestOf(epochId, users[i]).assets;
                shot.actorRedeemShares[i] = vault.redeemRequestOf(epochId, users[i]).shares;
            }
        }

        shot.consumedMask = _consumedMask(vault, users);
    }

    /// One bit per consumed claim or refund, matching the generator layout.
    function _consumedMask(SolEVMVault vault, address[USER_COUNT] memory users)
        private
        view
        returns (uint256 mask)
    {
        for (uint64 epoch = 0; epoch < MAX_TRACKED_EPOCHS; ++epoch) {
            for (uint256 user = 0; user < USER_COUNT; ++user) {
                if (vault.depositRequestOf(epoch, users[user]).settled) {
                    mask |= _consumedBit(epoch, user, false);
                }
                if (vault.redeemRequestOf(epoch, users[user]).settled) {
                    mask |= _consumedBit(epoch, user, true);
                }
            }
        }
    }

    /// Bit position of one consumption flag, matching the generator layout.
    function _consumedBit(uint64 epoch, uint256 user, bool redeem) private pure returns (uint256) {
        uint256 position = uint256(epoch) * 8 + user * 2 + (redeem ? 1 : 0);
        uint256 flag = 1;
        return flag << position;
    }

    // Comparison

    function _compare(Snapshot memory want, Snapshot memory got, string memory label) private {
        _eq(want.pendingDepositEscrow, got.pendingDepositEscrow, label, "pendingDepositEscrow");
        _eq(want.idleBacking, got.idleBacking, label, "idleBacking");
        _eq(want.claimReserve, got.claimReserve, label, "claimReserve");
        _eq(want.unattributedBalance, got.unattributedBalance, label, "unattributedBalance");
        _eq(want.totalSupply, got.totalSupply, label, "totalSupply");
        _eq(want.vaultShareBalance, got.vaultShareBalance, label, "vaultShareBalance");
        _eq(want.vaultAssetBalance, got.vaultAssetBalance, label, "vaultAssetBalance");
        _eq(want.status, got.status, label, "vault status");
        _eq(want.epochOpen ? 1 : 0, got.epochOpen ? 1 : 0, label, "epochOpen");
        _eq(want.epochPhase, got.epochPhase, label, "epochPhase");
        _eq(want.epochId, got.epochId, label, "epochId");
        _eq(want.nextEpochId, got.nextEpochId, label, "nextEpochId");
        _eq(want.epochDepositAssets, got.epochDepositAssets, label, "epochDepositAssets");
        _eq(want.epochRedeemShares, got.epochRedeemShares, label, "epochRedeemShares");
        _eq(want.consumedMask, got.consumedMask, label, "consumedMask");

        for (uint256 i = 0; i < USER_COUNT; ++i) {
            _eq(want.actorAssets[i], got.actorAssets[i], label, "actor asset balance");
            _eq(want.actorShares[i], got.actorShares[i], label, "actor share balance");
            _eq(want.actorDepositAssets[i], got.actorDepositAssets[i], label, "actor deposit");
            _eq(want.actorRedeemShares[i], got.actorRedeemShares[i], label, "actor redemption");
        }
    }

    /// Checks every epoch that had settled by this step. Settled terms are
    /// immutable, so a later change is a failure on its own.
    function _compareEpochs(
        SolEVMVault vault,
        EpochTerms[] memory epochs,
        address[USER_COUNT] memory users,
        uint256 step
    ) private {
        for (uint256 i = 0; i < epochs.length; ++i) {
            EpochTerms memory want = epochs[i];
            if (want.settledAtStep > step) continue;

            SettledEpoch memory got = vault.settledEpoch(want.epochId);
            _eq(want.outcome, uint8(got.outcome), "epoch terms", "outcome");
            _eq(want.cutoffAt, got.cutoffAt, "epoch terms", "cutoffAt");
            _eq(want.totalAssets, got.totalAssets, "epoch terms", "totalAssets");
            _eq(want.totalSupply, got.totalSupply, "epoch terms", "totalSupply");
            _eq(want.depositAssets, got.depositAssets, "epoch terms", "depositAssets");
            _eq(want.mintedShares, got.mintedShares, "epoch terms", "mintedShares");
            _eq(want.redeemShares, got.redeemShares, "epoch terms", "redeemShares");
            _eq(want.redeemAssets, got.redeemAssets, "epoch terms", "redeemAssets");
            _eq(want.depositDust, got.depositDust, "epoch terms", "depositDust");
            _eq(want.redeemDust, got.redeemDust, "epoch terms", "redeemDust");
            if (mismatch) return;

            for (uint256 user = 0; user < USER_COUNT; ++user) {
                address who = users[user];
                _eq(
                    want.actors[user].depositAssets,
                    vault.depositRequestOf(want.epochId, who).assets,
                    "epoch terms",
                    "settled deposit assets"
                );
                _eq(
                    want.actors[user].redeemShares,
                    vault.redeemRequestOf(want.epochId, who).shares,
                    "epoch terms",
                    "settled redeem shares"
                );
                _eq(
                    want.actors[user].claimShares,
                    vault.claimableDepositShares(want.epochId, who),
                    "epoch terms",
                    "claimable deposit shares"
                );
                _eq(
                    want.actors[user].claimAssets,
                    vault.claimableRedeemAssets(want.epochId, who),
                    "epoch terms",
                    "claimable redeem assets"
                );
                if (mismatch) return;
            }
        }
    }

    function _eq(uint256 want, uint256 got, string memory label, string memory field) private {
        if (want == got || mismatch) return;
        mismatch = true;
        _describe(label, field);
        console.log("  expected", want);
        console.log("  actual  ", got);
        if (keccak256(bytes(field)) == keccak256(bytes("result code"))) {
            console.log("  expected result", Code.name(uint8(want)));
            console.log("  actual result  ", Code.name(uint8(got)));
        }
        console.log("  reproduce", _reproduce());
    }

    function _fail(string memory label, string memory field) private {
        if (mismatch) return;
        mismatch = true;
        _describe(label, field);
        console.log("  reproduce", _reproduce());
    }

    // Reporting

    function _describe(string memory label, string memory field) private view {
        console.log("differential mismatch:", label);
        console.log("  scenario", ctxScenario);
        console.log("  step", ctxStep);
        console.log("  operation", Op.name(ctxAction.kind));
        console.log("  actor", uint256(ctxAction.actor));
        console.log("  amount", ctxAction.amount);
        console.log("  epoch", uint256(ctxAction.epochId));
        console.log("  timestamp", uint256(ctxAction.timestamp));
        console.log("  field", field);
    }

    function _reproduce() private view returns (string memory) {
        return string.concat(
            "cargo run -p evm-differential -- --seed ",
            vm.toString(runSeed),
            " --cases ",
            vm.toString(runCases),
            " --steps ",
            vm.toString(runSteps),
            " --describe ",
            vm.toString(ctxScenario)
        );
    }

    // Harness plumbing

    function _enabled() private view returns (bool) {
        return keccak256(bytes(vm.envOr("RUN_DIFFERENTIAL", string("")))) == keccak256(bytes("1"));
    }

    /// Runs the generator once and returns the encoded bundle.
    function _generate(uint256 seed, uint256 cases, uint256 steps) private returns (bytes memory) {
        string[] memory argv = new string[](13);
        argv[0] = "cargo";
        argv[1] = "run";
        argv[2] = "--quiet";
        argv[3] = "--release";
        argv[4] = "--manifest-path";
        argv[5] = string.concat(vm.projectRoot(), "/../../crates/evm-differential/Cargo.toml");
        argv[6] = "--";
        argv[7] = "--seed";
        argv[8] = vm.toString(seed);
        argv[9] = "--cases";
        argv[10] = vm.toString(cases);
        argv[11] = "--steps";
        argv[12] = vm.toString(steps);
        return vm.ffi(argv);
    }

    function _actorAddress(uint8 slot, address[USER_COUNT] memory users)
        private
        pure
        returns (address)
    {
        if (slot >= USER_COUNT) return Actors.addressOf(slot);
        return users[slot];
    }
}
