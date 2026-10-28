#!/usr/bin/env bash
# Builds the V2 devnet binaries of the leg and the authorization verifier with
# the CI toolchain into programs/solevm-remote-leg/target/deploy-devnet-v2.
# The test adapter is rebuilt only to confirm it reproduces the V1 binary V2
# reuses. The production build in target/deploy and the V1 build in
# target/deploy-devnet are left untouched.
# shellcheck source=common.sh
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"
require "$CARGO_BUILD_SBF"
require python3
"$CARGO_BUILD_SBF" --version | grep -q "cargo-build-sbf $CARGO_BUILD_SBF_VERSION" ||
  die "need cargo-build-sbf $CARGO_BUILD_SBF_VERSION: cargo install cargo-build-sbf --version $CARGO_BUILD_SBF_VERSION --locked"
[ "$DEPLOY_DIR" != "$V1_DEPLOY_DIR" ] || die "the V2 artifact directory must differ from V1's"

cd "$PROGRAM_DIR"
# One target directory per program: built as its own package, the verifier
# writes an unhashed library over the `no-entrypoint` one the leg links, so a
# shared directory breaks every rebuild after the first.
for build in leg:Cargo.toml adapter:test-strategy/Cargo.toml "verifier:$VERIFIER_DIR/Cargo.toml"; do
  RUSTFLAGS="--cfg solevm_devnet" CARGO_TARGET_DIR="$PROGRAM_DIR/target/devnet-v2/${build%%:*}" \
    "$CARGO_BUILD_SBF" --tools-version "$PLATFORM_TOOLS_VERSION" --arch v0 \
    --manifest-path "${build#*:}" --sbf-out-dir "$DEPLOY_DIR"
done
# cargo-build-sbf writes a random program keypair beside each binary; the
# deploy uses the keys in $SOLEVM_V2_HOME instead.
rm -f "$DEPLOY_DIR"/*-keypair.json

# Each binary must embed its V2 id and none of the V1 or production ids.
python3 - "$DEPLOY_DIR" "$LEG_V2_ID" "$VERIFIER_V2_ID" "$ADAPTER_DEVNET_ID" \
  "$V1_LEG_DEVNET_ID" "$V1_VERIFIER_DEVNET_ID" "$LEG_PRODUCTION_ID" "$ADAPTER_PRODUCTION_ID" <<'EOF'
import pathlib, sys
ALPHABET = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz"
def raw(key):
    value = 0
    for char in key:
        value = value * 58 + ALPHABET.index(char)
    return value.to_bytes(32, "big")
out, leg, verifier, adapter, *avoid = sys.argv[1:]
for name, want in (
    ("solevm_remote_leg", [leg, verifier]),
    ("es256_authorization_verifier", [verifier]),
    ("solevm_test_strategy", [adapter]),
):
    data = (pathlib.Path(out) / f"{name}.so").read_bytes()
    for key in want:
        if raw(key) not in data:
            sys.exit(f"{name}.so does not embed {key}")
    for key in avoid:
        if raw(key) in data:
            sys.exit(f"{name}.so embeds {key}, a V1 or production id")
EOF

adapter_hash="$(sha256 "$DEPLOY_DIR/solevm_test_strategy.so")"
[ "$adapter_hash" = "$ADAPTER_DEVNET_SHA256" ] ||
  die "the adapter build ($adapter_hash) no longer reproduces the reused V1 binary $ADAPTER_DEVNET_SHA256"

for name in solevm_remote_leg es256_authorization_verifier solevm_test_strategy; do
  binary="$DEPLOY_DIR/$name.so"
  [ "$(sbpf_version "$binary")" = 0 ] || die "$binary is not SBPFv0"
  printf '%s  %s bytes  sha256 %s\n' "$(basename "$binary")" "$(wc -c <"$binary" | tr -d ' ')" "$(sha256 "$binary")"
done
