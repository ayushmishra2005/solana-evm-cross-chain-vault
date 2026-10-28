#!/usr/bin/env bash
# Re-verifies a V2 record against the cluster it names: genesis, program bytes
# and upgrade authorities, every transaction's slot, programs and exact result
# at finalized commitment, and the authorization accounts and receipts.
# Reads public data only and needs no keys. A localnet record can only be
# checked while its validator still runs.
# Usage: verify.sh EVIDENCE_JSON
# Set SOLEVM_RPC_URL to check through a different RPC provider.
set -euo pipefail
[ $# -eq 1 ] || {
  echo "usage: $0 EVIDENCE_JSON" >&2
  exit 2
}
repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd -P)"
evidence="$(cd "$(dirname "$1")" && pwd -P)/$(basename "$1")"
[ "$evidence" != "$repo/evidence/devnet/v1.json" ] || {
  echo "error: evidence/devnet/v1.json is a V1 record; use scripts/devnet/verify.sh" >&2
  exit 1
}
# An inherited RUSTFLAGS would replace the driver's `--cfg solevm_devnet`.
unset RUSTFLAGS CARGO_ENCODED_RUSTFLAGS
cd "$repo/programs/solevm-remote-leg/devnet"
cargo run --locked --release --quiet -- verify-v2 "$evidence"
