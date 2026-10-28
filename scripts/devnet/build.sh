#!/usr/bin/env bash
# Builds the devnet binaries of the leg and the test adapter with the CI
# toolchain into programs/solevm-remote-leg/target/deploy-devnet. The
# production build in target/deploy is left untouched.
# shellcheck source=common.sh
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"
require_v1_sources
require "$CARGO_BUILD_SBF"
require python3
"$CARGO_BUILD_SBF" --version | grep -q "cargo-build-sbf $CARGO_BUILD_SBF_VERSION" ||
  die "need cargo-build-sbf $CARGO_BUILD_SBF_VERSION: cargo install cargo-build-sbf --version $CARGO_BUILD_SBF_VERSION --locked"

cd "$PROGRAM_DIR"
for manifest in Cargo.toml test-strategy/Cargo.toml; do
  RUSTFLAGS="--cfg solevm_devnet" CARGO_TARGET_DIR="$PROGRAM_DIR/target/devnet" \
    "$CARGO_BUILD_SBF" --tools-version "$PLATFORM_TOOLS_VERSION" --arch v0 \
    --manifest-path "$manifest" --sbf-out-dir "$DEPLOY_DIR"
done
# cargo-build-sbf writes a random program keypair beside each binary; the
# deploy uses the keys in $SOLEVM_HOME instead.
rm -f "$DEPLOY_DIR"/*-keypair.json

# Each binary must embed its devnet id and not its production id.
python3 - "$DEPLOY_DIR" "$LEG_DEVNET_ID" "$LEG_PRODUCTION_ID" "$ADAPTER_DEVNET_ID" "$ADAPTER_PRODUCTION_ID" <<'EOF'
import pathlib, sys
ALPHABET = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz"
def raw(key):
    value = 0
    for char in key:
        value = value * 58 + ALPHABET.index(char)
    return value.to_bytes(32, "big")
out, leg, leg_prod, adapter, adapter_prod = sys.argv[1:]
for name, want, avoid in (("solevm_remote_leg", leg, leg_prod), ("solevm_test_strategy", adapter, adapter_prod)):
    data = (pathlib.Path(out) / f"{name}.so").read_bytes()
    if raw(want) not in data or raw(avoid) in data:
        sys.exit(f"{name}.so does not declare the devnet id {want}")
EOF

for binary in "$DEPLOY_DIR"/solevm_remote_leg.so "$DEPLOY_DIR"/solevm_test_strategy.so; do
  printf '%s  %s bytes  sha256 %s\n' "$(basename "$binary")" "$(wc -c <"$binary" | tr -d ' ')" "$(sha256 "$binary")"
done
