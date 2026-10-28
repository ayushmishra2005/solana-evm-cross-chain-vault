#!/usr/bin/env bash
# Runs the V2 deployment-authorization scenario in a fresh V2 namespace, then
# re-verifies every recorded transaction from the RPC at finalized commitment.
# Devnet needs SOLEVM_V2_DEVNET_APPROVED=1 as a deliberate second step.
# Usage: run.sh [LABEL]   (LABEL: 1-32 of a-z, 0-9, '-'; one namespace per label)
# shellcheck source=common.sh
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"
require cargo
require git
require "$SOLANA_KEYGEN"

export SOLEVM_RUN_LABEL="${1:-${SOLEVM_RUN_LABEL:-run-1}}"
cluster_name="$(cluster)"
require_devnet_approval "$cluster_name" "running the scenario"
refuse_cli_wallet
require_runtime
export SOLEVM_CLUSTER="$cluster_name"

# Name the exact sources, including uncommitted edits, without touching the
# git index or creating a commit.
cd "$REPO_ROOT"
scratch="$(mktemp -d)"
trap 'rm -rf "$scratch"' EXIT
GIT_INDEX_FILE="$scratch/index" git read-tree HEAD
GIT_INDEX_FILE="$scratch/index" git add -A
SOLEVM_SOURCE_TREE="$(GIT_INDEX_FILE="$scratch/index" git write-tree)"
SOLEVM_SOURCE_COMMIT="$(git rev-parse HEAD)"
SOLEVM_SOURCE_DIRTY=0
[ "$SOLEVM_SOURCE_TREE" = "$(git rev-parse 'HEAD^{tree}')" ] || SOLEVM_SOURCE_DIRTY=1
export SOLEVM_SOURCE_COMMIT SOLEVM_SOURCE_TREE SOLEVM_SOURCE_DIRTY

# An inherited RUSTFLAGS would replace the driver's `--cfg solevm_devnet`.
unset RUSTFLAGS CARGO_ENCODED_RUSTFLAGS
cd "$DRIVER_DIR"
cargo run --locked --release --quiet -- run-v2
cargo run --locked --release --quiet -- verify-v2 \
  "$SOLEVM_V2_HOME/state/$cluster_name/evidence-v2-$SOLEVM_RUN_LABEL.json"
