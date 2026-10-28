#!/usr/bin/env bash
# Creates the disposable V2 keys under $SOLEVM_V2_HOME/keys (default
# ~/.config/solevm-devnet-v2/keys). Existing keys are kept, never replaced.
# Prints public keys only. The P-256 risk key is not here: the driver makes a
# fresh one in memory for every run.
# shellcheck source=common.sh
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"
require "$SOLANA_KEYGEN"

mkdir -p "$KEY_DIR"
chmod 700 "$SOLEVM_V2_HOME" "$KEY_DIR"
for role in $ROLES; do
  path="$KEY_DIR/$role.json"
  if [ ! -f "$path" ]; then
    "$SOLANA_KEYGEN" new --no-bip39-passphrase --silent --outfile "$path" >/dev/null
  fi
  chmod 600 "$path"
  printf '%-20s %s\n' "$role" "$("$SOLANA_KEYGEN" pubkey "$path")"
done
refuse_cli_wallet

leg="$(pubkey leg-program)"
verifier="$(pubkey verifier-program)"
for id in "$leg" "$verifier"; do
  forbidden "$id" && die "$id is a V1 or production program id"
done
if [ "$leg" != "$LEG_V2_ID" ] || [ "$verifier" != "$VERIFIER_V2_ID" ]; then
  cat >&2 <<EOF

These program keys differ from the ids the V2 build declares. To run your own
instance, change the cfg(solevm_devnet) declare_id! in
  programs/solevm-remote-leg/src/lib.rs       to $leg
  programs/authorization-verifier/src/lib.rs  to $verifier
and LEG_V2_ID / VERIFIER_V2_ID in scripts/devnet-v2/common.sh, then build.
EOF
fi
