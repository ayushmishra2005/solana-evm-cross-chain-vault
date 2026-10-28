#!/usr/bin/env bash
# Rehearses the V2 deploy and scenario on a throwaway local validator with
# the native secp256r1 program. The reused adapter is loaded from the pinned
# binary under its devnet id and V1 upgrade authority, so V2 deploys only the
# verifier and the leg, as it would on devnet. Its record is marked localnet
# and is never devnet validation.
# A test validator enables every feature it knows. To rehearse devnet rather
# than a future cluster, the script first reads devnet's feature set (genesis
# checked, read-only) and deactivates locally every gate devnet has not
# activated: among them SBPFv0 deployment, rent and account-limit changes.
# The mirror is recorded with the evidence.
# Usage: localnet.sh [LABEL]
# SOLEVM_AGAVE_BIN: directory with the Agave release to use (prepended to PATH).
# SOLEVM_DEVNET_RPC_URL: devnet endpoint to mirror (https://api.devnet.solana.com).
# SOLEVM_MIRROR_DEVNET_FEATURES=0 starts with every feature active instead.
# SOLEVM_KEEP_VALIDATOR=1 keeps the validator running after the run, until
# interrupted, so the record can be re-verified and inspected.
if [ -n "${SOLEVM_AGAVE_BIN:-}" ]; then
  PATH="$SOLEVM_AGAVE_BIN:$PATH"
  export PATH
fi
# shellcheck source=common.sh
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"
require solana-test-validator
require "$SOLANA"

export SOLEVM_RPC_URL="http://127.0.0.1:8899"
export SOLEVM_CLUSTER=localnet
scripts="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
ledger="$PROGRAM_DIR/target/localnet-v2-ledger"
log="$PROGRAM_DIR/target/localnet-v2-validator.log"
adapter="$DEPLOY_DIR/solevm_test_strategy.so"
[ -f "$adapter" ] || die "missing $adapter; run scripts/devnet-v2/build.sh"
[ "$(sha256 "$adapter")" = "$ADAPTER_DEVNET_SHA256" ] || die "$adapter is not the pinned adapter binary"

if "$SOLANA" cluster-version --url "$SOLEVM_RPC_URL" >/dev/null 2>&1; then
  die "something already answers on $SOLEVM_RPC_URL; stop it first"
fi
echo "validator: $(solana-test-validator --version)"

state="$(state_dir localnet)"
mirror="$state/localnet-feature-mirror.json"
rm -f "$mirror"
deactivate=()
if [ "${SOLEVM_MIRROR_DEVNET_FEATURES:-1}" = 1 ]; then
  require python3
  devnet_url="${SOLEVM_DEVNET_RPC_URL:-https://api.devnet.solana.com}"
  [ "$("$SOLANA" genesis-hash --url "$devnet_url")" = "$DEVNET_GENESIS" ] ||
    die "$devnet_url is not devnet; cannot mirror its features"
  devnet_slot="$("$SOLANA" slot --url "$devnet_url")"
  "$SOLANA" feature status --url "$devnet_url" --display-all --output json-compact >"$state/devnet-features.json"
  python3 - "$state/devnet-features.json" "$mirror" "$devnet_url" "$DEVNET_GENESIS" "$devnet_slot" \
    "$("$SOLANA" --version)" <<'PYTHON'
import datetime, json, sys
source, out, url, genesis, slot, cli = sys.argv[1:]
features = json.load(open(source))["features"]
inactive = [f for f in features if f["status"] != "active"]
json.dump({
    "mirrored_cluster": "devnet",
    "rpc_url": url,
    "genesis_hash": genesis,
    "slot": int(slot),
    "queried_at_utc": datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
    "cli": cli,
    "features_known_to_cli": len(features),
    "deactivated": [f["id"] for f in inactive],
    "descriptions": {f["id"]: f["description"] for f in inactive},
}, open(out, "w"), indent=2)
PYTHON
  rm -f "$state/devnet-features.json"
  while IFS= read -r id; do
    deactivate+=(--deactivate-feature "$id")
  done < <(python3 -c 'import json,sys; print("\n".join(json.load(open(sys.argv[1]))["deactivated"]))' "$mirror")
  echo "mirroring devnet at slot $devnet_slot: $((${#deactivate[@]} / 2)) features deactivated"
  export SOLEVM_FEATURE_MIRROR="$mirror"
fi

solana-test-validator --ledger "$ledger" --reset --quiet --bind-address 127.0.0.1 \
  --upgradeable-program "$ADAPTER_DEVNET_ID" "$adapter" "$ADAPTER_DEVNET_AUTHORITY" \
  ${deactivate[@]+"${deactivate[@]}"} \
  >"$log" 2>&1 &
validator=$!
trap 'kill "$validator" 2>/dev/null || true; wait "$validator" 2>/dev/null || true' EXIT
for _ in $(seq 1 60); do
  "$SOLANA" cluster-version --url "$SOLEVM_RPC_URL" >/dev/null 2>&1 && break
  sleep 1
done
"$SOLANA" cluster-version --url "$SOLEVM_RPC_URL" >/dev/null || die "local validator did not start; see $log"
cluster >/dev/null

require_runtime

"$SOLANA" airdrop --url "$SOLEVM_RPC_URL" 100 "$(pubkey deployer)" >/dev/null
"$scripts/deploy.sh"
"$scripts/run.sh" "${1:-local-1}"

if [ "${SOLEVM_KEEP_VALIDATOR:-0}" = 1 ]; then
  echo "validator $validator still serves $SOLEVM_RPC_URL; interrupt to stop it"
  wait "$validator"
fi
