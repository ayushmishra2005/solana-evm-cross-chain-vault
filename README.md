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
