# Shared settings for the devnet scripts. Sourced, never executed.
# Keys live outside the repository; nothing here prints private key material.
# shellcheck shell=bash disable=SC2034  # the sourcing scripts read these

set -euo pipefail
umask 077

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd -P)"
PROGRAM_DIR="$REPO_ROOT/programs/solevm-remote-leg"
DRIVER_DIR="$PROGRAM_DIR/devnet"
DEPLOY_DIR="$PROGRAM_DIR/target/deploy-devnet"

SOLEVM_HOME="${SOLEVM_HOME:-$HOME/.config/solevm-devnet}"
SOLEVM_RPC_URL="${SOLEVM_RPC_URL:-https://api.devnet.solana.com}"
KEY_DIR="$SOLEVM_HOME/keys"
export SOLEVM_HOME SOLEVM_RPC_URL

SOLANA="${SOLANA:-solana}"
SOLANA_KEYGEN="${SOLANA_KEYGEN:-solana-keygen}"
CARGO_BUILD_SBF="${CARGO_BUILD_SBF:-cargo-build-sbf}"
CARGO_BUILD_SBF_VERSION="4.4.0"
PLATFORM_TOOLS_VERSION="v1.58"

DEVNET_GENESIS="EtWTRABZaYq6iMfeYKouRu166VU2xqa1wcaWoxPkrZBG"
MAINNET_GENESIS="5eykt4UsFv8P8NJdTREpY1vzqKqZKvdpKuc147dw2N9d"
TESTNET_GENESIS="4uhcVJyU9pJkvQyS88uRDiswHXSCkY3zQawwpjk2NsNY"

# Every disposable key the run uses, by file name without `.json`.
ROLES="deployer leg-program adapter-program administrator guardian transport-verifier asset-transport escrow-owner attacker"

# The program ids the devnet build declares under `--cfg solevm_devnet`.
LEG_DEVNET_ID="27zReYgZvFTaUSWwob4qirN2Yxekcm32mnQDpMzeTyAE"
ADAPTER_DEVNET_ID="8bFiHDwdAurdhTv5F9JaVD4QgdJ2XxEHRK2BAyiCyjZ8"
LEG_PRODUCTION_ID="4sLaaRdiY74cvqVCXLjW2wPncy2ArQkTRauNTdphKByo"
ADAPTER_PRODUCTION_ID="GbFoY2KE6WAWHApg8na1Db8jtnR5cbaeCpA5TGA9UmcZ"

die() {
  printf 'error: %s\n' "$*" >&2
  exit 1
}

require() {
  command -v "$1" >/dev/null 2>&1 || die "$1 is not installed"
}

# Physical path of $1, resolved through its nearest existing ancestor.
resolve() {
  local path="$1" rest=""
  while [ ! -d "$path" ]; do
    rest="/$(basename "$path")$rest"
    path="$(dirname "$path")"
  done
  printf '%s%s\n' "$(cd "$path" && pwd -P)" "$rest"
}

case "$(resolve "$SOLEVM_HOME")/" in
  "$REPO_ROOT"/*) die "SOLEVM_HOME ($SOLEVM_HOME) must be outside the repository" ;;
esac

key() {
  local path="$KEY_DIR/$1.json"
  [ -f "$path" ] || die "missing $path; run scripts/devnet/keygen.sh"
  printf '%s\n' "$path"
}

pubkey() {
  "$SOLANA_KEYGEN" pubkey "$(key "$1")"
}

# Prints `devnet` or `localnet` for the configured RPC endpoint and refuses
# every other cluster. Localnet needs SOLEVM_ALLOW_LOCALNET=1.
cluster() {
  local genesis
  genesis="$("$SOLANA" genesis-hash --url "$SOLEVM_RPC_URL")" || die "cannot reach $SOLEVM_RPC_URL"
  case "$genesis" in
    "$DEVNET_GENESIS") echo devnet ;;
    "$MAINNET_GENESIS" | "$TESTNET_GENESIS") die "refusing $SOLEVM_RPC_URL: genesis $genesis is mainnet-beta or testnet" ;;
    *)
      [ "${SOLEVM_ALLOW_LOCALNET:-0}" = 1 ] || die "$SOLEVM_RPC_URL is not devnet (genesis $genesis)"
      echo localnet
      ;;
  esac
}

state_dir() {
  local dir="$SOLEVM_HOME/state/$1"
  mkdir -p "$dir"
  chmod 700 "$SOLEVM_HOME/state" "$dir"
  printf '%s\n' "$dir"
}

sha256() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | cut -d ' ' -f 1
  else
    shasum -a 256 "$1" | cut -d ' ' -f 1
  fi
}
