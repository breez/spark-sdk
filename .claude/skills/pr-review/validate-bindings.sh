#!/bin/bash
# Validates that SDK binding files are updated consistently.
# Run this when reviewing PRs that change the SDK's public interface.
#
# Usage:
#   ./validate-bindings.sh              # Check staged/unstaged changes
#   ./validate-bindings.sh <base-ref>   # Check changes vs base (e.g., main, HEAD~1)

set -euo pipefail

# Binding files that must be updated together for API changes
# Source of truth: `.claude/docs/sdk-interfaces.md`
BINDING_FILES=(
  "crates/breez-sdk/core/src/models.rs"
  "crates/breez-sdk/wasm/src/models.rs"
  "crates/breez-sdk/wasm/src/sdk.rs"
  "packages/flutter/rust/src/models.rs"
  "packages/flutter/rust/src/sdk.rs"
)

BASE_REF="${1:-}"

# Get list of changed files
if [[ -n "$BASE_REF" ]]; then
  CHANGED_FILES=$(git diff --name-only "$BASE_REF" 2>/dev/null || echo "")
else
  # Check both staged and unstaged changes
  CHANGED_FILES=$(git diff --name-only HEAD 2>/dev/null || git diff --name-only 2>/dev/null || echo "")
fi

if [[ -z "$CHANGED_FILES" ]]; then
  echo "No changes detected."
  exit 0
fi

# Check which binding files were modified
MODIFIED_BINDINGS=()
UNMODIFIED_BINDINGS=()

for file in "${BINDING_FILES[@]}"; do
  if echo "$CHANGED_FILES" | grep -q "^${file}$"; then
    MODIFIED_BINDINGS+=("$file")
  else
    UNMODIFIED_BINDINGS+=("$file")
  fi
done

# If no binding files were modified, nothing to check
if [[ ${#MODIFIED_BINDINGS[@]} -eq 0 ]]; then
  echo "✅ No binding files modified - no consistency check needed."
  exit 0
fi

# If some but not all binding files were modified, warn
if [[ ${#UNMODIFIED_BINDINGS[@]} -gt 0 ]]; then
  echo "⚠️  Binding files partially updated!"
  echo ""
  echo "Modified:"
  for file in "${MODIFIED_BINDINGS[@]}"; do
    echo "  ✅ $file"
  done
  echo ""
  echo "Not modified (may need updates):"
  for file in "${UNMODIFIED_BINDINGS[@]}"; do
    echo "  ❌ $file"
  done
  echo ""
  echo "If this PR changes the SDK's public interface, ensure all binding files are updated."
  echo "See `.claude/docs/sdk-interfaces.md` for details."
  exit 1
fi

# All binding files were modified
echo "✅ All binding files updated consistently:"
for file in "${MODIFIED_BINDINGS[@]}"; do
  echo "  ✅ $file"
done
exit 0
