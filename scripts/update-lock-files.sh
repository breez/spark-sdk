#!/usr/bin/env bash
#
# Re-adapted from https://github.com/rust-bitcoin/corepc/blob/master/contrib/update-lock-files.sh
# Update lockfiles from separate workspaces at once

set -euo pipefail

REPO_DIR="$(git rev-parse --show-toplevel)"

CRATES=(. packages/flutter/rust crates/breez-sdk/lnurl docs/breez-sdk/snippets/rust)

for crate in "${CRATES[@]}"; do
    cargo update -w --manifest-path "$REPO_DIR/$crate/Cargo.toml"
done
