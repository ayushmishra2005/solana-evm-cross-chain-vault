# Operations runbook

How to build, deploy, upgrade and operate the Solana remote leg, what cannot be rolled back, and what to do in an emergency. The trust model behind these steps is in [security-review.md](security-review.md#trust-model). The project has not been externally audited and is not ready to hold user funds.

## Roles and key custody

| Role | Where it is set | Recommended custody |
|---|---|---|
| Upgrade authority | Loader ProgramData of the leg | Multisig with a timelock. It controls the code that controls custody, and it alone can initialize a namespace |
| Administrator | `initialize_remote_leg` signer, fixed for the namespace | Multisig |
| Emergency guardian | `InitializeParams.emergency_guardian`, fixed | A separate key or multisig that can react quickly. It must differ from the administrator |
| Transport verifier | `InitializeParams.transport_verifier`, fixed | The relayer's hot key, scoped to this one namespace |
| Authorization verifier upgrade authority | Loader ProgramData of the authorization verifier | As for the leg's upgrade authority. It creates each namespace's deployment authority, and its program decides which deployments verify |
| Risk administrator | `initialize_deployment_authority` params, fixed | Multisig. It registers and revokes risk keys and advances the policy generation |
| Risk key | `register_risk_key`, revocable | The risk service's P-256 signing key, ideally non-exportable |

No instruction rotates the administrator, guardian or transport verifier, so choose them for the life of the namespace.

## Build

Use the toolchain CI uses: `cargo-build-sbf` 4.4.0 with platform-tools v1.58. From `programs/solevm-remote-leg`:

```bash
cargo-build-sbf --tools-version v1.58 --arch v0
cargo-build-sbf --tools-version v1.58 --arch v0 --manifest-path test-strategy/Cargo.toml --sbf-out-dir target/deploy
sha256sum target/deploy/solevm_remote_leg.so
```

Record the source commit and the binary's SHA-256 before deploying. After the deploy, compare it with the on-chain bytes (`solana program dump <PROGRAM_ID> onchain.so`, trimmed to the binary's length).

The program ids in `declare_id!` are the production ids. `--cfg solevm_devnet` selects the disposable devnet ids instead. It is a cfg rather than a feature, so `--all-features` builds keep the production ids. Use it only through `scripts/devnet/build.sh`.

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

The step 6 instructions, and the authorized deployment transaction, are built only in local LiteSVM tests, by `programs/solevm-remote-leg/tests/common/authorization.rs`:

- `install_deploy_authorization` runs step 6 in order;
- `initialize_deployment_authority_instruction`, `register_risk_key` and `adopt_instruction` build its three instructions;
- `authorized_deploy` builds `[secp256r1, consume_deployment_authorization, deploy_to_strategy]`.

Their risk key is a fixed test secret. A real deployment also needs:

- the authorization verifier deployed or upgraded with the deployment profile. The devnet verifier runs the first profile only;
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

## Devnet instance

`scripts/devnet/` deploys a disposable instance on Solana devnet and runs the verified scenario. To run your own, you need disposable keys whose program keys match the devnet ids in the source:

1. Run `scripts/devnet/keygen.sh`. It creates keys under `~/.config/solevm-devnet/keys`, outside the repository, and prints public keys only.
2. If your program keys differ from the source, set the `cfg(solevm_devnet)` `declare_id!` in `src/lib.rs` and `test-strategy/src/lib.rs`, and `LEG_DEVNET_ID` and `ADAPTER_DEVNET_ID` in `scripts/devnet/common.sh`.
3. Run `scripts/devnet/build.sh`, then `scripts/devnet/localnet.sh` for a free local rehearsal.
4. Run `scripts/devnet/fund.sh` to see what the deployer needs: about 5.36 SOL at current devnet rent, almost all of it the two programs' rent. Fund it from [faucet.solana.com](https://faucet.solana.com).
5. Run `scripts/devnet/deploy.sh`, then `scripts/devnet/run.sh <label>` with a new label per run.

`deploy.sh` sends the program's write transactions, more than a thousand of them, straight to validators, because public RPC endpoints rate-limit that volume. Rerunning an interrupted deploy resumes into the same buffer. The published run in [devnet-verification.md](devnet-verification.md) followed these steps.

The scripts check the cluster's genesis hash and refuse mainnet-beta and testnet. They keep keys and evidence drafts outside the repository and never print private keys.
