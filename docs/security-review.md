# Internal security review

This is an internal engineering review by the project's developers. It is not an external audit, and passing tests is not evidence that one took place. No external audit has been performed.

## Scope

- Solana remote leg (`programs/solevm-remote-leg`): PDA derivation and seeds, signer and upgrade-authority checks, SPL custody and the adapter boundary, inbound message authentication, canonical encoding, commitments, sequencing and replay, guardian, freeze and emergency paths, Close and Recover conservation, impairment, exactly-once settlement, arithmetic, account layouts, and compute.
- Solidity vault and coordinator (`contracts/solevm-vault`): custody, access control, reentrancy, and parity with the Rust accounting model.
- Protocol codec (`crates/protocol-types`), the relayer interface documented in the README, and CI.
- Strategy deployment authorization: the second profile of `programs/authorization-verifier` and its enforcement in `deploy.rs` and `authorization.rs`.

Method: manual reading of the code paths above, the existing suites (LiteSVM, Mollusk, Foundry unit, fuzz and invariant, the Rust-to-Solidity differential harness, cross-chain E2E), and the live negative tests of the devnet scenario (`programs/solevm-remote-leg/devnet`).

## Result

No critical or high-severity defect was found in the reviewed scope. The material findings are below. Each names the file, the scenario, the severity and the minimal fix.

| ID | Severity | Finding | Status |
|---|---|---|---|
| F1 | Low | Anyone can publish reports, and each one must be relayed in order | Open: needs an owner decision |
| F2 | Low | CI ran mutable action tags with a default-permission token | Fixed on this branch |
| F3 | Informational | Crate `repository` metadata pointed to a missing repository | Fixed on this branch |
| R1 | Release blocker | Stuck recall | Open: needs a loss policy |
| R2 | Release blocker | Close-pending epoch liveness | Open: needs a valuation policy |

### F1. Anyone can publish reports, and each one must be relayed in order

- **File:** `programs/solevm-remote-leg/src/instructions/publish_report.rs` (`PublishRemoteReport`).
- **Scenario:** `payer` is any signer and `epoch_id` is chosen by the caller. Nothing limits how often a report is published, and report records cannot be closed. Each report is built from on-chain state, so it is truthful and cannot misprice anything. But each one advances the outbound lane, and the EVM side consumes that lane strictly in order. A caller who publishes reports in a loop therefore forces the relayer to submit every one to `RemoteAccounting.processRemoteReport` before a later TransportSettlement or RecallSent can land. The caller pays a report record's rent and a fee per report.
- **Impact:** relayer cost and settlement delay. No funds or accounting are at risk.
- **Minimal fix:** require the signer to be the configured transport verifier or the administrator. That is one constraint, with no layout change. The trade-off is that reporting no longer stays live without the operator, which is why this needs the owner's decision.

### F2. CI ran mutable action tags with a default-permission token

- **File:** `.github/workflows/rust.yml`.
- **Scenario:** every action was referenced by a movable tag (`actions/checkout@v4`, `actions/upload-artifact@v4`, `actions/download-artifact@v4`, `foundry-rs/foundry-toolchain@v1`). The workflow set no `permissions`, checkout persisted its credentials, and the Foundry libraries were installed by tag. A moved tag or a compromised action could run arbitrary code with the job's token.
- **Fix applied:**
  - top-level `permissions: contents: read`;
  - every action pinned to a full commit SHA, with the release in a comment;
  - `persist-credentials: false` on every checkout;
  - `forge-std` and OpenZeppelin installed at the commit SHAs of their release tags.

### F3. Crate `repository` metadata pointed to a missing repository

The `repository` field in four Cargo manifests (`Cargo.toml`, `programs/solevm-remote-leg/Cargo.toml`, `test-strategy/Cargo.toml`, `e2e/Cargo.toml`) returned HTTP 404. It now names `https://github.com/ayushmishra2005/solana-evm-cross-chain-vault`.

### R1 and R2. Release blockers

Both are described, with the recommended solution and the decision each needs, under "Unresolved release risks" in the [README](../README.md#unresolved-release-risks). Neither is fixed here: each needs a financial-finality policy that only the owner can set.

## Trust model

| Role | Can | Cannot | If compromised |
|---|---|---|---|
| Program upgrade authority | Replace the program; initialize a namespace (`initialize.rs` reads the ProgramData upgrade authority and requires its signature) | Nothing is beyond it | Total control of custody. Hold it in a multisig with a timelock. Making the program immutable also stops new namespaces |
| Transport verifier | Submit inbound control messages: Allocate, Recall, Resolve, ConfigUpdate (`has_one = transport_verifier` in `allocate.rs`, `recall.rs`, `resolve.rs`, `settlement.rs`, `config_update.rs`) | Redirect custody: assets leave only for the pinned `outbound_escrow` (`send.rs`) or the pinned adapter vault (`deploy.rs`, `withdraw.rs`) | Forged but well-formed messages can corrupt accounting: a forged Close impairs an allocation, a forged Recall sends custody home early. The guardian or administrator freezes the leg; see the [runbook](operations.md#transport-verifier-compromise) |
| Administrator | Initialize state, freeze, advance the replay watermark, close consumed-message records | Submit control messages | Can freeze the leg and reclaim record rent. Cannot move custody |
| Emergency guardian | Freeze; co-sign recall, attribution, admission, close and recovery on a frozen leg | Start any flow | Can freeze the leg and co-sign flows the verifier submits. Cannot start them |
| Authorization verifier upgrade authority | Replace the verifier; create a namespace's deployment authority, choosing its risk administrator and network domain (`deployment_admin.rs`) | Move custody, or deploy without the leg's own checks (`authorization.rs`) | Can approve any deployment the leg would otherwise accept. Hold it like the leg's upgrade authority |
| Risk administrator | Register and revoke risk keys; advance the policy generation, voiding approvals signed for earlier generations | Sign approvals; move custody | Can register its own key, with the same reach as a compromised risk key |
| Risk key | Approve one deployment of one amount into one exact deployment context, for at most five minutes | Exceed attributed principal or the strategy ceiling; deploy on a frozen leg; approve twice with one nonce; affect recalls or unwinding | Can push exposure up to those limits. Revoke it and advance the generation; see the [runbook](operations.md#risk-key-compromise) |
| Strategy adapter | Hold deployed principal in its vault | Sign for custody: the leg signs adapter calls with a caller identity that owns nothing, then checks balances and custody control (`custody.rs`) | Must report principal exactly. `check_deployed` requires the adapter's principal to equal the leg's deployed principal, so a rounding or misreporting adapter halts reporting (`InvalidPrincipalDelta`) instead of mispricing |

The administrator, guardian and transport verifier are set once in `initialize_remote_leg`, and no instruction changes them. `ConfigUpdate` changes only risk parameters. Replacing a compromised role therefore means a new namespace, which only the upgrade authority can initialize.

Deployment authorization does not bind these:
- The network domain is configuration the verifier's upgrade authority attests to. A program cannot read the cluster's genesis hash.
- The adapter's code version is not bound. Its program id, state, authority and vault are, and an upgraded adapter is still held to `check_deployed`. An adapter upgrade voids no outstanding approval; the [runbook](operations.md#upgrade) advances the policy generation around it.
- The risk decision is off-chain. The repository contains only a deterministic test risk policy (`tests/risk_approval.rs`), not a production risk service.

## Properties exercised on a live cluster

The devnet scenario sends these as real transactions and checks the result. Each rejection must land on chain with the exact program error, and leave every tracked account and token balance byte-for-byte unchanged. The [recorded devnet run](devnet-verification.md) links every step.

| Property | Step | Result required |
|---|---|---|
| Only the upgrade authority can claim a namespace | N01 | `Unauthorized`, config not created |
| A replayed control message is refused | N02 | `InvalidSequence` |
| Only the transport verifier can deliver a control message | N03 | `Unauthorized` |
| A replayed Recover cannot credit the pool twice | N04 | `InvalidSequence` |
| A frozen leg refuses a close without the guardian | N05 | `Frozen` |
| An impostor cannot co-sign as the guardian | N06 | `Unauthorized` |
| Close and Recover conserve value and never double count | T09, T11, T15 | claim = credit + impaired + recovered after each settlement |

## Compute

The highest consumption is `process_settlement`. It peaked at about 85,000 compute units in the local rehearsal. On devnet, T09's whole transaction used 80,707. That is under Solana's default budget of 200,000 per instruction. The Mollusk suite (`tests/compute.rs`) guards these costs in CI.

An authorized deployment uses about 60,000 compute units in total in LiteSVM: about 16,000 for the verifier's consume and 44,000 for `deploy_to_strategy`. As a legacy transaction with a compute-budget prefix it is 1,196 of 1,232 bytes. Through an address lookup table it is 705 bytes. `tests/authorization_limits.rs` checks both.
