# Operations runbook

How to build, deploy, upgrade and operate the Solana remote leg, what cannot be rolled back, and what to do in an emergency. The project has not been externally audited and is not ready to hold user funds; the open release risks are in the [README](../README.md#unresolved-release-risks).

## Roles, trust and key custody

| Role | Where it is set | Recommended custody |
|---|---|---|
| Upgrade authority | Loader ProgramData of the leg | Multisig with a timelock. It controls the code that controls custody, and it alone can initialize a namespace |
| Administrator | `initialize_remote_leg` signer, fixed for the namespace | Multisig |
| Emergency guardian | `InitializeParams.emergency_guardian`, fixed | A separate key or multisig that can react quickly. It must differ from the administrator |
| Transport verifier | `InitializeParams.transport_verifier`, fixed | The relayer's hot key, scoped to this one namespace |
| Authorization verifier upgrade authority | Loader ProgramData of the authorization verifier | As for the leg's upgrade authority. It creates each namespace's deployment authority, and its program decides which deployments verify |
| Risk administrator | `initialize_deployment_authority` params, fixed | Multisig. It registers and revokes risk keys and advances the policy generation |
| Risk key | `register_risk_key`, revocable | The risk service's P-256 signing key, ideally non-exportable |

No instruction rotates the administrator, guardian or transport verifier, so choose them for the life of the namespace. `ConfigUpdate` changes only risk parameters. Replacing a compromised role therefore means a new namespace, which only the upgrade authority can initialize.

| Role | Can | Cannot | If compromised |
|---|---|---|---|
| Program upgrade authority | Replace the program; initialize a namespace (`initialize.rs` reads the ProgramData upgrade authority and requires its signature) | Nothing is beyond it | Total control of custody. Hold it in a multisig with a timelock. Making the program immutable also stops new namespaces |
| Transport verifier | Submit inbound control messages: Allocate, Recall, Resolve, ConfigUpdate (`has_one = transport_verifier` in `allocate.rs`, `recall.rs`, `resolve.rs`, `settlement.rs`, `config_update.rs`) | Redirect custody: assets leave only for the pinned `outbound_escrow` (`send.rs`) or the pinned adapter vault (`deploy.rs`, `withdraw.rs`) | Forged but well-formed messages can corrupt accounting: a forged Close impairs an allocation, a forged Recall sends custody home early. The guardian or administrator freezes the leg; see [Transport verifier compromise](#transport-verifier-compromise) |
| Administrator | Initialize state, freeze, advance the replay watermark, close consumed-message records | Submit control messages | Can freeze the leg and reclaim record rent. Cannot move custody |
| Emergency guardian | Freeze; co-sign recall, attribution, admission, close and recovery on a frozen leg | Start any flow | Can freeze the leg and co-sign flows the verifier submits. Cannot start them |
| Authorization verifier upgrade authority | Replace the verifier; create a namespace's deployment authority, choosing its risk administrator and network domain (`deployment_admin.rs`) | Move custody, or deploy without the leg's own checks (`authorization.rs`) | Can approve any deployment the leg would otherwise accept. Hold it like the leg's upgrade authority |
| Risk administrator | Register and revoke risk keys; advance the policy generation, voiding approvals signed for earlier generations | Sign approvals; move custody | Can register its own key, with the same reach as a compromised risk key |
| Risk key | Approve one deployment of one amount into one exact deployment context, for at most five minutes | Exceed attributed principal or the strategy ceiling; deploy on a frozen leg; approve twice with one nonce; affect recalls or unwinding | Can push exposure up to those limits. Revoke it and advance the generation; see [Risk key compromise](#risk-key-compromise) |
| Strategy adapter | Hold deployed principal in its vault | Sign for custody: the leg signs adapter calls with a caller identity that owns nothing, then checks balances and custody control (`custody.rs`) | Must report principal exactly. `check_deployed` requires the adapter's principal to equal the leg's deployed principal, so a rounding or misreporting adapter halts reporting (`InvalidPrincipalDelta`) instead of mispricing |

Deployment authorization does not bind the network domain, the adapter's code version or the off-chain risk decision; see the limits under [Strategy deployment needs a risk approval](../README.md#strategy-deployment-needs-a-risk-approval).

## Build

Use the toolchain CI uses: `cargo-build-sbf` 4.4.0 with platform-tools v1.58. From `programs/solevm-remote-leg`:

```bash
cargo-build-sbf --tools-version v1.58 --arch v0
cargo-build-sbf --tools-version v1.58 --arch v0 --manifest-path test-strategy/Cargo.toml --sbf-out-dir target/deploy
sha256sum target/deploy/solevm_remote_leg.so
```

Record the source commit and the binary's SHA-256 before deploying. After the deploy, compare it with the on-chain bytes (`solana program dump <PROGRAM_ID> onchain.so`, trimmed to the binary's length).

Default builds use the ids in the default `declare_id!`. For the leg and the test adapter these are the intended production ids. The authorization verifier's default id, `da9yz64zbrJ4fyi8VTR3xfVZu8GhfhUHkC7AZQxCGG7`, is not a production identity: it is the historical V1 devnet verifier, kept as the default for compatibility with that deployment, under a key and upgrade authority made for testing. Because the leg trusts the verifier its build compiles in, a production leg must not be released until the verifier has a dedicated production id and upgrade authority, chosen and reviewed with the leg's governance and security review; see [Unresolved release risks](../README.md#unresolved-release-risks).

`--cfg solevm_devnet` selects the disposable V2 devnet ids instead: leg `DcbREPYFa4WtjknjmT7ENqP7RV3KAfXBLcBbGqyq2jhD` and verifier `4dAHz4oRNVmQ1ru9WkUthKBKU65Bjn3jQrQ5GDfQ9CSw`. It is a cfg rather than a feature, so `--all-features` builds keep the default ids. Use it only through `scripts/devnet-v2/build.sh`.

## Fresh deployment

1. Deploy with the upgrade authority from the table above: `solana program deploy --program-id <PROGRAM_KEYPAIR> --upgrade-authority <AUTHORITY> target/deploy/solevm_remote_leg.so`. Without `--max-len`, the program data is sized to this binary, and a larger upgrade first needs `solana program extend`.
2. Claim the namespace with `initialize_remote_leg`, signed by the administrator and the upgrade authority. It binds the deployment and vault ids, both chain ids, the source and local application ids, both lane ids, the asset mint, the custody and outbound escrow token accounts, the transport verifier and the emergency guardian. The custody account must belong to the custody-authority PDA, with no delegate and no close authority.
3. `initialize_control_state` (administrator): risk limits, the config commitment and the watermark lag.
4. Initialize the strategy adapter, then `initialize_strategy_state` (administrator) with the principal ceiling. This pins the adapter program, its state and its vault.
5. `initialize_outbound_state` (administrator).
6. Set up deployment authorization:
   - The authorization verifier's upgrade authority runs `initialize_deployment_authority` for the namespace. It names the risk administrator and the network domain, which is the cluster's genesis hash.
   - The risk administrator runs `register_risk_key`.
   - The leg administrator runs `initialize_deploy_authorization`, once. It cannot be replaced or removed.

   Until then `deploy_to_strategy` fails, and nothing else is affected.
7. Point the relayer at the namespace. Open allocations only after it routes all seven message types.

`programs/solevm-remote-leg/devnet/src/scenario.rs` builds the instructions of steps 2 to 5 and can serve as a reference client for them. It builds nothing for step 6 and never calls `deploy_to_strategy`. The published devnet run used a leg without deployment authorization, and no devnet namespace has adopted it.

`programs/solevm-remote-leg/devnet/src/v2/scenario.rs` builds steps 2 to 6 and the authorized deployment transaction, signing approvals with a P-256 key generated in memory for each run. It has run only on a local validator: see [V2: deployment authorization](#v2-deployment-authorization).

In the local LiteSVM tests, the step 6 instructions and the authorized deployment transaction are built by `programs/solevm-remote-leg/tests/common/authorization.rs`:

- `install_deploy_authorization` runs step 6 in order;
- `initialize_deployment_authority_instruction`, `register_risk_key` and `adopt_instruction` build its three instructions;
- `authorized_deploy` builds `[secp256r1, consume_deployment_authorization, deploy_to_strategy]`.

Their risk key is a fixed test secret. A real deployment also needs:

- the authorization verifier deployed or upgraded with the deployment profile. The V1 devnet verifier, `da9yz64…`, runs the first profile only;
- the cluster's genesis hash as the network domain;
- a risk service that holds the risk key and signs approvals. This repository has none.

When an existing deployment adopts a release, follow the order under "Deployment order" in the [README](../README.md#deployment-order).

## Upgrade

1. Build and record the binary as above, and run the full suites.
2. Write it to a buffer: `solana program write-buffer target/deploy/solevm_remote_leg.so`. Hand the buffer to the upgrade authority: `solana program set-buffer-authority <BUFFER> --new-buffer-authority <AUTHORITY>`.
3. The upgrade authority executes the upgrade from the buffer, through the multisig if there is one. An interrupted write leaves a funded buffer. Resume it by deploying with the same `--buffer` key, or reclaim its rent with `solana program close <BUFFER>`.
4. Confirm that the on-chain bytes match the recorded SHA-256 and that `solana program show <PROGRAM_ID>` names the expected authority.

An approval binds the strategy adapter's program id, state, authority and vault, not its code. Upgrading the adapter program therefore voids no outstanding approval. Pause approvals, have the risk administrator run `advance_policy_generation`, upgrade the adapter, and resume approvals only after reviewing the new code. An accepted ConfigUpdate needs no such step, because it changes the configuration version the approval names.

## What cannot be rolled back

- **Program upgrades are one-way once accounts change.** As soon as an account holds a value the previous program does not know, the previous program cannot read it. Examples: a quarantined, closed or recovery transfer record, a consumed Resolve, a settlement report record. Redeploying the old binary does not restore the old account state.
- **The EVM vault and coordinator are not upgradeable.** The vault binds its coordinator once, so an EVM change means new contracts and a new Solana namespace.
- **A freeze is permanent.** There is no unfreeze instruction. A frozen leg can only unwind, and new allocations need a new namespace.
- **Roles are permanent.** Replacing the administrator, guardian or verifier means a new namespace.
- **Deployment authorization adoption is permanent.** A namespace that adopted a deployment authority deploys only through it.
- **Making the program immutable** (`solana program set-upgrade-authority --final`) cannot be undone. It also stops new namespaces, because initialization requires the upgrade authority's signature.

## Emergency procedure

### Freeze

The administrator or the guardian sends `freeze_remote_leg`. The leg then takes no new exposure: deploys and configuration changes are blocked, and a new allocation is quarantined. A recall accepted before the freeze keeps withdrawing, sending and reporting. These then need the guardian's co-signature:

- accepting a recall;
- attributing an allocation accepted before the freeze;
- admission;
- close;
- recovery.

A control message that reverts for lack of the co-signature stays unconsumed, so the ordered lane waits for the guardian.

To unwind a frozen leg, take the same steps as on an active one, each co-signed by the guardian:

1. close the short allocation;
2. recover late custody into principal;
3. recall that principal home.

The devnet scenario shows the close step live (T14, N05, N06, T15).

### Transport verifier compromise

1. Freeze immediately (administrator or guardian). New allocations are then quarantined, and close, recovery, admission, attribution and recall acceptance need the guardian.
2. Stop the relayer, and refuse to co-sign any control message whose EVM origin you cannot confirm on the coordinator.
3. Compare the leg's consumed sequences and transfer records with the coordinator's events, to find forged messages.
4. Unwind with guardian co-signatures. Open a new namespace with a new verifier and a new coordinator.

The verifier cannot redirect custody. Assets leave only for the pinned outbound escrow or the pinned adapter vault.

### Risk key compromise

1. The risk administrator runs `advance_policy_generation`, which voids every approval signed for an earlier generation, and `revoke_risk_key`. The revocation is required: it also stops approvals the key signed in advance for a later generation.
2. Register a new key. Freeze the leg if exposure must stop altogether.

A risk key alone can only approve deployments the leg would otherwise accept: within attributed principal and the strategy ceiling, into the pinned adapter vault, and never on a frozen leg. Recalls and unwinding never need it.

### Guardian or administrator compromise

Either key can freeze the leg, which only stops it. A compromised guardian can also co-sign flows the verifier submits, but cannot start them. In both cases, unwind and move to a new namespace.

### Upgrade authority compromise

Nothing on-chain can contain a hostile upgrade. Prevent it with multisig and timelock custody. Watch `solana program show <PROGRAM_ID>` for unexpected deploy slots or authority changes.

## Relayer failure handling and idempotency

Every case below can be retried safely.

- **Inbound messages are exactly-once.** Each control message needs the lane's next sequence and creates a consumed-message record. Re-delivering a processed message fails with `InvalidSequence` and changes nothing; the devnet scenario proves this live (N02, N04).
- **Quarantine is consumption.** A message that cannot run when it arrives is consumed and held with no financial effect, until the coordinator's Resolve names it.
- **Reverted messages stay pending.** A message that reverts, for example on a frozen leg without the guardian, is not consumed. Retry it once its condition is met.
- **Outbound reports are delivered in lane order.** Each report extends the outbound commitment chain. Deliver RemoteReport, RecallSent and TransportSettlement to the EVM side in sequence; a gap or duplicate is refused there.
- **Rent recovery.** `close_consumed_message` (administrator) returns a consumed record's rent once the replay watermark has passed it.

## Devnet instances

Both instances use disposable keys and a valueless test token. Their scripts check the cluster's genesis hash and refuse mainnet-beta and testnet. They keep keys, buffers and evidence drafts outside the repository and never print private keys.

### V1: the recorded run

The V1 instance, leg `27zReYgZvFTaUSWwob4qirN2Yxekcm32mnQDpMzeTyAE` and test adapter `8bFiHDwdAurdhTv5F9JaVD4QgdJ2XxEHRK2BAyiCyjZ8`, ran the scenario recorded in [`evidence/devnet/v1.json`](../evidence/devnet/v1.json) and summarized in the [README](../README.md#solana-devnet--verified-deployment-and-test-evidence). It is read-only now. The current sources declare the V2 devnet leg id, so `scripts/devnet/` refuses to generate keys, build, deploy, fund or run, and the driver's V1 `run` and `addresses` commands refuse before reading keys, the RPC or any record. `scripts/devnet/verify.sh` and the driver's `verify` and `report` still read the record.

The run was made with the scripts as they are at commit `1aebf3f`, which has its build inputs:

1. `scripts/devnet/keygen.sh` creates keys under `~/.config/solevm-devnet/keys`, outside the repository, and prints public keys only.
2. If your program keys differ from the source, set the `cfg(solevm_devnet)` `declare_id!` in `src/lib.rs` and `test-strategy/src/lib.rs`, and `LEG_DEVNET_ID` and `ADAPTER_DEVNET_ID` in `scripts/devnet/common.sh`.
3. `scripts/devnet/build.sh`, then `scripts/devnet/localnet.sh` for a free local rehearsal.
4. `scripts/devnet/fund.sh` shows what the deployer needs: about 5.36 SOL at the devnet rent of the time, almost all of it the two programs' rent. Fund it from [faucet.solana.com](https://faucet.solana.com).
5. `scripts/devnet/deploy.sh`, then `scripts/devnet/run.sh <label>` with a new label per run.

`deploy.sh` sends the program's write transactions, more than a thousand of them, straight to validators, because public RPC endpoints rate-limit that volume. Rerunning an interrupted deploy resumes into the same buffer.

### V2: deployment authorization

**Status: rehearsed on a local validator only. V2 is not deployed on devnet, and no V2 result is devnet validation.**

V2 is a separate instance in which every strategy deployment spends a single-use ES256 risk approval, in the transaction described under [Strategy deployment needs a risk approval](../README.md#strategy-deployment-needs-a-risk-approval).

| Role | Address |
|---|---|
| V2 leg program (new) | `DcbREPYFa4WtjknjmT7ENqP7RV3KAfXBLcBbGqyq2jhD` |
| V2 verifier program (new) | `4dAHz4oRNVmQ1ru9WkUthKBKU65Bjn3jQrQ5GDfQ9CSw` |
| Test adapter, reused from V1 and not redeployed | `8bFiHDwdAurdhTv5F9JaVD4QgdJ2XxEHRK2BAyiCyjZ8`, binary sha256 `02a978e2…bd39fb`, upgrade authority `HiiQJU2B…KtKytn` (the V1 deployer) |
| V2 deployer, upgrade authority of both new programs | `68zeWuMTyX2MKQVPo2aLd1vVeMJcSkEYyLq8YKtFushD` |

The V2 program ids come from `--cfg solevm_devnet` (see [Build](#build)). V2 namespace tags are prefixed `solevm/devnet-v2/`, so no V2 label can derive a V1 address. The risk key stands in for a risk service that does not exist yet: each run generates it in memory, registers only its public half and never stores it.

#### Safeguards

- **Network.** Every script requires `SOLEVM_RPC_URL` and `SOLEVM_CLUSTER` (`devnet` or `localnet`, checked before any network call), and checks the endpoint's genesis hash. Mainnet-beta and testnet are refused, and a localnet endpoint must be on 127.0.0.1 or localhost.
- **Approval.** Deploying, running or requesting an airdrop on devnet needs `SOLEVM_V2_DEVNET_APPROVED=1`. `fund.sh` only ever funds the V2 deployer.
- **Driver.** `run-v2` applies the cluster, genesis, approval and run-time feature checks itself, so running it without the scripts skips none of them; `programs/solevm-remote-leg/devnet/tests/cli.rs` runs it against each unsafe setting. It refuses to replace an existing devnet record.
- **Keys and state.** V2 keys, buffers, deploy records and evidence live under `SOLEVM_V2_HOME` (default `~/.config/solevm-devnet-v2`), which must be outside the repository, the V1 home and the Solana CLI config. No V2 role may be the CLI default wallet. Records live under `SOLEVM_V2_HOME/state/<cluster>`.
- **Binaries.** V2 builds go to `target/deploy-devnet-v2`. The build refuses a binary that embeds a V1 or production id, and an adapter that is not the pinned V1 binary.
- **No upgrades.** `deploy.sh` has no upgrade path: a V2 id running another binary stops it, and V1 and production ids are refused outright. Buffer keys are per cluster and live in the V2 home. An existing buffer is resumed only if it is an upgradeable-loader buffer whose authority is the V2 deployer.
- **Feature gates.** These are read from the target cluster before each deploy and run, never assumed. Secp256r1 (SIMD-0075) and in-SVM precompile verification (SIMD-0159) must be active. The SBPFv0 execution shutdown (SIMD-0161) and, for a deploy, the SBPFv0 deployment shutdown (SIMD-0500) must be inactive. The build, the deploy and the driver each check that the binaries are SBPFv0 (ELF `e_flags` 0).

#### Funding

At devnet rent quoted for the rehearsal's binaries, deploying costs about 6.53 SOL: 2.10 for the verifier (412,397 bytes of program data), 4.42 for the leg (870,509 bytes) and about 0.0064 in deploy fees. One scenario run costs about 0.036 SOL. The driver refuses to start a run below 0.2 SOL, so hold about 6.8 SOL before deploying, or about 7.5 SOL with headroom for resent deploy writes. `fund.sh` recomputes the estimate from live rent. Each authorized deployment also leaves a permanent 226-byte receipt with no close path, about 0.0018 SOL of rent on devnet.

#### Commands

Tools: `cargo-build-sbf` 4.4.0 (platform tools v1.58) for builds, and an Agave release with `solana-test-validator` for the rehearsal (4.3.2 was used).

```sh
scripts/devnet-v2/keygen.sh            # disposable V2 keys; never overwrites
scripts/devnet-v2/build.sh             # pinned SBPFv0 builds with the V2 ids

# Local rehearsal: mirrors devnet's feature set, loads the pinned adapter,
# deploys the verifier and leg, runs the scenario and re-verifies it.
SOLEVM_AGAVE_BIN=/path/to/agave-release/bin \
  scripts/devnet-v2/localnet.sh local-1

# Devnet, only after review (not done):
export SOLEVM_RPC_URL=https://api.devnet.solana.com SOLEVM_CLUSTER=devnet
scripts/devnet-v2/fund.sh              # what the deployer holds and needs
SOLEVM_V2_DEVNET_APPROVED=1 scripts/devnet-v2/deploy.sh
SOLEVM_V2_DEVNET_APPROVED=1 scripts/devnet-v2/run.sh devnet-1
scripts/devnet-v2/verify.sh ~/.config/solevm-devnet-v2/state/devnet/evidence-v2-devnet-1.json
```

`deploy.sh` deploys the verifier and then the leg, and checks that the reused adapter on the cluster is the pinned binary under its recorded authority.

#### The scenario

`run.sh <label>` initializes a new namespace in the order of [Fresh deployment](#fresh-deployment): the namespace claim co-signed by the leg's upgrade authority, control state, the adapter and strategy state, and the outbound lane. Then the verifier's upgrade authority creates the deployment authority on the cluster's genesis, and the risk administrator registers the run's risk key. The leg administrator adopts the authority only after allocation A has arrived, so the run first shows that a fully approved deployment fails before adoption. Two authorized deployments follow, then a recall that needs no approval, a new allocation and a guardian freeze. Sixteen attacks must fail along the way, covering wrong authorities, a second adoption, missing, replayed, stale, mismatched, altered and expired approvals, a failed strategy deposit that must roll its receipt back, and a deployment on the frozen leg.

Every expected rejection is simulated first. A step whose simulation does not fail exactly as expected stops the run with nothing submitted. Each rejection then lands as a finalized, fee-paying failed transaction. The driver re-checks its exact error and failing instruction index, and requires every tracked account to be byte-for-byte unchanged and no receipt to be created.

#### Verification

V2 records are schema 2, of kind `solevm-v2-deployment-authorization`. V1's `verify` refuses them, and `scripts/devnet-v2/verify.sh` refuses the V1 record. A record holds the cluster and a `validation` statement, the RPC, genesis hash, node version and feature gates (and, on localnet, the feature mirror), the source commit and tree, each program's id, owner, upgrade authority and sha256, the public keys of the signer roles, the namespace and authorization accounts, every receipt, and per transaction its signature, slot, confirmation, fee, compute units, size, programs, expected and observed result, simulation and account changes. No private key is recorded.

`scripts/devnet-v2/verify.sh <record>` (`solevm-devnet verify-v2`) re-checks a record from public data alone: the genesis, each program's bytes and upgrade authority, every transaction at its finalized slot with exactly the recorded result at the expected instruction, the deployment authority's binding to the leg, namespace and genesis, and the risk key, policy and receipts. Against a live rehearsal validator, it rejected copies of a record with an edited error, expected instruction index, signature, receipt amount, program hash, risk key or genesis. `solevm-devnet report-v2 <record>` renders a record as Markdown, and its first line states the cluster and whether the record is devnet validation. A localnet record can be re-verified only while its validator runs.

#### Known incompatibilities

- **SBPFv0 only.** The V2 builds and the reused adapter are SBPFv0. A test validator enables every feature it knows, and Agave 4.3.2 knows SIMD-0500, so a default validator refuses the V2 binaries while devnet, where SIMD-0500 is inactive, accepts them. `localnet.sh` therefore reads devnet's feature set, read-only after checking devnet's genesis, and deactivates every gate devnet has not activated. The driver confirms each is inactive locally and records the mirror, with the devnet slot it was read at. If SIMD-0500 activates on devnet, the V2 binaries must be rebuilt for SBPFv3 and rehearsed again; the program tests already pass on SBPFv3.
- **Rent.** The local validator charges 6960 lamports per byte, while devnet quoted about 5080, so local rent figures are upper bounds.
- **Transaction size.** Each authorized deployment is one legacy transaction of 1,144 of 1,232 bytes, with 3 instructions, 22 account keys and 2 signatures. v0 transactions and address lookup tables were not exercised on a validator.
