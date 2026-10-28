#!/usr/bin/env bash
# Creates the disposable devnet keys under $SOLEVM_HOME/keys (default
# ~/.config/solevm-devnet/keys). Existing keys are kept. Prints public keys only.
# shellcheck source=common.sh
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"
require_v1_sources
require "$SOLANA_KEYGEN"

mkdir -p "$KEY_DIR"
chmod 700 "$SOLEVM_HOME" "$KEY_DIR"
for role in $ROLES; do
  path="$KEY_DIR/$role.json"
  if [ ! -f "$path" ]; then
    "$SOLANA_KEYGEN" new --no-bip39-passphrase --silent --outfile "$path" >/dev/null
  fi
  chmod 600 "$path"
  printf '%-20s %s\n' "$role" "$("$SOLANA_KEYGEN" pubkey "$path")"
done

leg="$(pubkey leg-program)"
adapter="$(pubkey adapter-program)"
if [ "$leg" != "$LEG_DEVNET_ID" ] || [ "$adapter" != "$ADAPTER_DEVNET_ID" ]; then
  cat >&2 <<EOF

These program keys differ from the ids the devnet build declares. To run your
own instance, change the cfg(solevm_devnet) declare_id! in
  programs/solevm-remote-leg/src/lib.rs               to $leg
  programs/solevm-remote-leg/test-strategy/src/lib.rs to $adapter
and LEG_DEVNET_ID / ADAPTER_DEVNET_ID in scripts/devnet/common.sh, then build.
EOF
fi
