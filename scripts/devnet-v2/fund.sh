#!/usr/bin/env bash
# Shows what the V2 deployer holds against what deploying the V2 verifier and
# leg and one scenario run need. Reads balances and rent only, unless given an
# amount: then, with SOLEVM_V2_DEVNET_APPROVED=1, it asks the devnet faucet
# once for that many SOL for the V2 deployer and nobody else.
# Usage: fund.sh [SOL]
# shellcheck source=common.sh
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"
require "$SOLANA"

cluster_name="$(cluster)"
[ "$cluster_name" = devnet ] || die "fund.sh is for devnet; localnet.sh funds its own validator"
refuse_cli_wallet
deployer="$(pubkey deployer)"

rent() {
  "$SOLANA" rent --url "$SOLEVM_RPC_URL" --lamports "$1" | awk '/Rent-exempt minimum/ {print $3}'
}

# The run's floor in the driver (MIN_RUN_BALANCE), then per program: program
# data (a 45-byte header plus the binary), the 36-byte program account, and
# one 5000-lamport write per 900 bytes plus four two-signature transactions.
# The buffer's rent is paid first and drained back to the payer before the
# program data is funded, so it adds nothing at the peak. The adapter is
# reused, so it costs nothing here.
run_floor=200000000
need=$run_floor
for binary in "$DEPLOY_DIR/es256_authorization_verifier.so" "$DEPLOY_DIR/solevm_remote_leg.so"; do
  [ -f "$binary" ] || die "missing $binary; run scripts/devnet-v2/build.sh first"
  size=$(wc -c <"$binary")
  writes=$(((size + 899) / 900))
  program_data=$(rent $((size + 45)))
  cost=$((program_data + $(rent 36) + writes * 5000 + 4 * 10000))
  echo "$(basename "$binary"): $size bytes, program data rent $program_data, fees and program account $((cost - program_data))"
  need=$((need + cost))
done

if [ $# -gt 0 ]; then
  require_devnet_approval "$cluster_name" "requesting an airdrop"
  case "$1" in
    '' | *[!0-9.]* | *.*.*) die "the amount must be a number of SOL" ;;
  esac
  forbidden "$deployer" && die "$deployer is a V1 or production program id"
  echo "requesting $1 SOL from the devnet faucet for the V2 deployer $deployer"
  "$SOLANA" airdrop --url "$SOLEVM_RPC_URL" "$1" "$deployer" || {
    echo "The faucet refused. Request devnet SOL for $deployer at https://faucet.solana.com" >&2
    exit 1
  }
fi

have="$("$SOLANA" balance --url "$SOLEVM_RPC_URL" --lamports "$deployer" | awk '{print $1}')"
echo "deployer $deployer holds $have lamports; deploy and run need about $need (run floor $run_floor)"
[ "$have" -ge "$need" ] || {
  echo "Short by $((need - have)) lamports. Request devnet SOL for $deployer at https://faucet.solana.com" >&2
  exit 1
}
