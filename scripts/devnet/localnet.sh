#!/usr/bin/env bash
# Rehearses deploy and run on a throwaway local validator before spending
# devnet SOL. Its evidence is recorded as cluster "localnet", never as devnet.
# Usage: localnet.sh [LABEL]
# shellcheck source=common.sh
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"
require_v1_sources
require solana-test-validator
require "$SOLANA"

export SOLEVM_RPC_URL="http://127.0.0.1:8899"
export SOLEVM_ALLOW_LOCALNET=1
scripts="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
ledger="$PROGRAM_DIR/target/localnet-ledger"

solana-test-validator --ledger "$ledger" --reset --quiet --bind-address 127.0.0.1 \
  >"$PROGRAM_DIR/target/localnet-validator.log" 2>&1 &
validator=$!
trap 'kill "$validator" 2>/dev/null || true; wait "$validator" 2>/dev/null || true' EXIT
for _ in $(seq 1 60); do
  "$SOLANA" cluster-version --url "$SOLEVM_RPC_URL" >/dev/null 2>&1 && break
  sleep 1
done
"$SOLANA" cluster-version --url "$SOLEVM_RPC_URL" >/dev/null || die "local validator did not start; see target/localnet-validator.log"

"$SOLANA" airdrop --url "$SOLEVM_RPC_URL" 50 "$(pubkey deployer)" >/dev/null
"$scripts/deploy.sh"
"$scripts/run.sh" "${1:-local-1}"
