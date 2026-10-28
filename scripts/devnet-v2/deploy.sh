#!/usr/bin/env bash
# Deploys the V2 verifier and V2 leg under fresh program ids, with the
# disposable V2 deployer as upgrade authority. A program already running the
# same binary is left alone. There is no upgrade path: a V2 id running any
# other binary stops the script, and V1 or production ids are refused.
# The reused devnet adapter is only checked, read-only: it must run the
# pinned binary under the V1 deployer. The cluster must still accept SBPFv0
# deployments and run the gates V2 needs; an existing buffer is resumed only if
# the V2 deployer owns it.
# Devnet needs SOLEVM_V2_DEVNET_APPROVED=1 as a deliberate second step.
# Optional: SOLEVM_CU_PRICE sets a priority fee in micro-lamports per CU;
# SOLEVM_USE_RPC=1 sends writes through the RPC instead of validator TPUs.
# shellcheck source=common.sh
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"
require "$SOLANA"
require "$SOLANA_KEYGEN"

cluster_name="$(cluster)"
require_devnet_approval "$cluster_name" "deploying"
refuse_cli_wallet
require_runtime
require_v0_deployment
state="$(state_dir "$cluster_name")"
deployer="$(key deployer)"
deployer_id="$(pubkey deployer)"
options=()
if [ -n "${SOLEVM_CU_PRICE:-}" ]; then
  options+=(--with-compute-unit-price "$SOLEVM_CU_PRICE")
fi
if [ "${SOLEVM_USE_RPC:-0}" = 1 ]; then
  options+=(--use-rpc)
fi

# The adapter V2 reuses: present, pinned binary, V1 deployer as authority.
adapter="$DEPLOY_DIR/solevm_test_strategy.so"
[ -f "$adapter" ] || die "missing $adapter; run scripts/devnet-v2/build.sh"
[ "$(sha256 "$adapter")" = "$ADAPTER_DEVNET_SHA256" ] || die "$adapter is not the pinned adapter binary"
adapter_authority="$("$SOLANA" program show --url "$SOLEVM_RPC_URL" --output json "$ADAPTER_DEVNET_ID" |
  sed -n 's/.*"authority": *"\([1-9A-HJ-NP-Za-km-z]*\)".*/\1/p')" ||
  die "adapter $ADAPTER_DEVNET_ID is not deployed on $cluster_name"
[ "$adapter_authority" = "$ADAPTER_DEVNET_AUTHORITY" ] ||
  die "adapter $ADAPTER_DEVNET_ID upgrade authority is '$adapter_authority', not $ADAPTER_DEVNET_AUTHORITY"
runs_binary "$ADAPTER_DEVNET_ID" "$adapter" || die "adapter $ADAPTER_DEVNET_ID does not run the pinned binary"
echo "adapter $ADAPTER_DEVNET_ID runs the pinned binary under $ADAPTER_DEVNET_AUTHORITY (reused, not deployed)"

deploy() {
  local name="$1" program_key="$2" declared="$3"
  local binary="$DEPLOY_DIR/$name.so" id
  [ -f "$binary" ] || die "missing $binary; run scripts/devnet-v2/build.sh"
  [ "$(sbpf_version "$binary")" = 0 ] || die "$binary is not SBPFv0"
  id="$("$SOLANA_KEYGEN" pubkey "$program_key")"
  [ "$id" = "$declared" ] || die "$program_key is $id but the V2 build declares $declared"
  forbidden "$id" && die "$id is a V1 or production program; V2 never deploys to it"

  if "$SOLANA" program show --url "$SOLEVM_RPC_URL" "$id" >/dev/null 2>&1; then
    runs_binary "$id" "$binary" || die "$id runs a different $name binary; V2 scripts never upgrade"
    echo "$name already runs this binary at $id"
    return
  fi

  # A fixed buffer key lets an interrupted deploy resume instead of leaking rent.
  # Buffer keys are per cluster and live only in the V2 home, so no V1 or other
  # cluster's buffer is ever picked up.
  local buffer="$KEY_DIR/buffer-v2-$cluster_name-$name.json" buffer_id
  [ -f "$buffer" ] || "$SOLANA_KEYGEN" new --no-bip39-passphrase --silent --outfile "$buffer" >/dev/null
  buffer_id="$("$SOLANA_KEYGEN" pubkey "$buffer")"
  forbidden "$buffer_id" && die "$buffer_id is a V1 or production program id"
  case "$(probe buffer "$buffer_id" "$deployer_id")" in
    absent) ;;
    ok) echo "resuming the $name upload into buffer $buffer_id, owned by the V2 deployer" ;;
    *) die "cannot check buffer $buffer_id" ;;
  esac
  "$SOLANA" program deploy \
    --url "$SOLEVM_RPC_URL" \
    --keypair "$deployer" \
    --fee-payer "$deployer" \
    --program-id "$program_key" \
    --upgrade-authority "$deployer" \
    --buffer "$buffer" \
    --max-sign-attempts 50 \
    ${options[@]+"${options[@]}"} \
    --output json \
    "$binary" >"$state/deploy-$name.json.partial"
  mv "$state/deploy-$name.json.partial" "$state/deploy-$name.json"
  echo "$name deployed: $(cat "$state/deploy-$name.json")"
  runs_binary "$id" "$binary" || die "$id does not run $binary after deploy"
}

deploy es256_authorization_verifier "$(key verifier-program)" "$VERIFIER_V2_ID"
deploy solevm_remote_leg "$(key leg-program)" "$LEG_V2_ID"
for id in "$VERIFIER_V2_ID" "$LEG_V2_ID"; do
  "$SOLANA" program show --url "$SOLEVM_RPC_URL" "$id"
done
