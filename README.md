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

The accounting model also builds without the standard library.

```bash
cargo check -p accounting-model --no-default-features
```

Raise the property test case count for a longer run.

```bash
PROPTEST_CASES=250000 cargo test --release -p accounting-model --test properties
```
