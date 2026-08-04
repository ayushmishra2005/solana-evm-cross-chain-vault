# solana-evm-cross-chain-vault
Production-grade cross-chain asynchronous vault protocol for Solana and EVM, built with Rust, Anchor, Solidity and Foundry. Includes NAV accounting, cross-chain reconciliation, invariant testing and a Rust indexer.

## Build and test

Install a stable Rust toolchain. The workspace needs Rust 1.90 or newer.

```bash
cargo fmt --all --check
cargo check --workspace --all-targets
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
```

All library crates also build without the standard library.

```bash
cargo check -p accounting-model --no-default-features
cargo check -p protocol-types --no-default-features
cargo check -p xchain-sim --no-default-features
```

Raise the property test case count for a longer run.

```bash
PROPTEST_CASES=250000 cargo test --release -p accounting-model --test properties
PROPTEST_CASES=250000 cargo test --release -p protocol-types --test properties
PROPTEST_CASES=250000 cargo test --release -p xchain-sim --test properties
```

## Solidity vault

The Foundry project lives in `contracts/solevm-vault`. Install its pinned
dependencies once, then run the suite from that directory.

```bash
cd contracts/solevm-vault
forge install foundry-rs/forge-std@v1.16.2 --no-git --shallow
forge install OpenZeppelin/openzeppelin-contracts@v5.7.0 --no-git --shallow

forge fmt --check
forge build
forge test
forge test --gas-report
forge coverage --no-match-coverage "(test|script)"
```

The `soak` profile raises the fuzz and invariant budgets.

```bash
FOUNDRY_PROFILE=soak forge test
```

## Differential harness

`crates/evm-differential` generates scenarios from the Rust accounting model
and the Foundry harness replays them against the vault. The `differential`
profile enables FFI, which the normal suite does not need.

```bash
cd contracts/solevm-vault
RUN_DIFFERENTIAL=1 FOUNDRY_PROFILE=differential \
  forge test --match-contract RustModelDifferentialTest -vv
```

`DIFF_SEED`, `DIFF_CASES` and `DIFF_STEPS` widen the run.

```bash
RUN_DIFFERENTIAL=1 DIFF_SEED=1 DIFF_CASES=512 DIFF_STEPS=5 \
  FOUNDRY_PROFILE=differential \
  forge test --match-contract RustModelDifferentialTest -vv
```

Inspect one scenario, or the reachability of a whole run, without Foundry.

```bash
cargo run -p evm-differential -- --seed 1 --cases 64 --steps 4 --stats
cargo run -p evm-differential -- --seed 1 --cases 64 --steps 4 --describe 12
```

## Solana remote leg

`programs/solevm-remote-leg` is a separate Cargo workspace. Build the program
first, because the LiteSVM and Mollusk suites load the compiled object.

```bash
cd programs/solevm-remote-leg
cargo-build-sbf --tools-version v1.54
```

Then run the suite from the same directory.

```bash
cargo fmt --all --check
cargo check --workspace --all-targets
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
```

Run one layer on its own.

```bash
cargo test --lib
cargo test --test initialize --test freeze --test adversarial
cargo test --test compute -- --nocapture
```
