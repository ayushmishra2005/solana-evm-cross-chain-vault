# ES256 verifier on Solana devnet

## Overview

`src/main.rs` runs the verifier's consume-and-replay flow against a deployed program. It builds instructions with the verifier crate's account and instruction types and the adapter's precompile builder, the same encoding the tests use.

Verdicts are locally generated test data, signed by a P-256 key generated in memory for each run and never written out. No Saifuro key or token is involved.

## Prerequisites

- The toolchain listed under Testing in the [verifier README](../README.md#testing).
- The Solana CLI.
- Two disposable devnet keypairs kept outside the repository: a deployer, which pays and becomes the upgrade authority and administrator, and a consumer.

## Run

1. Build SBPFv0. `declare_id!` must match the program keypair you deploy with.

   ```bash
   cd programs/authorization-verifier
   cargo-build-sbf --tools-version v1.58 --arch v0
   shasum -a 256 target/deploy/es256_authorization_verifier.so
   ```

2. Deploy. Without `--use-rpc` the CLI sends buffer writes to validators directly; the public RPC endpoint can rate-limit them. To resume an interrupted deploy, rerun with `--buffer <buffer address>`.

   ```bash
   solana program deploy target/deploy/es256_authorization_verifier.so \
     --url https://api.devnet.solana.com --commitment finalized \
     --keypair <deployer.json> --upgrade-authority <deployer.json> \
     --program-id <program-keypair.json>
   ```

3. Run the flow. Progress goes to stderr and a JSON summary to stdout.

   ```bash
   cd devnet && mkdir -p ../target/devnet
   AUTHORITY_KEYPAIR=<deployer.json> CONSUMER_KEYPAIR=<consumer.json> \
     cargo run --locked > ../target/devnet/summary.json
   ```

   The runner refuses any cluster whose genesis hash is not devnet's, and checks that the deployer is the upgrade authority. It funds the consumer with 0.02 SOL, initializes the Sandbox config, and registers a `sandbox-` key and one mandate. It then consumes a verdict issued against the cluster clock and checks every receipt field. Finally it replays the verdict under a new blockhash: it simulates the replay, expects `AccountAlreadyInUse`, submits it without preflight so the rejection lands on chain, and compares the program accounts before and after. Transactions are read back at finalized commitment, and any mismatch stops the run. It stops if the Sandbox config already exists, so it runs once per deployment.

4. Check a deployment without keys:

   ```bash
   solana program show <program id> --url https://api.devnet.solana.com
   solana program dump <program id> onchain.so --url https://api.devnet.solana.com
   shasum -a 256 onchain.so
   solana confirm -v <signature> --url https://api.devnet.solana.com
   ```

## Verified devnet deployment

| | |
|---|---|
| Source | `main` at `c88b456` |
| Program ID | [`da9yz64zbrJ4fyi8VTR3xfVZu8GhfhUHkC7AZQxCGG7`](https://explorer.solana.com/address/da9yz64zbrJ4fyi8VTR3xfVZu8GhfhUHkC7AZQxCGG7?cluster=devnet) |
| ProgramData | `8sWQvyuukDkq6fi7TXDd3m99sypRLF71TPFNkkt79wt2` |
| Upgrade authority | `9WUUr2WNUiKMzwxJgbb4oxS81oYAyhrBFkv3NSg2mjbj` |
| Deployment | [`3AqRXiXM…3ykaBUr`](https://explorer.solana.com/tx/3AqRXiXM2kodhojwqKDmYXwR6SYmxxd137Buit5ENviwnWuu2HfD28Z65bSqHH4LSoYF53hS6f8H223wu3ykaBUr?cluster=devnet), slot 515,606,874 |
| Binary | SBPFv0, 329,952 bytes, built with `cargo-build-sbf` 4.4.0 and platform-tools v1.58 |
| SHA-256 | `efe5d65e04d6c593a0eeedae9fc117407468ed0d0c1a89d3c156b154dccdb3a4` |
| CI | [ES256 run 40468566582](https://github.com/ayushmishra2005/solana-evm-cross-chain-vault/actions/runs/40468566582) on `c88b456` |

The ELF extracted from ProgramData is byte-identical to a fresh build of `c88b456`. At that commit the program lived in `examples/es256-authorization-verifier` and had only the first profile. Later sources add the deployment profile and build a different binary, which has not been deployed.

## Authorization and replay test

The test ran against this deployment on 2026-10-27. It reused the Sandbox config `F3SUhPwNis6poeP5v17b6LhbxqWfgQjtchDTdt7ic9k6`, with consumer `2zn2Cq5r2bcwgUHW8iaXArpQbSghzWJiRGKLwrQnryDj`. It also reused the mandate `Byv1Jn8irus84ydLe58JM5eXEwt6cqUqayT5EqEABGtY`: `mnd_devnet_01`, 340 USD to `vendor:acme_saas`, policy `pol_2026-08-12.1`, agent `agt_procurement_01`, category `saas`. It registered a fresh key, `sandbox-devnet-02`, at `H2of2f8sqXmvzzNX6pkLo1HA1JPP7gNPtWpuFK3WfYNr`. The verdict used nonce `n_devnet_e8b250b5` and a 510-byte signing input.

| | Transaction | Slot | Result |
|---|---|---|---|
| Consume | [`58fEGFDR…tcVsWCcyZ`](https://explorer.solana.com/tx/58fEGFDRiBrcE9YEAmgPJjoJriCeXRxjCiykDfADSW7bw6jwE3PqbhSZZWkCgof9LmmeJqNBHqPRrJ6tcVsWCcyZ?cluster=devnet) | 515,623,744 | success, 10,000 lamports, 47,668 CU |
| Replay | [`x1FTE77H…L3KUqjqL6`](https://explorer.solana.com/tx/x1FTE77H3BJununVmSTkF2FKAUCyoYX6PXKuRN7FXGgr1F7urGAU3PVgbnrGwLcvpB9JDEFVyL3guvL3KUqjqL6?cluster=devnet) | 515,623,770 | `InstructionError(1, Custom(0))`, 10,000 lamports |

The receipt [`B5Yx1npYQTGyNTKAbLqL5RGNfudb6Wtm1JzrgTPBrniJ`](https://explorer.solana.com/address/B5Yx1npYQTGyNTKAbLqL5RGNfudb6Wtm1JzrgTPBrniJ?cluster=devnet) is a 185-byte account owned by the program. Its config, nonce hash, signing-input hash, jti hash, mandate, `expires_at`, `consumed_at` and bump all matched the signed verdict.

The replay carried the same instructions under a different recent blockhash while the verdict was still valid. Simulation returned the error above; the transaction was then sent with `skipPreflight` and was finalized with the same error. The precompile at index 0 passed, and the system program rejected the receipt allocation at index 1 with `AccountAlreadyInUse`. The config, both trusted keys, the mandate and both receipts were byte-identical before and after. The consumer paid only the fee.

An earlier run, made when the program was deployed, used key `sandbox-devnet-01`. It produced [consume](https://explorer.solana.com/tx/3fA2qvGCtKD5i2Jrj4wdfgXSAA1KXVNv3ig8ovagvEj5mgf56VNvvLSHHB3FLbPTYLitygC3gPARJKjrSJJJojBy?cluster=devnet), [replay](https://explorer.solana.com/tx/4UQcqvKvfJs1qvPUaVHFErt3ZnCYLtQoUmjQZKJTYrGdkoATCDAjcbJgqq75wNUkt52FL7P8RNJBFAi75mEhjSwU?cluster=devnet) and receipt `E2DFaam4iJM7m8nPeADLetVLmwVuc4YwF1G8CdvD9iV8`, with the same outcome.

## Limitations

- Verdicts are synthetic, so acceptance of a Saifuro-issued verdict is not demonstrated.
- Devnet only. There is no replay protection across clusters, deployments or configs.
- Only legacy transactions were sent, one at a time.
- The deployer is also the administrator, and the consumer key is disposable.
- Compute units differ from the LiteSVM figures in the verifier README: 47,668 and 41,697 on devnet, against 37,327. They vary with the verdict size and with the receipt address's bump search.
