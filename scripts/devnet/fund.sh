#!/usr/bin/env bash
# Shows what the deployer holds against what a full deploy and run need.
# With an amount, asks the devnet faucet for that many SOL once.
# Usage: fund.sh [SOL]
# shellcheck source=common.sh
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"
require "$SOLANA"

cluster_name="$(cluster)"
[ "$cluster_name" = devnet ] || die "fund.sh is for devnet; localnet.sh funds its own validator"
deployer="$(pubkey deployer)"

rent() {
  "$SOLANA" rent --url "$SOLEVM_RPC_URL" --lamports "$1" | awk '/Rent-exempt minimum/ {print $3}'
}

# The run's floor in the driver (MIN_RUN_BALANCE), then per program: program
# data (a 45-byte header plus the binary), the 36-byte program account, and
# one 5000-lamport write per 900 bytes plus four two-signature transactions.
need=150000000
for binary in "$DEPLOY_DIR/solevm_remote_leg.so" "$DEPLOY_DIR/solevm_test_strategy.so"; do
  [ -f "$binary" ] || die "missing $binary; run scripts/devnet/build.sh first"
  size=$(wc -c <"$binary")
  writes=$(((size + 899) / 900))
  need=$((need + $(rent $((size + 45))) + $(rent 36) + writes * 5000 + 4 * 10000))
done

if [ $# -gt 0 ]; then
  "$SOLANA" airdrop --url "$SOLEVM_RPC_URL" "$1" "$deployer" || {
    echo "The faucet refused. Request devnet SOL for $deployer at https://faucet.solana.com" >&2
    exit 1
  }
fi

have="$("$SOLANA" balance --url "$SOLEVM_RPC_URL" --lamports "$deployer" | awk '{print $1}')"
echo "deployer $deployer holds $have lamports; deploy and run need about $need"
[ "$have" -ge "$need" ] || {
  echo "Short by $((need - have)) lamports. Request devnet SOL for $deployer at https://faucet.solana.com" >&2
  exit 1
}
