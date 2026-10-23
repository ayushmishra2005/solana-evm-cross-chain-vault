#!/usr/bin/env bash
# Deploys the devnet builds of the leg and the test adapter, with the
# disposable deployer as upgrade authority. A program already running the
# same binary is left alone; a different binary needs SOLEVM_ALLOW_UPGRADE=1.
# Optional: SOLEVM_CU_PRICE sets a priority fee in micro-lamports per CU.
# Writes go to validator TPUs; SOLEVM_USE_RPC=1 sends them through the RPC
# instead, which public endpoints rate-limit heavily.
# shellcheck source=common.sh
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"
require "$SOLANA"
require "$SOLANA_KEYGEN"

cluster_name="$(cluster)"
state="$(state_dir "$cluster_name")"
deployer="$(key deployer)"
options=()
if [ -n "${SOLEVM_CU_PRICE:-}" ]; then
  options+=(--with-compute-unit-price "$SOLEVM_CU_PRICE")
fi
if [ "${SOLEVM_USE_RPC:-0}" = 1 ]; then
  options+=(--use-rpc)
fi

deploy() {
  local name="$1" program_key="$2" declared="$3"
  local binary="$DEPLOY_DIR/$name.so"
  [ -f "$binary" ] || die "missing $binary; run scripts/devnet/build.sh"
  local id
  id="$("$SOLANA_KEYGEN" pubkey "$program_key")"
  [ "$id" = "$declared" ] || die "$program_key is $id but the devnet build declares $declared"

  if "$SOLANA" program show --url "$SOLEVM_RPC_URL" "$id" >/dev/null 2>&1; then
    local scratch
    scratch="$(mktemp -d)"
    "$SOLANA" program dump --url "$SOLEVM_RPC_URL" "$id" "$scratch/onchain.so" >/dev/null
    head -c "$(wc -c <"$binary")" "$scratch/onchain.so" >"$scratch/prefix.so"
    if [ "$(sha256 "$scratch/prefix.so")" = "$(sha256 "$binary")" ]; then
      rm -rf "$scratch"
      echo "$name already runs this binary at $id"
      return
    fi
    rm -rf "$scratch"
    [ "${SOLEVM_ALLOW_UPGRADE:-0}" = 1 ] || die "$id runs a different $name binary; set SOLEVM_ALLOW_UPGRADE=1 to upgrade it"
  fi

  # A fixed buffer key lets an interrupted deploy resume instead of leaking rent.
  local buffer="$KEY_DIR/buffer-$name.json"
  [ -f "$buffer" ] || "$SOLANA_KEYGEN" new --no-bip39-passphrase --silent --outfile "$buffer" >/dev/null
  "$SOLANA" program deploy \
    --url "$SOLEVM_RPC_URL" \
    --keypair "$deployer" \
    --program-id "$program_key" \
    --upgrade-authority "$deployer" \
    --buffer "$buffer" \
    --max-sign-attempts 50 \
    ${options[@]+"${options[@]}"} \
    --output json \
    "$binary" >"$state/deploy-$name.json.partial"
  mv "$state/deploy-$name.json.partial" "$state/deploy-$name.json"
  echo "$name deployed: $(cat "$state/deploy-$name.json")"
}

deploy solevm_remote_leg "$(key leg-program)" "$LEG_DEVNET_ID"
deploy solevm_test_strategy "$(key adapter-program)" "$ADAPTER_DEVNET_ID"
"$SOLANA" program show --url "$SOLEVM_RPC_URL" "$LEG_DEVNET_ID"
"$SOLANA" program show --url "$SOLEVM_RPC_URL" "$ADAPTER_DEVNET_ID"
