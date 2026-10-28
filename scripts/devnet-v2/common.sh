# Shared settings for the V2 devnet scripts. Sourced, never executed.
# V2 owns its keys, artifacts, buffers and state; nothing here reads or writes
# the V1 instance under scripts/devnet. No private key material is printed.
# shellcheck shell=bash disable=SC2034  # the sourcing scripts read these

set -euo pipefail
umask 077

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd -P)"
PROGRAM_DIR="$REPO_ROOT/programs/solevm-remote-leg"
VERIFIER_DIR="$REPO_ROOT/programs/authorization-verifier"
DRIVER_DIR="$PROGRAM_DIR/devnet"
DEPLOY_DIR="$PROGRAM_DIR/target/deploy-devnet-v2"
V1_DEPLOY_DIR="$PROGRAM_DIR/target/deploy-devnet"

SOLEVM_V2_HOME="${SOLEVM_V2_HOME:-$HOME/.config/solevm-devnet-v2}"
KEY_DIR="$SOLEVM_V2_HOME/keys"
export SOLEVM_V2_HOME

SOLANA="${SOLANA:-solana}"
SOLANA_KEYGEN="${SOLANA_KEYGEN:-solana-keygen}"
CARGO_BUILD_SBF="${CARGO_BUILD_SBF:-cargo-build-sbf}"
CARGO_BUILD_SBF_VERSION="4.4.0"
PLATFORM_TOOLS_VERSION="v1.58"

DEVNET_GENESIS="EtWTRABZaYq6iMfeYKouRu166VU2xqa1wcaWoxPkrZBG"
MAINNET_GENESIS="5eykt4UsFv8P8NJdTREpY1vzqKqZKvdpKuc147dw2N9d"
TESTNET_GENESIS="4uhcVJyU9pJkvQyS88uRDiswHXSCkY3zQawwpjk2NsNY"

# Every disposable V2 key, by file name without `.json`.
ROLES="deployer leg-program verifier-program administrator guardian transport-verifier asset-transport escrow-owner attacker risk-administrator"

# The ids the V2 build declares under `--cfg solevm_devnet`.
LEG_V2_ID="DcbREPYFa4WtjknjmT7ENqP7RV3KAfXBLcBbGqyq2jhD"
VERIFIER_V2_ID="4dAHz4oRNVmQ1ru9WkUthKBKU65Bjn3jQrQ5GDfQ9CSw"

# The V1 devnet test adapter, reused as is. V2 never deploys or upgrades it;
# it only checks that the program still runs this exact binary under the V1
# deployer, so its behaviour is the one V1 recorded.
ADAPTER_DEVNET_ID="8bFiHDwdAurdhTv5F9JaVD4QgdJ2XxEHRK2BAyiCyjZ8"
ADAPTER_DEVNET_SHA256="02a978e2e299cb089edcd8d24396e9b9579cf01d34d2b5ba58d54a3eadbd39fb"
ADAPTER_DEVNET_AUTHORITY="HiiQJU2B2KrC5ujyrD5EJbAsA5WHyL5PwPoKmiVtKytn"

# Programs V2 must never deploy to, upgrade or treat as its own.
V1_LEG_DEVNET_ID="27zReYgZvFTaUSWwob4qirN2Yxekcm32mnQDpMzeTyAE"
V1_VERIFIER_DEVNET_ID="da9yz64zbrJ4fyi8VTR3xfVZu8GhfhUHkC7AZQxCGG7"
LEG_PRODUCTION_ID="4sLaaRdiY74cvqVCXLjW2wPncy2ArQkTRauNTdphKByo"
ADAPTER_PRODUCTION_ID="GbFoY2KE6WAWHApg8na1Db8jtnR5cbaeCpA5TGA9UmcZ"
VERIFIER_PRODUCTION_ID="$V1_VERIFIER_DEVNET_ID"
FORBIDDEN_TARGETS="$V1_LEG_DEVNET_ID $V1_VERIFIER_DEVNET_ID $LEG_PRODUCTION_ID $ADAPTER_PRODUCTION_ID $ADAPTER_DEVNET_ID"

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

v2_home="$(resolve "$SOLEVM_V2_HOME")/"
case "$v2_home" in
  "$REPO_ROOT"/*) die "SOLEVM_V2_HOME ($SOLEVM_V2_HOME) must be outside the repository" ;;
  "$(resolve "$HOME/.config/solevm-devnet")"/*) die "SOLEVM_V2_HOME must not be inside the V1 home" ;;
  "$(resolve "$HOME/.config/solana")"/*) die "SOLEVM_V2_HOME must not be inside the Solana CLI config" ;;
esac
if [ -n "${SOLEVM_HOME:-}" ] && [ "$(resolve "$SOLEVM_HOME")/" = "$v2_home" ]; then
  die "SOLEVM_V2_HOME must differ from the V1 SOLEVM_HOME"
fi

forbidden() {
  local id="$1" target
  for target in $FORBIDDEN_TARGETS; do
    [ "$id" = "$target" ] && return 0
  done
  return 1
}

key() {
  local path="$KEY_DIR/$1.json"
  [ -f "$path" ] || die "missing $path; run scripts/devnet-v2/keygen.sh"
  printf '%s\n' "$path"
}

pubkey() {
  "$SOLANA_KEYGEN" pubkey "$(key "$1")"
}

# Refuses a V2 role that is the Solana CLI's default wallet.
refuse_cli_wallet() {
  local wallet="$HOME/.config/solana/id.json" role default
  [ -f "$wallet" ] || return 0
  default="$("$SOLANA_KEYGEN" pubkey "$wallet")"
  for role in $ROLES; do
    [ "$(pubkey "$role")" != "$default" ] || die "the $role key is the Solana CLI default wallet"
  done
}

# Checks the explicit endpoint against the cluster the caller named and
# prints that cluster. Nothing falls back to the CLI's configured URL.
cluster() {
  [ -n "${SOLEVM_RPC_URL:-}" ] || die "set SOLEVM_RPC_URL explicitly"
  local expected="${SOLEVM_CLUSTER:-}" genesis
  case "$expected" in
    devnet | localnet) ;;
    *) die "set SOLEVM_CLUSTER to devnet or localnet" ;;
  esac
  genesis="$("$SOLANA" genesis-hash --url "$SOLEVM_RPC_URL")" || die "cannot reach $SOLEVM_RPC_URL"
  case "$genesis" in
    "$MAINNET_GENESIS" | "$TESTNET_GENESIS") die "refusing $SOLEVM_RPC_URL: genesis $genesis is mainnet-beta or testnet" ;;
  esac
  case "$expected" in
    devnet)
      [ "$genesis" = "$DEVNET_GENESIS" ] || die "$SOLEVM_RPC_URL is not devnet (genesis $genesis)"
      ;;
    localnet)
      [ "$genesis" != "$DEVNET_GENESIS" ] || die "$SOLEVM_RPC_URL is devnet, not a local validator"
      case "$SOLEVM_RPC_URL" in
        http://127.0.0.1:* | http://localhost:*) ;;
        *) die "a localnet endpoint must be on 127.0.0.1 or localhost" ;;
      esac
      ;;
  esac
  echo "$expected"
}

# Devnet writes need a deliberate second step from the operator.
require_devnet_approval() {
  [ "$1" != devnet ] || [ "${SOLEVM_V2_DEVNET_APPROVED:-0}" = 1 ] ||
    die "$2 on devnet needs SOLEVM_V2_DEVNET_APPROVED=1"
}

# Reads one account from SOLEVM_RPC_URL at finalized commitment and answers a
# question about it. Exits non-zero on an RPC error, so no check ever reads an
# unreachable endpoint as an absent account.
#   probe feature ID         prints active, pending or inactive
#   probe buffer ADDRESS KEY prints absent, or ok if ADDRESS is a loader buffer
#                            whose authority is KEY; fails otherwise
probe() {
  python3 - "$SOLEVM_RPC_URL" "$@" <<'PYTHON'
import base64, json, sys, urllib.request
url, mode, address, *rest = sys.argv[1:]
ALPHABET = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz"
def b58(raw):
    value, out = int.from_bytes(raw, "big"), ""
    while value:
        value, digit = divmod(value, 58)
        out = ALPHABET[digit] + out
    return "1" * (len(raw) - len(raw.lstrip(b"\0"))) + out
body = json.dumps({"jsonrpc": "2.0", "id": 1, "method": "getAccountInfo",
                   "params": [address, {"encoding": "base64", "commitment": "finalized"}]}).encode()
request = urllib.request.Request(url, body, {"Content-Type": "application/json"})
reply = json.load(urllib.request.urlopen(request, timeout=30))
if "result" not in reply:
    sys.exit(f"RPC error reading {address}: {reply.get('error')}")
account = reply["result"]["value"]
data = base64.b64decode(account["data"][0]) if account else b""
if mode == "feature":
    if account is None:
        print("inactive")
    elif account["owner"] != "Feature111111111111111111111111111111111111":
        sys.exit(f"{address} is not a feature account")
    else:
        print("active" if data[:1] == b"\1" else "pending")
elif mode == "buffer":
    if account is None:
        print("absent")
    elif (account["owner"] != "BPFLoaderUpgradeab1e11111111111111111111111"
          or data[:4] != b"\1\0\0\0" or data[4:5] != b"\1"):
        sys.exit(f"{address} exists and is not an upgradeable-loader buffer with an authority")
    elif b58(data[5:37]) != rest[0]:
        sys.exit(f"buffer {address} belongs to {b58(data[5:37])}, not {rest[0]}")
    else:
        print("ok")
else:
    sys.exit(f"unknown probe {mode}")
PYTHON
}

# Gates the V2 programs depend on, read from the cluster itself, never assumed:
# the secp256r1 precompile, in-SVM precompile verification (so a bad signature
# lands as a failed, fee-paying transaction), and SBPFv0 execution, which the
# V2 builds and the reused adapter need.
SECP256R1_FEATURE="srremy31J5Y25FrAApwVb9kZcfXbusYMMsvTK9aWv5q"
PRECOMPILE_IN_SVM_FEATURE="9ypxGLzkMxi89eDerRKXWDXe44UY2z4hBig4mDhNq5Dp"
DISABLE_SBPF_V0_EXECUTION_FEATURE="TestFeature11111111111111111111111111111111"
DISABLE_SBPF_V0_DEPLOYMENT_FEATURE="B8JJXCy5amZyWG9r7EnUYLwzXSXTxG7GZ1qZ1qggo83g"

require_runtime() {
  local feature state
  require python3
  for feature in "$SECP256R1_FEATURE" "$PRECOMPILE_IN_SVM_FEATURE"; do
    state="$(probe feature "$feature")" || die "cannot read feature $feature"
    [ "$state" = active ] || die "feature $feature is $state on $SOLEVM_RPC_URL; V2 needs it active"
  done
  state="$(probe feature "$DISABLE_SBPF_V0_EXECUTION_FEATURE")" || die "cannot read feature $DISABLE_SBPF_V0_EXECUTION_FEATURE"
  [ "$state" = inactive ] || die "SBPFv0 execution is being disabled ($state); the V2 builds and the reused adapter are SBPFv0"
}

# Additionally refuses a cluster that will not accept an SBPFv0 deployment.
require_v0_deployment() {
  local state
  state="$(probe feature "$DISABLE_SBPF_V0_DEPLOYMENT_FEATURE")" || die "cannot read feature $DISABLE_SBPF_V0_DEPLOYMENT_FEATURE"
  [ "$state" = inactive ] ||
    die "SIMD-0500 is $state on $SOLEVM_RPC_URL: SBPFv0 deployments are refused; rebuild for SBPFv3 and rehearse again"
}

# The SBPF version an ELF declares in e_flags: 0 for SBPFv0, 3 for SBPFv3.
sbpf_version() {
  od -An -tu4 -j48 -N4 "$1" | tr -d ' '
}

state_dir() {
  local dir="$SOLEVM_V2_HOME/state/$1"
  mkdir -p "$dir"
  chmod 700 "$SOLEVM_V2_HOME/state" "$dir"
  printf '%s\n' "$dir"
}

sha256() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | cut -d ' ' -f 1
  else
    shasum -a 256 "$1" | cut -d ' ' -f 1
  fi
}

# Whether program $1 on the cluster runs exactly binary $2: the program data
# starts with it and the rest is zero padding.
runs_binary() {
  local id="$1" binary="$2" scratch result=1
  scratch="$(mktemp -d)"
  if "$SOLANA" program dump --url "$SOLEVM_RPC_URL" "$id" "$scratch/onchain.so" >/dev/null 2>&1; then
    head -c "$(wc -c <"$binary")" "$scratch/onchain.so" >"$scratch/prefix.so"
    tail -c "+$(($(wc -c <"$binary") + 1))" "$scratch/onchain.so" | tr -d '\0' >"$scratch/tail"
    if [ "$(sha256 "$scratch/prefix.so")" = "$(sha256 "$binary")" ] && [ ! -s "$scratch/tail" ]; then
      result=0
    fi
  fi
  rm -rf "$scratch"
  return "$result"
}
