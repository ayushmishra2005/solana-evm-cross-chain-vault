#!/usr/bin/env bash
# Re-verifies an evidence file against the cluster it names: program bytes,
# upgrade authority, and every transaction's slot, program and result at
# finalized commitment. Reads public data only; needs no keys.
# Usage: verify.sh [EVIDENCE_JSON]   (default: evidence/devnet/v1.json)
# Set SOLEVM_RPC_URL to check through a different RPC provider.
set -euo pipefail
repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd -P)"
evidence="$(cd "$(dirname "${1:-$repo/evidence/devnet/v1.json}")" && pwd -P)/$(basename "${1:-$repo/evidence/devnet/v1.json}")"
# An inherited RUSTFLAGS would replace the driver's `--cfg solevm_devnet`.
unset RUSTFLAGS CARGO_ENCODED_RUSTFLAGS
cd "$repo/programs/solevm-remote-leg/devnet"
cargo run --locked --release --quiet -- verify "$evidence"
