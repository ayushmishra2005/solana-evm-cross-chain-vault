# Devnet evidence

[`EVIDENCE.md`](EVIDENCE.md) records one deployment of the verifier on Solana devnet. On it, one allow verdict was consumed and the same verdict, replayed, was rejected. `src/main.rs` is the runner that produced the record. It builds every instruction with the verifier crate's account and instruction types and the adapter's precompile builder, so it exercises the same encoding the tests use.

The verdict is locally generated test data: the P-256 key is generated in memory for each run and never written out. No Saifuro key or token is involved.

## Reproducing

Requirements: the toolchain listed under Testing in the [example README](../README.md#testing), the Solana CLI, and two disposable devnet keypairs kept outside the repository. One is the deployer, which pays and becomes the upgrade authority and administrator. The other is the consumer.

1. Build SBPFv0 and record its hash. `declare_id!` must match the program keypair you deploy with.

   ```bash
   cd examples/es256-authorization-verifier
   cargo-build-sbf --tools-version v1.58 --arch v0
   shasum -a 256 target/deploy/es256_authorization_verifier.so
   ```

2. Deploy to devnet. Without `--use-rpc` the CLI sends buffer writes to validators directly; through the public RPC endpoint they can be rate-limited. If a deploy stops part-way, rerun the command with `--buffer <buffer address>` to resume into the same buffer, or close the buffer with `solana program close` to recover its rent.

   ```bash
   solana program deploy target/deploy/es256_authorization_verifier.so \
     --url https://api.devnet.solana.com --commitment finalized \
     --keypair <deployer.json> --upgrade-authority <deployer.json> \
     --program-id <program-keypair.json>
   ```

3. Run the flow. The runner refuses any cluster whose genesis hash is not devnet's, checks that the deployer is the program's upgrade authority, and stops if the Sandbox config already exists, so it runs once per deployment. Progress goes to stderr; the JSON summary goes to stdout.

   ```bash
   cd devnet && mkdir -p ../target/devnet
   AUTHORITY_KEYPAIR=<deployer.json> CONSUMER_KEYPAIR=<consumer.json> \
     cargo run --locked > ../target/devnet/summary.json
   ```

   It funds the consumer with 0.02 SOL, initializes the Sandbox config and registers a `sandbox-` key and one mandate. It then consumes a verdict issued against the cluster clock and checks every receipt field against the verdict. Finally it replays the verdict under a fresh blockhash: it simulates the replay, expects the system program's `AccountAlreadyInUse`, submits it without preflight so the rejection lands on chain, and compares the config, key, mandate and receipt accounts before and after. Every transaction is read back at finalized commitment. Any mismatch stops the run with an error.

## Checking the record

Anyone can check the record against devnet without keys:

```bash
solana program show <program id> --url https://api.devnet.solana.com
solana program dump <program id> onchain.so --url https://api.devnet.solana.com
shasum -a 256 onchain.so
solana confirm -v <signature> --url https://api.devnet.solana.com
```

`solana account <receipt address> --url https://api.devnet.solana.com --output json` shows the receipt's 185 bytes, which decode as `ConsumedAuthorization`.
